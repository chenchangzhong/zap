# T10 第二轮独立复审:下载(T10.3)+ 拖放(T10.1)—— 结论与处置(2026-09-22)

> 两名独立审查者(只读、未跑构建、未碰进程),范围互不重叠:
> **A** = 下载 + 保存面板焦点恢复;**B** = 拖放。
> 两人都判 **With fixes、零 Critical**;都强调 **本轮无 GUI 验证**,并把"没有 GUI 就无法确认"的项单列。
> 本文记录:① 两人提出的问题 ② 本轮处置(已修/仅记录)③ 仍无法确认的项。

## 0 前提:验证手段只剩静态/自动化

用户已**停止 GUI 测试**。因此本轮的"已修"**全部没有运行时证据**;可用的证据只有:
两种 cfg `cargo check` 0 warning、`cargo nextest -E 'test(cef_backend)+test(loopback)+test(webview_init)'`
30/30、`script/macos/cef_smoke` 构建 + `codesign --verify --deep --strict` OK。
审查者另外指出:**这些都不覆盖焦点/拖放行为**;`0 warning` 也不覆盖 ObjC(`cef_support.m` 由
`app/build.rs` 经 `cc` 编译)。

## 1 审查者 A(下载 + 焦点)的发现与处置

### 已修

| 编号 | 问题 | 处置 |
|---|---|---|
| A-I-1 | `run_download_save_panel` 在 `activate()` **之后**才读 `keyWindow()`;Apple 文档明说 activate 不保证立即生效(甚至不保证一定激活),而失活 app 没有 key window ⇒ 最需要兜底的场景反而取到 `None`,整层静默失效 | 改为**先取** `keyWindow()` 再 `activate()`;关闭后用 `previous_key_window.or(mainWindow())` 兜底 |
| A-I-2 | `restore_focus_now` **无条件**抢焦点,与仓库既有纪律(`on_load_end`:"仅在本视图仍是 first responder 时才补,免得把用户在终端里的焦点抢走")冲突;后台页面触发的下载会把焦点从终端抢走 | 新增 `page_had_key_focus(id)`:**面板弹出前**快照(OSR 用自建视图、windowed 用 CEF 原生视图的 first-responder 状态),只有为真才恢复;否则记一行"不抢焦点" |
| A-M-1 | 未复用 `focus()` 的 0→1 补同步(cef#3870:只 `set_focus(1)` 不够),正是"设了焦点但收不到键"的形态 | OSR 改走 `focus(id, true)`(含自建视图 `makeFirstResponder` + 0→1 补同步);windowed 另补原生视图 `makeFirstResponder` |
| A-M-2 | 注释里的幂等依据(`focus()` 自带守卫)指向不存在的代码;且 `__restoreFocused` 自带重试链,**重复调用会开第二条链**并可能折叠用户此时的选择 | 注释改为"AppKit 的 `makeFirstResponder` 对已是 first responder 是 no-op;`focus()` 自带守卫";延迟那一拍**不再重复 DOM 恢复**(`with_dom_focus=false`) |
| A-M-4 | `makeFirstResponder:` 返回值被丢弃,失败无法察觉 | `warp_cef_restore_key_focus` 改为返回 `i32`(1=成功/已是 first responder),Rust 侧记进日志 |
| A-M-5 | (a) 缺 `app.isActive()` 守卫;且它改的是 **wry/CEF 共用函数**(严格按"wry 行为不得改变"属越界) | 加 `if app.isActive()` 守卫;越界性在本文与 OSR-PLAN 显式记录(不静默改 wry) |
| A-M-6 | 焦点恢复排在"提交下载决定"(`callback.cont` / 记取消标记)**之前**;而 §4.1 已证明"callback 未执行 = 停在 target-pending"是本路径最贵的失败模式 | 移到 `match` **之后** |
| A-M-7 | OSR 下 `native_view` 兜底与 `detach_view` 的注释自相矛盾;若将来 OSR 设上 `parent_view`,兜底会把 warpui 的容器视图设成 first responder | `native_view()` 在 `render_mode() == Osr` 时**直接短路**返回 null |
| A-M-8 | 文档把假设写成事实;`OSR-T10-DOWNLOAD.md` §6"文件名截断"整段重复 | §5.2 依据列标注「未验证推测」;删掉重复段(保留更完整的一条) |

### 仅记录(未改)

| 编号 | 问题 | 为何不改 |
|---|---|---|
| A-M-3 | 延迟一拍(`exec_async`)的收益无证据,可能纯冗余 | 源码级已确认**安全**(闭包只捕获 `u64`,`browser_snapshot`/`with_browser` 均 `try_borrow` + 句柄克隆 ⇒ 对已销毁 webview 是 no-op;OSR 指针先置空再释放 ⇒ 无悬垂)。留作"覆盖竞态"的保险,待 GUI 验证时判定 |
| A-R-1 | 更稳的结构性改法:不用 `app.keyWindow()` 猜窗口,改用"发起下载的 webview 自己的 NSWindow"(传参给共用面板函数);更彻底是改用 sheet | 需要改共用函数签名(wry 侧同步 handler 也要安排),属独立一轮;本轮先用零成本守卫把风险压住 |

## 2 审查者 B(拖放)的发现与处置

### 已修

| 编号 | 问题 | 处置 |
|---|---|---|
| B-I-1 | destination 恒返回"来源全掩码" = 声明接受 Copy\|Move\|Link ⇒ 同卷文件从 Finder 拖入时 Finder 默认走 **Move**,页面即使拒收也可能**删掉源文件**(数据丢失) | 新增 `warpDragAcceptedOps()`:`draggingEntered/Updated` 收敛为 `Copy\|Link\|Generic`(来源只给别的操作时兜底 Copy)。依据:网页不消费 Move 语义;windowed 下 Chromium 原生视图给的就是 Copy(实机光标为绿色 +)。**未验证推测** |
| B-I-3 | `start_drag` 注释把"鼠标位置"与 CEF drag start 写成"等价" | 改为**近似等价、并非严格等价**:晚一个拖拽阈值 + IPC/60Hz 泵延迟;鼠标移出视图时拖拽框会落到可见区外(AppKit 以可见区裁剪) |
| B-M2 | `LAST_DRAG_ALLOWED` 用 `u32::MAX` 当哨兵,与合法值 `DRAG_OPERATION_EVERY` 撞值 ⇒"每次拖拽至少一行"不成立;且只在 target 方向重置,page→system 会串味 | 改 `Option<u32>`;`start_dragging` 里也重置 |
| B-M5 | `OSR-PLAN.md` 把 `drag_target_*` 记在 `RenderHandler` 名下 | 改为 **`BrowserHost`**(仅 `start_dragging`/`update_drag_cursor` 属 `RenderHandler`),并注明"归属写错过两次" |
| B-M6 | "OSR 视图遮蔽终端拖放路径"这条上一轮结论在规格里 0 落笔 | 写入 OSR-PLAN 待验证表(限定在 webview 矩形内;webview 隐藏时不参与命中) |
| B-M7 | 测试里 `as u32` 恰好掩盖 `Every` 两侧的宽度差异 | 加注释说明:断言证明的是"低 32 位相同",生产侧 ObjC 边界同样按 `uint32_t` 传,属良性 |

### 仅记录(未改)

| 编号 | 问题 | 为何不改 |
|---|---|---|
| B-I-2 | `setDraggingFrame:contents:nil` 按 `NSDraggingItem` 契约是"隐藏该项" ⇒ 页面→系统拖拽**没有拖拽图像**(代码与规格都没写) | 记为**有意接受**(与参考实现同款),写进 OSR-PLAN;是否给一张真实 32×32 图属产品/观感决定。注意参考实现该处注释写成 "placeholder image",与头文件相反,勿照抄 |
| B-M1 | `(void)x; (void)y;` 与本仓"未使用参数直接删除"的规矩冲突 | 二者是 FFI 入参,删掉要改 C ABI 与 Rust 声明;保留并在注释里写明"Rust 侧要打日志留证" |
| B-M3 | `drag over` 完全没有日志通道 ⇒ "高亮是否延迟"无证据 | 需要时再加 debug + 限流(避免每帧刷屏) |
| B-M4 | `update_drag_cursor`(target 方向回报)的值被写进 `_dragAllowedOps`,供 source 方向的 `sourceOperationMaskForDraggingContext:` 读取;同时 destination 返回值又忽略页面判定 | **未验证推测**(本机无 libcef 源码,只有头文件一句描述)。属跨方向复用同一字段的设计缺陷,需要 GUI 才能判;记入待验证表 |
| B-M8 | `draggingEntered:` 未立即补 `drag_target_drag_over`(参考实现会立即发) | 最坏只是高亮晚一个周期(AppKit 随后周期性发 `draggingUpdated:`);加一行是零风险,但属行为变更,本轮不动 |

## 3 没有 GUI 就无法确认的项(汇总)

1. (a) `runModal` 后主窗是否真的失去 key;`activate()`/`keyWindow()` 时序;`makeKeyAndOrderFront` 是否还回正确窗口。
2. (b) 面板期间/关闭后 OSR 视图是否仍是 first responder(机理 A/B 未判);`makeFirstResponder` 是否 no-op;windowed 视图是否接受 first-responder。
3. (c) `__restoreFocused` 是否真折回光标。
4. (d) 延迟一拍是补偿了竞态,还是纯冗余。
5. 焦点恢复的"抢焦点"触发输入是否真实存在(后台页面触发的下载)。
6. windowed 模式下的下载面板与焦点(只验过 OSR 下的下载行为)。
7. `DOWNLOAD_PANEL_OPEN` 抑制泵超过冻结阈值(默认 300s)时长开面板的时序后果。
8. 拖放:回调是否被调用、页面是否收到 enter/over/drop 与落点、高亮是否延迟、page→system 会话能否开起来(定时器回调里 `[NSApp currentEvent]` 很可能为 nil)、无拖拽图像的实际观感、鼠标移出视图时越界 `draggingFrame` 是否仍有效、**页面拒收时源文件是否被移动/删除**(掩码收敛后应已消除)、`update_drag_cursor` 的真实方向/时机。
9. T10.1/T10.2 的验收判据本身(见 OSR-PLAN §3.5 各自小节)。

## 4 判定

- 两位审查者的结论都是 **With fixes、零 Critical**;本轮已按上表修完 A 的 2 条 Important + 6 条 Minor、
  B 的 1 条 Important(数据风险)+ 4 条 Minor,其余按"仅记录"处理并写明理由。
- **不得据此宣称 T10.1/T10.3 完成**:已修项中凡标注"未验证推测"的都没有运行时证据;
  T10.1/T10.2 的实机验收仍是零证据。
