# 悬浮编辑器浮层:多 tab 未保存内容在「切到 Markdown 预览」时被静默丢弃

> **状态:未修(2026-09-25)。本文记录待办,不是已完成项。**
>
> 涉及:`app/src/workspace/view.rs`(`install_floating_editor_pane` 的订阅、
> `replace_floating_editor_with_file_pane`)、`app/src/code/view.rs`(`RenderMarkdown`、
> `cleanup_all_tabs`)。行号按当时工作树,以符号名为准。

---

## 1. 现象

浮层的代码编辑器里同时打开多个 tab(`A.md` 是活动 tab、`B.md` 有未保存修改)时,在 `A` 上点
「Markdown 预览」(pane 头部的分段控件):

- `A.md` 会**先保存**,然后切到预览;
- **`B.md` 的未保存修改既不写盘、也没有任何确认,直接被丢弃** —— 用户得不到任何提示。

## 2. 触发条件(可验证)

1. `open_file_layout = Floating`;
2. 在浮层里先后打开两个文件 —— 浮层内多次打开会并入同一个 `CodeView` 形成多 tab
   (见 `open_floating_editor_lazily` 的并入分支);
3. 其中一个 tab 有未保存修改,且它不是当前活动 tab;
4. 点「Markdown 预览」。

## 3. 根因

- `CodeViewAction::RenderMarkdown` 只处理**活动 tab**:`active_tab_has_unsaved_changes` →
  `save_local(self.active_tab_index, ...)`,随后 emit
  `CodeViewEvent::Pane(PaneEvent::ReplaceWithFilePane { .. })`;
- 浮层宿主接住该事件后调 `replace_floating_editor_with_file_pane`,其中
  `discard_floating_editor_pane` → `cleanup_floating_editor_pane` →
  `CodeView::cleanup_all_tabs`;后者把整个 `tab_group` 的 `file_id` 逐个 `close_buffer`,
  再 `tab_group.clear()` —— 其余 tab 的未保存内容随之消失。

## 4. 与既有行为的关系

- 参考实现 `PaneGroup::replace_pane` → `clean_up_pane(DetachType::Closed)` →
  `CodePane::detach` 有同一缺口(非浮层布局下同样会丢),属**继承**行为;
- 但本次改动之前,浮层里点这个按钮是**无订阅者的空操作**(既不切换也不丢数据);打通浮层内
  就地替换之后,这条丢数据路径才第一次对浮层生效 —— 是本次改动让它变得**可达**。

## 5. 修法建议(未实施,需产品决策)

浮层的收起流程已经给出正确原型:`dismiss_floating_editor` 改用
`CodeView::close_all_tabs_with_callback` 逐个确认未保存 tab,并注释说明「宿主只查一次未保存
状态再保存活动 tab 会让其余 tab 的内容静默丢失」。

替换路径需要同样的确认(或退一步:先全部保存)。两种都需要「确认/保存完成后再执行替换」的
排队机制,可复用 `dispatch_typed_action_deferred` + 一个 pending 字段的模式(参见
`WorkspaceAction::FinishDismissFloatingEditor` 的实现)。

## 6. 代码里的标记

`replace_floating_editor_with_file_pane` 里有一条 `TODO(floating)` 指向本文。
