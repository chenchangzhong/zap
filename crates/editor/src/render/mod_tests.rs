//! End-to-end editor tests.

use sum_tree::SumTree;
use warp_core::features::FeatureFlag;
use warpui::{App, ModelHandle, ReadModel};

use crate::content::{
    buffer::{
        AutoScrollBehavior, Buffer, BufferEditAction, BufferEvent, BufferSelectAction, EditOrigin,
        InitialBufferState,
    },
    selection_model::BufferSelectionModel,
    text::{BlockType, BufferBlockItem, IndentBehavior, TextStyles},
    version::BufferVersion,
};
use string_offset::CharOffset;

use super::model::{
    BlockItem, RenderEvent, RenderState,
    test_utils::{TEST_STYLES, init_logging},
};
use crate::content::buffer::ShouldAutoscroll;

#[test]
fn test_simple_edit() {
    init_logging();
    App::test((), |mut app| async move {
        let state = TestState::new(&mut app);

        state
            .edit(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                &mut app,
            )
            .await;
        // See comments in EditDelta::layout_delta on why this paragraph has two characters even
        // though it (a) doesn't include the initial `<text>` marker and (b) doesn't end in an
        // explicit newline.
        state.assert_rendered(
            &app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (2 characters, 1 lines, 32.00px tall)
"#,
        );
    });
}

#[test]
fn test_edit_many_lines() {
    init_logging();
    App::test((), |mut app| async move {
        let state = TestState::new(&mut app);

        // Reset with several lines of Markdown at once.
        state
            .markdown(
                r#"a
bb
ccc
dddd
eeeee
ffffff
ggggggg
hhhhhhhh
iiiiiiiii
jjjjjjjjjj
kkkkkkkkkkk
llllllllllll
mmmmmmmmmmmmm
nnnnnnnnnnnnnn
ooooooooooooooo
pppppppppppppppp
qqqqqqqqqqqqqqqqq
rrrrrrrrrrrrrrrrrr
sssssssssssssssssss
tttttttttttttttttttt
uuuuuuuuuuuuuuuuuuuuu
vvvvvvvvvvvvvvvvvvvvvv
wwwwwwwwwwwwwwwwwwwwwww
xxxxxxxxxxxxxxxxxxxxxxxx
yyyyyyyyyyyyyyyyyyyyyyyyy
zzzzzzzzzzzzzzzzzzzzzzzzzz"#,
                &mut app,
            )
            .await;

        // Assert that paragraphs are laid out in the correct order.
        state.assert_rendered(
            &app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (2 characters, 1 lines, 32.00px tall)
-------- 32.00px / 2 characters --------
Paragraph (3 characters, 1 lines, 32.00px tall)
-------- 64.00px / 5 characters --------
Paragraph (4 characters, 1 lines, 32.00px tall)
-------- 96.00px / 9 characters --------
Paragraph (5 characters, 1 lines, 32.00px tall)
-------- 128.00px / 14 characters --------
Paragraph (6 characters, 1 lines, 32.00px tall)
-------- 160.00px / 20 characters --------
Paragraph (7 characters, 1 lines, 32.00px tall)
-------- 192.00px / 27 characters --------
Paragraph (8 characters, 1 lines, 32.00px tall)
-------- 224.00px / 35 characters --------
Paragraph (9 characters, 1 lines, 32.00px tall)
-------- 256.00px / 44 characters --------
Paragraph (10 characters, 1 lines, 32.00px tall)
-------- 288.00px / 54 characters --------
Paragraph (11 characters, 1 lines, 32.00px tall)
-------- 320.00px / 65 characters --------
Paragraph (12 characters, 1 lines, 32.00px tall)
-------- 352.00px / 77 characters --------
Paragraph (13 characters, 1 lines, 32.00px tall)
-------- 384.00px / 90 characters --------
Paragraph (14 characters, 1 lines, 32.00px tall)
-------- 416.00px / 104 characters --------
Paragraph (15 characters, 1 lines, 32.00px tall)
-------- 448.00px / 119 characters --------
Paragraph (16 characters, 1 lines, 32.00px tall)
-------- 480.00px / 135 characters --------
Paragraph (17 characters, 1 lines, 32.00px tall)
-------- 512.00px / 152 characters --------
Paragraph (18 characters, 1 lines, 32.00px tall)
-------- 544.00px / 170 characters --------
Paragraph (19 characters, 1 lines, 32.00px tall)
-------- 576.00px / 189 characters --------
Paragraph (20 characters, 1 lines, 32.00px tall)
-------- 608.00px / 209 characters --------
Paragraph (21 characters, 1 lines, 32.00px tall)
-------- 640.00px / 230 characters --------
Paragraph (22 characters, 1 lines, 32.00px tall)
-------- 672.00px / 252 characters --------
Paragraph (23 characters, 1 lines, 32.00px tall)
-------- 704.00px / 275 characters --------
Paragraph (24 characters, 1 lines, 32.00px tall)
-------- 736.00px / 299 characters --------
Paragraph (25 characters, 1 lines, 32.00px tall)
-------- 768.00px / 324 characters --------
Paragraph (26 characters, 1 lines, 32.00px tall)
-------- 800.00px / 350 characters --------
Paragraph (27 characters, 1 lines, 32.00px tall)
"#,
        );
    });
}

