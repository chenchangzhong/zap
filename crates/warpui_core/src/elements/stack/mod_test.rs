use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

use itertools::Itertools;
use pathfinder_geometry::rect::RectF;

use super::*;

use crate::{
    elements::{
        resizable_state_handle, Clipped, DispatchEventResult, DragBarSide, Hoverable,
        MouseStateHandle, Resizable,
    },
    platform::WindowStyle,
    TypedActionView,
};
use crate::{
    elements::{ConstrainedBox, EventHandler, ParentElement, Rect, ZIndex},
    App, AppContext, Entity, Event, Presenter, ViewContext, ViewHandle, WindowId,
    WindowInvalidation,
};

#[derive(Default)]
struct View {
    // maps view id to number of mouse downs
    mouse_downs: HashMap<usize, u32>,
    mouse_ups: HashMap<usize, u32>,
    mouse_dragged: HashMap<usize, u32>,
}

pub fn init(app: &mut AppContext) {
    app.add_action("test_view:mouse_down", View::mouse_down);
    app.add_action("test_view:mouse_up", View::mouse_up);
    app.add_action("test_view:mouse_dragged", View::mouse_dragged);
}

impl View {
    fn mouse_down(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on view_id {view_id}");
        let entry = self.mouse_downs.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_up(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_up on view_id {view_id}");
        let entry = self.mouse_ups.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_dragged(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_dragged on view_id {view_id}");
        let entry = self.mouse_dragged.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }
}

impl TypedActionView for View {
    type Action = ();
}

impl Entity for View {
    type Event = String;
}

impl crate::core::View for View {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut s = Stack::new();
        s.add_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(50.)
                    .with_width(50.)
                    .finish(),
            )
            .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_down", 0usize);
                DispatchEventResult::StopPropagation
            })
            .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_up", 0usize);
                DispatchEventResult::StopPropagation
            })
            .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_dragged", 0usize);
                DispatchEventResult::StopPropagation
            })
            .finish(),
        );
        s.add_child(
            Positioned::new(
                EventHandler::new(
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(50.)
                        .with_width(50.)
                        .finish(),
                )
                .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_down", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_up", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_dragged", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            )
            .with_offset(OffsetPositioning::offset_from_parent(
                vec2f(25., 25.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ))
            .finish(),
        );
        s.add_child(
            Positioned::new(
                Clipped::sized(
                    EventHandler::new(
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(50.)
                            .with_width(50.)
                            .finish(),
                    )
                    .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_down", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_up", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_dragged", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
                    vec2f(25., 25.),
                )
                .finish(),
            )
            .with_offset(OffsetPositioning::offset_from_parent(
                vec2f(100., 100.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ))
            .finish(),
        );
        s.finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

const FIRST_CHILD_POSITION_ID: &str = "RelativePositionedView::first_child_position_id";

/// A view for testing that renders the second child in a stack based on what's specified in
/// `second_child_positioning`.
#[derive(Default)]
struct RelativePositionedView {
    second_child_positioning: Option<OffsetPositioning>,
    second_child_size: Option<Vector2F>,
}

impl RelativePositionedView {
    fn new() -> Self {
        Self {
            second_child_positioning: None,
            second_child_size: None,
        }
    }

    fn first_child_position_id() -> &'static str {
        FIRST_CHILD_POSITION_ID
    }
}

impl Entity for RelativePositionedView {
    type Event = String;
}

impl crate::core::View for RelativePositionedView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut s = Stack::new();
        s.add_child(
            SavePosition::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(50.)
                    .with_width(50.)
                    .finish(),
                FIRST_CHILD_POSITION_ID,
            )
            .finish(),
        );

        if let Some(second_child_positioning) = &self.second_child_positioning {
            s.add_child(
                Positioned::new(if let Some(second_child_size) = &self.second_child_size {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(second_child_size.x())
                        .with_height(second_child_size.y())
                        .finish()
                } else {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(50.)
                        .with_width(50.)
                        .finish()
                })
                .with_offset(second_child_positioning.clone())
                .finish(),
            );
        }

        // Force the Stack to take up the full size of the window by pulling
        // the minimum size constraint up to the size of the window.
        ConstrainedBox::new(s.finish())
            .with_min_width(f32::MAX)
            .with_min_height(f32::MAX)
            .finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

impl TypedActionView for RelativePositionedView {
    type Action = ();
}

#[test]
fn test_paint_sets_z_index() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let mut updated = crate::EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            let scene = presenter.build_scene(vec2f(300., 300.), 1., None, ctx);
            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 5);
            let presenter = Rc::new(RefCell::new(presenter));

            // Fire event on first child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on second child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on third child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on clipped part of third child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(1, *view.mouse_downs.get(&0).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&1).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&2).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&0).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&1).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&2).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&0).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&1).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&2).unwrap());
        });
    })
}

