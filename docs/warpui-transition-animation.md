# WarpUI 过渡动画实现指南

> 本文记录在实现「垂直标签栏悬浮侧栏的滑出动画」时踩过的坑。核心结论是:
> **WarpUI 没有内建动画能力,唯一的连续动画机制是元素 `paint` 里的 `repaint_after`
> 心跳;而它只触发重绘、不会重跑 `render`。** 这一点没意识到的话,动画会表现为
> "瞬间就结束",而且极难从表象反推。
>
> 姊妹篇:`docs/warpui-layering-and-shadows.md`(渲染层与阴影)。
>
> 适用场景:面板/侧栏滑入滑出、淡入淡出、任何需要逐帧插值的 UI。

---

## 1. 框架没有内建动画能力

搜遍 `warpui_core` / `warpui` / `ui_components`,**没有** `Animation`、`Spring`、
`with_transition`、`Transition`、`ease*`、`lerp`、`interpolate` 这类 API。

仓库里所有连续动画都走同一套机制 —— 在元素 `paint` 末尾请求下一次重绘:

| 位置 | 用途 |
|---|---|
| `app/src/ui_components/spinner.rs:115` | braille 加载动画(最简范例) |
| `app/src/dsh/pane.rs:113` | dsh pane 的省略号动画 |
| `crates/warpui_core/src/elements/shimmering_text.rs:273` | 闪烁文字 |
| `crates/editor/src/render/element/mod.rs:967` | 编辑器光标闪烁 |
| `app/src/terminal/grid_renderer.rs:2081,2163` | 终端动画 |
| `crates/warpui_core/src/elements/image.rs:214` | 动图 |

spinner 的注释把话说得很直白:

```rust
fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
    // 关键:每帧 paint 完请求 80ms 后再次重绘,触发下一帧字符切换。
    // 不调用 repaint_after 则 spinner 静止——这是动画的引擎心跳。
    ctx.repaint_after(Duration::from_millis(FRAME_INTERVAL_MS));
}
```

## 2. 心跳链路

`ctx.repaint_after(d)` 只是"请求 `d` 之后至少重绘一次":

```
PaintContext::repaint_after(d)            // presenter.rs:632,取更早的到期时间
  → PaintContext::repaint_at / 字段 repaint_at
  → Presenter 收集后交给 App::manage_delayed_repaint_timers   // presenter.rs:365
  → self.foreground.spawn(async { Timer::after(..).await })    // app.rs:3407
  → 到期后 window_invalidations[..].redraw_requested = true
  → app.update_windows()
```

注意最后一跳:**它复用的是 async `Timer`**(与 `ViewContext::spawn` 同一条底层)。
只要 `repaint_after` 被调用,链路就是通的(实测有效,光标闪烁/省略号都靠它)。

## 3. 推论 1(最重要):心跳只重绘,不重跑 `render`

**这是整篇文档的核心。**

`repaint_after` 走的是"重建 scene"这条路,**元素树被复用**,`View::render` 不会被
重新调用。实测数据(探针统计一轮 0.26s 的滑出动画):

```
[vtabs-probe3] slide finished: frames=1 paints=141 elapsed_ms=1124
```

- `paints=141`:元素 `paint` 一直在跑,心跳正常;
- `frames=1`:同一轮里 `render` 只跑了 **1 次**。

**后果**:如果动画进度是在 `View::render` 里按时间算的,它永远停在第 1 帧的值上;
下一个真实事件(鼠标移动)到来时才重新 `render`,而那时通常已经超时 → 直接跳到终态。
表象就是"**瞬间就结束 / 只有起点和终点**"。

**正确做法**:动画状态活在**元素内部**,在 `layout` 或 `paint` 里按时间计算:

- spinner:`SpinnerStateHandle`(`Arc<Mutex<Instant>>` 记起始时刻)在 `layout` 里选帧;
- dsh pane 的 `EllipsisText`:同样在 `layout` 里按 `Instant` 算省略号档位;
- 自制滑出动画:`PanelSlideRepaint`(原 `VerticalTabsSlideRepaint`,现由垂直标签栏悬浮侧栏
  与悬浮工具面板共用)持有 `{ started_at, from, to }`,在 `paint`
  里算进度并给 child 的 `origin` 加偏移。

