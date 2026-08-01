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

## 快速构建（debug 运行）

```bash
cargo run --bin zap-oss
```

## 发布版打包

### 1. 清理旧缓存（可选）

```bash
cargo clean
```

清理编译缓存（实测本机约 149GB），随后构建时长约 6–8 分钟（增量约 3 分钟）。

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
