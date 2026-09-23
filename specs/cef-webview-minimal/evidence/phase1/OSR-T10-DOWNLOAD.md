# T10.3(P0-2)下载:保存面板 + 取消 —— 落地与实机验证(2026-09-22)

> 结论:**已落地,并通过用户实机验证**。CEF 下导出 dsh Session 现在弹原生保存面板、可取消、
> 可改路径;取消后不再留下隐藏临时文件、item 也不再永远停在"进行中"。
> 本文所有结论都附命令与原始输出;未验证项集中在 §5,不得当结论用。

## 1 实现前的问题(有证据)

`cef_backend.rs` 此前**没有任何 download handler**(`grep -i download` 为空;`impl Client` 的 7 个
handler 里没有 `download_handler`)⇒ CEF 走默认下载路径:**静默落盘**到 `~/Downloads`,无面板、无法取消。

证据是 CEF 自己的 Chromium 下载库(实例 profile),三条记录全部无面板:

| 时间 | 来源页面 | 落盘路径 | state |
|---|---|---|---|
| 2026-09-22 22:41:12 | `http://127.0.0.1:61877/` | `~/Downloads/dsh-session-…8d2e7145….zip` | 1 = COMPLETE |
| 2026-09-22 19:12:51 | `http://127.0.0.1:52040/` | `~/Downloads/dsh-session-…4f9c571c….zip` | 1 = COMPLETE |
| 2026-09-21 23:50:59 | `http://127.0.0.1:63203/` | `~/Downloads/dsh-session-…cf682c3b….zip` | 1 = COMPLETE |

```bash
# 运行中的实例会锁住 History,需先拷贝(连 -wal/-shm 一起,否则读到旧快照)
D="$HOME/Library/Application Support/zap-cef/root-cache/Default"; T=$(mktemp -d)
cp "$D"/History* "$T/" && sqlite3 "$T/History" \
  "select id, target_path, state, received_bytes, datetime((start_time/1000000)-11644473600,'unixepoch','localtime') from downloads order by start_time desc limit 5;"
# state: 0=IN_PROGRESS 1=COMPLETE 2=CANCELLED
```

**头文件语义与实测现象不一致(成因未定)**:`cef_download_handler.h:110-121` 称不实现
`OnBeforeDownload` 会 "proceed with default handling (**cancel with Alloy style**)",而实测是
**默认目录静默落盘**(上表三条 COMPLETE)。上游 master 的
`CefDownloadManagerDelegateImpl::DetermineDownloadTarget` 末尾确实有
`if (browser->IsAlloyStyle()) { RunDownloadTargetCallback(空路径); return true; }`(Alloy 下"未处理"
= 取消),而我们自己的浏览器就是 Alloy(`cef_backend.rs` 设 `runtime_style: RuntimeStyle::ALLOY`)
⇒ 最可能的解释是 **CEF 152 尚无这条 Alloy 分支**(或本 build 走的不是该分支),但**未验证**。
判别实验(未做):把取消分支临时改成 `return 0`,看下载库是 `state=2 CANCELLED`(该分支存在)
还是仍落 `~/Downloads`(非 Alloy / 无该分支)。

因此本文只把**现象**记为已确认:**本 build 下"未处理"⇒ 默认目录静默落盘**;成因不下结论。
该疑点原本记为「未验证疑点」(`OSR-ALIGNMENT-VS-REFSWIFT.md` §3③),现降级为"现象已确认、成因未定"。

## 2 实现(改了哪些)

| 位置 | 改动 |
|---|---|
| `cef_backend.rs` `impl Client` | 接上 `download_handler()`(此前 7 个 handler 里唯独缺它) |
| `cef_backend.rs` `wrap_download_handler!` | `can_download` 放行;`on_before_download` 弹面板;`on_download_updated` 负责取消与终态日志 |
| `cef_backend.rs` `download_default_path` / `download_file_name` | 默认 `~/Downloads` + **清洗建议文件名**(只取最后一段,防写到下载目录外;空名/`..` 退回 `download`) |
| `cef_backend.rs` `DOWNLOAD_PANEL_OPEN` + `DownloadPanelGuard` | 面板期间抑制消息泵(见 §4.2) |
| `cef_backend.rs` `CANCELED_DOWNLOADS` | 取消标记(见 §4.1) |
| `browser_web_view.rs` | `run_download_save_panel` 由 `BrowserWebViewManager` 私有 fn 提为 `pub(crate)`,wry/CEF **两后端共用同一面板**(wry 调用点行为不变) |

