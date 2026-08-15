//! DshPane:渲染 dsh Web UI 的 pane。
//!
//! 包装 [`BrowserPane`],复用其全部 webview 生命周期管理(创建/定位/焦点/
//! 销毁)。差异:
//! - 关闭(`DetachType::Closed`)时停止 dsh runtime(与打开时的启动对称);
//! - 会话快照仍返回 `LeafContents::Browser`(恢复时作为普通浏览器 pane,
//!   不持有 runtime 引用,避免恢复竞态)。

use crate::app_state::LeafContents;
use crate::browser::BrowserPane;
use crate::dsh::DshRuntime;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::{DetachType, PaneContent, ShareableLink, ShareableLinkError};
use crate::pane_group::{PaneConfiguration, PaneGroup, PaneId};
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity, ViewContext};
pub struct DshPane {
    inner: BrowserPane,
}

impl DshPane {
    /// Create a new dsh pane, opening `url` in an embedded webview.
    pub fn new<V: warpui::View>(url: String, ctx: &mut ViewContext<V>) -> Self {
        Self {
            inner: BrowserPane::new(url, ctx),
        }
    }
}

impl PaneContent for DshPane {
    fn id(&self) -> PaneId {
        self.inner.id()
    }

    fn attach(
        &self,
        group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.inner.attach(group, focus_handle, ctx);
    }

    fn detach(&self, group: &PaneGroup, detach_type: DetachType, ctx: &mut ViewContext<PaneGroup>) {
        self.inner.detach(group, detach_type, ctx);
        if matches!(detach_type, DetachType::Closed) {
            // 关闭 dsh pane:同步停止 runtime(启动/停止对称)。
            DshRuntime::handle(ctx).update(ctx, |runtime, _ctx| {
                runtime.request_stop();
            });
        }
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        self.inner.snapshot(app)
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.inner.has_application_focus(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.inner.focus(ctx);
    }

    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        self.inner.shareable_link(ctx)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.inner.pane_configuration()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.inner.is_pane_being_dragged(ctx)
    }
}
