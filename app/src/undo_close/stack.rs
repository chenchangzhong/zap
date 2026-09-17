use uuid::Uuid;
use warpui::{
    r#async::SpawnedFutureHandle, AppContext, ClosedWindowData, Entity, EntityId, ModelContext,
    ModelHandle, SingletonEntity, ViewHandle, WeakViewHandle, WindowId,
};

use crate::{
    ai::blocklist::BlocklistAIHistoryModel,
    pane_group::{PaneGroup, PaneId},
    send_telemetry_from_app_ctx,
    server::telemetry::{TelemetryEvent, UndoCloseItemType},
    tab::TabData,
    window_settings::WindowSettings,
    workspace::Workspace,
};

use super::{settings::UndoCloseSettingsChangedEvent, UndoCloseSettings};

/// A unique identifier for an item in the undo close stack.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ItemId(Uuid);

impl ItemId {
    /// Constructs a new ItemId.
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Data for an item in the undo close stack.
struct UndoData {
    closed_item: ClosedItem,
    expiry_data: ExpiryData,
}

/// Data needed to handle expiration for items in the undo close stack.
struct ExpiryData {
    id: ItemId,
    task_handle: SpawnedFutureHandle,
}

impl std::ops::Drop for ExpiryData {
    fn drop(&mut self) {
        // Make sure we abort the expiry task when we drop the expiry data.
        self.task_handle.abort();
    }
}

/// Data needed to restore a closed pane.
pub(super) struct PaneData {
    /// The pane ID - content is retrieved from the pane group during restoration
    pane_id: PaneId,
    /// Reference to the pane group that contained this pane
    pane_group: WeakViewHandle<PaneGroup>,
}

/// An item in the undo close stack which can be re-opened.
pub enum ClosedItem {
    Window(Box<ClosedWindowData>),
    Tab {
        workspace: WeakViewHandle<Workspace>,
        tab_index: usize,
        data: TabData,
    },
    Pane {
        data: PaneData,
    },
}

impl ClosedItem {
    /// `has_restorable_window`:丢弃后 undo 栈里是否还有别的可恢复窗口。
    /// 由调用方给出——本函数运行在 `UndoCloseStack` 的 update 内,回读该单例
    /// 会 panic(`circular model reference`)。
    fn discard(self, ctx: &mut ModelContext<UndoCloseStack>, has_restorable_window: bool) {
        let history_model = BlocklistAIHistoryModel::handle(ctx);

        match self {
            ClosedItem::Window(data) => {
                let ClosedWindowData { window_id, .. } = *data;
                if let Some(workspace) = window_workspace(window_id, ctx) {
                    workspace.update(ctx, |workspace, ctx| {
                        for pane_group in workspace.tab_views() {
                            // Mark conversations from all terminal panes in each tab
                            Self::mark_conversations_historical_for_pane_group(
                                pane_group,
                                &history_model,
                                ctx,
                            );
                            Self::clean_up_pane_group(pane_group, ctx);
                        }
                    });
                } else {
                    // 窗口已销毁且确认不再恢复:它的 pane 不会再产生 Closed
                    // detach(见 lib.rs 窗口关闭处的说明),这里兜底登记 dsh 待停,
                    // 避免 dsh 子进程一直活到 app 退出。判定要算上「栈里其他仍可
                    // 恢复的窗口」——只关了一个窗口不代表 dsh 不再需要。
                    crate::dsh::runtime::stop_if_no_dsh_pane(ctx, has_restorable_window);
                }
            }
            ClosedItem::Tab { data, .. } => {
                // Mark conversations from all terminal panes in the tab
                Self::mark_conversations_historical_for_pane_group(
                    &data.pane_group,
                    &history_model,
                    ctx,
                );
                Self::clean_up_pane_group(&data.pane_group, ctx);
            }
            ClosedItem::Pane { data } => {
                ctx.emit(UndoCloseStackEvent::DiscardPane(data.pane_id));
            }
        }
    }