纯函数单测:`download_file_name_strips_path_components`、`download_file_name_falls_back_when_empty`、
`download_default_path_stays_in_download_dir`。

## 3 实机验证(2026-09-22,用户操作)

日志(`~/Library/Logs/zap.log`,实例 PID 39290,**渲染模式 = Osr**),逐字原样:

```
2026-09-22T15:21:23Z [INFO] [warp::browser::cef_backend] [cef] render mode = Osr (env ZAP_CEF_OSR="(unset)", setting use_osr_rendering=true)
2026-09-22T15:22:05Z [INFO] [warp::browser::cef_backend] [cef] download canceled by user: dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip (id=7)
2026-09-22T15:22:05Z [INFO] [warp::browser::cef_backend] [cef] download cancel requested: id=7
2026-09-22T15:22:05Z [INFO] [warp::browser::cef_backend] [cef] download canceled: id=7
2026-09-22T15:22:21Z [INFO] [warp::browser::cef_backend] [cef] download: dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip -> /Users/zhong/Downloads/dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip
2026-09-22T15:22:21Z [INFO] [warp::browser::cef_backend] [cef] download completed: /Users/zhong/Downloads/dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip
2026-09-22T15:22:25Z [INFO] [warp::browser::cef_backend] [cef] download canceled by user: dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip (id=9)
2026-09-22T15:22:25Z [INFO] [warp::browser::cef_backend] [cef] download cancel requested: id=9
2026-09-22T15:22:25Z [INFO] [warp::browser::cef_backend] [cef] download canceled: id=9
2026-09-22T15:22:42Z [INFO] [warp::browser::cef_backend] [cef] download: dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip -> /Users/zhong/Documents/dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip
2026-09-22T15:22:42Z [INFO] [warp::browser::cef_backend] [cef] download completed: /Users/zhong/Documents/dsh-session-session-8d2e7145-53be-4794-bc41-66ddd73c8f8a.zip
```

> 覆盖范围说明:**只验了 OSR 模式**(windowed CEF 未验);两条取消 + 两条保存均为**单轮操作**,
> 未做重复/边界用例(如同名文件已存在、超长建议名)。

下载库状态(同一实例,读法同 §1):

| id | target_path | state | received_bytes |
|---|---|---|---|
| 7 | (空) | 2 = CANCELLED | 0 |
| 8 | `~/Downloads/dsh-session-…zip` | 1 = COMPLETE | 583926 |
| 9 | (空) | 2 = CANCELLED | 0 |
| 10 | `~/Documents/dsh-session-…zip` | 1 = COMPLETE | 583926 |

| 验收项 | 结果 |
|---|---|
| 导出 → 弹保存面板(默认 `~/Downloads` + 建议名) | ✅ |
| 面板取消 → 无文件、**无隐藏临时文件**、item 进 CANCELLED | ✅(`ls ~/Downloads/.dev.zap.cef-smoke.*` 为空) |
| 改路径保存 → 文件出现在所选路径 | ✅(`~/Documents/…`) |
| 实例稳定性 | ✅ 无 panic/abort |

## 4 踩坑(两条)

### 4.1 "返回 1 但不执行 callback"不会取消,只会把下载卡住

**可观测事实**(2026-09-22 实机,面板点取消两次):

- 页面一直显示"下载中"(用户报告的现象);
- 数据照旧全下载完,落在隐藏临时文件 `~/Downloads/.dev.zap.cef-smoke.<rand>`(单次观测:两个,
  各 446147 字节 —— 现已清理,见 §3);
- 日志只有我们自己的 `download canceled by user`,**没有任何** `on_download_updated` 终态。

参考实现 `CefDownloads.swift` 的注释称"不执行 callback ⇒ callback 销毁时 CEF 取消下载",上游源码里
确有这条逻辑:

```cpp
~CefBeforeDownloadCallbackImpl() override {
  if (!callback_.is_null()) { RunDownloadTargetCallback(std::move(callback_), base::FilePath()); }  // 空路径 = 取消
}
```

