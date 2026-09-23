# 构建与打包指南

## 前置条件

- Rust 工具链（`rustup`）
- Xcode + Command Line Tools
- 签名证书（Apple Development / Distribution）
- `create-dmg`（bundle 脚本制作 DMG 必需）：

  ```bash
  brew install create-dmg
  ```

- `cargo-about`（生成 `THIRD_PARTY_LICENSES.txt`，缺失会导致脚本中止）：

  ```bash
  cargo install cargo-about --features cli
  ```

  > 注意：必须带 `--features cli`，否则 cargo-about 0.9+ 默认不安装二进制。

- **CEF(Chromium)二进制分发**（只有 `--cef` 打包 / 跑 CEF 开发版才需要）：放在 `$CEF_PATH`，
  默认 `~/.local/share/cef`。要求见下：

  - 该目录必须**直接包含** `Chromium Embedded Framework.framework`（即"平铺"布局；若用版本化
    下载目录，要指向内层的 `<版本>/cef_macos_aarch64/`）。
  - 缺失时 `cargo` 构建会**自动下载**对应版本（实测压缩包 ~132MB，解开约 336MB）到
    `$CEF_PATH/<版本>/cef_macos_aarch64/`；此时需把 `CEF_PATH` 指向该内层目录，或把它上移成平铺。
  - 版本由 `app/Cargo.toml` 与 `tools/cef-helper/Cargo.toml` 的 `cef = "…"` 决定，**必须两处一致**；
    升级步骤、目录布局陷阱与回滚见 [specs/cef-webview-minimal/CEF-UPGRADE.md](../specs/cef-webview-minimal/CEF-UPGRADE.md)。

## 快速构建（debug 运行）

```bash
cargo run --bin zap-oss
```

**CEF(Chromium) 内核需要 `.app` 形态**：`cef_webview` feature 会链接 CEF，framework 必须位于
bundle 内；裸二进制（`target/debug/zap-oss`）启动时**加载失败并自动回退 wry**（日志里有一行
`[cef] failed to load Chromium Embedded Framework (需以 .app 形态运行)`）。要跑带 CEF 的 debug 版本：

```bash
script/macos/cef_smoke          # 构建 + 组装 .app + 嵌 framework/5 个 helper + 分层签名
script/macos/cef_smoke --run    # 再启动、校验 CEF 初始化成功、然后清理自启实例
# 产出:target/cef-smoke/ZapCEF.app
```

两点与开关有关：

- `cef_webview` **不在 default features** —— 不带该 feature 的构建里**根本没有 CEF 代码**（自动走 wry）。
- 带该 feature 的构建里 **CEF 后端默认启用**，不需要再设环境变量；`ZAP_CEF_WEBVIEW` 现在只是
  "显式强制"的旁路（用于绕过 settings 里用户偏好对 flag 的覆盖）。`ZAP_CEF_OSR=1|0` 可强制
  渲染模式（覆盖设置项 `general.webview.use_osr_rendering`）。

## 发布版打包

### 1. 清理旧缓存（可选）

```bash
cargo clean
```

清理编译缓存（实测本机约 149GB），随后构建时长约 6–8 分钟（增量约 3 分钟）。

#### 不必全清：用 sccache + 关增量编译

本仓库 `.cargo/config.toml` 已强制 `rustc-wrapper = "sccache"`。debug 默认开启的增量编译会让 sccache
对绝大多数 crate 判定为 `non-cacheable`（incremental 与 sccache 缓存单元冲突），导致 `target/`
无限膨胀、且 `cargo clean` 全清后必须全量重编。

> **关增量编译必须走环境变量，不能写在 `.cargo/config.toml` 的 `[env]` 段。**
> `CARGO_INCREMENTAL` 是 cargo 的特殊变量，`.cargo/config.toml` 的 `[env]` 设置对它无效——
> 写在 `[env]` 里会造成"已配置"的假象，但增量编译实际仍然开着、`target/debug/incremental` 持续累加。
> 已在 `~/.zshrc` 写入 `export CARGO_INCREMENTAL=0`（新开终端自动生效）。临时单步调试时
> 用 `CARGO_INCREMENTAL=1 cargo build ...` 覆盖即可。

