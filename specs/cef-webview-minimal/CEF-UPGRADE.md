# CEF 升级手册(cef-webview-minimal)

> 2026-09-23。范围:把 zap 的 CEF(Chromium)后端从一个钉死的版本升到新版本,给出
> **可执行的步骤 + 每步判据 + 回滚路径 + 升级后回归清单**。
>
> **✅ 本手册已于 2026-09-23 实跑一次并完成升级:`152.0.8` → `154.0.23`
> (cef-rs `152.4.0` → `154.0.0`)。实际执行记录(含逐步证据与踩到的坑)见 §8;
> 下表 §0 已更新为**升级后**的基线。**
>
> **核查方式(初版)**:只读打开本仓源码、`~/.cargo/registry` 里的依赖源码、`~/.local/share/cef`,
> 加三条只读命令(crates.io / CEF CDN 版本查询、`codesign --verify`、`grep`/`ls`)。
> **§8 的升级执行是真正跑了构建与打包的**(判据见该节)。
>
> 标注约定(对齐 `AGENTS.md` §5.6.1):
> - 有 `文件:行` 或原始命令输出的 → 直接给依据。
> - 结论能从源码逐行读出、但**没有实跑验证**的 → 标 **「源码可溯·未实跑」**。
> - 只有推断、没有可复核依据的 → 标 **「未验证推测」**。

---

## 0 版本基线(升级后,2026-09-23 实测)

