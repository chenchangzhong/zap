# WarpUI 渲染层与阴影机制

> 本文记录在实现「悬浮编辑器浮层」时踩过的渲染层问题。这些机制在框架里没有集中文档,
> 只看单个元素（`Stack` / `Clipped` / `Dismiss` / `Container`）的源码很难拼出全貌,
> 而误判的代价是反复试错。
>
> 适用场景:自制浮层 / 模态面板 / 下拉菜单 / 任何"要盖在别的内容之上"的 UI。

---

## 1. 两套层数组,绘制顺序固定

`Scene` 维护**两个独立的层数组**（`crates/warpui_core/src/scene.rs:22-23`）:

```rust
layers: Vec<Layer>,          // 普通层
overlay_layers: Vec<Layer>,  // overlay 层
```

绘制顺序**先全部普通层、再全部 overlay 层**,与创建时间无关
（`crates/warpui/src/platform/mac/rendering/metal/renderer.rs:407-414`）:

```rust
for layer in self.scene.normal_layers() { self.draw_layer(layer, &platform_view_holes); }
// Overlay layers (menus, modals) render above platform views; no holes.
for layer in self.scene.overlay_layers() { self.draw_layer(layer, &[]); }
```

**推论 1**:只要一个元素落在 `overlay_layers`,`normal_layers` 里的任何东西都盖不住它 ——
反过来也一样。

## 2. 层索引只增不减

`push_*_layer` 用**各自数组的当前长度**作为 z-index
（`crates/warpui_core/src/scene.rs:534-544`）:

```rust
fn push_overlay_layer(&mut self, layer: Layer) {
    self.active_layer_index_stack.push(ZIndex::Overlay(self.overlay_layers.len()));
    self.overlay_layers.push(layer);
}
```

**推论 2**:在**同一个层数组内**,先绘制的元素索引必然更低。
后绘制的兄弟内容永远盖在先绘制的之上,无法靠"提高层级"翻盘 —— 只能**延后绘制**。

## 3. 进入哪个数组取决于当前活动层

`start_layer` 会看当前活动层决定去哪
（`crates/warpui_core/src/scene.rs:494-501`）:

```rust
pub fn start_layer(&mut self, bounds: ClipBounds) {
    let layer = self.create_layer(bounds);
    match *self.active_layer_index_stack.last() {
        ZIndex::Normal(_)  => self.push_normal_layer(layer),
        ZIndex::Overlay(_) => self.push_overlay_layer(layer),   // ← 注意
    }
}
```

**推论 3（最容易踩的坑）**:一个 overlay 容器内部的**所有**后代（包括它们自己开的
`start_layer`）都会继续落进 `overlay_layers`,于是**内部元素之间只能按索引比高低**。

### 这解释了一个常见疑问:为什么同样结构的菜单,在普通标签页里正常,在浮层里就被盖住?

| | 常规标签页 | 自制的 overlay 浮层 |
|---|---|---|
| 容器加入方式 | `stack.add_child(...)` → 普通 child | `add_positioned_overlay_child(...)` → overlay child |
| 容器内容所在 | `normal_layers` | `overlay_layers` |
| 内部菜单所在 | 菜单是 `Stack` 的 overlay child → `overlay_layers` | 同左,**但和内容挤在同一个数组** |
| 结果 | 内容 normal、菜单 overlay → 渲染器先画完 normal → **菜单天然在最上** | header 先绘制(索引低)、编辑器后绘制(索引高) → **菜单被盖** |

## 4. 浮层该用哪种 child?两者都不能少

- **用普通 `add_positioned_child` 反而更糟**:浮层会落进 `normal_layers`,而主内容
  (`panels`)内部若有 overlay 层（菜单、下拉等）,它们在 `overlay_layers` →
  渲染器后画 → **浮层被内容盖住**,表现为"浮层透明且点不动"。
- **必须用 `add_positioned_overlay_child`**,让浮层进 `overlay_layers`。