- sccache 本地缓存上限 `10 GiB`（`Max cache size`），超出按 LRU 自动淘汰最旧的，**无需手动全清**。
- 回收磁盘空间优先删增量中间产物，而不是 `cargo clean` 全清：

  ```bash
  rm -rf target/debug/incremental   # 回收空间，不触发全量重编
  ```

- 日常构建直接复用 `target/` 里未改动 crate 的产物；确需清 `target/` 时，sccache 已缓存，
  重建会命中（实测清 `deps` 后二次构建比首次快约 1 分钟）。
- 需要临时单步调试（lldb / VS Code F5）时，用以下命令覆盖、临时开回增量编译：

  ```bash
  CARGO_INCREMENTAL=1 cargo build --bin zap-oss
  ```

### 2. 构建 DockTilePlugin

bundle 脚本会尝试嵌入 DockTilePlugin，若不存在则跳过（不影响 .app 运行）。

如需构建：

```bash
make -C app/DockTilePlugin clean
make -C app/DockTilePlugin
cp -R app/DockTilePlugin/ZapDockTilePlugin.docktileplugin target/release-lto/
```

### 3. 构建 .app bundle（一键完成）

```bash
./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64
```

`--selfsign` 自动搜索本地 Apple Development 证书签名，
同时自动编译自适应图标。**一步完成构建、签名、图标编译**，无需额外操作。

如需兼容 Intel Mac，去掉 `--nouniversal`。

#### 带 CEF(Chromium)内核的发布包

```bash
export CEF_PATH="$HOME/.local/share/cef"   # 必须先 export:见下面第 2 条
./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef
```

`--cef` 做的事：把 `cef_webview` 追加进 features → 构建 `tools/cef-helper`（**独立 workspace**，
与根 workspace 各自的 `Cargo.lock` 都要同步）→ 调 `script/macos/cef_embed` 把
`Chromium Embedded Framework.framework` + 5 个 helper.app 装进 bundle 并**分层签名**。

四条硬约束（都可在 `script/macos/bundle` 里溯源）：

1. **前置检查**：`$CEF_PATH/Chromium Embedded Framework.framework` 不存在时脚本直接 `exit 1`
   （提示"先准备 CEF 二进制"）。
2. **必须由调用方 `export CEF_PATH`**：脚本只在 Step 1.5 的 helper 构建里传 `CEF_PATH`
   （`bundle:782`），**主构建（`bundle:732`）不传**。
   **已实测（2026-09-23，`env -u CEF_PATH cargo check -p warp --features cef_webview`）**：不传时
   `cef-dll-sys` 会把 CEF **另下一份到 `target/debug/build/cef-dll-sys-*/out/cef_macos_aarch64/`**
   （额外 132MB 下载 + 339MB 磁盘），而 Step 1.5 的 `${CEF_PATH:-~/.local/share/cef}` 仍取
   `~/.local/share/cef` ⇒ **主二进制与嵌入的 framework 来自两份不同的副本**。构建**不会报错**
   （静默不同源）：两份版本一致时无可见影响;一旦版本分叉（典型:升级后 `~/.local/share/cef`
   还是旧的）就会出现 wrapper 与 framework 不匹配的风险。
3. **`--cef` 不支持交叉架构**：`DEFAULT_TARGET != HOST_TARGET` 时直接报错
   （"暂不支持交叉架构构建"）⇒ arm64 机器上要配 `--nouniversal --arch aarch64`。
4. **版本必须同步**（两个 `Cargo.toml` + 两个 `Cargo.lock`；helper 的 lock 不随根更新）——升级流程见
   [specs/cef-webview-minimal/CEF-UPGRADE.md](../specs/cef-webview-minimal/CEF-UPGRADE.md)。

产出的 .app 和 .dmg（最终产物在 `target/release-lto/bundle/osx/`）：

```
target/release-lto/bundle/osx/Zap.app
target/release-lto/bundle/osx/Zap.dmg
```

> 脚本内部构建目录为 `target/aarch64-apple-darwin/release-lto/bundle/osx/`
> （含签名完整的 .app），步骤 6 会把 .app 和 .dmg 拷贝到上面的最终位置。

