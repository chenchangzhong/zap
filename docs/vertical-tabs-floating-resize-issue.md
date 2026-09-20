# 悬浮侧栏拖拽改宽:根因、修正与残余风险

> **状态:两个缺陷均已修正并实测通过(2026-09-20),改动在当前工作树、尚未提交。**
>
> 本文**替换** 2026-09-20 早前版本的结论。早前版本把根因归为 `Stack` 的 `if child.painted`,
> 那条结论已证伪(见 §5),据此提出的"View 化改造"修不了本现象,不要再照做。
>
> 涉及:`app/src/workspace/view.rs`(悬浮浮层 + 贴边 hover 探测层)、
> `app/src/workspace/view/vertical_tabs.rs`(`Resizable` 与面板内容)。
> 行号按当前工作树(2026-09-20 19:2x),同一文件有并行改动时以符号名为准。

---

## 1. 现象

1. 悬浮展开后,按住面板右边缘拖拽**完全无反应**(停靠态拖拽一直正常);
2. 缺陷 1 修好后暴露出的第二个:**拖宽可以,往回拖会触发收起**,无法改小。

---

## 2. 根因

### 2.1 拖拽完全无反应 —— 贴边探测层把 `drag` 吃掉了

- 悬浮态在面板之后又加了一层贴边 hover 探测层(`view.rs:22975-23037`,`Hoverable` + `Empty`,
  展开时宽度 = 面板宽 + `2 * VERTICAL_TABS_PROBE_MARGIN` = 面板宽 + 16,恰好盖住面板右边缘的拖拽带);
- `Hoverable` 默认 `suppress_drag = true`(`hoverable.rs:208`);`LeftMouseDown` 命中它时会无条件写下
  `click_count`(`hoverable.rs:619`),随后每一条 `LeftMouseDragged` 都会命中
  `if suppress_drag && is_clicked() { return true }`(`hoverable.rs:675-677`);
- 外层 `Stack::new()` 在 **debug** 构建下走 `Waterfall`(`stack/mod.rs:103-106`),而 `Waterfall` 是
  **逆序**派发且首个 `true` 即返回(`stack/mod.rs:313-321`);探测层加在面板之后(`view.rs:23041` vs
  面板 overlay 的 `view.rs:22938`)→ `drag` 先到探测层就被截断,**面板子树(含 `Resizable`)永远收不到**;
- 于是呈现"按下有反应、拖拽一条都不到"的签名 —— 与探针观测完全一致(§4)。

两个对照点,用来解释"为什么只有这里坏":

| | 停靠态(正常) | 悬浮态(原缺陷) |
|---|---|---|
| 面板右边缘的 `Resizable` | 有,同样内联建在 `Workspace::render` 里 | 有 |
| 贴边 hover 探测层 | **没有** | 有,且宽度盖住拖拽带 |
| `Stack` 派发模式 | debug = `Waterfall` | debug = `Waterfall`,探测层先被派发 |

release 构建下 `Stack` 走 `Broadcast`(`stack/mod.rs:106`),探测层返回 `true` 不影响兄弟节点 →
这是**仅 debug 现形**的缺陷(本机 `target/` 下只有 debug 构建)。

### 2.2 往回拖就收起 —— 合成 `MouseMoved` 用的是过期坐标

- 框架每次重绘后会补发一条合成 `MouseMoved`(`app.rs:2819-2827`),它的坐标取"最后一次**真实**
  `MouseMoved`"——`set_last_mouse_move_event` 只认 `MouseMoved`(`app.rs:2258-2267`),而拖拽期间
  指针只发 `LeftMouseDragged`;
- 探测层宽度跟着面板一起变(`view.rs:22975-22981`):**拖宽**时它变大,过期坐标仍在里面(所以没事);
  **往回拖**时它变小,过期坐标落到探测层外 → 合成 hover out → `on_hover(false)` →
  `WorkspaceAction::VerticalTabsAutoHideCollapse` → 收起;
- 阈值很具体:`w + 16 < 按下前的指针 x`,即"拖回到原宽度 −16px 以下"就收起。

---

## 3. 修正(2 行 + 注释)

`app/src/workspace/view.rs` 探测层:

```rust
.with_propagate_drag()          // view.rs:23023
.with_skip_synthetic_hover_out()// view.rs:23037
```

- `with_propagate_drag()`:让探测层不再吞 `drag`(与 pane 分隔条的 hover 目标同一处理,
  `app/src/pane_group/tree.rs`)。停靠态无探测层,release 语义不变;
- `with_skip_synthetic_hover_out()`:让合成事件不触发 hover out(与同文件 tab 行 hover 同一处理,
  `vertical_tabs.rs:429`)。**真实** `MouseMoved` 的 hover out 照常生效,自动隐藏主路径不受影响。

---

## 4. 证据