代价就是推论 3:浮层内部元素之间只能比索引。

## 5. 解决方案:把需要置顶的元素**延后绘制**

在 overlay 上下文里,想让 A 盖住同层的 B,唯一办法是让 A 在 B **之后**创建层。

实际做法（`app/src/pane_group/pane/view/mod.rs` 的 `with_elevated_overflow_menu`）:
把 pane 头部的下拉菜单从 header 内部**取出来**,放到整棵 pane **之后**作为 overlay child:

```rust
let mut outer = Stack::new().with_child(element);        // element 已含 header + 内容
outer.add_positioned_overlay_child(
    ChildView::new(header.overflow_menu()).finish(),     // 最后才创建层 → 索引最高
    OffsetPositioning::offset_from_save_position_element(/* 按钮位置 */),
);
```

配套两点(缺一不可):
1. header 侧要**停止自己渲染**菜单,否则会画两遍（用 `overflow_menu_rendered_by_host` 标记)。
2. **响应者链会断**:菜单的父视图从 `PaneHeader` 变成了 `PaneView`,菜单项派发的
   `PaneHeaderAction` 找不到处理者（日志表现为
   `Action ... was dispatched, but no view handled it`）。
   需要让 `PaneView` 接住并转发:

   ```rust
   impl<P: BackingView> TypedActionView for PaneView<P> {
       type Action = PaneHeaderAction<P::PaneHeaderOverflowMenuAction, P::CustomAction>;
       fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
           let header = self.header.clone();
           header.update(ctx, |header, ctx| TypedActionView::handle_action(header, action, ctx));
       }
   }
   ```

   这类"提升渲染"必然改变响应者链,`dispatch_typed_action` 沿视图树**向上**找处理者,
   而 header 是 `PaneView` 的**子**视图,不提升就收不到。

3. 提升渲染依赖 `SavePosition` 的位置 id:父 `Stack::paint` 按 child 顺序处理,
   child[0]（含 header）先 paint 并写入位置缓存,child[1]（菜单）随后即可查到
   (`presenter.rs` 的 `SavePosition::paint` → `cache_position_indefinitely` → `end()`)。

## 6. 菜单项被选中时"文字看不见":优先查层级,不要先怀疑布局

同一类症状（"菜单项选中后看不见文字"）可能来自两个完全不同的原因,排查顺序很重要:

| 原因 | 判断方法 |
|---|---|
| **层级被盖**(本文主题) | 取消选中（移动鼠标）后文字是否恢复;背景色是否像是"别的元素"的颜色 |
| 布局/颜色 | 打印 `bg` / `fg` / `text_bg` 的实际色值,看对比度是否正常 |

本次先猜了布局（把 `LabeledText` 换成 `StackedText`）,方向错误;打印颜色后发现
`bg=#3994bc`（蓝）与 `fg=rgba(25,26,27,0.9)`（深）对比度完全正常,才转向层级。

**教训**:先打色值与层索引这两个数字,再决定改哪里。

## 7. 阴影:支持,但有"可见空间"约束

### 7.1 传递链

```
Container::with_drop_shadow(DropShadow)
  → Container::paint: scene.draw_rect_with_hit_recording(背景矩形).with_drop_shadow(..)
  → Scene: layer.rects.push(Rect { bounds: 背景矩形, drop_shadow })   // bounds 就是背景矩形本身
  → 渲染器: 为阴影单独提交一个**外扩**的 quad
```

渲染器（`renderer.rs:673-676`）:

```rust
let padding     = drop_shadow.spread_radius * self.scene.scale_factor();
let shadow_origin = bounds.origin() + drop_shadow.offset * scale_factor - padding;
let shadow_size   = bounds.size() + vec2f(2. * padding, 2. * padding);   // 向外扩 padding
```

fragment shader 再从该 quad **向内收** `padding` 还原出阴影本体 —— 正好等于背景矩形
（`shaders.metal:230-236`）:

```glsl
float2 shadowed_rect_origin = in.rect_origin + in.drop_shadow_padding_factor;
float2 shadowed_rect_size   = in.rect_size  - 2 * in.drop_shadow_padding_factor;
color.a *= roundedBoxShadow(shadowed_rect_origin, ..., in.drop_shadow_sigma, outer_corner_radius);
```

### 7.2 关键约束

**阴影只在这个外扩 quad 的范围内可见**,而 quad 受所在 layer 的 scissor 裁剪
（`renderer.rs:417-437`)。

**推论 4**:如果卡片大到几乎占满可用区域,阴影外扩的那一圈就**没有落脚空间** ——
大部分落到窗口外,表现为"完全看不到阴影"。放大 `alpha` / `blur` 都无济于事,因为
问题不在参数而在几何。

本次实证:卡片 90% 窗口时只有下边缘露出一线（表现为"下面两个角有黑色直角"——
那其实就是阴影在工作)。

### 7.3 正确做法:容器铺满、内容内缩

```
Align                      ← 铺满 100% 可用区域,负责居中
  └─ Percentage(0.9, 0.9)  ← 卡片占 90%
       └─ Container(背景 + 边框 + 圆角 + 阴影)   ← 四周各 5% 是留给阴影的空间
            └─ Clipped(内容)                    ← 只裁内容,不影响父级阴影
```

要点:
- 带阴影的容器**必须与卡片同尺寸**。若让它在 `Percentage` 外面自由撑开,它会拿到
  接近全屏的尺寸,阴影 quad 随之变成全屏矩形、与卡片不对齐。
- `Clipped` 只能放在**内层**。它会给子树新开裁剪层,放外层会把阴影整片裁掉。

### 7.4 默认值偏淡

`DropShadow::default()` 走 `new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 32))`
—— alpha 只有 32/255(约 12%),是为小控件（按钮、tooltip）调的。大面积面板需要自己
指定 `color` / `offset` / `blur_radius` / `spread_radius`。

## 8. 原生磨砂不等于元素级背景模糊

窗口的磨砂在**原生 NSWindow 层**实现
（`crates/warpui/src/platform/mac/window.rs:580-603`,通过 `background_blur_radius_pixels`
传给 `create_warp_nswindow`),是整窗背后的一层材质。

仓库**没有**元素级背景模糊能力(`warpui_core/src/elements` 下无 backdrop 类元素,
Metal 渲染器只有阴影模糊)。

**因此**:想做出"局部磨砂"效果,只能让元素**半透明**,透出窗口那层磨砂 ——
终端就是这么做的（`app/src/workspace/util.rs` 的 `get_terminal_background_fill`):
`theme.background().with_opacity(terminal_opacity)`。

若要真正的局部背景模糊,需要在 Metal 渲染器里新增能力(读回背景 buffer + 高斯模糊),
属渲染管线级改造,需单独评估性能。

## 9. 重入:`Circular view update`

`AppContext::update_view` 会**先把 view 从 `window.views` 取出**再执行闭包
（`crates/warpui_core/src/core/app.rs:~4430`）:

```rust
let mut view = if let Some(window) = self.windows.get_mut(&window_id) {
    if let Some(view) = window.views.remove(&handle.id()) { view }
    else { panic!("Circular view update"); }        // ← 闭包内再 update 同一个 view 就到这里
}
```

**推论 5**:在一个 view 的 `update` 闭包内,不能再 `update` 同一个 view。
回调用 "延后到安全时机" 的方式绕开 —— `ViewContext::dispatch_typed_action_deferred`
（`crates/warpui_core/src/core/view/context.rs`,文档明确写着"used to avoid re-entrant
view updates")。

本次实证:确认框回调里直接销毁浮层（销毁要 update 那个 `CodeView`)→ panic。

## 10. 键盘 Esc 与"焦点优先"的键绑定