**为什么它没生效:未验证推测。** 上游那个 `CefRefPtr<CefBeforeDownloadCallbackImpl> callbackObj` 是
`DetermineDownloadTarget` 的函数局部变量,`handled=true` 时函数直接 `return true`,按理它应当析构并
触发上面的取消;cef-rs 的 trampoline 也在返回时对 callback 参数 `release()`。因此"包装对象未析构"
只是**推断,不是实测结论**(独立审查 I-1 指出:上游源码甚至是反向证据)。可证伪的候选解释还有
"析构跑了但空路径在本 build 不触发取消"。要定案需要一条判别实验(临时在取消分支打点/比对下载库
state),本轮未做。

**修法(不依赖上述机制)**:面板取消时把 id 记进 `CANCELED_DOWNLOADS`,`on_download_updated` 里对标记
id 在 IN_PROGRESS 阶段调 `CefDownloadItemCallback::Cancel()`(CEF 侧 `item->Cancel(true)`)。这条通道有
头文件依据(`cef_download_handler.h:123-131` 明说可在 `OnDownloadUpdated` 里执行 callback 取消),且
上游 `CefDownloadItemCallbackImpl::DoCancel` 只要求 `GetState()==IN_PROGRESS`(同文件 `DoPause` 的注释
明确 TARGET_PENDING_INTERNAL 也映射为外部 IN_PROGRESS)⇒ 面板取消阶段调用有效。
新增的 `[cef] download cancel requested: id=N` 就是这条路径的判定证据。

### 4.2 面板是同步模态,必须抑制消息泵

`on_before_download` 在 CEF 回调内部(即 `do_message_loop_work()` 之内),而泵定时器挂在
`NSRunLoopCommonModes` ⇒ 不抑制会在消息循环里**重入**它,而 CEF 的消息循环工作函数不可重入。
依据:Apple 文档明确 Cocoa 的 common modes 默认含 default / **modal** / event-tracking 三种模式
([Run Loops](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/RunLoopManagement/RunLoopManagement.html)),
故模态保存面板期间该定时器照常触发。`DOWNLOAD_PANEL_OPEN` + RAII guard 让 `pump()` 在面板期间直接返回。

**代价(有意接受,已记入代码注释)**:面板期间 CEF 完全不被推进 —— 所有 webview 的 IPC/重绘、
external begin-frame、隐藏超时冻结(`freeze_hidden_overdue`)都停;这与同步 `runModal` 阻塞 CEF UI 线程
的效果一致,模态期间可接受。

**已知残余风险(未验证推测)**:重入防护只覆盖 `pump()` 一个入口;嵌套 run loop 期间若从其它路径
(`set_bounds`/`was_hidden`/`focus`/`navigate`/`close`)调回 CEF,不在抑制范围内。本轮两次面板操作
无 panic/abort,未观察到问题,但给不出触发输入。

## 5 保存面板后的焦点恢复(2026-09-22,**无 GUI 验证**)

### 5.1 现象(用户实机报告)

导出 → 保存面板弹出 → **点取消**后,页面**收不到键盘输入**(必须先点一下页面);
windowed 与 OSR 两种模式都被报告过。面板弹出期间页面失焦是预期的,问题在"取消之后没还回来"。

### 5.2 三层修复