**旧探针(已删)**:`crates/warpui_core/src/elements/resizable.rs` 的 `dispatch_event` 里
`[rz-probe] down … dragbar=Some(<249,41,254,985>) hit=true`,之后 drag 计数 **0**;
`crates/warpui_core/src/presenter.rs` 的 `dispatch_event_on_view` 里 down 5 条 / drag 195 条,
两侧 view 集合相同且 `rendered=true` → 事件确实进了 warpui、确实派发到了同样的视图集合,
断点在**元素树内部**。

**修正后临时探针(标签 `[vt-tmp]`,已删)** —— 11:28:10 → 11:28:15:

```
11:28:10  probe hover=true  pos=(0.00, 444.87)    w=472.81  progress=0   ← 展开(此前已拖宽到 472.81)
11:28:15  probe hover=true  pos=(244.95, 533.58)  w=239.19  progress=1   ← 宽度已缩到 239.19
11:28:15  probe hover=false pos=(272.37, 534.48)  w=239.19  progress=1   → collapse(正常收起)
```

宽度从 472.81 缩到 239.19 期间**一条 `probe hover=false` 都没有**(合成 out 被跳过),
却仍出现了一次 `hover=true` 的**状态转移**(244.95 在探测层 `[0, 255.19]` 内、`progress` 已是 1)——
只有 `is_hovered` 当时是 `false` 才可能有这次转移。这直接证明"跳过的是回调、状态仍被写成 `false`"
(见 §6)。

**用户侧最小验证**:悬浮展开 → 拖宽 → 松手 → 再按住边缘往回拖,宽度可连续改小;移开面板仍自动收起。

**回归**:`cargo nextest run -p warp -E 'test(vertical_tabs)'` → **70 passed / 4109 skipped**;
`cargo build --bin zap-oss` → exit 0。

---

## 5. 已证伪的旧结论(不要再按它改)

旧版本把根因归为 `crates/warpui_core/src/elements/stack/mod.rs:308,317` 的 `if child.painted`:
"`Workspace` 重渲染后 `panel_layer` 是新实例(`painted = false`),紧接着的 `LeftMouseDragged` 被挡掉"。

不成立,理由:

1. `Presenter::invalidate` 只在 `build_scene` 内被调用(`app.rs:2787`),紧接着同一函数里 layout + paint
   (`child.painted = true`,`stack/mod.rs:288-289`)。两者是**同一同步过程中的一对**,中间不派发任何
   鼠标事件 → 不存在"新实例 `painted = false` 时正好被 drag 撞上"的窗口;
2. `down` 能命中就说明 `dragbar.bounds` 已被 `Resizable::paint` 写过(该字段只在 paint 里写),
   即子树确实被画过;
3. 停靠态的 `Resizable` **同样**内联建在 `Workspace::render` 里(`render_config_panel`),按该理论
   也该失效,但它一直正常 —— 该对照表抓错了差异(真正的差异是"有没有贴边探测层");
4. `Presenter::dispatch_event_on_view` 的 `else { false }` 分支与平台层都已被探针排除,这点旧文档是
   对的,只是结论停错了地方。

**推论**:旧文档 §6 的"把 `Resizable` 搬进独立持久 View"改造**修不了本现象** —— 拦截点在面板子树
**之上**(外层 `Stack` 的兄弟节点),拖拽根本进不到那个 View。

---

## 6. 残余风险(已知,当前不修)

`Hoverable::handle_mouse_moved_without_delay` 是**先写状态、再判跳过**:`is_hovered` 在
`hoverable.rs:492` 已被置为 `false`,`:511` 才 `return` 掉回调。于是:

- 收缩越过 §2.2 的阈值后,探测层状态停在 `false`(回调没发,面板正确保持展开);
- 此时若松手后**第一条真实 `MouseMoved` 直接跳到探测层之外**(单次事件跨 >11px 的"甩出去"),
  那次真实 hover out 会被 `hoverable.rs:488-490` 的 `was_hovered == is_hovered` 早退吞掉,
  面板需"移回探测层内再移出"或点面板外(`Dismiss`)才收起。

不改框架公共路径的原因:

- 把 `is_hovered` 改回 `true` 会破坏框架有意为之的取舍(状态更新让视觉 hover 跟得上布局变化,
  回调跳过避免副作用),会让 tab 行 / 会话列表出现"滚出指针下的行仍保留 hover 高亮";
- 根治做法是让 `LeftMouseDragged` 也更新 `last_mouse_moved_event` 的坐标(`app.rs`),合成事件在拖拽期
  不再用过期的按下前坐标 —— 但这会改变**拖拽期间全应用**的 hover 语义,需要单独一轮回归。

