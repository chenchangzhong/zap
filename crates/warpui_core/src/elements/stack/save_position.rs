use pathfinder_geometry::{rect::RectF, vector::Vector2F};

use crate::elements::Selection;
use crate::EntityId;
use crate::{
    elements::{Point, SelectableElement, SelectionFragment},
    event::DispatchedEvent,
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

pub struct SavePosition {
    child: Box<dyn Element>,
    position_id: String,
    for_single_frame: bool,
    only_when_visible: bool,
}

impl SavePosition {
    pub fn new(child: Box<dyn Element>, position_id: &str) -> Self {
        Self {
            child,
            position_id: position_id.to_string(),
            for_single_frame: false,
            only_when_visible: false,
        }
    }

    /// Only saves the position for a single frame.  At the start
    /// of rendering the next frame the position is cleared.
    pub fn for_single_frame(mut self) -> Self {
        self.for_single_frame = true;
        self
    }

    /// Only saves the position when the element is actually visible — i.e. not
    /// fully removed by the enclosing clip bounds.
    ///
    /// Pair this with [`Self::for_single_frame`] to turn the position cache into a
    /// **visibility signal**: a read-side `element_position_by_id_at_last_frame(..)`
    /// returning `None` then means "this element was not visible in the last frame".
    ///
    /// Without it, a virtualized list that renders a margin beyond its viewport (so
    /// that scrolling stays smooth) will keep publishing positions for elements that
    /// are already scrolled out of sight, and a reader cannot tell those apart from
    /// genuinely visible ones.
    pub fn only_when_visible(mut self) -> Self {
        self.only_when_visible = true;
        self
    }
}

impl Element for SavePosition {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let size = self.child.size().expect("size must be set before paint");

        // With `only_when_visible`, skip caching when the clip bounds fully remove
        // this element. For single-frame positions the entry is cleared at the start
        // of every frame, so skipping leaves it absent — which is the signal readers
        // rely on.
        let clipped_away = self.only_when_visible
            && ctx
                .scene
                .visible_rect(Point::from_vec2f(origin, ctx.scene.z_index()), size)
                .is_none();

        if clipped_away {
            // 单帧位置每帧起始就被清掉,「不写入」天然等于「不存在」;而永久位置不会
            // 自动过期,必须显式清除,否则会留下上一帧的陈旧条目、让本开关失效。
            if !self.for_single_frame {
                ctx.position_cache.clear_position(&self.position_id);
            }
        } else {
            let bounds = RectF::new(origin, size);
            if self.for_single_frame {
                ctx.position_cache
                    .cache_position_for_one_frame(self.position_id.clone(), bounds);
            } else {
                ctx.position_cache
                    .cache_position_indefinitely(self.position_id.clone(), bounds);
            }
        }

        self.child.paint(origin, ctx, app);
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for SavePosition {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.get_selection(selection_start, selection_end, is_rect)
            })
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.expand_selection(point, direction, unit, word_boundaries_policy)
            })
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.is_point_semantically_before(absolute_point, absolute_point_other)
            })
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.smart_select(absolute_point, smart_select_fn)
            })
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        self.child
            .as_selectable_element()
            .map(|selectable_child| selectable_child.calculate_clickable_bounds(current_selection))
            .unwrap_or_default()
    }
}

pub fn get_rich_content_position_id(view_id: &EntityId) -> String {
    format!("rich_content_position_{view_id}")
}