#[test]
fn test_relative_positioning() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });

        position_child_and_assert_location(
            OffsetPositioning::offset_from_save_position_element(
                RelativePositionedView::first_child_position_id(),
                vec2f(25., 25.),
                PositionedElementOffsetBounds::Unbounded,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
            RectF::new(vec2f(25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Update the view to position the top right of the child offset from the top right of
        // the parent (this should mean part of the child is clipped offscreen on the left).
        position_child_and_assert_location(
            OffsetPositioning::offset_from_save_position_element(
                RelativePositionedView::first_child_position_id(),
                vec2f(25., 25.),
                PositionedElementOffsetBounds::Unbounded,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::TopRight,
            ),
            RectF::new(vec2f(-25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Offset with the same position, but bound horizontally to the parent so the element is
        // no longer clipped past the left side of the screen.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(0., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Now just bound vertically to the parent. This should not change the positioning since
        // the element is already bound vertically within the parent.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(-25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Update the view to position the top left of the child offset from the top right of the
        // parent.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(75., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Now, bound vertically with the parent--this should have no effect here since the
        // child is fully contained within its parent.
        let new_positioning = OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                RelativePositionedView::first_child_position_id(),
                PositionedElementOffsetBounds::Unbounded,
                OffsetType::Pixel(25.),
                AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
            ),
            PositioningAxis::relative_to_stack_child(
                RelativePositionedView::first_child_position_id(),
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(25.),
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
            ),
        );

        position_child_and_assert_location(
            new_positioning,
            RectF::new(vec2f(75., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Position the child's bottom right corner on the parent's bottom right corner. With
        // no offset this means they should be stacked directly on top of each other.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Bottom),
                ),
            ),
            RectF::new(vec2f(0., 0.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Align the child vertically from the parent and horizontally from the child.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(5.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(5.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(55., 5.), vec2f(50., 50.)),
            app,
            window_id,
            view,
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_window_by_size() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });
        let window_size = view.update(app, |_, ctx| {
            ctx.notify();
            ctx.windows()
                .platform_window(window_id)
                .expect("Window should exist for platform.")
                .size()
        });

        let offset = vec2f(25., 25.);
        let positioning = OffsetPositioning::offset_from_save_position_element(
            RelativePositionedView::first_child_position_id(),
            offset,
            PositionedElementOffsetBounds::WindowBySize,
            PositionedElementAnchor::BottomRight,
            ChildAnchor::TopLeft,
        );
        view.update(app, |view, ctx| {
            view.second_child_positioning = Some(positioning);

            // Set the offset-positioned child's size to the window size so the bounding
            // behavior is actually tested.
            view.second_child_size = Some(window_size);
            ctx.notify();
        });

        // Simulate a render frame to ensure the scene is built.
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        let presenter_ref = app
            .presenter(window_id)
            .expect("Test window should have a presenter since first frame is rendered.");
        let presenter = presenter_ref.borrow();
        let scene = presenter
            .scene()
            .expect("Presenter should have rendered a scene after the view was updated.");

        // The expected bounds should go from the anchor position with offset to the edge of
        // the window bounds. Note the usage of `RectF::from_points`, which specifies top-left
        // and bottom-right coordinates, rather than the default `RectF::new()` constructor.
        let expected_bounds = RectF::from_points(vec2f(75., 75.), window_size);
        assert_eq!(
            scene
                .layers()
                .collect_vec()
                .get(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_bounds]
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_window_by_position() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });
        let window_size = view.update(app, |_, ctx| {
            ctx.notify();
            ctx.windows()
                .platform_window(window_id)
                .expect("Window should exist for platform.")
                .size()
        });

        let offset = vec2f(25., 25.);
        let positioning = OffsetPositioning::offset_from_save_position_element(
            RelativePositionedView::first_child_position_id(),
            offset,
            PositionedElementOffsetBounds::WindowByPosition,
            PositionedElementAnchor::BottomRight,
            ChildAnchor::TopLeft,
        );
        view.update(app, |view, ctx| {
            view.second_child_positioning = Some(positioning);

            // Set the offset-positioned child's size to the window size so the bounding
            // behavior is actually tested.
            view.second_child_size = Some(window_size);
            ctx.notify();
        });

        let presenter_ref = app
            .presenter(window_id)
            .expect("Test window should have a presenter since first frame is rendered.");
        let presenter = presenter_ref.borrow();
        let scene = presenter
            .scene()
            .expect("Presenter should have rendered a scene after the view was updated.");

        // The expected bounds should have a modified position to accommodate the size of the
        // positioned child (it should be moved back to (0,0) from it's 'default' (75, 75).
        //
        // Note the usage of `RectF::from_points`, which specifies top-left
        // and bottom-right coordinates, rather than the default `RectF::new()` constructor.
        let expected_bounds = RectF::from_points(vec2f(0., 0.), window_size);
        assert_eq!(
            scene
                .layers()
                .collect_vec()
                .get(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_bounds]
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_missing_anchor() {
    App::test((), |mut app| async move {
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| {
            let mut view = RelativePositionedView::new();

            view.second_child_positioning = Some(OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    "nonexistent_anchor",
                    PositionedElementOffsetBounds::WindowBySize,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
                )
                .with_conditional_anchor(),
                PositioningAxis::relative_to_stack_child(
                    "nonexistent_anchor",
                    PositionedElementOffsetBounds::WindowBySize,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                )
                .with_conditional_anchor(),
            ));

            view
        });

        let mut presenter = Presenter::new(window_id);

        let invalidation = WindowInvalidation {
            updated: [app.root_view_id(window_id).expect("Root view must exist")]
                .into_iter()
                .collect::<crate::EntityIdSet>(),
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);

            let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
            let scene = presenter.build_scene(window_size.size(), 1., None, ctx);

            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 3);

            let stack_layer = scene.layers().nth(2).expect("Should be 3 layers");
            assert!(
                stack_layer.rects.is_empty(),
                "Relative-positioned element should not have been laid out"
            );
            // In addition to the assertion that there's no rect for the second
            // child, this implicitly tests that we don't panic during layout.
        });
    });
}

/// Positions the second child using the positioning and asserts the child is at bounds
/// indicated within `expected_child_bounds`.
fn position_child_and_assert_location(
    positioning: OffsetPositioning,
    expected_child_bounds: RectF,
    app: &mut App,
    window_id: WindowId,
    view: ViewHandle<RelativePositionedView>,
) {
    view.update(app, |view, _| {
        view.second_child_positioning = Some(positioning);
    });

    let mut presenter = Presenter::new(window_id);

    let mut updated = crate::EntityIdSet::default();
    updated.insert(app.root_view_id(window_id).unwrap());
    let invalidation = WindowInvalidation {
        updated,
        ..Default::default()
    };

    app.update(move |ctx| {
        presenter.invalidate(invalidation, ctx);
        let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
        let scene = presenter.build_scene(window_size.size(), 1., None, ctx);

        assert_eq!(scene.z_index(), ZIndex::new(0));
        assert_eq!(scene.layer_count(), 3);

        assert_eq!(
            scene
                .layers()
                .nth(1)
                .unwrap()
                .rects
                .iter()
                .map(|r| r.bounds)
                .collect::<Vec<_>>(),
            vec![RectF::new(Vector2F::zero(), vec2f(50., 50.))]
        );
        assert_eq!(
            scene
                .layers()
                .nth(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_child_bounds]
        );
    });
}

/// 渲染:根 `Stack` 的 overlay child 里放一个 `Clipped::sized(.., 100x100)`,裁剪层内再放一个
/// 内层 `Stack`(普通矩形 + 一个 `add_positioned_overlay_child` 的矩形)。
///
/// 用来守住:`Positioned::is_overlay` 必须转发给子元素 —— 否则在 **overlay 上下文**里
/// (`Overlay::paint` 因 `already_in_overlay` 不再开新层)`add_positioned_overlay_child` 的子树会
/// 退化成"继承祖先裁剪",面板/滚动列表里的弹出菜单会被裁到视口内(实测过的 bug)。
struct PositionedOverlayInsideClippedOverlayView;

impl Entity for PositionedOverlayInsideClippedOverlayView {
    type Event = String;
}

impl crate::core::View for PositionedOverlayInsideClippedOverlayView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut inner = Stack::new();
        inner.add_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_width(20.)
                .with_height(20.)
                .finish(),
        );
        // 放到 (120, 0):完全落在 `Clipped` 的 100x100 之外。
        inner.add_positioned_overlay_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_width(20.)
                .with_height(20.)
                .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(120., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut root = Stack::new();
        root.add_overlay_child(Clipped::sized(inner.finish(), vec2f(100., 100.)).finish());
        root.finish()
    }

    fn ui_name() -> &'static str {
        "PositionedOverlayInsideClippedOverlayView"
    }
}