#[test]
fn test_enter_before_horizontal_rule() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(11, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Enter {
                    force_newline: false,
                    style: Default::default(),
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 32.00px tall)
-------- 32.00px / 11 characters --------
Paragraph (1 characters, 1 lines, 32.00px tall)
-------- 64.00px / 12 characters --------
Horizontal Rule (1 characters, 1 lines, 18.00px tall)
-------- 82.00px / 13 characters --------
Paragraph (12 characters, 1 lines, 32.00px tall)
"#,
        );
    })
}

#[test]
fn test_enter_after_horizontal_rule() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(13, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Enter {
                    force_newline: false,
                    style: Default::default(),
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 32.00px tall)
-------- 32.00px / 11 characters --------
Horizontal Rule (1 characters, 1 lines, 18.00px tall)
-------- 50.00px / 12 characters --------
Paragraph (1 characters, 1 lines, 32.00px tall)
-------- 82.00px / 13 characters --------
Paragraph (12 characters, 1 lines, 32.00px tall)
"#,
        );
    })
}

#[test]
fn test_edit_at_horizontal_rule_end() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(12, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 32.00px tall)
-------- 32.00px / 11 characters --------
Horizontal Rule (1 characters, 1 lines, 18.00px tall)
-------- 50.00px / 12 characters --------
Paragraph (2 characters, 1 lines, 32.00px tall)
-------- 82.00px / 14 characters --------
Paragraph (12 characters, 1 lines, 32.00px tall)
"#,
        );
    })
}

#[test]
fn test_edit_after_style() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state
            .markdown("Some **styled** text\nAnd `more`", app)
            .await;

        // Set the cursor to just after the bold text.
        state.set_cursor(12, app);

        // Insert some new text with style inheritance.
        state
            .edit(
                BufferEditAction::Insert {
                    text: "!",
                    style: TextStyles::default().bold(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;

        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (18 characters, 1 lines, 32.00px tall)
-------- 32.00px / 18 characters --------
Paragraph (9 characters, 1 lines, 32.00px tall)
        "#,
        );
    })
}