| 层 | 位置 | 做什么 | 依据 |
|---|---|---|---|
| a. key window 归还 | `browser_web_view.rs` 的 `run_download_save_panel`(**wry/CEF 共用**) | **`activate()` 之前**记 `app.keyWindow()`(Apple 文档:activate 不保证立即生效,激活后再取最需要它的场景反而取到 None);`runModal` 返回后,仅当 `app.isActive()` 时 `previous_key_window.or(mainWindow())` → `makeKeyAndOrderFront(None)` | 面板是 **app-modal**,关闭时 AppKit **不保证**把 key 状态还给原窗口;没有 key window,后面怎么设 first responder 都收不到键盘。**未验证推测**(Apple 只说 activate 不保证立即生效,没有明文说 key 状态不归还) |
| b. 视图/CEF 焦点 | `cef_backend.rs` 的 `restore_focus_after_panel` → `restore_focus_now` + `cef_support.m` 的 `warp_cef_restore_key_focus` | OSR 走 `warp_cef_osr_view_focus`,windowed 走 `[view.window makeFirstResponder:view]`;随后 `host.set_focus(1)` | 面板期间主窗 resign key ⇒ 两种模式的视图都可能不再是 first responder |
| c. DOM 焦点 | 同上,`evaluate(id, "window.__restoreFocused && …")` | 让页面把光标折回输入框末尾 | 与 pane 的 `focus_webview_restoring_input` 同款 —— 面板期间页面的 `activeElement` 也会失焦,内核不会自动还回去 |
| d. 做两次 | `dispatch2::DispatchQueue::main().exec_async(...)` | 立即一次 + 主队列下一拍再补一次 | 面板关闭后 AppKit 还会走一次窗口 key 转换,可能覆盖立即设置的第一响应者;两步幂等(`focus()` 自带"已是 first responder 就不做 0→1 同步"的守卫) |

### 5.3 验证状态(重要)

- **没有任何运行时证据**:用户已明确停止 GUI 测试;此前两版修复(只做 b 的一半 / 做 b+c)**都没有解决**用户报告的现象,所以本节的 a 是新增的机理假设,**未经验证**。
- 已有证据只有:`cargo check` 两种 cfg 0 warning、`cargo nextest -E 'test(cef_backend)+…'` 30/30、`cef_smoke` 构建 + 签名 OK —— **都不覆盖焦点行为**。
- 代码里留了观测点:`[cef] 面板关闭后恢复页面焦点 (id=…, windowed=…, windowed_restore=…, dom=…)`
  以及"弹出前焦点不在页面上、不抢焦点"的那一行。恢复 GUI 测试时先看这两行:都不出现 =
  `restore_focus_after_panel` 没走到;出现但焦点仍未恢复 = 走到了却被覆盖(下一步查 app 是否被弄成非激活)。
- **若仍不生效的下一假设**:面板把整个 app 弄成非激活态(此前日志出现过 `active window changed: None`),那时需要显式 `app.activate()` + 让主窗成为 key window,而不是只恢复 first responder。**未验证推测**。
- **⏸ 2026-09-23 用户决定:此项暂不追,只记录。** 与 `<select>` 弹层同处置(T6 已决定不做,见
  `OSR-PLAN.md:144`)。CEF 已升级到 154.0.23(`CEF-UPGRADE.md` §8),但该路径**在升级后的实机回归里
  同样未被覆盖**(`OSR-T10-DRAG.md` §4:本轮日志没有任何 `download` / `面板关闭后恢复页面焦点` 行)。
  将来要动这项时,从上面那两行观测点开始排查即可。

## 6 遗留(未验证/未做)
- **进度 UI 未做**:目前只有终态日志(与 wry 对齐),没有进度条/完成提示。是否需要由产品决定。
- **文件名截断现象未复现**:首轮实机保存出的名字是 `e7145-…zip`(建议名的尾部,前面少一截,
  原始行为见 `zap.log.old.0` 15:10:37 落盘 `~/Downloads/e7145-53be-4794-bc41-66ddd73c8f8a.zip`),
  本轮同样操作得到的是完整名 `dsh-session-session-8d2e7145-…-8f8a.zip`。无法判定是用户手动编辑还是
  面板行为,记为「未验证」;若再出现,查 `NSSavePanel::setNameFieldStringValue` 与长名的交互
  (该代码 wry 路径同样在用)。
- **取消依赖"面板取消后还会再来一次 IN_PROGRESS 的 `on_download_updated`"**:本轮两次取消都拿到了
  item callback(日志有 `cancel requested`)。若该回调不再来,后果是下载**停在 target-pending**
  (既不取消、也无终态日志),且 id 会留在 `CANCELED_DOWNLOADS` 里(该集合在命中时消费、终态时清理;
  id 是 per-profile 单调 u32,复用误取消实际不可能)。将来若出现"取消后仍卡住",第一个要查的就是
  这条回调是否还发得出来。
- dsh 的导出实现是**服务端端点**(`/api/session.export?sessionId=…&includeDescendants=true`,
  由 `downloads_url_chains` 读到),不是 blob URL —— 这条顺带解答了此前"导出机制未知"。
