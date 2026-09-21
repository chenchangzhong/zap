# CEF 打包链路证据(`script/macos/bundle --cef` + `script/macos/cef_embed`)

> 2026-09-21。本文件归档"Cef 装进 Zap.app"这一段的实测输出与两条硬要求。
> 命令与 `script/macos/bundle` 的 Step 1.5 逐字一致;完整正式打包(release-lto)
> 尚未跑,见文末"未覆盖"。

## 1 演练命令与输出

```
$ CEF_PATH="$HOME/.local/share/cef" cargo build \
    --manifest-path "$PWD/tools/cef-helper/Cargo.toml" --release
    Finished `release` profile [optimized] target(s) in 0.08s        # 增量;首次约 31s

$ ./script/macos/cef_embed /tmp/zap-cef-smoke2/ZapCEF.app \
    --cef-path "$HOME/.local/share/cef" \
    --helper-executable "$PWD/tools/cef-helper/target/release/zap_cef_helper"
==> 嵌入 CEF framework (/Users/zhong/.local/share/cef)
==> 生成 zap-oss Helper.app … Helper (GPU|Renderer|Plugin|Alerts).app
==> 分层签名(identity=-):framework → helpers
framework: OK
zap-oss Helper.app: OK
zap-oss Helper (GPU).app: OK
zap-oss Helper (Renderer).app: OK
zap-oss Helper (Plugin).app: OK
zap-oss Helper (Alerts).app: OK
==> CEF 嵌入完成:…

$ codesign --force --deep -s - …/ZapCEF.app && codesign --verify --deep --strict …/ZapCEF.app
VERIFY-OK
包体: 905 MB
```

## 2 两条实测硬要求(踩坑记录)

| 现象 | 根因 | 结论 |
|------|------|------|
| `gpu_process_host ... error_code=1003`,GPU/Renderer 反复重启,最终 `FATAL: GPU process isn't usable` | helper.app 内的可执行文件**必须**命名为 `<主可执行名> <Helper>` 且 `CFBundleExecutable` 与之一致。保留原文件名(如 `zap_cef_helper`)必失败;bundle id 用后缀/前缀无影响(二分实验确认) | `cef_embed` 按该约定生成 |
| helper 一启动就 panic:`LibraryLoader` canonicalize 失败 | helper 的 framework 在**外层 app** 的 `Contents/Frameworks` 下,必须以 helper 模式加载(`LibraryLoader::new(exe, helper=true)`) | 落在地 `tools/cef-helper`(helper=true)与 `cef_backend::load_library(helper)` |
| 5 个 helper.app 各含一份 app 可执行文件 → debug 包体 **3833MB** | 用主二进制当 helper 的天然代价 | 改用专用 helper(`tools/cef-helper`,**0.43MB**)→ 同配置 **905MB**(589 app + 317 framework + 5×0.43) |
| helper 进程在 `channel/state.rs:277` panic(AppId 需 3 段) | 分流派发晚于 bin 的 `ChannelState::new`;helper bundle id 带后缀 | 分流提前到 bin `main()` 第一行(`warp::maybe_run_as_cef_subprocess()`) |

## 3 helper 真实拉起验证(无 GUI 依赖)

把 `tools/cef-helper` 装进 spike 的 app(`cef-spike.app`)后启动:

```
GPU 启动失败次数: 0
FATAL 次数: 0
页面事件(回环通道): probe.loaded
```

即:CEF 用**我们的 helper** 成功拉起 GPU/Renderer 并完成页面加载。

## 4 未覆盖(留给受控实跑)

- `script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef` 的**完整正式打包**(release-lto + create-dmg)未执行:耗时长,且需要 `create-dmg`/签名环境;`--cef` 分支只做了语法检查与逐字命令演练。
- zap 本体 + CEF 的 GUI 实跑(dsh pane 渲染、隐藏即销毁效果、像素证据)未做:实跑时段用户正式版 Zap 正在运行,按约束不启停实例。
