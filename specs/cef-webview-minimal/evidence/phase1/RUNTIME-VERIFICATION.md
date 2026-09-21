# 阶段 1 实跑验证(zap 本体 + CEF,2026-09-21)

> 环境:macOS 27.0 arm64,CEF 152.0.8 minimal,debug 构建(`--features cef_webview`),
> 测试 bundle `~/Applications/ZapCEF.app`(bundle id `dev.zap.cef-smoke`),
> 开关 `ZAP_CEF_WEBVIEW=1`。
> **结论:成功** —— dsh pane 在 CEF 后端下正常渲染,用户实机确认"不再停在启动中"。

## 1 用户可见结果

用户实际操作后确认:**打开 dsh pane 后直接显示界面,不再是"启动中"**。
(此前症状可复现:遮罩长期停在"启动中",而页面其实已加载。)

## 2 日志证据

```
[browser] create webview 1 url=http://127.0.0.1:56698/?token=… backend=Cef   ← pane 选到 CEF 后端
[cef] webview 1 browser created                                            ← CEF 浏览器创建
[cef] webview 1 loaded (shim 注入)                                          ← 页面加载完成 + 回环 shim 注入
```

进程侧:主实例存活,CEF 子进程 6 个(browser/renderer/GPU/utility);泵在跑
(日志 `[cef] message pump active (60 ticks)`)。

## 3 实跑定位出的三个真问题(均已修)

### 3.1 pane 一直"启动中"(用户原始症状)

**根因**:pane 的 `webview_loaded` 只由 wry 的 `on_page_load_handler` → `UrlChanged` 事件翻转;
CEF 侧没有对应回调,页面渲染完成后遮罩不会撤(等 15s 超时兜底)。
**修复**:CEF 主 frame `on_load_end` 回传 `notify_webview_page_loaded(id)`(与 wry 同一事件路径),
并记录真实 URL(首载 `?token=…` → 303 后裸地址);shim 改为**每次**主 frame 加载都注入
(原来一次性,reload/二次文档后 IPC 静默失效)。

### 3.2 CEF 被初始化了两次 + 缓存路径配置错误

**根因**:`zap-oss terminal-server …`(以及崩溃恢复 watchdog)子进程同样会走到 `warp::run()`,
于是**两个进程都初始化 CEF**;同时 `cache_path` 与 `root_cache_path` 被配成兄弟目录
(CEF 报 `cache_path is not a valid child of the root_cache_path`,回退到内存存储),
两者叠加触发 Chromium `ProcessSingleton` 冲突 —— 表现为**主进程启动 ~30s 后被 SIGKILL(退出码 137)**,
以及用户看到的"又开启了一个实例却没界面"。
**修复**:① 仅**主 app 启动**(无 CLI 子命令、非 `--crash-recovery-mechanism`)才初始化 CEF;
② `cache_path` 放到 `root_cache_path` 之下。

### 3.3 CefAppProtocol 协议桥是必需项(bisect 证据)

二分(用临时开关测):
| 配置 | 结果 |
|------|------|
| 初始化 + 协议桥 + 泵 | ✅ 存活 |
| 初始化 + 协议桥,无泵 | ✅ 存活 >50s |
| 仅初始化(无协议桥) | ❌ 死亡 |

⇒ 证实评审 Important #3 的判断:CEF 要求 `NSApplication` 实现 `CefAppProtocol`,缺了它进程会挂;
本仓用 category + 交换 `sendEvent:` 的实现是必要且有效的。

## 4 实跑后续:隐藏策略两次修正

**4.1 "隐藏时视图不消失"**(用户报告:切到别的 tab,底下还显示着 webview)
根因:`do_close` 按 CEF 契约返回 1 = "由客户端接管关闭",但我们只调了 `close_browser(1)`,
既没摘视图也没丢句柄 ⇒ 浏览器成为僵尸:视图留在窗口里(切 tab 仍可见),重建时又叠一层。
日志证据:只有 `webview 1: 重现,重建浏览器`,**从未出现 `webview 1 closed`**。

**4.2 取消"隐藏即销毁" + 新增"隐藏超时冻结"**(用户决定)
- 隐藏:仅 `setHidden:` + `was_hidden(1)`,renderer 保活(切回不重载、不丢页面状态);
- 冻结:隐藏超过阈值后发 CDP `Page.setWebLifecycleState=frozen`,切回发 `active`;
  设置项 `general.webview.freeze_after_secs`(默认 300s,`0`=关闭),
  dev 覆盖 `ZAP_CEF_FREEZE_AFTER_SECS`;
- dsh 的 Node 服务端/agent 任务不受冻结影响;代价是隐藏期间页面发不出消息
  (例如"任务完成"的 zap 通知会延后到切回)——这正是阈值默认给到 5 分钟的原因。

## 5 仍未验证

- `do_close` 是否确实阻止"关闭转发给顶层窗口"(本次未关窗口,未触发);
- 完整 release-lto 打包(磁盘余量不足,未跑);
- 像素级渲染证据(屏幕锁定期间无法截图);
- 运行中的测试实例仍带临时探针(会写 `~/Library/Logs/zap-cef-probe.log`);源码里已删除,
  下次重新打包即消失。

## 6 实跑迭代出的产品化项(2026-09-21)

| 项 | 结论 |
|----|------|
| 使用 Chromium 内核开关 | 设置项 `general.webview.use_chromium`(默认 true,仅 MAC),UI 在功能页「通用」节、位于冻结超时**上方**;切换对**新打开**的 dsh pane 生效 —— `cef_backend::ensure_initialized()` 按需初始化(无需重启)。回环 IPC 路由改为**只按 feature 编译期**门控,否则懒初始化后端点缺失、页面 `zap.*` IPC 静默失效 |
| 隐藏超时冻结 | 设置项 `general.webview.freeze_after_secs`(默认 300s,0=关闭);CDP `Page.setWebLifecycleState` 冻结/解冻;dsh 的 Node 服务端与 agent 任务不受影响 |
| 右键菜单 | 客户端 `ContextMenuHandler` **清空**默认菜单后只放「重新加载」「检查元素」。实测 CEF(Alloy)默认项只有 Back/Forward/分隔符/Print…/View Page Source,**没有"自动填充"**,故无需按标题保留(标题匹配还会随语言失效);代价:右键不再有复制/粘贴(Cmd+C/V 不受影响) |
| 背景色 | **windowed CEF 无法真透明**(`cef_types.h:701-708`:透明 alpha → 回退 `CefSettings.background_color`,再透明 → 不透明白色;wry 依赖的是 WKWebView 私有 KVC `drawsBackground`,CEF 无对应键)。故显式填 zap 工作区底色(`appearance::window_surface_color`,与 workspace 窗口背景同一函数);真透明需改 OSR(windowless)渲染,属独立工作量 |
| 设置页间距 | 该行比邻居略高的间距来自**下拉行本身**(`render_dropdown_item` 的标签 bottom margin + 控件内边距),非本行特有;用户确认所有下拉框一致 |

## 7 复现命令

```bash
CEF_PATH="$HOME/.local/share/cef" cargo build -p warp --features cef_webview --bin zap-oss
# 组装/嵌入(同 script/macos/cef_smoke),然后:
ZAP_CEF_WEBVIEW=1 <app>/Contents/MacOS/zap-oss
```