键绑定匹配是**焦点优先**的:获得焦点的 `EditorView` 会先吃掉 Esc,画在元素树上的
外层捕获层根本收不到。

**让位机制**:编辑器在 `keymap_context` 里检查"本窗口是否有模态浮层占据 Esc",
若有则给自己的 escape 绑定加一个否定上下文标识:

```rust
// app/src/editor/view/mod.rs 与 app/src/code/editor/view.rs
if escape_owner(self.window_id) != EscapeOwner::None {
    context.set.insert("...EscapeYieldsToHost");
}
// 绑定侧
FixedBinding::new("escape", Action::Escape, ctx_id & !id!("...EscapeYieldsToHost"))
```

**两个容易漏的点**:

1. **代码编辑器有自己的一套 escape 绑定**(`app/src/code/editor/view/actions.rs`
   的 `CodeEditorView`,上下文是 `CodeEditorView`),与 `EditorView` 是两套。
   只改一处会出现"终端里 Esc 生效、代码编辑器里不生效"。
2. **这会让所有编辑器让位**,作用域是整个窗口 —— 是"浮层必须能用 Esc 收起"所必需的
   代价,但要有意识地接受它（参考 `refactor(tool-panel): 收敛 Esc 让位机制`
   那次提交曾专门删掉类似机制,理由是会改变终端 Esc 行为)。

## 11. 事件派发顺序:debug 与 release 不同

`Stack::new()` 在 `cfg!(debug_assertions)` 下用 `Waterfall`(逆序、命中即停),
否则用 `Broadcast`(**正序且不停止**)。

**推论 6**:同一个 `Stack` 上挂多个捕获同种按键的 overlay child 时,**release 下会全部收到**。
需要显式让位(在回调里判断"另一个浮层是否开着",开着则 `PropagateToParent`)。

---

## 12. 悬浮面板里的弹出层:三条实测结论(2026-09,悬浮工具面板)

这一节是 §3 推论 3 / §5 在"整块面板都在 overlay 层里"时的具体化,三条都踩过:

1. **要"新开一层但不裁剪",用 `Stack::new()` + `add_overlay_child(child)`**。
   `Clipped` 的裁剪范围与它自己的矩形绑定,拿它当"新层"会顺手把子树裁到该矩形 —— 悬浮工具面板
   的文件树右键菜单、会话列表溢出菜单就是这样被裁到面板内、出不了面板的。
   `Stack::paint` 对 overlay 子元素走 `start_overlay_layer(ClipBounds::None)`,正好是"要层号、
   不要裁剪"(而层号仍高于 `Dismiss` 的整窗 underlay,所以"点面板内空白不误关"照旧成立)。

2. **`add_positioned_overlay_child` 在 overlay 上下文里也必须不裁剪**。
   `Stack::paint` 只认 `child.element.is_overlay()`,而 positioned overlay child 的子树是
   `Positioned(Overlay(..))`:`Positioned` 不转发时会被当普通子元素(`start_layer(ActiveLayer)`,
   **继承祖先裁剪**),此时 `Overlay::paint` 又因 `already_in_overlay` 跳过开层 → 滚动容器里的
   弹出菜单被裁到列表视口内(普通上下文里靠上游那层 `start_overlay_layer(None)` 掩盖了)。
   修法:`Positioned::is_overlay()` 转发给子元素(层号/层数都不变,只把 `clip_bounds` 从"继承"
   变成 `None`)。回归测试:`elements::stack::tests::positioned_overlay_child_is_unclipped_in_overlay_context`。