### 4. 验证

```bash
# 检查签名（正常应输出 Signature size=xxxx，不是 adhoc）
codesign -dv target/release-lto/bundle/osx/Zap.app

# 检查自适应图标
ls -l target/release-lto/bundle/osx/Zap.app/Contents/Resources/Assets.car

# 校验签名完整
codesign --verify --deep --strict target/release-lto/bundle/osx/Zap.app
```

带 `--cef` 的包额外核对（`--verify --deep --strict` 已覆盖 framework 与 helper，这里确认版本与数量）：

```bash
APP=target/release-lto/bundle/osx/Zap.app
# framework 版本应与 Cargo.lock 里 cef 的 "+" 后版本一致
plutil -p "$APP/Contents/Frameworks/Chromium Embedded Framework.framework/Resources/Info.plist" | grep -i shortversion
# 应为 5 个:zap-oss Helper{,(GPU),(Renderer),(Plugin),(Alerts)}.app
ls "$APP/Contents/Frameworks/" | grep -c Helper
```

## 环境变量说明

OMP 模型列表（`omp models --json`）只显示有 API key 的 provider。
Release 构建从 Finder 启动时没有终端的环境变量，
`OmpModelRegistry` 会在后台通过 `$SHELL -l -i -c printenv` 获取用户环境。

常见 API Key（如 `DEEPSEEK_API_KEY`、`ANTHROPIC_API_KEY`）会自动传入 `omp` 子进程，
无需额外配置。

预览可用模型：

```bash
omp models --json | python3 -m json.tool
```

## 常见问题

### 签名回退到 ad-hoc

`--selfsign` 依赖 `security find-identity | grep "Apple Development"` 查找证书。
若没找到或选错，会回退到 ad-hoc 签名。

手动用证书哈希签名：

```bash
# 先用 security find-identity 找到证书哈希
security find-identity -p codesigning -v

# 然后手动签名
codesign --force --deep --options runtime \
  --sign "证书哈希" \
  --entitlements script/Debug-Entitlements.plist \
  target/aarch64-apple-darwin/release-lto/bundle/osx/Zap.app
```

### DockTilePlugin 找不到

`cp: target/release-lto/ZapDockTilePlugin.docktileplugin: No such file or directory`

不影响 app 运行，只是 Dock 上的进度条等插件功能不可用。
执行 `cargo clean` 后需重新构建（见步骤 2）。

### 图标不跟随系统主题切换

检查 `Contents/Resources/Assets.car` 是否存在。若缺失，运行：

```bash
script/compile_icon oss target/release-lto/bundle/osx/Zap.app
```

图标源文件位于 `app/channels/oss/icon/AppIcon.icon/`，若该目录不存在 `compile_icon` 会静默跳过。

### OMP 模型列表为空

可能原因：

1. **OMP 二进制没找到** — 从 Finder 启动时 PATH 不含 `/opt/homebrew/bin`。
   代码已内置 `resolve_binary()` 自动搜索常见 Homebrew 路径。

2. **API Key 环境变量缺失** — GUI app 不继承 shell 的 `DEEPSEEK_API_KEY` 等。
   代码已内置 `get_user_env()` 从 login shell 获取环境变量。

3. **debug 构建功能正常但 release 没有** — 检查 feature flag 是否被 `cfg!(debug_assertions)` 误包裹。
   OmpModelSelector 此前就是这个原因。

### 第三方许可证生成失败

```text
error: no such command: `about`
```

需安装 `cargo-about`（**必须带 `--features cli`**）：

```bash
cargo install cargo-about --features cli
```

注意：许可证生成失败不会只跳过该步骤——`prepare_bundled_resources` 内的
`set -e` 会让整个 bundle 脚本**提前中止**，后续的图标编译、签名、DMG 制作
全部被跳过（脚本 exit 101）。安装 cargo-about 后重跑即可。

### create-dmg 找不到

```text
./script/macos/bundle: line 867: create-dmg: command not found
```

`--selfsign` 分支用 `create-dmg` 制作带背景图与 Applications 拖放链接的 DMG，
缺失时脚本 exit 127。安装：

```bash
brew install create-dmg
```
### `--skip-build` 的问题