#[test]
fn test_undo_at_block_boundary() {
    // This is a regression test for CLD-1178.
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state
            .markdown("- [x] A\n- [x] B\n- [ ] C\n- [ ] D", app)
            .await;
        state.assert_buffer(app, "<cl0:true>A<cl0:true>B<cl0:false>C<cl0:false>D<text>");

        // Select from the start of item C up through A and B.
        state.set_cursor(5, app);
        state.select(BufferSelectAction::SetLastHead { offset: 1.into() }, app);

        // Press backspace, deleting A and B.
        state
            .edit(BufferEditAction::Backspace, EditOrigin::UserTyped, app)
            .await;
        state.assert_buffer(app, "<cl0:true>C<cl0:false>D<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 18.00px / 2 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 36.00px / 4 characters --------
Trailing Newline (1 characters, 1 lines, 32.00px tall)
        "#,
        );

        // Undo that change and ensure we revert to the original contents.
        state
            .edit(BufferEditAction::Undo, EditOrigin::UserInitiated, app)
            .await;
        state.assert_buffer(app, "<cl0:true>A<cl0:true>B<cl0:false>C<cl0:false>D<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 18.00px / 2 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 36.00px / 4 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 54.00px / 6 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 72.00px / 8 characters --------
Trailing Newline (1 characters, 1 lines, 32.00px tall)
        "#,
        )
    });
}

#[test]
fn test_convert_first_line() {
    // This is a full-stack analogue to test_remove_prefix_and_insert_block_item.
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        // This only uses 2 dashes so it's not parsed as Markdown yet.
        state.markdown("--\n```\ncode\n```\n", app).await;
        state.assert_buffer(app, "<text>--<code:Shell>code<text>");

        // Mimic a Markdown shortcut on the first line.
        state.set_cursor(3, app);
        state
            .edit(
                BufferEditAction::RemovePrefixAndStyleBlocks(BlockType::Item(
                    BufferBlockItem::HorizontalRule,
                )),
                EditOrigin::UserInitiated,
                app,
            )
            .await;
        state.assert_buffer(app, "<hr><code:Shell>code<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Horizontal Rule (1 characters, 1 lines, 18.00px tall)
-------- 18.00px / 1 characters --------
Code Block - Shell (5 characters, 1 lines, 84.00px tall)
-------- 102.00px / 6 characters --------
Trailing Newline (1 characters, 1 lines, 32.00px tall)
"#,
        );

        // Undo that change and ensure we revert to the original contents.
        state
            .edit(BufferEditAction::Undo, EditOrigin::UserInitiated, app)
            .await;
        state.assert_buffer(app, "<text>--<code:Shell>code<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (3 characters, 1 lines, 32.00px tall)
-------- 32.00px / 3 characters --------
Code Block - Shell (5 characters, 1 lines, 84.00px tall)
-------- 116.00px / 8 characters --------
Trailing Newline (1 characters, 1 lines, 32.00px tall)
"#,
        )
    });
}

/// Helper for testing edits end-to-end. This is essentially a stripped-down editor model.
struct TestState {
    content: ModelHandle<Buffer>,
    selection: ModelHandle<BufferSelectionModel>,
    render: ModelHandle<RenderState>,
    layout_updates: async_channel::Receiver<()>,
}