impl TypedActionView for PositionedOverlayInsideClippedOverlayView {
    type Action = ();
}

#[test]
fn positioned_overlay_child_is_unclipped_in_overlay_context() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            PositionedOverlayInsideClippedOverlayView
        });

        app.update(|ctx| ctx.simulate_render_frame(window_id));

        let presenter_ref = app
            .presenter(window_id)
            .expect("Test window should have a presenter since first frame is rendered.");
        let presenter = presenter_ref.borrow();
        let scene = presenter
            .scene()
            .expect("Presenter should have rendered a scene after the view was updated.");

        // positioned overlay child 的矩形必须落在**不裁剪**的 overlay 层里(该层里只有它)。
        let escaping_layer = scene
            .overlay_layers()
            .find(|layer| layer.clip_bounds.is_none() && !layer.rects.is_empty())
            .expect(
                "positioned overlay child 必须拿到 `ClipBounds::None` 的 overlay 层;\
                 退化成继承祖先裁剪说明 `Positioned::is_overlay` 转发失效",
            );
        assert!(
            escaping_layer
                .rects
                .iter()
                .any(|rect| rect.bounds.min_x() >= 120.),
            "不裁剪层里应当有那个被放到裁剪区之外 (x=120) 的矩形,实际: {:?}",
            escaping_layer.rects.iter().map(|r| r.bounds).collect::<Vec<_>>()
        );
    });
}