`--skip-build` 会导致 bundle 脚本跳过 `compile_icon`，产出的 .app 没有自适应图标。
还会导致路径错位（.app 跑到 `dmg/Zap.app` 而非 `osx/Zap.app`）。
除非调试打包流程，否则不要使用。

### 手动补图标后 DMG 需要重建

如果手动运行 `compile_icon` 和 `codesign` 补图标，DMG 是在脚本中早先生成的，
内容仍是旧版 .app。需要重新打包：

```bash
BUNDLE=target/release-lto/bundle
rm -f "$BUNDLE/osx/Zap.dmg"
hdiutil create -volname "Zap" -srcfolder "$BUNDLE/osx/Zap.app" -ov -format UDZO "$BUNDLE/osx/Zap.dmg"
```

### CEF 相关（`--cef` 打包 / `cef_smoke`）

**`Error: 未找到 CEF framework($CEF_DIR)`**
→ `export CEF_PATH=<直接含 Chromium Embedded Framework.framework 的那一层>`，或按前置条件准备 CEF 二进制。

**`Error: --cef 暂不支持交叉架构构建`**
→ 去掉 universal：`--nouniversal --arch aarch64`（在 arm64 机器上构建）。

**升级 CEF 后仍然加载旧内核 / bundle 里 Info.plist 版本没变**
→ 典型陷阱：`$CEF_PATH` 放的是**旧的平铺目录**，而 `cef-dll-sys` 的 `check_archive_json` 只在
`archive > expected` 时报错（`archive <= expected` 视为通过）⇒ 升级 crate 版本后它仍用旧目录、
**不下载也不告警**。按 [CEF-UPGRADE.md](../specs/cef-webview-minimal/CEF-UPGRADE.md) §3.4 先把旧目录挪走再构建。

**启动即崩 / 日志出现 `Check failed: api_hash`**
→ wrapper 与 framework 版本不一致（两个 `Cargo.toml` / 两个 lock 没同步，或 bundle 里嵌的是旧 framework）。
CEF 的 `libcef_dll/wrapper/libcef_dll_wrapper.cc` 里有
`CHECK(!strcmp(cef_api_hash(CEF_API_VERSION, 0), CEF_API_HASH_PLATFORM))`，不匹配直接 abort。
改齐版本后重跑 `script/macos/cef_smoke`（或 `bundle --cef`）重嵌。

**`cargo run --bin zap-oss` 里 CEF 没生效**
→ 预期行为：裸二进制不是 `.app`，framework 加载不到，自动回退 wry（见「快速构建」）。用 `script/macos/cef_smoke`。

**oss 渠道 `--cef` 后设置里没有 CEF 项、内核还是系统的**
→ 症状：`render mode = ` 一类的 `[cef]` 特征串在二进制里搜不到（`strings` 计数 0），但 framework
照嵌、签名全绿。根因曾是 `bundle` 的 oss 分支用**覆盖赋值**拼 `FEATURES`，把 `--cef` 追加的
`cef_webview` 丢掉（2026-09-23 已修：oss 分支在 `CEF=true` 时补回）。快速判据：
`strings <app>/Contents/MacOS/zap-oss | grep -c "render mode = "` —— 0 = 没编进 CEF。

**dsh pane 打开即提示"已崩溃"，日志刷 `TS_PROCESS_CRASHED code=5`**
→ renderer 子进程在 hardened runtime 下起 V8 即 `EXC_BREAKPOINT/SIGTRAP`（崩溃报告
`faultingThread: CrRendererMain`）。根因：Step 3 的 `--deep` 整包重签会把 Step 1.5 给 helper
做的分层签名（含 JIT entitlements）覆盖成主包 entitlements —— 主包没有
`allow-jit`/`allow-unsigned-executable-memory`（2026-09-23 已修：Step 3.5 按
`script/macos/cef-helper-entitlements.plist` 重签 5 个 helper）。判据：
`codesign -d --entitlements - "<app>/Contents/Frameworks/zap-oss Helper (Renderer).app" | grep allow-jit`。
参考：CEF 官方论坛 [macOS] Renderer Process Crash(SIGTRAP)（t=20345）。