impl TestState {
    fn new(app: &mut App) -> Self {
        let content = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(content.clone()));
        let render = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));

        let (layout_tx, layout_rx) = async_channel::unbounded();
        app.update(|ctx| {
            let render2 = render.clone();
            ctx.subscribe_to_model(&content, move |_, event, ctx| match event {
                BufferEvent::SelectionChanged { .. } => (),
                BufferEvent::ContentChanged {
                    delta,
                    should_autoscroll,
                    ..
                } => render2.update(ctx, |render_state, _| {
                    render_state.add_pending_edit(delta.clone(), BufferVersion::new());
                    if matches!(should_autoscroll, ShouldAutoscroll::Yes) {
                        render_state.request_autoscroll();
                    }
                }),
                BufferEvent::AnchorUpdated { .. } | BufferEvent::ContentReplaced { .. } => (),
            });

            let content2 = content.clone();
            ctx.subscribe_to_model(&render, move |render_state, event, ctx| match event {
                RenderEvent::NeedsResize => {
                    let delta = content2.as_ref(ctx).invalidate_layout();
                    render_state.update(ctx, |render_state, _| {
                        render_state.add_pending_edit(delta, BufferVersion::new())
                    });
                }
                RenderEvent::LayoutUpdated => {
                    let _ = layout_tx.try_send(());
                }
                _ => (),
            });
        });

        Self {
            content,
            selection,
            render,
            layout_updates: layout_rx,
        }
    }

    /// Move the cursor to an offset.
    fn set_cursor(&self, location: impl Into<CharOffset>, app: &mut App) {
        self.select(
            BufferSelectAction::AddCursorAt {
                offset: location.into(),
                clear_selections: true,
            },
            app,
        );
    }

    fn select(&self, action: BufferSelectAction, app: &mut App) {
        self.content.update(app, |buffer, ctx| {
            buffer.update_selection(
                self.selection.clone(),
                action,
                AutoScrollBehavior::Selection,
                ctx,
            );
        });
    }

    /// Apply an edit to the buffer and wait for it to be laid out.
    async fn edit(&self, action: BufferEditAction<'_>, origin: EditOrigin, app: &mut App) {
        self.content.update(app, |buffer, ctx| {
            buffer.update_content(action, origin, self.selection.clone(), ctx)
        });
        self.layout_updates
            .recv()
            .await
            .expect("Layout channel should not be closed");
    }

    /// Replace the buffer with the given Markdown.
    async fn markdown(&self, markdown: &str, app: &mut App) {
        let state = InitialBufferState::markdown(markdown);
        self.edit(
            BufferEditAction::ReplaceWith(state),
            EditOrigin::SystemEdit,
            app,
        )
        .await
    }

    /// Assert that the render state has the expected contents, as produced by describing its
    /// `SumTree` of `BlockItem`s.
    #[track_caller]
    fn assert_rendered(&self, ctx: &impl ReadModel, expected: &str) {
        let rendered = self.render.read(ctx, |render_state, _| {
            let content = render_state.content();
            let described_content = content.describe_content();
            described_content.to_string()
        });
        // TODO: Consider using https://github.com/rust-analyzer/expect-test.
        let rendered = rendered.trim();
        let expected = expected.trim();

        if rendered != expected {
            panic!(
                "\nExpected:
====
{expected}
====

Actual:
====
{rendered}
===="
            );
        }
    }

    /// Assert that the buffer has the expected contents.
    #[track_caller]
    fn assert_buffer(&self, ctx: &impl ReadModel, expected: &str) {
        let buffer = self.content.read(ctx, |buffer, _| buffer.debug());

        let buffer = buffer.trim();
        let expected = expected.trim();

        if buffer != expected {
            panic!(
                "\nExpected:
====
{expected}
====

Actual:
====
{buffer}
===="
            );
        }
    }
}

#[test]
fn test_markdown_table_render_starts_at_zero_offset() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
        let state = TestState::new(&mut app);
        state
            .markdown("| Name | Age |\n| --- | --- |\n| Alice | 30 |\n", &mut app)
            .await;

        state.render.read(&app, |render_state, _| {
            let content = render_state.content();
            let block = content
                .block_at_offset(CharOffset::zero())
                .expect("table block should exist at offset 0");
            assert_eq!(block.start_char_offset, CharOffset::zero());
            assert!(matches!(block.item, BlockItem::Table(_)));
        });
    });
}

#[test]
fn test_markdown_table_count_counts_rendered_tables() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
        let state = TestState::new(&mut app);
        state
            .markdown("| Name | Age |\n| --- | --- |\n| Alice | 30 |\n", &mut app)
            .await;

        let count = state
            .render
            .read(&app, |render_state, _| render_state.markdown_table_count());
        assert_eq!(count, 1);
    });
}