/// 回归:`Stack` 里后加的 hover 探测层不能吞掉兄弟节点(拖拽带)的 `drag`。
///
/// 复刻真实场景(`docs/vertical-tabs-floating-resize-issue.md`):`Resizable` 的拖拽带被后加的
/// 贴边 hover 探测层盖住。`Stack` 的 `Waterfall` 模式**逆序**派发、首个返回 `true` 的子元素即
/// 终止;而 `Hoverable` 默认 `suppress_drag = true`,并在 `LeftMouseDown` 命中它时写下
/// `click_count`,于是它把后续每一条 `LeftMouseDragged` 都 `return true` 吃掉 —— 拖拽带再也
/// 收不到 drag,表现为"按下有反应、拖拽完全无反应"。解法是给这类 hover 目标加
/// `with_propagate_drag()`(与 pane 分隔条同构)。
///
/// 这里显式钉住 `Waterfall`:`Stack::new()` 只在 debug 构建下默认用它,用例不该依赖构建模式。
/// 用真实的 `Resizable` 而不是 `EventHandler` 当拖拽带:`EventHandler` 会做 `at_z_index`
/// 覆盖判定,被上层盖住时它本来就不会响应 —— 那不是本例要证的机制。
struct ProbeOverDragView {
    probe_propagates_drag: bool,
    /// `Resizable::on_resize` 的调用次数:按下那次 1 次,之后每完成一步拖拽再 +1。
    resize_callbacks: Rc<Cell<u32>>,
}