    /// Marks conversations as historical for all terminal panes in a pane group so they remain searchable.
    /// Historical conversations consist of non-live conversations that were read from disk on startup,
    /// and conversations (recorded here) that were live this session but have now been cleared.
    fn mark_conversations_historical_for_pane_group(
        pane_group: &ViewHandle<PaneGroup>,
        history_model: &ModelHandle<BlocklistAIHistoryModel>,
        ctx: &mut AppContext,
    ) {
        // Check if the window and view still exist before attempting to read
        let window_id = pane_group.window_id(ctx);
        let view_id = pane_group.id();

        if ctx.view_with_id::<PaneGroup>(window_id, view_id).is_some() {
            let terminal_view_ids: Vec<EntityId> = pane_group.read(ctx, |pg, ctx| {
                pg.terminal_pane_ids()
                    .filter_map(|pane_id| {
                        pg.terminal_view_from_pane_id(pane_id, ctx)
                            .map(|terminal_view| terminal_view.id())
                    })
                    .collect()
            });

            for terminal_view_id in terminal_view_ids {
                history_model.update(ctx, |history_model, _| {
                    history_model.mark_conversations_historical_for_terminal_view(terminal_view_id);
                });
            }
        }
    }

    fn clean_up_pane_group(pane_group: &ViewHandle<PaneGroup>, ctx: &mut AppContext) {
        let window_id = pane_group.window_id(ctx);

        if !ctx.is_window_open(window_id) {
            return;
        }

        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.clean_up_panes(ctx);
        });
    }
}

pub enum UndoCloseStackEvent {
    DiscardPane(PaneId),
}

/// A stack of closed items which can be re-opened in LIFO order.
pub struct UndoCloseStack {
    stack: Vec<UndoData>,
}