## 4. 推论 2:不要用 `ctx.spawn(Timer::after(..))` 驱动逐帧

最初的实现是每帧 `ctx.spawn(Timer::after(16ms))` 递归调用自己。结果是动画"瞬间
结束",而且有两个额外问题:

1. **定时精度不够**驱动 60fps 级的逐帧推进;
2. **重复触发会累积并行帧链**:每次重新开始动画都在原有链之外再排一帧,于是两条链
   各自每 16ms 再排一帧,重绘次数成倍增长(需要用 `SpawnedFutureHandle::abort()`
   或帧编号去挡)。

改用 `repaint_after` 心跳后这两个问题一起消失。

## 5. 推论 3:跨组件共享的进度必须由元素实时写回

滑动动画的**起点**必须是"面板当前的真实位置"。若进度字段只在 `render` 里更新
(第 3 节已说明它在动画期间几乎不跑),收起时读到的就是一个**滞后值**。实测:

```
render@slide elapsed_ms=8  from=0        to=1   ← 展开开始
render@slide elapsed_ms=8  from=0.0035   to=0   ← 收起开始,起点却是 0.0035
```

一条"0.0035 → 0"的动画当然等于不动,表象又是"收起没有过渡"。

**做法**:进度用 `Rc<Cell<f32>>` 由 `Workspace` 与渲染元素共享,元素每帧 `paint` 时
`set(progress)` 写回;`render` 侧只读。这样动画起点永远接得上,也不需要 `render`
被唤醒。

## 6. 推论 4:`Hoverable` 的延迟不是定时器

与动画同源的一类坑。`with_hover_in_delay` / `with_hover_out_delay` **不会定时翻转**
`is_hovered` —— 它们只用 `ctx.notify_after` 记下到期时间,真正的翻转发生在
**下一次 `MouseMoved` 到达时**(`hoverable.rs::handle_mouse_moved`)。

后果:

- 鼠标移进去**停住不动** → 永远不展开;
- 鼠标移开后没有新事件 → 永远不收起(与"没有过渡"是两类现象,容易混为一谈)。

`is_mouse_over_element` 同样是事件驱动更新的,元素一旦不被渲染(或不再被事件命中),
它就会保持旧值。

**结论**:需要"鼠标停住也生效"的判定,必须**零延迟 + 显式状态**,不要让框架的延迟
去承担时序职责。

## 7. 位移还是宽度

| 方案 | 做法 | 观感 |
|---|---|---|
| 宽度驱动 | `ConstrainedBox::with_width(w * progress)` | 内容随宽度反复重排,像被"挤压"出来 |
| 位移驱动 | 元素在 `paint` 时给 `origin` 加 `-w * (1 - progress)` | 内容只布局一次,整块平移 |

位移更自然。实现上有两点要记牢:

1. **偏移必须施加在 `paint` 的 origin 上**(`child.paint(origin + offset, ..)`)。这样子元素的
   `origin()` 与它记录的命中矩形都会**跟着位移走**,点击位置与视觉位置一致 —— 已核对
   (`Hoverable`/`Dismiss` 的矩形都取自 paint 时的 origin)。若改成"paint 完再改 origin"或
   只在 `render` 里另算偏移,就会变成命中与视觉错开。
2. **位移不改布局**:包装元素的 `size()` 不变,外层 `Stack` 每帧仍按未位移的锚点算位置。
   所以 `layout` / `after_layout` 阶段读到的几何是"未位移"的;动画期间不要用它反推视觉位置。

## 8. 展开 / 收起的判据要对称

动画期间必须继续渲染,否则第一帧就被判成"不需要渲染",面板直接消失。判据要同时覆盖
三种情形:

```rust
let revealed = progress > 0.            // 收起动画尚未走完
    || pinned;                          // 显式打开(点击按钮/快捷键)
```