impl Entity for ProbeOverDragView {
    type Event = String;
}

impl TypedActionView for ProbeOverDragView {
    type Action = ();
}

impl crate::core::View for ProbeOverDragView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new().with_event_dispatch_mode(EventDispatchMode::Waterfall);

        let resize_callbacks = self.resize_callbacks.clone();
        // 先加:拖拽带(等价于悬浮侧栏面板右边缘那一条)。`Waterfall` 逆序派发,所以它排在
        // 探测层**之后**才被派发。
        stack.add_child(
            Resizable::new(
                resizable_state_handle(100.),
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(100.)
                    .with_width(100.)
                    .finish(),
            )
            .with_dragbar_side(DragBarSide::Right)
            .on_resize(move |_ctx, _app| {
                resize_callbacks.set(resize_callbacks.get() + 1);
            })
            .finish(),
        );

        // 后加:透明的 hover 探测层,盖住拖拽带(真实代码里它是 overlay 子元素,派发顺序一致)。
        let mut probe = Hoverable::new(MouseStateHandle::default(), |_| {
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(100.)
                .finish()
        });
        if self.probe_propagates_drag {
            probe = probe.with_propagate_drag();
        }
        stack.add_child(probe.finish());

        stack.finish()
    }

    fn ui_name() -> &'static str {
        "probe_over_drag_view"
    }
}

/// `expected_resize_callbacks`:按下 1 次 + 每被拖到的新位置 1 次。放行 drag 时两条 drag 都应
/// 落到 `Resizable`(3 次);被探测层吃掉时只有按下那 1 次。
fn assert_drag_through_hover_probe(probe_propagates_drag: bool, expected_resize_callbacks: u32) {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, move |_| {
            ProbeOverDragView {
                probe_propagates_drag,
                resize_callbacks: Rc::new(Cell::new(0)),
            }
        });

        let mut presenter = Presenter::new(window_id);
        let mut updated = crate::EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 200.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            // 按在拖拽带上(x ∈ [95, 100])。探测层盖着同一块区域,所以它会记下 `click_count`。
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(97., 50.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            // 两条 drag:不放行时被探测层 `return true` 截断,放行时才会落到拖拽带上。
            for x in [80., 60.] {
                ctx.simulate_window_event(
                    Event::LeftMouseDragged {
                        position: vec2f(x, 50.),
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter.clone(),
                );
            }
        });

        view.read(app, |view, _| {
            assert_eq!(
                expected_resize_callbacks,
                view.resize_callbacks.get(),
                "probe_propagates_drag={probe_propagates_drag} 时,拖拽带收到的 drag 步数不对"
            );
        });
    });
}

#[test]
fn test_hover_probe_added_last_swallows_sibling_drag_without_propagate() {
    // 陷阱本身:默认 `suppress_drag` 的探测层会把兄弟节点的 drag 全部吃掉(1 步都到不了)。
    assert_drag_through_hover_probe(false, 1);
}

#[test]
fn test_hover_probe_with_propagate_drag_lets_sibling_receive_drag() {
    // 解法:`with_propagate_drag()` 之后,drag 正常落到被压住的拖拽带上(按下 1 次 + 两步拖拽)。
    assert_drag_through_hover_probe(true, 3);
}