impl UndoCloseStack {
    /// Constructs a new undo close stack.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&UndoCloseSettings::handle(ctx), |me, _, event, ctx| {
            me.handle_settings_event(event, ctx);
        });

        Self {
            stack: Default::default(),
        }
    }

    /// Returns whether or not the stack is empty.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns true only if the pane group is present in the undo close stack as part of a closed tab.
    pub fn is_pane_group_tab_in_stack(&self, pane_group_id: EntityId) -> bool {
        self.stack
            .iter()
            .any(|undo_data| matches!(&undo_data.closed_item, ClosedItem::Tab { data, .. } if data.pane_group.id() == pane_group_id))
    }

    /// undo 栈中是否还有「已关闭窗口」的条目(窗口仍可撤销恢复)。
    ///
    /// 供 dsh 判断子进程是否还该活着:关窗路径不登记待停,而 `any_dsh_pane`
    /// 只枚举打开的窗口,看不到已关闭窗口里仍可恢复的 dsh pane。窗口的视图由
    /// warpui 的 `ClosedWindowData` 持有、无法在此探测其中的 pane,故保守处理:
    /// 只要还有窗口条目可恢复,就继续保留 dsh 子进程(它可能正是恢复后要用的)。
    pub fn has_restorable_window(&self) -> bool {
        self.stack
            .iter()
            .any(|undo_data| matches!(&undo_data.closed_item, ClosedItem::Window(_)))
    }

    /// Discards a pane group from the undo close stack early.
    pub fn discard_pane_group_parent(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(pos) = self
            .stack
            .iter()
            .position(|undo_data| match &undo_data.closed_item {
                ClosedItem::Tab { data, .. } => data.pane_group.id() == pane_group_id,
                ClosedItem::Pane { data } => data.pane_group.id() == pane_group_id,
                _ => false,
            })
        {
            let removed_item = self.stack.remove(pos);
            removed_item.expiry_data.task_handle.abort();
            // 丢弃后仍可恢复的窗口(供 dsh 判断是否还该保留子进程)。
            let has_restorable_window = self.has_restorable_window();
            removed_item.closed_item.discard(ctx, has_restorable_window);
        }
    }

    /// Handles a window being closed, adding the necessary data to the undo
    /// stack.
    pub fn handle_window_closed(&mut self, data: ClosedWindowData, ctx: &mut ModelContext<Self>) {
        self.push_item(ClosedItem::Window(Box::new(data)), ctx);
    }

    /// Handles a tab being closed, adding the necessary data to the undo
    /// stack.
    pub fn handle_tab_closed(
        &mut self,
        workspace: WeakViewHandle<Workspace>,
        tab_index: usize,
        data: TabData,
        ctx: &mut ModelContext<Self>,
    ) {
        self.push_item(
            ClosedItem::Tab {
                workspace,
                tab_index,
                data,
            },
            ctx,
        );
    }

    /// Handles a pane being closed, adding the necessary data to the undo stack.
    pub fn handle_pane_closed_by_id(
        &mut self,
        pane_group: WeakViewHandle<PaneGroup>,
        pane_id: PaneId,
        ctx: &mut ModelContext<Self>,
    ) {
        let pane_data = PaneData {
            pane_id,
            pane_group,
        };

        self.push_item(ClosedItem::Pane { data: pane_data }, ctx);
    }

    /// Undoes the last close action in the stack, if possible.
    pub fn undo_close(&mut self, ctx: &mut AppContext) {
        let Some(UndoData { closed_item, .. }) = self.stack.pop() else {
            return;
        };

        match closed_item {
            ClosedItem::Window(data) => {
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::UndoClose {
                        item_type: UndoCloseItemType::Window,
                    },
                    ctx
                );

                let window_id = data.window_id;
                // 恢复原生窗口时要带上当前的外观设置:背景模糊只在窗口创建那一刻
                // 应用(`AddWindowOptions`),漏掉就会得到「保留了透明度、丢了磨砂」
                // 的窗口——看起来比正常窗口透明得多。
                let (blur_radius_pixels, blur_texture) = {
                    let settings = WindowSettings::handle(ctx).as_ref(ctx);
                    (
                        Some(*settings.background_blur_radius),
                        *settings.background_blur_texture,
                    )
                };
                ctx.reopen_closed_window(*data, blur_radius_pixels, blur_texture);

                if let Some(workspace) = window_workspace(window_id, ctx) {
                    workspace.update(ctx, |workspace, ctx| {
                        workspace.handle_reopen(ctx);
                    });
                }

                // Make sure we update our session restoration state now that the
                // window has been reopened.
                ctx.dispatch_global_action("workspace:save_app", &());
            }
            ClosedItem::Tab {
                workspace,
                tab_index,
                data,
            } => {
                if let Some(workspace) = workspace.upgrade(ctx) {
                    send_telemetry_from_app_ctx!(
                        TelemetryEvent::UndoClose {
                            item_type: UndoCloseItemType::Tab,
                        },
                        ctx
                    );
                    workspace.update(ctx, |workspace, ctx| {
                        workspace.restore_closed_tab(tab_index, data, ctx);
                    });
                    ctx.windows()
                        .show_window_and_focus_app(workspace.window_id(ctx));
                }
                // Make sure we update our session restoration state now that the
                // tab has been reopened.
                ctx.dispatch_global_action("workspace:save_app", &());
            }
            ClosedItem::Pane { data } => {
                if let Some(pane_group) = data.pane_group.upgrade(ctx) {
                    let pane_id = data.pane_id;
                    let window_id = pane_group.window_id(ctx);
                    let pane_group_id = pane_group.id();
                    let restored = pane_group.update(ctx, |pane_group, ctx| {
                        pane_group.restore_closed_pane(pane_id, ctx)
                    });

                    if restored {
                        send_telemetry_from_app_ctx!(
                            TelemetryEvent::UndoClose {
                                item_type: UndoCloseItemType::Pane,
                            },
                            ctx
                        );

                        // Focus the window first
                        ctx.windows().show_window_and_focus_app(window_id);

                        // Now properly focus the restored pane by activating its tab and focusing the pane
                        if let Some(workspace) = window_workspace(window_id, ctx) {
                            workspace.update(ctx, |workspace, ctx| {
                                let locator = crate::workspace::PaneViewLocator {
                                    pane_group_id,
                                    pane_id,
                                };
                                workspace.focus_pane(locator, ctx);
                            });
                        }

                        ctx.dispatch_global_action("workspace:save_app", &());
                    }
                }
            }
        }
    }

    /// 取出 undo 栈中「属于指定 workspace 的、已关闭的 dsh tab」(最近关闭的一个)。
    ///
    /// 供 `open_dsh_pane` 在菜单打开 dsh 时复用它,而不是新建 pane:新建 pane
    /// 会创建新 webview 并重新加载页面,观感像「重启」;复用原 pane(同一
    /// webview)才能回到原会话。
    ///
    /// 取出即从栈中移除,`ExpiryData` 随之 drop 并取消到期定时器——调用方
    /// 必须立即恢复该项(等价于自动执行一次撤销)。
    pub fn take_closed_dsh_tab(
        &mut self,
        workspace_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(usize, TabData)> {
        // 从栈尾往前找:取「最近关闭」的一个,与 `undo_close` 的 LIFO 语义一致。
        let pos = self.stack.iter().rposition(|undo_data| {
            matches!(
                &undo_data.closed_item,
                ClosedItem::Tab { workspace, data, .. }
                    if workspace.id() == workspace_id
                        && data
                            .pane_group
                            .try_as_ref(ctx)
                            .is_some_and(|pane_group| pane_group.dsh_panes().next().is_some())
            )
        })?;
        let removed = self.stack.remove(pos);
        match removed.closed_item {
            ClosedItem::Tab { tab_index, data, .. } => Some((tab_index, data)),
            // 上面已按 `ClosedItem::Tab` 过滤,此处不可达;保守返回 None
            // (项已从栈中取出,不留半恢复状态)。
            ClosedItem::Window(_) | ClosedItem::Pane { .. } => None,
        }
    }

    /// Handles a change to the undo close settings.
    fn handle_settings_event(
        &mut self,
        event: &UndoCloseSettingsChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            UndoCloseSettingsChangedEvent::UndoCloseEnabled { .. } => {
                let settings = UndoCloseSettings::as_ref(ctx);
                if !*settings.enabled {
                    // 整栈已无意义(undo 关闭):没有「别的可恢复窗口」可言。
                    for undo_data in self.stack.drain(..) {
                        undo_data.closed_item.discard(ctx, false);
                    }
                }
            }
            UndoCloseSettingsChangedEvent::UndoCloseGracePeriod { .. } => {}
        }
    }

    /// Pushes a new item onto the stack.
    fn push_item(&mut self, closed_item: ClosedItem, ctx: &mut ModelContext<Self>) {
        let settings = UndoCloseSettings::as_ref(ctx);
        if !*settings.enabled {
            let has_restorable_window = self.has_restorable_window();
            closed_item.discard(ctx, has_restorable_window);
            return;
        }

        let id = ItemId::new();
        let grace_period = *settings.grace_period;
        let task_handle = ctx.spawn_abortable(
            warpui::r#async::Timer::after(grace_period),
            move |me, _, ctx| {
                let initial_len = me.stack.len();
                if let Some(pos) = me.stack.iter().position(|item| item.expiry_data.id == id) {
                    let removed_item = me.stack.remove(pos);
                    let has_restorable_window = me.has_restorable_window();
                    removed_item.closed_item.discard(ctx, has_restorable_window);
                }
                // Log errors if the expired item was not found or multiple items were found
                if me.stack.len() == initial_len {
                    log::error!("Undo close expiry task did not find item in stack!");
                } else if me.stack.len() < initial_len - 1 {
                    log::error!("Undo close expiry task found multiple matching items in stack!");
                } else {
                    log::debug!("Removed expired item from undo stack");
                }
            },
            |_, _| {},
        );

        self.stack.push(UndoData {
            closed_item,
            expiry_data: ExpiryData { id, task_handle },
        })
    }
}

/// Find the root [`Workspace`] view for a window.
fn window_workspace(window_id: WindowId, ctx: &mut AppContext) -> Option<ViewHandle<Workspace>> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|views| views.first().cloned())
}

impl Entity for UndoCloseStack {
    type Event = UndoCloseStackEvent;
}

impl SingletonEntity for UndoCloseStack {}