3. **面板头部必须"在内容之后绘制"**。
   overlay 上下文里层号只按绘制顺序比高低,头部先画 → 它按钮的 tooltip
   (`ButtonTooltipPosition` 默认 `Below`,压在内容区上)会被后画的内容区盖住。
   **当前采用的修法:把头部按钮的 tooltip 改成朝上**
   (`with_tooltip_position(ButtonTooltipPosition::Above)`,与 tab bar 那排按钮的 tooltip 一致),
   不动层结构。
   **曾试过、已回退的修法**:把头部做成根 `Stack` 的 positioned overlay child(内容前补等高占位,
   与 §5 同一手法)。它确实修好了遮挡,但 `Stack::paint` 会给**每个 child 各开一层**,改变了面板
   内部各元素的层关系;同时它自身引入过两个缺陷(绝对定位子元素拿到窗口尺寸约束 → `MainAxisSize::Max`
   的头部行被撑到窗口宽、关闭按钮跑到窗口另一侧;头部移出列后列宽不再被钉住 → 面板宽度与
   `Resizable` 的 state 脱钩)。若要走这条路,先补上 `with_constrain_absolute_children()` 和
   "铺满的等高占位",并**用探针确认**层变化没有影响 hover/光标/命中。
   注意:同期"拖拽光标闪烁"**并不是它造成的**(见下一条),别把它当替罪羊。
   - 必须 `with_constrain_absolute_children()`:`Stack::new()` 默认**不**约束绝对定位子元素
     (`stack/mod.rs` 的 `SHOULD_ENABLE_NEW_STACK_CONSTRAINT_BEHAVIOR`),子元素拿到的是**窗口**
     尺寸约束;头部行内若是 `Flex::row(MainAxisSize::Max)`,会被撑到窗口宽,`SpaceBetween` 的
     关闭按钮就被推到窗口另一侧。
   - 占位不能是宽度为 0 的 `Empty`:头部移出列之后,列的交叉轴宽度只由内容自然宽决定,面板宽度
     会与 `Resizable` 的 state 脱钩(内容窄时"拖不动"重现)。占位自身要铺满。

4. **"resize 光标一闪就没"不要先怀疑层/hover**(2026-09 排查结论,含一次误判)。
   现象:鼠标沿悬浮面板右边缘移动时,resize 光标出现一次就回默认。我按"层遮挡 / z 仲裁"连改三轮
   (拖拽带单独分层、每次 MouseMoved 重声明光标、加宽命中带),**都没修好**;最后用户自己发现
   **系统里所有 app 的拖拽光标都这样** —— 是环境级现象,不是本面板的 bug,所有改动已回退。
   **排查教训**:
   - 这类"输入设备/光标"现象**先确认是否只在本应用出现**(换个 app 试同一动作),再决定要不要查代码;
   - 探针要打**事实**(命中区范围、`is_hovering`/`is_covered`、仲裁的 z 与胜负),不要打推演结论
     —— 本次 `is_hovering=true` 235 次、`is_covered=true` **0 次**,直接推翻了我"被层遮挡"的假设;
   - `Presenter::set_cursor` 是**"z 高者胜"**、`reset_cursor` 只在"本次派发没人设过"时生效;
     `Resizable` 只在**进入/离开拖拽带**时设一次光标(这是既有设计,别当成 bug 去改)。

---

## 排查清单

遇到"层级/可见性"问题时按顺序做:

1. **打印层索引**。在渲染器 `draw_layer` 或 `Scene` 侧打印每个 layer 的
   `z_index` / `clip_bounds` / `rects.len()` / `glyphs.len()`。数字能一次区分
   "被裁"、"层号低"、"根本没画"三种情况。
2. **确认元素进了哪个数组**。看它是通过 `add_child` 还是 `add_positioned_overlay_child`
   加入,以及它的祖先是否已经在 overlay 上下文里。
3. **检查几何**。阴影/描边这类"向外扩散"的效果,先确认它有没有可见空间,
   不要先调参数。
4. **检查重入**。若症状是 panic 且消息含 `Circular view update`,就在调用栈里找
   "闭包内 update 同一个 view"。
5. **颜色 vs 层级**。文字"看不见"时先打色值,再怀疑层级。

**探针用完即删**:定位后立刻移除,不要把调试日志留在提交里。