**结论:保留现状**。用户侧多轮测试(含按 §8 专门构造"拖窄越线后甩鼠标")**未能复现** —— 需要
"松手后第一条真实 `MouseMoved` 就跳出探测层"这个很窄的条件;而且有三条兜底路径(点面板外、
移回探测层内再移出、重开面板),不值得为它改框架公共路径。机制与取舍已由
`warpui_core` 的 `test_skip_synthetic_hover_out_ignores_synthetic_mouse_moved` 钉住(见 §9)。

---

## 7. 工作树里与本功能相关的既有改动(保留)

以下三项早前为追查本缺陷而改,虽不构成本缺陷的根因,但都是与悬浮工具面板对齐的合理改动,予以保留:

- `render_vertical_tabs_panel` 不再对悬浮态 early-return `ConstrainedBox`,两种模式共用同一个
  `Resizable`(`vertical_tabs.rs:1518`);
- 删除 `FLOATING_MIN_PANEL_WIDTH`,宽度下限与停靠态统一为 `MIN_PANEL_WIDTH`(200)
  (`vertical_tabs.rs:662-670`);
- 面板外层由 `Clipped` 改为 `Stack::add_overlay_child`(与悬浮工具面板一致,面板内的弹出菜单不再
  被面板矩形裁掉,`view.rs:22938-22953`)。

同时仍有几处注释与现状不符(下次顺手改):
`view.rs:21311-21312`("悬浮态已经不提供改宽…`is_resizing` 那条守卫是死分支已删" —— 现在提供改宽了)、
`view.rs:22936`("`Clipped` 为面板内容新开一层")、
`vertical_tabs.rs:1516`("拖拽只 notify 本视图" —— 实际 `Resizable` 内联建在 `Workspace::render` 里,
`ctx.notify()` 通知的是 `Workspace`,**这正是旧文档错误根因的种子**)。

> 上列 4 处已于 2026-09-20 改完(含 `vertical_tabs.rs:661` 的宽度下限说明)。

---

## 8. 验证方法

用户侧最小验证(一条):

1. 鼠标移到侧边 → 面板悬浮展开;
2. 按住右边缘**拖宽** → 松手;
3. 再按住边缘**往回拖** → 宽度应连续改小(缺陷 2 的验收点);
4. 移开鼠标 → 应自动收起;点面板外 → 应收起。

需要日志时,临时在这几处插 `log::info!("[vt-tmp] …")`,判读完即删:

- 探测层 `on_hover`(`view.rs:22987` 起):`hover / pos / w / progress`;
- `VerticalTabsAutoHideReveal` / `Collapse` 分支(`view.rs:21299` / `:21305`);
- 面板 `Dismiss::on_dismiss`(`view.rs:22952`)。

---

## 9. 遗留

- 回归用例已补(2026-09-20,`cargo nextest run -p warpui_core` 294 passed):
  - `elements::stack::tests::test_hover_probe_added_last_swallows_sibling_drag_without_propagate`
    —— 钉住陷阱本身:显式 `Waterfall` 的 `Stack` 里,后加、默认 `suppress_drag` 的 `Hoverable`
    会把兄弟节点(真实 `Resizable` 拖拽带)的 `drag` 全部吃掉(按下 1 次回调、拖拽 0 步);
  - `elements::stack::tests::test_hover_probe_with_propagate_drag_lets_sibling_receive_drag`
    —— 钉住解法:加 `with_propagate_drag()` 后,两步 drag 都落到拖拽带上(共 3 次回调);
  - `elements::hoverable::tests::test_skip_synthetic_hover_out_ignores_synthetic_mouse_moved`
    —— 钉住 `skip_synthetic_hover_out` 的行为与取舍:合成 `MouseMoved` 不触发 hover out,
    但 `is_hovered` 仍被置 false,随后的真实 hover out 被早退(§6 的残余风险);
    **若将来在框架侧改掉这个取舍,这条断言会失败**,提醒一并更新本文。
  - 覆盖面边界:三条用例都钉的是**框架机制**,不是 `app/` 里探测层那两行的接线;后者只有
    §8 的用户侧验证。
- §6 的根治(拖拽更新鼠标位置)未做 —— 用户侧难以复现,已决定保留现状。
- 另一条**未验证推测**(不进待修清单):面板的 `SavePosition` 用默认的 indefinite 缓存
  (`presenter.rs:168` 写 `committed_positions`,全仓只有 x_ray 调过 `clear_position`),
  而 `element_position_by_id_at_last_frame` 读的正是这份缓存 → 悬浮面板**收起后**它那块矩形
  可能仍被 `tab_bar_rects_for_window`(`view.rs:25131`)当作"标签栏等价区域"返回,理论上影响
  拖拽标签页的落点判定。已确认的只有"矩形留在缓存里"这一点;"造成可见问题"**未观察到**,
  触发条件与验证方法(在 `tab_bar_rects_for_window` 插一条日志,面板展开后收起再看返回值)
  也尚未执行。
