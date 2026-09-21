# 阶段 0 spike 原始证据(2026-09-21,macOS 27.0 arm64,CEF 152.0.8 minimal)

本文档归档 `docs/dsh-webview-engine-evaluation.md` §6.1 各验收项的原始命令与输出摘录。
日志原件同目录(`*.log`);私人足迹数据当次会话转录,已标注。

## a) 包体(bundle 产物逐次构建;产物不留工作树,数据转录自终端输出)

```
$ cargo run --bin make-bundle -- cef-spike -o target/bundle
bundle ready: target/bundle/cef-spike.app
$ du -sm target/bundle/cef-spike.app
322   (未裁剪)
317   Contents/Frameworks/Chromium Embedded Framework.framework(引擎 dylib 独占 224MB,Libraries 16MB,Resources 84MB)
$ # locale 裁剪:ls -d *.lproj | wc -l → 220;保留 en/zh_CN/zh_TW 系 16 个后:
du -sh ./Resources   → 34M(84M - 50M)
du -sm target/bundle/cef-spike.app → 275
```
注:裁剪是对 make-bundle 产物做的后处理(bundle 每次重建都会恢复全量 locale,正式裁剪应落在 spec 阶段 1 的打包脚本里)。

## b) 签名(codesign)

```
$ cd tools/cef-spike/target/bundle
$ codesign --force --deep -s - cef-spike.app
cef-spike.app: replacing existing signature
$ codesign --verify --deep --strict cef-spike.app && echo SIGN-OK
SIGN-OK
$ codesign -dv cef-spike.app
Executable=…/cef-spike.app/Contents/MacOS/cef-spike
Identifier=dev.zap.cef-spike.cef-spike
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20400 size=4532 flags=0x2(adhoc) hashes=135+3 location=embedded
```

## c) 裸窗启动(excerpt from cef-spike-run.log)

```
launch browser process
[WARNING] Please customize CefSettings.root_cache_path for your application.
$ ps aux | grep -c "[c]ef-spike"   → 9(主进程 + 5 helper + 常驻)
```
(Metal 挖洞合并未在本阶段执行——见 docs §6.1 与 TECH.md 阶段 1。)

## d) IPC(excerpt from evidence/server.log,回环 HTTP shim 路径)

测试页在一次加载后直接执行:
```js
fetch('http://127.0.0.1:9911/webview-ipc', {method:'POST', body:'zap.switch_project\n1\n{"path":"/tmp"}'})
```
server.log:
```
server up
[zap-ipc-received] zap.switch_project
1
{"path":"/tmp"}
```
即 `zap:` 三段协议(method/id/params)字节级到达 zap 侧应处理的位置。

## e) 内存(私有足迹,vmmap --summary)

页面:`/tmp/cef-page/index.html`(16ms 间隔追加 DOM 行 + scrollTo,无限增长负载)。

```
CEF 组(7 进程,私有足迹合计 ~634MB):
  browser 73.6M / renderer(Base) 365.5M / 其余(GPU/网络/工具) 21.1+20.1+56.4+26.1+70.8M
WKWebView 对照组(同页,Swift 最小宿主 /tmp/wk-probe,WebKit 系统共享宿主):
  应用宿主 29.2M / GPU 120.8M / WebContent 306.8M(peak 367.0M) → ~456MB
```
口径说明:RSS 总和会跨进程重复计入共享的 224MB libcef 映射(实测 RSS 总和 ~870MB,虚报);
`vmmap --summary <pid>` 的 Physical footprint 是 jetsam 同口径的记账单位。

## f) 高频更新压测(excerpt from cef-spike-stress.log + 终端转录)

```
启动后 150 秒连续 60Hz DOM 更新:
进程存活数(应≥5): 7          ← 原样转录($alive 变量拼接笔误,意为存活 7 个)
grep -cE "CRASH|FATAL|Segmentation|terminated" → 0
tail: 无崩溃行
压力结束后 renderer(Helper (Renderer))私有足迹:47.1M(峰值期未采,GC 收敛值)
```
原始文件:`cef-spike-stress.log` 同目录(录于 /tmp/cef-spike-stress.log)。