**不要**再补一个 `|| slide.is_some()`:`slide` 只在 `render` 里被清,而动画结束后没有东西再
触发 `render`,拿它当条件会让面板子树(连同它的 `SavePosition` 位置 id)永久常驻。这条
坑在下面「附加教训 1」里有完整复盘。

只写 `progress > 0` 会出现**不对称**:展开侧被 `pinned` 兜住了所以正常,收起侧却
因为第一帧进度已落终态而瞬间消失 —— 这类"一边有一边没有"的现象,回想本节即可。

## 9. 缓动与时长

- `ease-out cubic`(`1-(1-t)³`)起点速度最大,视觉上像"猛冲出来再急停",**偏生硬**;
- `smoothstep`(`3t²-2t³`)两端速度都为 0,作为滑入滑出更自然;
- **时长要比"看起来够快"再长一点**:debug 构建下整窗重绘成本高,窗口期内可用的
  帧数本来就少,时长短会让动画显得一顿一顿。

## 10. 排查清单

遇到动画不工作时按顺序做:

1. **先分清是"没有帧"还是"帧不对"**。在元素 `paint` 里计数、并在动画结束时打一行
   `render` 计数:两个数字一对比就能区分"心跳没生效"(paints 不涨)与"进度算错了"
   (paints 涨而 frames 不涨)。
2. **确认进度算在哪**。若在 `render` 里,先按第 3 节搬到元素内部,再谈别的。
3. **打印动画的 `from`**。它不等于"动画开始时的真实位置",就是进度没有实时写回
   (第 5 节)。
4. **检查是否误用了 `Hoverable` 的延迟**做时序(第 6 节)。
5. **对照现成实现**。spinner 是最小范例,`EllipsisText` 是"文字+状态"版范例。

**探针用完即删**:定位后立刻移除,不要把调试日志留在提交里。

---

## 附加教训（2026-09 悬浮侧栏实践）

**1. 显隐判据不要依赖"只由 `render` 清理"的状态。**

`revealed` 一开始写成 `progress > 0 || pinned || slide.is_some()`。而 `slide` 只在
`render` 里的那个函数被清 —— 动画结束后没有任何东西再触发 `render`,于是它永远停在
`Some`,面板子树连同它的 `SavePosition` 位置 id(一个窗口外矩形)永久留在元素树里,
还会污染 `tab_bar_rects_for_window` 的拖拽落点判定。

**判据要用元素每帧写回的共享值**(`Rc<Cell<_>>`),不要用只有 `render` 会清的状态。

**2. `offset_from_parent` 的负偏移会被钳制。**

`crates/warpui_core/src/elements/stack/offset_positioning.rs` 在 x 轴会执行
`clamp(parent_rect.min_x(), parent_rect.max_x())`。父级铺满窗口时 `min_x = 0`,所以
"把浮层向左外扩 8px"实际等于 0:探测层覆盖的是 `[0, 14]` 而不是设计中的 `[-8, 6]`。

功能可能照常(命中阈值恰好也是 14),但**注释和常量会与实际行为脱节**,后人按注释改必翻车。
要真正越过父级边界,得换 `WindowByPosition` 之类的定位方式。

**3. 只读审查必须基于冻结的快照。**

同一份改动审过两次,两次都因为"文件在审查期间被并发改写"而部分失效,审查者自己都注明
了"结论以代码内容为准、行号可能漂移"。让审查有效的前提是:**派出去之后到它返回之前,
不要再改被审文件**——所以正确顺序是"改完 → 冻结 → 派审查 → 等结果",而不是边审边改。

## 参考实现

- `app/src/ui_components/spinner.rs` —— 最小的心跳 + 状态句柄写法
- `app/src/dsh/pane.rs` 的 `EllipsisText` —— 起始时刻用 `Arc<Mutex<Instant>>` 跨帧保存
- `crates/warpui_core/src/elements/shimmering_text.rs` —— 同一模式的另一例
- `crates/editor/src/render/element/mod.rs` —— 光标闪烁(无其它事件时也会重绘)