| 项 | 实测值 | 依据 |
|----|--------|------|
| cef-rs crate 版本(主二进制) | `154.0.0` | [app/Cargo.toml:258](../../app/Cargo.toml#L258) |
| lock 解析结果 | `cef` / `cef-dll-sys` 均 `154.0.0+154.0.23` | `Cargo.lock`(2026-09-23 升级后) |
| cef-rs crate 版本(CEF helper) | `154.0.0` | `tools/cef-helper/Cargo.toml:17` |
| helper 自己的 lock | `cef` `154.0.0+154.0.23` | `tools/cef-helper/Cargo.lock`(同上) |
| 目标 CEF 二进制 | `154.0.23+g062ebe4+chromium-154.0.8037.17` | `~/.local/share/cef/include/cef_version.h:38` |
| `archive.json` | `type=minimal`、`name=cef_binary_154.0.23+g062ebe4+chromium-154.0.8037.17_macosarm64_minimal.tar.bz2`、`sha1=9fdef241a8c682d98743c9fc6b27f5e54045551e` | `cat ~/.local/share/cef/archive.json` |
| CEF 目录布局 | **平铺**:`archive.json`/framework/`include`/`libcef_dll`/`cmake`/`CMakeLists.txt`/`CREDITS.html` 直接在 CEF 目录根(下载时落成 `<版本>/cef_macos_aarch64/`,已按 §2.2 拍平) | `ls -la ~/.local/share/cef/` |
| framework | 317 MB(152 时实测),内含 `Resources/`(含 `*.lproj/locale.pak`) | `du -sh`、`ls …/Resources` |
| helper 可执行文件 | `tools/cef-helper/target/release/zap_cef_helper` = **471808 字节** | `ls -l`(TECH.md:72 记的是 `0.43MB`,与本次实测数值略有差,可能来自不同次构建) |
| 盘上头文件的 API 版本 | `#define CEF_API_VERSION_LAST CEF_API_VERSION_15400` | `~/.local/share/cef/include/cef_api_versions.h` |
| crate bindings 的 API 版本 | `pub const CEF_API_VERSION_LAST: i32 = 15400;` | `cef-dll-sys-154.0.0+154.0.23/src/bindings/aarch64_apple_darwin.rs:34` |
| 旧版本备份 | `~/.local/share/cef-152.0.8-flat/`(343 MB,拍平布局,可作回滚源) | `ls -d ~/.local/share/cef*` |

**下一次升级的候选(2026-09-23 查得,三条独立来源)**:

| 来源 | 结果 |
|------|------|
| crates.io `cef` | 本次已取到最新 `154.0.0+154.0.23`(2026-09-23 发布);**`152.4.0` 与它之间没有任何 `153.x`** ⇒ 下一次若出现 `155.x` 仍是跨版本跳 |
| CEF CDN `index.json` | `macosarm64` 下 `154.0.23+g062ebe4+chromium-154.0.8037.17` 存在 `type=minimal`:`cef_binary_154.0.23+g062ebe4+chromium-154.0.8037.17_macosarm64_minimal.tar.bz2` |
| cef-rs git 仓库 | tag `export-cef-dir-v154.0.0+154.0.23` 存在 |

⇒ **本次跳版本不需要改走 git**(crates.io 已有 `154.0.0`,且 CDN 有配套 minimal 包)。
`git` 只在"crates.io 上没有对应 crate 版本、而 CDN 已有该 CEF 二进制"时才需要。

---

## 1 一句话结论

**升级时机自主,版本链必须同步。**

不是"换文件式"独立升级:CEF 二进制(framework + `libcef_dll` 的 C++ wrapper)、
cef-rs crate(含预生成 bindings)、我们的 5 个 helper.app 三者必须来自**同一个** CEF 版本,
缺一环就是编译期 `CHECK` abort 或运行期静默错版(§2)。

好消息是**不依赖系统 WebKit**:CEF 二进制由 `CEF_PATH` 供,`cef-dll-sys` 的 `build.rs` 会在缺失时
自动从 CDN 下载到版本化子目录(`CEF_PATH/<CEF 二进制版本>/cef_macos_aarch64/`),旧目录保留 ⇒
天然可并存、可回滚(§4)。

---

## 2 版本锁点表

| # | 位置 | 现在写什么 | 升级要改成什么 | 为什么 |
|---|------|-----------|---------------|--------|
| 1 | `app/Cargo.toml:258` | `cef = { version = "152.4.0", optional = true }` | 目标 cef-rs 版本(如 `"154.0.0"`) | 主二进制(crate `warp`)的依赖声明,唯一的**输入** |
| 2 | `Cargo.lock:2715`、`:2735` | `152.4.0+152.0.8` | 由 `cargo update` 写 | 唯一决定"编哪个 CEF 二进制"的是 `+` 后的 build 元数据(`+152.0.8`);`152.4.0` 只是 cef-rs 版本 |
| 3 | `tools/cef-helper/Cargo.toml:17` | `cef = "152.4.0"` | 同上 | helper 是**独立 workspace**(`[workspace]`,注释在 `Cargo.toml:8-11`),版本**不跟随**根 |
| 4 | `tools/cef-helper/Cargo.lock:165`、`:185` | `152.4.0+152.0.8` | 由 helper 自己的 `cargo update` 写 | 它有独立 lock,根 lock 的更新不会带动它 |
| 5 | CEF 二进制目录(`CEF_PATH`,默认 `~/.local/share/cef`) | 平铺的 152.0.8 | 目标版本的目录 | `build.rs` 从它取 `include/`(定 `CEF_API_VERSION`)与 `libcef_dll/`(编 wrapper);`cef_embed` 从它嵌 framework 与资源 |

第 2、4 项是**产物**,不要手改;第 1、3 项是**输入**,第 5 项是**外部物料**。

### 2.1 两个不在"三处"里、但会让升级悄悄错的约束

**(a) 版本字符串是 caret 需求,`cargo update` 单靠它升不上去。**
`"152.4.0"` 是 `^152.4.0`(允许 `>=152.4.0, <153.0.0`)。所以必须先改第 1、3 项的版本字符串,
再 `cargo update`;顺序反了会得到"`cargo update` 跑成功但版本没动"。
(Cargo 的 caret 语义,**未在本仓实跑**。)

**(b) `script/macos/bundle` 不会替你设 `CEF_PATH`。** **✅ 已实测(2026-09-23)**
`script/macos/bundle:521` 只是本地变量 `CEF_DIR="${CEF_PATH:-$HOME/.local/share/cef}"`,全脚本无 `export CEF_PATH`。
Step 1 的主构建(`bundle:732`)命令行里**不带** `CEF_PATH`;只有 Step 1.5 的 helper 构建(`bundle:782`)显式
`CEF_PATH="$CEF_DIR" cargo build …`。

实测方式:`env -u CEF_PATH cargo check -p warp --features cef_webview`(模拟调用方忘记 export),然后读
`target/debug/build/cef-dll-sys-*/output` 的 `cargo::metadata=CEF_DIR=…`:

| | 解析结果 |
|---|---|
| 主构建(CEF_PATH 未 export) | `target/debug/build/cef-dll-sys-<hash>/out/cef_macos_aarch64`(**新下一份**:132MB 下载 + 339MB 磁盘) |
| Step 1.5(`${CEF_PATH:-~/.local/share/cef}`) | `~/.local/share/cef` |

⇒ **两边确实不是同一份**(构建**不会报错**,是静默分叉);本次两份都是 154.0.23(sha1 相同)所以无可见影响,
但版本一旦分叉(典型:升级后 `~/.local/share/cef` 还是旧的)就会出现 wrapper 与 framework 不匹配的风险。
**命令里必须显式带 `CEF_PATH=`(并 `export`)。**

### 2.2 "取哪一层目录"的统一口径

`CEF_PATH` 必须指向**直接含 `Chromium Embedded Framework.framework` 与 `archive.json` 的那一层**:

- 平铺布局时 = `~/.local/share/cef`(现状);
- 版本化布局时 = `~/.local/share/cef/<CEF 二进制版本>/cef_macos_aarch64/`。

依据:`build.rs:25-26` 在 `location` 下再拼 `cef_macos_aarch64`(`download-cef` 的 `Display for OsAndArch`
= `cef_{os}_{arch}`,见 `download-cef-3.0.0/src/lib.rs:626-631`);而 `cef_smoke:15,23` 与 `cef_embed:25,45`
都要求"给定目录下直接有 framework"。两边口径一致,才能让 `check_archive_json` 直接命中。

---

## 3 升级步骤(可执行清单)

下面以跳 `154.0.0+154.0.23` 为例。**每步都给了判据;判据不过就不要进下一步。**

### 3.1 选目标版本(先确认上游三边齐备)

```bash
python3 - <<'PY'
import json, urllib.request
def get(u):
    r = urllib.request.Request(u, headers={'User-Agent': 'zap-cef-upgrade/1.0'})
    return json.load(urllib.request.urlopen(r, timeout=30))
cr = get('https://crates.io/api/v1/crates/cef/versions')
print('crates.io cef 最新 5 个:', [v['num'] for v in cr['versions'][:5]])
idx = get('https://cef-builds.spotifycdn.com/index.json')
for v in idx['macosarm64']['versions']:
    if v['cef_version'].startswith('154.0.23'):
        print('CDN minimal:', [f['name'] for f in v['files'] if f['type'] == 'minimal'])
        print('CDN sha1   :', [f['sha1'] for f in v['files'] if f['type'] == 'minimal'])
        break
PY
```

**判据**:crates.io 上某个版本的 `A+B`(`+` 之后是 CEF 二进制版本 `B`),
在 CDN `macosarm64` 的 `type == "minimal"` 列表里能找到同名文件。
`download-cef` **只取 minimal**(`download-cef-3.0.0/src/lib.rs:255`、`:380-384`),没有 minimal 就等于没有。

**不满足时**:该 CEF 二进制版本还没有 crate → 依赖改走
`cef = { git = "https://github.com/tauri-apps/cef-rs", tag = "export-cef-dir-v<A>+<B>" }`
(cef-rs 每个版本都有同名 tag,本次核对到 `export-cef-dir-v154.0.0+154.0.23`);
若 CDN 也没有 minimal,则只能退到上一个有 minimal 的版本。
**改走 git 会同时改第 1、3 项的依赖形态,`Cargo.lock` 的 `source` 字段会从 registry 变成 git —— 这属于另一套回滚路径,本手册不展开。**

### 3.2 同步两处版本字符串(输入)

```bash
sed -i '' 's|^cef = { version = "152\.4\.0"|cef = { version = "154.0.0"|' app/Cargo.toml
sed -i '' 's|^cef = "152\.4\.0"$|cef = "154.0.0"|'              tools/cef-helper/Cargo.toml
git diff --numstat app/Cargo.toml tools/cef-helper/Cargo.toml
```

**判据**:`git diff --numstat` 每个文件恰好 `1 1`(一行改一行),没有别的行被动到;
`grep -n '^cef' app/Cargo.toml tools/cef-helper/Cargo.toml` 显示新版本。

### 3.3 两个 workspace 各自更新 lock(产物)

```bash
cargo update -p cef -p cef-dll-sys
cargo update --manifest-path tools/cef-helper/Cargo.toml -p cef -p cef-dll-sys
grep -A1 'name = "cef' Cargo.lock tools/cef-helper/Cargo.lock
```

**判据**:
- 两处 lock 的 `cef` 与 `cef-dll-sys` 版本都变成 `<目标 cef-rs>+<目标 CEF 二进制>`;
- helper lock 的更新**不能**靠根 workspace 带动 —— 只跑第一条时 `tools/cef-helper/Cargo.lock` 仍是
  `152.4.0+152.0.8`,这正是第 3 项锁点的意义。

**不过时**:仍停在旧版本 ⇒ 3.2 的字符串没改对(见 §2.1(a));报 `no matching package` ⇒ 3.1 的目标版本在
crates.io 不存在,回到 3.1 决定是否走 git。

### 3.4 升级前处理"平铺旧目录"(最容易踩的一步)

**这一条是升级期最危险的行为,已实测(2026-09-23)。**

- `build.rs:87-105`:若 `CEF_PATH/<cef_version>` 不存在,就 `check_archive_json(CEF_PATH)`;通过则
  **直接返回平铺目录、不下载**。
- `download-cef-3.0.0/src/lib.rs:102-123`:该检查只在 `archive > expected` 时报错 ——
  `if archive <= expected { Ok(()) }`。

⇒ 平铺目录里放的是**更旧**版本的 `archive.json`(152.0.8)而期望是 154.0.23 时,检查**通过**,
既不下载也不告警,构建继续用旧头文件与旧 `libcef_dll`。
(即:"升级后不需要手工准备二进制、会自动落成版本化子目录"**只在平铺目录不存在时**成立;
平铺旧目录存在时结论相反 —— 它会静默钉住旧版本。)

#### 实测记录(2026-09-23:crate/锁定 154.0.23,把 `CEF_PATH` 指到 152.0.8 的旧平铺目录)

| 阶段 | 实测结果 |
|---|---|
| `cargo check -p warp --features cef_webview` | **成功、0 warning、无下载、无告警**;`target/debug/build/cef-dll-sys-*/output` 里 `CEF_DIR=…/cef-152.0.8-flat`、**`CEF_API_VERSION=15200`**(期望 15400)⇒ 静默用了旧 CEF |
| `script/macos/cef_smoke`(打包) | 同样**成功**;bundle 内 `CFBundleShortVersionString` = **152.0.8.0** |
| **启动** | **立即崩溃**:stderr 只有一行<br>`[0923/154403.776588:ERROR:cef/libcef_dll/libcef_dll2.cc:105] Request for unsupported CEF API version 15400`<br>随后 `Trace/BPT trap: 5`(崩溃报告 `EXC_BREAKPOINT/SIGTRAP`),栈顶:<br>`cef_command_line_create` ← `cef::args::Args::as_cmd_line` ← `cef_backend::handle_subprocess_or_continue` ← `maybe_run_as_cef_subprocess` |

⇒ **构建/打包全静默,代价在启动时一次性暴露**:框架侧报"unsupported CEF API version 15400"(Rust bindings 是
154,面前这份 libcef 是 152)并直接 trap。**若不 launch 就发现不了**(CI 也不会拦,见 §6)。

**处理(升级前二选一)**:

(a) 把平铺内容挪成版本化目录(推荐,新旧天然并存、可并存构建):

```bash
cd "$HOME/.local/share/cef"
mkdir -p 152.0.8/cef_macos_aarch64
mv archive.json "Chromium Embedded Framework.framework" cmake CMakeLists.txt \
   CREDITS.html include libcef_dll 152.0.8/cef_macos_aarch64/
ls -d "$HOME/.local/share/cef"/*
```

**判据**:`ls` 只剩 `152.0.8/`;`cat 152.0.8/cef_macos_aarch64/archive.json` 仍是 152.0.8 的内容。
挪完旧版本**仍可构建**:`build.rs:57-66` 会走 `resolve_from_versioned`(`CEF_PATH/152.0.8/cef_macos_aarch64` 存在),
`check_archive_json` 判定 `152.0.8 <= 152.0.8` 通过。
**为什么这样就绕开了上面的坑**:`check_archive_json` 是去读 `<location>/archive.json`
(`download-cef-3.0.0/src/lib.rs:110`、`:125-131`),挪走后根目录下**没有**这个文件 ⇒ 升级构建时
`check_archive(root)` 必然失败 ⇒ 直接走 `download_to_versioned`(`build.rs:98-104`),不会误用旧目录。

(b) 直接把平铺目录整体改名备份,让 `build.rs` 走自动下载:

```bash
mv "$HOME/.local/share/cef" "$HOME/.local/share/cef.152.0.8.bak"
```

**判据**:构建日志出现 `CEF_PATH does not exist, downloading archive to:`(`build.rs:106-108`)。
**不要先 `mkdir` 目标目录**:下载器自己会 `create_dir_all`(`download-cef-3.0.0/src/lib.rs:258`);
若你手动建了空目录,走的是 `check_archive` 失败分支,打印的是 `CEF_PATH is invalid (…)`(`build.rs:98-104`)。
下载的 archive 会按 CDN 的 `sha1` 校验,不匹配则删掉重下(`同文件 :262-285`)。

### 3.5 构建,并确认"到底取了哪一份 CEF 二进制"

```bash
export CEF_PATH="$HOME/.local/share/cef"          # 3.4(a) 之后这里是"根",只有版本子目录
cargo build -p warp --features cef_webview --bin zap-oss -vv 2>&1 | tee /tmp/cef-build.log
grep -nE 'Using (versioned )?CEF path|downloading archive to|Using downloaded CEF path|CEF_PATH (is invalid|does not exist)' /tmp/cef-build.log
```

**判据(逐条)**:

1. 必须出现下面三者之一:
   - `Using versioned CEF path from environment: …/<版本>/cef_macos_aarch64`(`build.rs:60-63`),或
   - `downloading archive to: …/<版本>` + `Using downloaded CEF path: …`(`build.rs:68-76`)。
2. **不得**出现 `Using CEF path from environment: …`(`build.rs:94-97`)——那是"直接用了平铺目录"。
   一旦出现,立刻查 `cat "$CEF_PATH/archive.json"`:其 `name` 里的版本必须**等于**目标 CEF 二进制版本;
   不等就是 3.4 没做干净,回到 3.4。
3. 新版本的目录与 `archive.json` 就位:
   ```bash
   cat "$CEF_PATH/<目标 CEF 二进制版本>/cef_macos_aarch64/archive.json"
   ```
   `type` 必须是 `minimal`,`name` 必须含目标版本,`sha1` 必须与 3.1 打印的 CDN `sha1` **逐字相同**。
4. 编译期 API 版本与盘上头文件一致(两者都必须随版本一起变):
   ```bash
   grep -n 'CEF_API_VERSION_LAST' "$CEF_PATH/<版本>/cef_macos_aarch64/include/cef_api_versions.h"
   grep -n 'CEF_API_VERSION_LAST' ~/.cargo/registry/src/*/cef-dll-sys-<新版本>/src/bindings/aarch64_apple_darwin.rs
   ```
   两者数值必须相等(升级前:头部 `15200`,`bindings` 也是 `15200`)。

**这一步为什么关键(版本链的强制点)**:`build.rs:138` 用 `cef_api_version_last(<CEF 目录>)` 读**盘上头文件**的
`CEF_API_VERSION_LAST`,并以 `CEF_COMPILER_DEFINES=-DCEF_API_VERSION=<值>` 编 `libcef_dll_wrapper`
(`build.rs:141-152`);C++ wrapper 在每次 `CefExecuteProcess`/`CefInitialize` 入口做
`CHECK(!strcmp(cef_api_hash(CEF_API_VERSION, 0), CEF_API_HASH_PLATFORM))`
(`~/.local/share/cef/libcef_dll/wrapper/libcef_dll_wrapper.cc:65-66`、`:84-85`)——
**不匹配就是进程 abort**,错误文本 `API hashes for libcef and libcef_dll_wrapper do not match.`。

> **纠正一处常见误解**:`app/src/browser/cef_backend.rs:1329` 的
> `let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);` **不是**运行期校验。
> `api_hash` 的返回类型是 `*const c_char`(hash **字符串**,签名见
> `~/.local/share/cef/include/cef_api_hash.h` 与
> `cef-152.4.0+152.0.8/src/bindings/aarch64_apple_darwin.rs:56189`),
> 返回值被 `let _ =` 丢弃;C 头文件的注释写明它只是"配置 API 版本"(`version` 参数在第一次调用后即固定,
> 之后传入的值被忽略)。也就是说这一行的作用**不构成**版本门禁。
> `sys::CEF_API_VERSION_LAST` 来自 crate 里**预生成**的 bindings(`…/bindings/aarch64_apple_darwin.rs:33`),
> 不来自磁盘头文件。真正会拦人的是上面那个 C++ `CHECK`。
> (`tools/cef-helper/src/main.rs:75` 同样是 `let _ = api_hash(...)`。)

### 3.6 冒烟打包(先 `cef_smoke`,不动正式产物)

```bash
export CEF_INNER="$CEF_PATH/<目标 CEF 二进制版本>/cef_macos_aarch64"
CEF_PATH="$CEF_INNER" script/macos/cef_smoke
```

`cef_smoke` 做的是:构建 `--features cef_webview` → 组装 `.app` → release 构建 `tools/cef-helper`
→ 调 `cef_embed` 嵌 framework + 5 个 helper + 分层签名 → `codesign --deep -s -` → `--verify --deep --strict`
(`script/macos/cef_smoke:15,29,31-57,58-59`)。注意它把 `CEF_PATH` 当"含 framework 的目录"用(`:15,23`),
故这里传的是 `CEF_INNER`(§2.2)。

**判据(逐字对照)**:脚本自己会打印,必须全为 `OK`:

```
framework: OK
zap-oss Helper.app: OK
zap-oss Helper (GPU).app: OK
zap-oss Helper (Renderer).app: OK
zap-oss Helper (Plugin).app: OK
zap-oss Helper (Alerts).app: OK
签名校验:OK
```

**基线对照(本次核查在现有 152.0.8 产物上跑的只读命令,供逐字比对)**:

```
$ cd target/cef-smoke/ZapCEF.app/Contents/Frameworks
$ for h in "zap-oss Helper.app" "zap-oss Helper (GPU).app" "zap-oss Helper (Renderer).app" \
           "zap-oss Helper (Plugin).app" "zap-oss Helper (Alerts).app"; do
    printf '%-32s executable=%-24s ' "$h" "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$h/Contents/Info.plist")"
    codesign --verify --deep --strict "$h" >/dev/null 2>&1 && echo "verify=OK" || echo "verify=FAIL"
  done
zap-oss Helper.app               executable=zap-oss Helper            verify=OK
zap-oss Helper (GPU).app         executable=zap-oss Helper (GPU)      verify=OK
zap-oss Helper (Renderer).app    executable=zap-oss Helper (Renderer) verify=OK
zap-oss Helper (Plugin).app      executable=zap-oss Helper (Plugin)   verify=OK
zap-oss Helper (Alerts).app      executable=zap-oss Helper (Alerts)   verify=OK

$ codesign --verify --deep --strict "Chromium Embedded Framework.framework"   # → 退出码 0
$ codesign --verify --deep --strict target/cef-smoke/ZapCEF.app              # → 退出码 0
$ codesign -dv target/cef-smoke/ZapCEF.app 2>&1 | grep -E 'Signature|Identifier'
Identifier=dev.zap.cef-smoke
Signature=adhoc
```

**helper 命名是硬要求,不是观测项**:helper.app 内可执行文件必须重命名为 `<主可执行名> <Helper>` 且
`CFBundleExecutable` 与之一致(`script/macos/cef_embed:83,91`);保留原名(如 `zap_cef_helper`)会让
GPU/Renderer 启动失败(`gpu_process_host … error_code=1003`),二分实验见
`evidence/phase1/BUNDLE-WIRING.md:37`。上面的 `executable=` 列就是这条的判据。

### 3.7 实跑冒烟(`--run`,验证 CEF 真的初始化)

```bash
CEF_PATH="$CEF_INNER" script/macos/cef_smoke --run
```

**判据**(脚本自带断言,`script/macos/cef_smoke:96-101`):本轮 `~/Library/Logs/zap.log` 的 mtime
**增长**(防"上一轮成功过"的假绿)且含 `[cef] message pump active` 或
`[cef] 由设置项触发,已初始化 CEF 后端` ⇒ 打印 `结果:CEF 在 zap 内初始化成功 ✅`。

**反向判据**:看到 `API hashes for libcef and libcef_dll_wrapper do not match.` ⇒ 编 wrapper 用的 CEF 目录
与 `.app` 里嵌的 framework 不是同一份(回去核对 §3.5 判据 2 + `cef_embed --cef-path`)。

### 3.8 正式打包 + 签名

```bash
CEF_PATH="$CEF_INNER" script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef
# 路径取自 AGENTS.md §5.9;本机 target/release-lto/bundle/osx/ 当前不存在(ls 报 No such file),
# 故这条路径本身「未验证」—— 以 bundle 打印的 OUT_DIR 为准。
codesign -dv target/release-lto/bundle/osx/Zap.app
codesign --verify --deep --strict target/release-lto/bundle/osx/Zap.app
```

`--cef` 会追加 `cef_webview` feature(`bundle:341-344`),并在 Step 1.5 构建 `tools/cef-helper` +
调 `cef_embed`(`bundle:775-790`),签名身份用 `APPLE_TEAM_ID`(=`2BBY89MBSN`,`bundle:774`)经
`CEF_EMBED_SIGN_IDENTITY` 透传(`bundle:786`,`cef_embed:109`)。

**判据**(`AGENTS.md` §5.9):`codesign -dv` 输出 `Signature size=…`(**不是** `adhoc`);
`codesign --verify --deep --strict` 退出码 0。

**注意**:`--cef` 的完整 release-lto 打包**至今没有跑过**(`evidence/phase1/BUNDLE-WIRING.md:54-57`、
`TECH.md:73` 都记着"未跑"),所以这一步没有历史输出可对照,首次要按新路径对待。

### 3.9 行为回归

跑 §5 的清单。**不要**把"`cargo check` 绿 + 签名 OK"当作行为回归 —— `TECH.md:78` 与
`evidence/phase1/OSR-T10-DOWNLOAD.md:§5.3` 都明确记过"这些都不覆盖行为"。

---

## 4 回滚方案

回滚是**三条链**一起退,任何一条没退都会出现 §2 的错版。

### 4.1 二进制侧(版本化布局天然并存)

```bash
export CEF_PATH="$HOME/.local/share/cef/152.0.8/cef_macos_aarch64"
cat "$CEF_PATH/archive.json"     # name 必须含 152.0.8
```

**判据**:`archive.json` 的 `name` 含 `152.0.8`,`type=minimal`。
若升级时走了 §3.4(b)(把旧目录整体改名备份),则 `mv` 回来即可 —— 但**平铺与版本化不能同时占位**,
同时存在时 `build.rs:91` 优先版本化目录,而打包脚本只认"给定目录里直接有 framework",两边会错位。

### 4.2 代码侧

```bash
git checkout -- app/Cargo.toml tools/cef-helper/Cargo.toml Cargo.lock tools/cef-helper/Cargo.lock
grep -A1 'name = "cef' Cargo.lock tools/cef-helper/Cargo.lock
```

**判据**:两处 lock 回到 `152.4.0+152.0.8`。
(工作树里若有**其它**未提交改动,不要用上面这条泛化的 `git checkout --`;逐文件回退。)

### 4.3 重打包

```bash
CEF_PATH="$HOME/.local/share/cef/152.0.8/cef_macos_aarch64" script/macos/cef_smoke
```

**判据**:同 §3.6 —— 6 个 `OK` + `签名校验:OK`。

### 4.4 运行期回滚入口(与版本无关,先试这两个)

| 想要什么 | 怎么退 | 判据 | 依据 |
|---------|--------|------|------|
| OSR → windowed | 设置项 `general.webview.use_osr_rendering=false`(默认 **true**)或 `export ZAP_CEF_OSR=0`(非空即覆盖设置项) | 日志 `[cef] render mode = Windowed (…)` | `app/src/settings/cef_webview.rs:36`;`TECH.md:174-176`、`:183` |
| CEF → wry | 设置项 `general.webview.use_chromium=false`(默认 true) | dsh pane 走 wry 路径、无 `[cef]` 初始化日志 | `app/src/settings/cef_webview.rs:15-17`;`TECH.md:83` |

---

## 5 升级后行为回归清单

**前置提醒**:用户已停止 GUI 测试(`OSR-PLAN.md:307` 明写"用户已停止 GUI 测试 ⇒ 下表待验证各项
保持待验证")。因此表中"现状"列本身就有若干项是**待验证** —— **升级回归不能用它们当基线**,
只能比较"升级前后同一条命令的输出"。

| # | 项 | 依据 | 升级后怎么验 | 现状 |
|---|----|------|-------------|------|
| 1 | 未处理下载的默认行为:无 `download_handler` 时**默认目录静默落盘** | `evidence/phase1/OSR-T10-DOWNLOAD.md:10,16-18,38`;`evidence/phase1/OSR-ALIGNMENT-VS-REFSWIFT.md:62-64` | 临时摘掉 handler(或在 windowed 下导一次 dsh session),看是否落到 `~/Downloads/<建议名>`;三条历史行见 `OSR-T10-DOWNLOAD.md:16-18` | 现象已确认;成因未定(上游 Alloy 取消分支是否存在**未验证**) |
| 2 | mac 上 `<select>` **不产生 `PET_POPUP`** | `evidence/phase1/OSR-T6-POPUP-MENU.md:29`;`evidence/phase1/OSR-REVIEW-3.md:71` | 往页面注入一个可见 `<select>` 并点击(做法见 T6:29-33),看日志有无 `on_popup_show`/`on_popup_size` | 已实测;**当前链路无触发路径**(已实现未验证) |
| 3 | Alloy 风格相关:右键菜单默认项集合 / 下载取消分支 | `evidence/phase1/RUNTIME-VERIFICATION.md:85`;`evidence/phase1/CODE-REVIEW.md:44`;`evidence/phase1/OSR-T10-DOWNLOAD.md:29-34`;`app/src/browser/cef_backend.rs:1982`(`runtime_style: RuntimeStyle::ALLOY`) | 右键看默认项是否仍是 Back/Forward/分隔符/Print…/View Page Source(我们 `clear()` 后只放「重新加载」「检查元素」);下载见第 1、5 行 | 152 上实测过;**跨大版本必须重测** |
| 4 | OSR:透明 + `external_begin_frame` + `OnAcceleratedPaint`/IOSurface | `evidence/phase1/OSR-SPIKE-A.md:21,26,30,46,58`;`evidence/phase1/OSR-T4-MAIN-REPO.md:13,22`;`evidence/phase1/RUNTIME-VERIFICATION.md:107` | ① 加速路径:日志 `OSR surface <W>x<H>px (view_rect=(w,h) DIP, scale=s)`,必须 `W == w*s`(无重复缩放);② 透明:按 SPIKE-A 的 IOSurface 像素读回手法(`osr_surface_dump`)读回 alpha;③ `ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME=1` **默认关、属未实测项**,要另测 | 渲染/缩放 ✅;外部 begin-frame **未实测** |
| 5 | `CefDownloadItemCallback::Cancel()` 的取消语义 | `evidence/phase1/OSR-T10-DOWNLOAD.md:120` 与 §3;`TECH.md:78` | 导出 → 保存面板 → 取消:日志出现 `[cef] download cancel requested: id=N`;`ls ~/Downloads/.dev.zap.cef-smoke.*` 为空;item 进 `CANCELLED` 而非停在 target-pending | 实机验证过;机制为何如此**未验证** |
| 6 | `start_dragging` 的坐标口径(CEF 头文件写 screen coordinates) | `OSR-PLAN.md:316`;`TRANSPARENCY.md:85-87` | 从页面往系统拖一个元素,看拖拽图像是否跟手(实现已改为用 AppKit 当前鼠标位置,不消费 `x`/`y`,两种假设下都正确) | 契约违反已修;**无 GUI 复验** |
| 7 | `ProcessSingleton` 与缓存路径约束 | `evidence/phase1/RUNTIME-VERIFICATION.md:34-42`;实现在 `app/src/browser/cef_backend.rs:1387-1403`(`cef_cache_paths`)、`:1413-1428`(`cache_path` 必须是 `root_cache_path` 的子目录) | 启动后存活 >30s(**不得**被 SIGKILL/退出码 137),且 `~/Library/Application Support/zap-cef/root-cache/cache` 存在;不得出现 `cache_path is not a valid child of the root_cache_path` | 已修(两次叠加触发过 SIGKILL) |
| 8 | `NSApplication` / `CefAppProtocol` 协议桥 | `evidence/phase1/RUNTIME-VERIFICATION.md:44-54`(二分表:仅初始化无协议桥 → 死);`TECH.md:51-55,69`;`app/src/browser/cef_backend.rs:1280` | 启动后存活 >50s;不出现早期死亡(二分表的"仅初始化"列) | 已落并实测 |
| 9 | **版本链一致性(非行为,但必须验)**:wrapper 与 libcef 的 API hash | `~/.local/share/cef/libcef_dll/wrapper/libcef_dll_wrapper.cc:65-66`、`:84-85` | 不出现 `API hashes for libcef and libcef_dll_wrapper do not match.`;`app/src/browser/cef_backend.rs:1329` 的 `api_hash` 调用点保持"在 `execute_process` 之前"的顺序(顺序反了会 SIGSEGV,见 `TECH.md:68`) | 当前 152 一致(`15200` / `15200`) |
| 10 | **打包一致性(非行为,但必须验)**:5 个 helper 命名 + 三层签名 | `script/macos/cef_embed:75,83,91,113,118`;`evidence/phase1/BUNDLE-WIRING.md:37`;`TECH.md:71-72` | §3.6 的逐字判据(framework / 5×helper / app 全部 `verify=OK`,`CFBundleExecutable` 逐字为 `<主可执行名> <Helper>`) | 152 基线已跑通并在 §3.6 存了输出 |
| 11 | 隐藏超时冻结(CDP `Page.setWebLifecycleState`) | `TECH.md:77`;`evidence/phase1/RUNTIME-VERIFICATION.md:115` | 日志 `隐藏超时,已冻结页面` → `重新可见,解冻页面`,无 panic | ✅(152 上实测) |
| 12 | IME(`NSTextInputClient` → `ime_set_composition`/`ime_commit_text`) | `evidence/phase1/OSR-T7-IME.md` 结论段;`evidence/phase1/OSR-SPIKE-B.md:1-8`;`evidence/phase1/RUNTIME-VERIFICATION.md:110` | 拼音 → 候选 → 上屏,日志 `ime_set_composition "…"` → `ime_commit_text "…"`;候选框跟随光标 | ✅(152 上实测) |

> **文档口径提醒**:`evidence/phase1/OSR-T8-SWITCH-EQUIV.md:10` 写 `use_osr_rendering` "默认 **false**",
> 与当前代码不符 —— `app/src/settings/cef_webview.rs:36` 是 `default: true`(2026-09-22 用户决定,
> 见同文件 `:32-33` 注释)。**以代码为准**;回归时默认路径是 **OSR**,不是 windowed。

---

## 6 已知未落地项

1. **`channel_versions` 里没有任何 cef / chromium 记录。**
   `TECH.md:150` 的风险表写了"升级节奏写进 channel_versions 记录",但这半句**没落地**:
   `grep -rn 'cef\|chromium' crates/channel_versions/src/` 只命中 `channel_versions` 这个标识符本身;
   `crates/channel_versions/src/lib.rs:16-22` 的 `ChannelVersions` 只有 `dev`/`preview`/`stable` + `changelogs`,
   **没有"组件版本"这个概念**。落地形态(给 `ChannelVersions` 加字段 vs 单独记录文件)**未定,需工程决定** ——
   本文件只是过程记录,不构成 channel 口径。
2. **`script/macos/bundle` 不设 `CEF_PATH`**(§2.1(b))。**源码可溯·未实跑**。
   后果:调用方忘记 `export` 时,Step 1 构建与 Step 1.5 嵌入可能取到不同的 CEF 副本。
3. **平铺旧目录会静默钉住旧版本**(§3.4)。**源码可溯·未实跑**。
   依据是 `build.rs:87-105` 与 `download-cef-3.0.0/src/lib.rs:102-123` 的"只拒绝更新的 archive"判定。
4. **`--cef` 的 release-lto 正式打包从未执行过**:`BUNDLE-WIRING.md:54-57`、`TECH.md:73`。
   ⇒ §3.8 的判据没有历史输出可对照。
5. **没有自动门禁**:`.github/` 下 grep `cef` / `cef_webview` / `--cef` **无命中**
   (`grep -rn 'cef' .github/` 唯一命中是 `.github/actions/prepare_environment/action.yml:97` 注释里的单词
   `gracefully`)。
   ⇒ CEF 版本升级**不会**被任何 CI 步骤拦住,回归只能本地手工做。
6. **除两处 `Cargo.toml` 外,没有把 CEF 版本当"值"用的地方**(工作树里另有若干注释/测试夹具提到版本号)。
   复核命令与结果(2026-09-23,含未提交的工作树改动):
   ```bash
   grep -rn '152\.\|154\.' --include='*.rs' --include='*.toml' --include='*.sh' --include='*.m' \
     --include='*.mm' app/ crates/ script/ tools/ | grep -v target/ | grep -iE 'cef|chrom'
   ```
   命中的**输入**只有 `app/Cargo.toml:258` 与 `tools/cef-helper/Cargo.toml:17`;
   其余全是注释或测试夹具(`app/build.rs:66,586` 注释、`app/src/browser/cef_update.rs:11,62,80,96` 注释、
   `app/src/browser/cef_update_tests.rs:6-24` 断言字面量)——
   这些**不需要**跟着改,但测试夹具里的版本号与新版语义不符时应当更新(见第 7 条)。
   ⇒ "三处 + 二进制目录"仍是完整集合;每次升级按同一条 grep 复核一遍即可。
7. **新增的"已链接版本"推导也要跟着升级**(仅当工作树里存在该文件时适用)。
   截至 2026-09-23,工作树里有一份**未跟踪**的 `app/src/browser/cef_update.rs`(设置页「CEF 内核更新」用),
   它把 `INSTALLED_VERSION` 定义为 `env!("ZAP_CEF_VERSION")`,注释写明该常量由 `app/build.rs` 从
   `Cargo.lock` 里 `cef` crate 的版本推出(`152.4.0+152.0.8` → `152.0.8`),并显式引用本文件作为配套流程。
   ⇒ 升级时除 §2 的 5 个锁点外,**再核一次这条推导的输出**:
   ```bash
   grep -n 'ZAP_CEF_VERSION' app/build.rs
   ```
   判据:`app/build.rs` 里推出的字符串 == 新版 `Cargo.lock` 中 `cef` 版本的 `+` 之后部分 ==
   `~/.local/share/cef/<该版本>/cef_macos_aarch64/include/cef_version.h` 的 `CEF_VERSION` 前缀。
   (该文件当时**未提交**,内容可能变化;本条不是对已落地行为的断言。)

---

## 7 相关文档

本目录:

- [TECH.md](TECH.md) —— CEF 承载 dsh webview 的技术 spec(阶段 1 集成进度、风险表 `:150`)
- [OSR-PLAN.md](OSR-PLAN.md) —— OSR 真透明实施计划(拖放坐标口径 `:316`、T10 收尾与待验证项)
- [TRANSPARENCY.md](TRANSPARENCY.md) —— 透明背景评估(为什么 windowed 不行、`:155` 起的实现期硬约束)
- [CEFSWIFT-EVALUATION.md](CEFSWIFT-EVALUATION.md) —— 参考实现 CefSwift 的 API 清单与取舍

证据(`evidence/phase1/`):

- [RUNTIME-VERIFICATION.md](evidence/phase1/RUNTIME-VERIFICATION.md) —— 实跑验证(协议桥二分、ProcessSingleton、缓存路径)
- [BUNDLE-WIRING.md](evidence/phase1/BUNDLE-WIRING.md) —— 打包链路证据 + 两条实测硬要求
- [OSR-T10-DOWNLOAD.md](evidence/phase1/OSR-T10-DOWNLOAD.md) —— 下载 + 取消语义
- [OSR-T10-REVIEW.md](evidence/phase1/OSR-T10-REVIEW.md) —— 下载/拖放独立复审(风险清单)
- [OSR-T6-POPUP-MENU.md](evidence/phase1/OSR-T6-POPUP-MENU.md) —— 弹层与右键菜单(`<select>` 不产生 `PET_POPUP`)
- [OSR-T4-MAIN-REPO.md](evidence/phase1/OSR-T4-MAIN-REPO.md) —— OSR 主仓落地(三开关、绘制路径)
- [OSR-SPIKE-A.md](evidence/phase1/OSR-SPIKE-A.md) —— IOSurface → CALayer 真透明 spike
- [OSR-ALIGNMENT-VS-REFSWIFT.md](evidence/phase1/OSR-ALIGNMENT-VS-REFSWIFT.md) —— 与参考实现的逐项比对
- [RESULT.md](evidence/phase1/RESULT.md) —— 阶段 1 第一生死项(spike 结果)

仓库根:

- [AGENTS.md](../../AGENTS.md) —— 仓库纪律(§5.6.1 结论必须可验证、§5.9 构建与打包流程)

外部:

- crates.io `cef`:<https://crates.io/api/v1/crates/cef/versions>
- CEF 构建索引:<https://cef-builds.spotifycdn.com/index.json>
- cef-rs 源码:<https://github.com/tauri-apps/cef-rs>

---

## 8 本次升级执行记录(2026-09-23,`152.0.8` → `154.0.23`)

> 与前面各节不同,本节是**实际执行**的结果,每步附命令/输出判据。

### 8.1 步骤与结果

| 步 | 命令 / 动作 | 结果判据 |
|---|---|---|
| 1 | `curl https://crates.io/api/v1/crates/cef` | `max_version = 154.0.0+154.0.23`(无 153.x);`cef-dll-sys` 同为 `154.0.0+154.0.23` |
| 2 | 备份旧 CEF 目录:`mv ~/.local/share/cef ~/.local/share/cef-152.0.8-flat` | 343MB 平铺目录保留为回滚源(零额外磁盘占用) |
| 3 | 改两处版本字符串:`app/Cargo.toml`、`tools/cef-helper/Cargo.toml` → `154.0.0` | **必须先改字符串**:`"152.4.0"` 是 `^152.4.0`(上限 <153) |
| 4 | `cargo update -p cef`(根)+ `cargo update --manifest-path tools/cef-helper/Cargo.toml -p cef` | 两个 lock 各输出 `Updating cef v152.4.0+152.0.8 -> v154.0.0+154.0.23` |
| 5 | `CEF_PATH=$HOME/.local/share/cef cargo check -p warp --features cef_webview` | CEF 目录缺失 ⇒ **自动下载**到 `~/.local/share/cef/154.0.23/cef_macos_aarch64/`;`Finished`,**0 error / 0 warning** ⇒ 代码零改动即通过(无 API 破坏) |
| 6 | 拍平 `154.0.23/cef_macos_aarch64/*` → `~/.local/share/cef/`,删除下载残留 tarball(132MB) | 顶层重新出现 framework 与 `archive.json`(154.0.23)⇒ 文档约定的 `CEF_PATH=~/.local/share/cef` 与脚本口径不变 |
| 7 | `CEF_PATH=… script/macos/cef_smoke` | 嵌入 154 framework + 重编 helper(cef 154)+ 5 个 helper 分层签名;`签名校验:OK`;`打包完成` |
| 8 | 启动实例(**不带任何环境变量**) | `[cef] render mode = Osr` / `context initialized` / `message pump active`;`panic/fatal/abort/CHECK failed` **0 条** ⇒ wrapper(154)与 framework(154)匹配,`libcef_dll_wrapper.cc` 的 API hash `CHECK` 通过 |
| 9 | 核对产物 | bundle 内 `CFBundleShortVersionString = 154.0.23.0`;主二进制内含 `154.0.23`(关于页「CEF 内核」显示的已链接版本,由 `app/build.rs` 从 lock 推出) |

### 8.2 代码改动量

**只有两处版本字符串**(`app/Cargo.toml`、`tools/cef-helper/Cargo.toml`)+ 两个 lock。
`app/src/**` 与 `app/src/platform/mac/objc/cef_support.m` **一行未改**:我们用到的 API
(OSR/`RenderHandler`、`download_handler`、`BrowserHost::drag_target_*`、`window_handle`、
IME、`set_focus` 等)在 152 → 154 之间没有破坏性变更。**注意:这只说明"编译期不破",
不等于行为等价。**

### 8.3 升级后行为回归结果(2026-09-23 用户实机复验)

> 与 §8.1 的"编译/启动"不同,本节是**升级到 154 之后**的实机结果。
> 证据类型:用户实机确认(仓库接受的一手证据);具体逐项未逐一列名,用户的原话是
> "除 `<select>` 与保存面板后焦点恢复之外,其他测试通过"。

| 回归项(§5 清单) | 结果 |
|---|---|
| 下载 / 取消 / 保存面板、改路径保存(`CefDownloadItemCallback::Cancel()` 语义) | ✅ 用户实机通过 |
| 拖放双向(Finder → pane、页面 → 系统)、拖放光标 | ✅ 用户实机通过 |
| 焦点(T10.2 随窗口 key)、中文 IME 上屏、Cmd+C/V 等编辑快捷键 | ✅ 用户实机通过 |
| 右键菜单(两项:重新加载 / 检查元素) | ✅ 用户实机通过 |
| OSR 渲染(含透明)与实际观感 | ✅ 用户实机通过 |
| **mac 上 `<select>` 弹层(`PET_POPUP`)** | ⏸ **用户决定暂不处理、仅记录**(见下) |
| **保存面板关闭后的焦点恢复** | ⏸ **用户决定暂不处理、仅记录**(见下) |
| `external begin-frame` 内部时序、`start_dragging` 的坐标口径取证 | 未单独验证(前者无用户可见判据、后者需专门取证) |

**两条被明确推迟的项(仅记录,不做改动)**:

1. **`<select>` 弹层**:CEF 在 mac 上不产生 `PET_POPUP`(T6 实测,`evidence/phase1/OSR-T6-POPUP-MENU.md`),
   当前 dsh 页面也未触发到它。**该决定早在 T6 已作出**(`OSR-PLAN.md:144`:"`<select>` 弹层经用户决定不做 ——
   没用到"),本轮用户重申"先不用管、有记录就行";如将来页面需要原生下拉弹层,再按 T6 的方案单独处理。
2. **保存面板关闭后的焦点恢复**:早前实机现象是"点取消后页面收不到键盘";三层修复
   (key window 归还 / 视图·CEF 焦点 / DOM `__restoreFocused`)已落地但**无运行时确认**,
   且前两版修复都未解决该现象。用户决定暂不追,记录在案:
   复现时先看日志 `[cef] 面板关闭后恢复页面焦点 (id=…, windowed=…, windowed_restore=…, dom=…)`;
   若该行出现而焦点仍不回,下一步查"面板是否把 app 弄成非激活态"。
   相关细节:[evidence/phase1/OSR-T10-DOWNLOAD.md](evidence/phase1/OSR-T10-DOWNLOAD.md) §5。

**本轮唯一做到的运行时确认**是第 8 步(CEF 能初始化、泵在跑、无 abort),**不覆盖任何 UI 行为**。

### 8.4 回滚

```bash
# 1) 恢复依赖版本(两处字符串 + 两个 lock)
#    app/Cargo.toml:258 与 tools/cef-helper/Cargo.toml:17 改回 "152.4.0"
cargo update -p cef                                  # 若要精确复原,可用 git checkout 恢复 Cargo.lock
cargo update --manifest-path tools/cef-helper/Cargo.toml -p cef
# 2) 换回旧 CEF 目录
mv ~/.local/share/cef ~/.local/share/cef-154.0.23-flat   # 保留新版本以便回退
mv ~/.local/share/cef-152.0.8-flat ~/.local/share/cef
# 3) 重打
CEF_PATH=~/.local/share/cef ./script/macos/cef_smoke
```

### 8.5 `--cef` release-lto 正式打包首跑(2026-09-23,`bundle --channel oss --selfsign … --cef`)

> 本节是 §6 第 4 条("从未执行过")的闭环:**首次跑通**,踩了三个脚本级坑,均已修复并实机验证。

| # | 坑 | 症状(实测) | 修法 |
|---|----|-----------|------|
| 1 | `--selfsign` 与 CEF 分层签名身份不兼容 | Step 1.5 写死 `APPLE_TEAM_ID=2BBY89MBSN`,本机无此身份 ⇒ `no identity found`,`set -e` 直接中断(且 shell 误报 exit 0) | `bundle` Step 1.5:`--selfsign` 时改用与 Step 3 同款本机 `Apple Development` 身份(无证书回退 adhoc) |
| 2 | **oss 分支 `FEATURES` 覆盖赋值丢 `cef_webview`** | 打包全绿、framework 嵌入、签名 OK,但设置里无任何 CEF 项、内核仍是系统 webview;`strings` 查 `render mode = ` 为 **0**(好包为 1) | `bundle` oss 分支在 `CEF=true` 时把 `cef_webview` 补回;**其余渠道都是追加赋值,不受影响** |
| 3 | **`--deep` 重签覆盖 helper 的 JIT entitlements** | dsh pane 打开即"已崩溃";日志每秒刷 `renderer 终止 TS_PROCESS_CRASHED code=5`;崩溃报告 `faultingThread: CrRendererMain`、`EXC_BREAKPOINT/SIGTRAP`,栈在 `cef_execute_process` → V8 init。smoke 包(adhoc、无 runtime)不复现 | 新增 `script/macos/cef-helper-entitlements.plist`(allow-jit + allow-unsigned-executable-memory + disable-library-validation),`cef_embed` 分层签 helper 时带上;`bundle` 在 Step 3 之后加 **Step 3.5** 按该 plist 重签 5 个 helper(只碰 helper,主包/framework 不动) |

**验证(闭环)**:重签现包(不重编)→ 替换 `/Applications/Zap.app` → 用户实机 dsh 正常打开;
`zap.log` 零条 `TS_PROCESS_CRASHED`、出现 `OSR surface … view_rect × scale`(retina 正确);
`DiagnosticReports` 无新崩溃(最新仍是修复前 19:31 批次);`codesign --verify --deep --strict` 通过。
坑 3 的外部依据:CEF 官方论坛 [macOS] Renderer Process Crash(SIGTRAP) (t=20345) 与
Electron 同款案例(osx-sign#232)——hardened runtime 的 renderer helper 缺 JIT 权限即 SIGTRAP。
**smoke 包(adhoc 签名)测不出坑 2/3**:`cef_smoke` 与正式包的差异(coverage 与 entitlements)
正好是这两个 bug 的藏身处 ⇒ 正式打包首跑必须单独实机验证一次 dsh。
