# T10.1 拖放 / T10.2 焦点:OSR 实机运行证据(2026-09-23)

> 实例:`target/cef-smoke/ZapCEF.app`(PID 27751),启动参数 `--env ZAP_CEF_WEBVIEW=1 --env ZAP_CEF_OSR=1`,
> 日志首行 `render mode = Osr (env ZAP_CEF_OSR="1", setting use_osr_rendering=true)`,视图 1918×954 DIP。
> 日志来源:`~/Library/Logs/zap.log`。**用户结论:测试通过。**
> 本文只记录日志能证明的部分;日志证明不了的显式标注。

## 1 系统 → 页面(验收①:从 Finder 拖文件到 pane)

```
03:24:58 [cef] osr 1: drag enter (827.90625,712.16015625) allowed=0xffffffff
03:24:58 [cef] drag cursor: 页面回报允许操作 0x1
03:24:58 [cef] osr 1: drag drop (843.87109375,757.5546875)
```

- 我们的 OSR 宿主视图**确实收到了拖拽**(`draggingEntered:` → `handle_drag_enter` → `BrowserHost::drag_target_drag_enter`)。
- **页面回报 `0x1`(Copy)= 页面接受了这次拖放** —— 这是"页面到底收没收到"的唯一可观测信号(CEF 只在
  `DragTargetDragOver` 之后回报它)。
- 落点 `(843.9, 757.6)` 在视图范围内(1918×954),与既有鼠标路径的"视图 DIP、左上原点"口径一致。
- **日志证明不了**:页面里是否真的插入了路径 / dragover 是否高亮(需看屏幕)。

## 2 页面 → 系统(验收②:页面选中内容拖出 pane)

```
03:25:12 [cef] start_dragging (771,727) allowed=0xffffffff
03:25:12 [cef] osr 1: drag enter (770,287) allowed=0xffffffff      ← 同一视图既是源又是目标(落在 pane 上)
03:25:12 [cef] drag cursor: 页面回报允许操作 0x0 → 0x1
03:25:13 [cef] osr 1: drag drop (823.3359375,855.9375)
03:25:13 [cef] osr 1: drag session ended (823.3359375,855.9375) op=0x7
03:25:17 [cef] start_dragging (664,766) … drag session ended op=0x7   ← 可重复
```

- 页面发起了对外拖拽会话(`RenderHandler::start_dragging`),会话正常结束(`draggingSession:endedAtPoint:`)。
- `op=0x7` = 接受侧的操作掩码(Copy|Link|Generic)。
- 注:`start_dragging` 的 `x`/`y` 按 CEF 契约是**屏幕坐标**,代码已不再用它定位(见
  [OSR-T10-REVIEW.md](OSR-T10-REVIEW.md) §2 B-I-1);此处数值不用于验证坐标口径。

## 3 T10.2:窗口 key 同步在日志中可见

```
03:21:28 window key=1 → 03:21:29 key=0
03:22:13 key=1 → key=0(伴随平台层 active window changed: None)
03:24:51..56 key=1/0 抖动;03:25:19 key=0
```

机制(窗口 key 变化 → `set_focus(0/1)`)成对出现。**"切走光标停闪 / 切回恢复闪烁 + 中文上屏"的视觉判据
日志证明不了**,本轮由用户整体确认"测试通过"。

## 4 本轮未覆盖 / 仍未验证

- **保存面板 + 三层焦点恢复**:本轮日志**没有任何 `download` 或 `面板关闭后恢复页面焦点` 行** ⇒ 该路径未被覆盖,
  **仍无运行时证据**(见 [OSR-T10-DOWNLOAD.md](OSR-T10-DOWNLOAD.md) §5.3)。
- **成功 drop 之后没有 `drag leave`**:`draggingEnded:` 应补一次 leave,日志里没有。可能原因:leave 的
  trampoline 走 `browser_snapshot` 的 `try_borrow`,那一刻借不到就静默跳过(T8 借用纪律的副作用)。
  **未验证推测**;用户实测未见 dragover 高亮残留(测试通过)。若将来观察到高亮残留,先查
  `cef_backend.rs` 里 `osr_drag_leave_trampoline` 的早返回。
- **源文件是否被移动**:日志不覆盖;掩码已收敛为 `Copy|Link|Generic`(不接受 Move),机理上不应再删除源文件。
- windowed 模式下的下载面板与焦点。