/// Reproduces the `RichTextElement::renderable_blocks` build path (the `viewport_items`
/// walk + the same `RenderableXxx::new(item).finish()` mapping) over a large content tree,
/// simulating a scroll, and times how long the *build* portion of `layout` costs.
///
/// This isolates the prime-suspect cost (rebuilding the visible `blocks` Vec every frame)
/// from `block.layout()` (which needs the full warpui presenter and is measured separately
/// in the GUI via the `[perf] richtext block.layout total took` instrumentation).
///
/// The block mix mirrors a double-column diff: most rows are plain paragraphs / text blocks,
/// with blank `TemporaryBlock` spacers interleaved (side-by-side alignment rows).
#[test]
fn bench_renderable_blocks_build_cost() {
    init_logging();
    use crate::render::element::{Empty, RenderableBlock, RenderableHeader, RenderableParagraph, RenderableTextBlock};
    use crate::render::model::test_utils::mock_paragraph;
    use instant::Instant;
    use std::time::Duration;
    use warpui::units::Pixels;

    let line_height = 18.0_f32;
    let width = 1200.0_f32;
    let viewport_height = 800.0_f32;
    let num_lines = 6300usize;

    // Build a large content tree: many paragraphs (matching a large diff's visible block
    // mix). All block `new()` constructors just store the `ViewportItem`, so the build-path
    // cost measured here is the `viewport_items` seek + Vec allocation + Box construction,
    // independent of which concrete block type is used.
    let mut content = SumTree::new();
    for i in 0..num_lines {
        let content_length = 40 + (i % 80);
        content.push(mock_paragraph(line_height, width, content_length));
    }

    let styles = TEST_STYLES;
    let mut model = RenderState::new_for_test(styles, Pixels::new(width), Pixels::new(viewport_height));
    model.set_content(content);

    let visible_px = Pixels::new(viewport_height);
    let width_px = Pixels::new(width);
    let total_height = model.height();

    // Number of scroll steps to traverse the whole document (simulating a scroll pass).
    let steps = 400usize;
    let step_px = (total_height.as_f32() - viewport_height) / steps as f32;

    let mut build_total = Duration::ZERO;
    let mut seek_total = Duration::ZERO;
    let mut max_build = Duration::ZERO;
    let mut blocks_per_frame_sum = 0usize;
    let mut frames = 0usize;

    for s in 0..=steps {
        let scroll_top = Pixels::new((s as f32) * step_px);
        model.set_scroll_top_for_test(scroll_top);

        let content = model.content();

        // (a) Isolate the `viewport_items` seek + walk cost (no Box construction).
        let seek_start = Instant::now();
        let mut seek_count = 0usize;
        for (item, _block) in content.viewport_items(visible_px, width_px, scroll_top) {
            std::hint::black_box(item);
            seek_count += 1;
        }
        seek_total += seek_start.elapsed();

        // (b) The full build path: same mapping `renderable_blocks` uses.
        let build_start = Instant::now();
        let blocks: Vec<Box<dyn RenderableBlock>> = content
            .viewport_items(visible_px, width_px, scroll_top)
            .map(|(item, block)| match block {
                BlockItem::Paragraph(_) => RenderableParagraph::new(item).finish(),
                BlockItem::TextBlock { .. } => RenderableTextBlock::new(item).finish(),
                BlockItem::Header { .. } => RenderableHeader::new(item).finish(),
                BlockItem::TrailingNewLine(_) => Empty::new(item).finish(),
                other => panic!("unexpected block type in diff benchmark: {other:?}"),
            })
            .collect();
        let build_elapsed = build_start.elapsed();
        build_total += build_elapsed;
        max_build = build_elapsed.max(max_build);
        blocks_per_frame_sum += blocks.len();
        frames += 1;
        std::hint::black_box(&blocks);
        std::hint::black_box(seek_count);
    }

    let avg_build_ms = build_total.as_secs_f64() * 1000.0 / frames as f64;
    let avg_seek_ms = seek_total.as_secs_f64() * 1000.0 / frames as f64;
    let max_build_ms = max_build.as_secs_f64() * 1000.0;
    let avg_blocks = blocks_per_frame_sum as f64 / frames as f64;

    eprintln!(
        "[bench] renderable_blocks build: avg {:.3}ms/frame, max {:.3}ms, seek-only avg {:.3}ms/frame, ~{:.0} blocks/frame, {} frames ({} lines, {:.0}px content)",
        avg_build_ms, max_build_ms, avg_seek_ms, avg_blocks, frames, num_lines, total_height.as_f32(),
    );
}
