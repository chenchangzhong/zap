use crate::event::DispatchedEvent;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};
use pathfinder_geometry::{rect::RectF, vector::Vector2F};

/// A leaf element that declares a native view hole in the scene. The platform
/// layer maps `id` to an actual native view (e.g. an embedded WKWebView) and
/// positions it at the element's laid-out rect. warpui itself renders nothing
/// here, and hit testing is left to the OS (the native view is a separate
/// NSView that receives events directly).
pub struct PlatformViewElement {
    id: u64,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl PlatformViewElement {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            size: None,
            origin: None,
        }
    }
}

impl Element for PlatformViewElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        // Fill the available space, matching the behavior of `Empty`.
        let max_constraint = constraint.max;

        log::debug!(
            "[platform-view] layout id={} constraint min={:?} max={:?}",
            self.id,
            constraint.min,
            max_constraint
        );

        let x = if max_constraint.x().is_infinite() {
            constraint.min.x()
        } else {
            max_constraint.x()
        };

        let y = if max_constraint.y().is_infinite() {
            constraint.min.y()
        } else {
            max_constraint.y()
        };

        let size = Vector2F::new(x, y);

        self.size = Some(size);
        log::debug!("[platform-view] layout id={} -> size={size:?}", self.id);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        if let Some(size) = self.size {
            log::debug!(
                "[platform-view] paint id={} origin={origin:?} self.size={size:?} -> push {:?}",
                self.id,
                RectF::new(origin, size)
            );
            ctx.scene
                .push_platform_view(self.id, RectF::new(origin, size));
        }
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
