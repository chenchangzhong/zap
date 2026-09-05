use super::{OffsetMap, SelectableTextRun};
use crate::render::model::FrameOffset;
use string_offset::CharOffset;

#[test]
fn test_offset_map_basic() {
    // Baseline test for a no-placeholder OffsetMap. The content_start is non-zero to mimic
    // paragraphs within a code block.
    let map = OffsetMap::new(vec![SelectableTextRun {
        content_start: CharOffset::from(12),
        frame_start: FrameOffset::zero(),
        length: 10,
    }]);

    // The returned offset should be adjusted by the content start.
    assert_eq!(map.to_content(FrameOffset::from(4)), 16.into());
    // Mapping should clamp to run bounds.
    assert_eq!(map.to_content(FrameOffset::from(12)), 22.into());
}

#[test]
fn test_offset_map_placeholders() {
    // Set up an offset map for the following structure:
    //           |placeholder|text|placeholder|text|placeholder...
    // Frame:    0           6    14          24   28
    // Content:  0           1     9          10   14
    let map = OffsetMap::new(vec![
        SelectableTextRun {
            // Even in the zero-state placeholder case, there's an empty content run just before it.
            content_start: CharOffset::zero(),
            frame_start: FrameOffset::zero(),
            length: 0,
        },
        SelectableTextRun {
            content_start: CharOffset::from(1),
            frame_start: FrameOffset::from(6),
            length: 8,
        },
        SelectableTextRun {
            content_start: CharOffset::from(10),
            frame_start: FrameOffset::from(24),
            length: 4,
        },
    ]);

    // Depending on what they're closer to, characters at the start of the frame map to either
    // the start of the line or the first content run.
    assert_eq!(map.to_content(FrameOffset::from(2)), CharOffset::zero());
    assert_eq!(map.to_content(FrameOffset::from(4)), CharOffset::from(1));
    assert_eq!(map.to_frame(CharOffset::zero()), FrameOffset::zero());
    assert_eq!(map.to_frame(CharOffset::from(1)), FrameOffset::from(6));

    // Characters within the first text range map within the range.
    assert_eq!(map.to_content(FrameOffset::from(7)), CharOffset::from(2));
    assert_eq!(map.to_frame(CharOffset::from(2)), FrameOffset::from(7));

    // Characters within the second placeholder map to the closer run.
    assert_eq!(map.to_content(FrameOffset::from(16)), CharOffset::from(9));
    assert_eq!(map.to_content(FrameOffset::from(20)), CharOffset::from(10));
    assert_eq!(map.to_frame(CharOffset::from(9)), FrameOffset::from(14));
    assert_eq!(map.to_frame(CharOffset::from(10)), FrameOffset::from(24));

    // Characters in the last placeholder map to the end of the last text run.
    assert_eq!(map.to_content(FrameOffset::from(28)), CharOffset::from(14));
    assert_eq!(map.to_content(FrameOffset::from(50)), CharOffset::from(14));
    assert_eq!(map.to_frame(CharOffset::from(14)), FrameOffset::from(28));
}

/// Walkthrough test to demonstrate how placeholders are represented in the [`OffsetMap`] and
/// [`TextFrame`].
///
/// This test only runs on macOS because it needs a text-layout implementation for [`EditDelta`]
/// that creates non-empty text frames.
#[test]
#[cfg(target_os = "macos")]
fn test_end_to_end() {
    // Group imports here so they don't cause "unused import" warnings on other targets.

    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, text_layout::LayoutCache,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::{
            layout::TextLayout,
            model::{
                BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
                PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderLayoutOptions, RichTextStyles,
                TableStyle, test_utils::TEST_BASELINE_OFFSET,
            },
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let layout_cache = LayoutCache::new();
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let inline_code = InlineCodeStyle {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            background: ColorU::black(),
            font_color: ColorU::white(),
        };
        let checkbox = CheckBoxStyle {
            border_color: ColorU::white(),
            border_width: 2.,
            icon_path: "bundled/svg/check-thick.svg",
            background: ColorU::black(),
            hover_background: ColorU::black(),
        };
        let horizontal_rule = HorizontalRuleStyle {
            rule_height: 2.,
            color: ColorU::black(),
        };
        let broken_link = BrokenLinkStyle {
            icon_path: "bundled/svg/link-broken-02.svg",
            icon_color: ColorU::black(),
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles,
            code_text: paragraph_styles,
            embedding_text: paragraph_styles,
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: inline_code,
            check_box_style: checkbox,
            horizontal_rule_style: horizontal_rule,
            broken_link_style: broken_link,
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        // Start by creating a buffer with a single line of text that includes a placeholder.
        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));

        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "HelloWorld",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserInitiated,
                selection_handle.clone(),
                ctx,
            );
            buffer.update_content(
                BufferEditAction::InsertPlaceholder {
                    text: "test",
                    location: CharOffset::from(6),
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );

            assert_eq!(
                buffer.debug(),
                "<text>Hello<placeholder_s>test<placeholder_e>World"
            );
            // The placeholder only counts as 1 character, so there are 11 buffer characters.
            assert_eq!(buffer.max_charoffset(), 12.into());
        });

        // Now, lay out the buffer, which should produce a single `Paragraph` block.
        let layout = app.read(|ctx| {
            let delta = buffer_handle.as_ref(ctx).invalidate_layout();
            let text_layout = TextLayout::new(
                &layout_cache,
                font_cache.text_layout_system(),
                &styles,
                1000.,
            );
            delta.layout_delta(
                &text_layout,
                None,
                RenderLayoutOptions::default(),
                None,
                ctx,
            ).0
        });
        let paragraph = match &layout.laid_out_line[..] {
            [BlockItem::Paragraph(paragraph)] => paragraph,
            other => panic!("Unexpected blocks: {other:?}"),
        };

        // The `TextFrame` includes each character we paint: "HellotestWorld".
        let line = &paragraph.frame.lines()[0];
        assert_eq!(
            line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
            14
        );
        assert_eq!(line.first_index(), 0); // The "H" glyph.
        assert_eq!(line.last_index(), 13); // The "d" glyph.

        // Because "test" is a placeholder, it creates a gap in the `OffsetMap`:
        // - Characters 0-4 in the buffer map to characters 0-5 in the text frame ("Hello")
        // - The character at buffer index 5 is the placeholder ("test"). It's not in the map, but
        //   is painted by characters 5-8 in the TextFrame
        // - Characters 6-10 in the buffer map to characters 9-13 in the text frame ("World").
        // Overall, it looks like this:
        // Character:       H  e  l  l  o | t  e  s  t | W  o  r  l  d
        // Buffer Index:    0  1  2  3  4 |     5      | 6  7  8  9 10
        // TextFrame Index: 0  1  2  3  4 | 5  6  7  8 | 9 10 11 12 13

        // In the OffsetMap representation, we only store the runs of non-placeholder characters,
        // while placeholder characters form un-selectable "holes".
        assert_eq!(
            paragraph.offsets.runs,
            vec![
                // The run for "Hello":
                SelectableTextRun {
                    content_start: 0.into(),
                    frame_start: 0.into(),
                    length: 5
                },
                // The run for "World":
                SelectableTextRun {
                    content_start: 6.into(),
                    frame_start: 9.into(),
                    length: 5
                }
            ]
        );
        // To go from a buffer character to a TextFrame character, we find the run that contains
        // it - for offset `i`, this is the run where `run.content_start <= i < run.content_start + run.length`.
        // Going from a TextFrame character to a buffer character is more complicated, because the
        // character might belong to a placeholder. In that case, we find the two adjacent runs and
        // pick the closest.
        // Some examples:

        // The "e" in "Hello":
        assert_eq!(paragraph.offsets.to_frame(1.into()), 1.into());
        assert_eq!(paragraph.offsets.to_content(1.into()), 1.into());

        // The "s" in "test":
        // Since it's in a placeholder, we can only use the placeholder's buffer char offset.
        assert_eq!(paragraph.offsets.to_frame(5.into()), 5.into());
        // When going the other direction, it's closer to World than Hello.
        assert_eq!(paragraph.offsets.to_content(7.into()), 6.into());

        // The "r" in "World": Note that the offsets don't map 1:1 because we have to account for
        // the placeholder gap.
        assert_eq!(paragraph.offsets.to_frame(8.into()), 11.into());
        assert_eq!(paragraph.offsets.to_content(11.into()), 8.into());
    });
}


/// Split-delta (batching): a large initial load is chunked into ROWS_PER_FLUSH rows.
/// The first chunk is laid out immediately (tree not complete yet), and the remaining
/// rows are queued with append coordinates (extent+1..extent+1) so existing row numbers
/// never change — side-by-side spacer alignment stays stable. Repeated
/// `try_layout_pending_edits` calls flush the queue until the tree is complete.
#[test]
#[cfg(target_os = "macos")]
fn test_split_delta_appends_in_flushes() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles, TableStyle,
            WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let inline_code = InlineCodeStyle {
            font_family: paragraph_styles.font_family,
            background: ColorU::black(),
            font_color: ColorU::white(),
        };
        let checkbox = CheckBoxStyle {
            border_color: ColorU::white(),
            border_width: 2.,
            icon_path: "bundled/svg/check-thick.svg",
            background: ColorU::black(),
            hover_background: ColorU::black(),
        };
        let horizontal_rule = HorizontalRuleStyle {
            rule_height: 2.,
            color: ColorU::black(),
        };
        let broken_link = BrokenLinkStyle {
            icon_path: "bundled/svg/link-broken-02.svg",
            icon_color: ColorU::black(),
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles,
            code_text: paragraph_styles,
            embedding_text: paragraph_styles,
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: inline_code,
            check_box_style: checkbox,
            horizontal_rule_style: horizontal_rule,
            broken_link_style: broken_link,
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        // 3000-line code buffer loaded via ReplaceWith, exactly like code review does.
        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });
        buffer_handle.update(&mut app, |buffer, ctx| {
            let mut text = String::new();
            for i in 0..3000 {
                text.push_str(&format!("let value{i} = {i};\n"));
            }
            buffer.update_content(
                BufferEditAction::ReplaceWith(
                    crate::content::buffer::InitialBufferState::plain_text(&text),
                ),
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });

        let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
        assert!(delta.new_lines.len() > 300, "test needs a large file");
        let expected_extent = delta.old_offset.end;

        let mut render_state = RenderState::new_for_test(
            styles.clone(),
            1000.0.into_pixels(),
            600.0.into_pixels(),
        );
        render_state.lazy_layout = true;
        // Code editors scroll horizontally (no wrapping) ⇒ constant line height.
        render_state.width_setting = WidthSetting::InfiniteWidth;

        let empty_hidden = Some(rangemap::RangeSet::new());
        app.read(|ctx| {
            render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
        });

        // 1. Only the first chunk is applied — the tree is NOT complete yet, and the
        // remaining rows are queued with append coordinates.
        let extent_after_first = render_state.content.borrow().extent::<CharOffset>();
        assert!(
            extent_after_first < expected_extent,
            "tree must not be complete after the first chunk (got {extent_after_first:?} of {expected_extent:?})"
        );
        assert!(
            !render_state.pending_edits.lock().is_empty(),
            "remaining rows must be queued for later flushes"
        );

        // 1b. The viewport char range must be bounded by the visible region (this used to
        // return the whole file, forcing syntax highlighting to query the entire tree).
        let visible_total: usize = render_state
            .viewport_charoffset_range()
            .iter()
            .map(|r| r.end.as_usize() - r.start.as_usize())
            .sum();
        assert!(
            visible_total < 2000,
            "viewport char range must be bounded by the viewport, got {visible_total}"
        );

        // 2. Flushing the queue appends the remaining chunks; the tree is append-only
        // (existing row numbers never change) and eventually reaches the full extent.
        let mut flush_iterations = 0;
        loop {
            let had_pending = app.read(|ctx| render_state.try_layout_pending_edits(ctx));
            if !had_pending {
                break;
            }
            flush_iterations += 1;
            assert!(flush_iterations < 100, "flush must terminate");
        }
        assert_eq!(
            render_state.content.borrow().extent::<CharOffset>(),
            expected_extent,
            "tree must be complete after flushing all chunks"
        );

        // 3. All 3000 code rows are present as measured paragraphs after the final flush
        // (batching has no placeholder concept — every block is real layout).
        let content = render_state.content.borrow();
        let mut cursor = content.cursor::<CharOffset, CharOffset>();
        cursor.descend_to_first_item(&content, |_| true);
        let mut paragraph_rows = 0usize;
        while let Some(item) = cursor.item() {
            match item {
                crate::render::model::BlockItem::Paragraph(_) => paragraph_rows += 1,
                crate::render::model::BlockItem::RunnableCodeBlock { paragraph_block, .. } => {
                    paragraph_rows += paragraph_block.paragraphs().len();
                }
                _ => {}
            }
            cursor.next();
        }
        assert_eq!(paragraph_rows, 3000, "all code rows must be present after flush");
    });
}

/// Side-by-side spacer blocks (TemporaryBlocks) must not be inserted while the tree is
/// still incomplete: during split-delta layout their `insert_before` line can exceed the
/// current tree, and `reset_temporary_block` would treat it as past-the-end and drop it —
/// permanently misaligning diff rows after scrolling. They must be deferred until the
/// queue of pending Edit chunks is drained (tree complete), then land at the right line.
#[test]
#[cfg(target_os = "macos")]
fn test_temporary_blocks_deferred_until_tree_complete() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            edit::TemporaryBlock,
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            LineCount, PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles,
            TableStyle, WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });
        buffer_handle.update(&mut app, |buffer, ctx| {
            let mut text = String::new();
            for i in 0..3000 {
                text.push_str(&format!("let value{i} = {i};\n"));
            }
            buffer.update_content(
                BufferEditAction::ReplaceWith(
                    crate::content::buffer::InitialBufferState::plain_text(&text),
                ),
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });

        let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
        let expected_extent = delta.old_offset.end;

        let mut render_state = RenderState::new_for_test(
            styles.clone(),
            1000.0.into_pixels(),
            600.0.into_pixels(),
        );
        render_state.lazy_layout = true;
        render_state.width_setting = WidthSetting::InfiniteWidth;

        let empty_hidden = Some(rangemap::RangeSet::new());
        app.read(|ctx| {
            render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
        });

        // 1. First chunk applied — tree incomplete (1500-line spacer target is out of range).
        assert!(
            render_state.content.borrow().extent::<CharOffset>() < expected_extent,
            "tree must not be complete after the first chunk"
        );

        // 2. Queue a side-by-side spacer anchored at line 1500 and preload (which drains the
        //    layout channel into the pending queue and flushes it).
        render_state.add_temporary_blocks(
            vec![TemporaryBlock {
                content: "spacer".to_string(),
                insert_before: 1500.into(),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            }],
            false,
        );
        app.read(|ctx| {
            render_state.preload_pending_edits(ctx);
        });

        // The spacer must NOT be in the tree yet (deferred), and pending Edit chunks remain.
        let tree_has_spacer = |render_state: &RenderState| -> bool {
            let content = render_state.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            while let Some(item) = cursor.item() {
                if matches!(item, BlockItem::TemporaryBlock { .. }) {
                    return true;
                }
                cursor.next();
            }
            false
        };
        assert!(!tree_has_spacer(&render_state), "spacer must be deferred while tree is incomplete");
        assert!(
            render_state.pending_edits.lock().iter().any(|p| matches!(
                p,
                crate::render::model::PendingLayout::Edit { .. }
            )),
            "pending edit chunks must remain after preload"
        );

        // 3. Flush the rest of the queue; once the tree is complete the spacer must land at
        //    line 1500.
        loop {
            let had_pending = app.read(|ctx| render_state.try_layout_pending_edits(ctx));
            if !had_pending {
                break;
            }
        }
        assert_eq!(
            render_state.content.borrow().extent::<CharOffset>(),
            expected_extent,
            "tree must be complete after flushing all chunks"
        );
        let spacer_at_1500 = {
            let content = render_state.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            let mut found = false;
            while let Some(item) = cursor.item() {
                if matches!(item, BlockItem::TemporaryBlock { .. })
                    && cursor.end_seek_position() == 1500.into()
                {
                    found = true;
                    break;
                }
                cursor.next();
            }
            found
        };
        assert!(spacer_at_1500, "spacer must be inserted at line 1500 once the tree is complete");
    });
}

/// 回归测试:mark_lazy 的同步补排空在「队列空 + 树不完整」(树尚未从 channel
/// 收到内容 chunk)时直达 `layout_temporary_blocks`——旧实现无条件 reset,
/// 越过树尾的整批 spacer 被 reset 的越界 break **静默销毁**(live 实测 21 块
/// 全丢、该列空行永久不渲染,直到重新创建 view)。新实现按当前树行数分区:
/// 已覆盖的立即插入,未覆盖的作为增量条目放回队列,树长后由 flush 补齐——
/// 任何瞬态树状态下 spacer 数据都不丢失。
#[test]
fn test_layout_temporary_blocks_never_destroys_uncovered_spacers() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            edit::TemporaryBlock,
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles, TableStyle,
            WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            let mut text = String::new();
            for i in 0..1000 {
                text.push_str(&format!("let v{i} = {i};\n"));
            }
            buffer.update_content(
                BufferEditAction::ReplaceWith(
                    crate::content::buffer::InitialBufferState::plain_text(&text),
                ),
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });

        let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
        let expected_extent = delta.old_offset.end;

        let mut render_state = RenderState::new_for_test(
            styles.clone(),
            1000.0.into_pixels(),
            600.0.into_pixels(),
        );
        render_state.lazy_layout = true;
        render_state.width_setting = WidthSetting::InfiniteWidth;
        let empty_hidden = Some(rangemap::RangeSet::new());

        // 模拟 live 崩坏时序:内容 chunk 尚未从 channel 排入(树近乎为空)时,
        // spacer 直达 `layout_temporary_blocks`。
        let spacer_blocks = vec![
            TemporaryBlock {
                content: " ".repeat(2),
                insert_before: 100.into(),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".repeat(3),
                insert_before: 250.into(),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".repeat(7),
                insert_before: 500.into(),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
        ];
        let boundaries: Vec<usize> =
            spacer_blocks.iter().map(|b| b.insert_before.as_usize()).collect();
        let tree_rows_before = render_state.content_row_count();
        app.read(|ctx| render_state.layout_temporary_blocks(spacer_blocks, false, ctx));

        // 无损分区:边界 ≤ 当前树行数的块立即在树上,其数量必须精确等于
        // 已覆盖边界数(不多插、不少插);未覆盖的放回队列,绝不销毁。
        let (tree_n, _) = render_state.spacer_stats();
        let expected_covered = boundaries.iter().filter(|b| **b <= tree_rows_before).count();
        assert_eq!(
            tree_n, expected_covered,
            "树 {tree_rows_before} 行时应有 {expected_covered} 块在树上"
        );
        assert!(
            render_state.pending_edits.lock().iter().any(|p| matches!(
                p,
                crate::render::model::PendingLayout::TemporaryBlocks { blocks, .. } if !blocks.is_empty()
            )),
            "未覆盖 spacer 必须留在队列,不能被销毁"
        );

        // 内容 chunk 与 spacer 余量全部排空后,三块必须全部在树上。
        app.read(|ctx| {
            render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
        });
        loop {
            let had_pending = app.read(|ctx| render_state.try_layout_pending_edits(ctx));
            if !had_pending {
                break;
            }
        }
        assert_eq!(
            render_state.content.borrow().extent::<CharOffset>(),
            expected_extent,
            "tree must be complete after flushing all chunks"
        );
        assert_eq!(render_state.spacer_stats().0, 3, "排空后 3 块 spacer 全部上树");
    });
}

/// Verifies that when the pending queue contains ONLY TemporaryBlocks (no Edit items),
/// `layout_temporary_blocks` defers the blocks rather than processing them immediately.
///
/// The old check (`has_pending_edits` — only looked for `PendingLayout::Edit` items)
/// would return `false` when the queue only had `TemporaryBlocks`, causing
/// `reset_temporary_block` to be called immediately. If the async channel had not
/// yet delivered all content, spacers with `insert_before` exceeding the current
/// tree height would be silently dropped, permanently misaligning side-by-side diffs.
///
/// The fix changes the check to `!pending.is_empty()` (any pending layout item),
/// ensuring blocks are always appended to the queue when it is non-empty and
/// processed in FIFO order after all prior items.
#[test]
#[cfg(target_os = "macos")]
fn test_temporary_blocks_deferred_when_queue_only_has_temporary_blocks() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            edit::TemporaryBlock,
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            LineCount, PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles,
            TableStyle, WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        // Set up a small buffer (so the tree is "complete" for a small spacer).
        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "x\n",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });

        let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());

        let mut render_state = RenderState::new_for_test(
            styles.clone(),
            1000.0.into_pixels(),
            600.0.into_pixels(),
        );
        render_state.lazy_layout = true;
        render_state.width_setting = WidthSetting::InfiniteWidth;

        let empty_hidden = Some(rangemap::RangeSet::new());
        app.read(|ctx| {
            render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
        });

        // 1. Tree is complete (small buffer, one layout call).
        let tree_extent = render_state.content.borrow().extent::<CharOffset>();
        assert!(
            tree_extent > CharOffset::zero(),
            "tree should have some content"
        );

        // 2. Queue only has TemporaryBlocks (no Edit items) — the OLD bug scenario.
        //    Add a spacer targeting a valid position within the tree.
        render_state.add_temporary_blocks(
            vec![TemporaryBlock {
                content: "spacer".to_string(),
                insert_before: LineCount::from(1),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            }],
            false,
        );

        // `add_temporary_blocks` submits through the async layout channel, and
        // `new_for_test` never spawns the channel handler, so mimic the handler's
        // lazy-mode behavior by draining the channel into the pending queue (exactly
        // what `preload_pending_edits` does). The queue now holds TemporaryBlocks but
        // NO Edit items.
        while let Ok(action) = render_state.layout_rx.try_recv() {
            if let crate::render::model::LayoutAction::LayoutTemporaryBlock {
                blocks,
                replace_existing,
            } = action
            {
                render_state
                    .pending_edits
                    .lock()
                    .push(crate::render::model::PendingLayout::TemporaryBlocks {
                        blocks,
                        replace_existing,
                    });
            }
        }
        let queue_before = render_state.pending_edits.lock().len();
        assert!(
            queue_before > 0,
            "pending queue should have items after add_temporary_blocks"
        );
        let has_only_temp_blocks = render_state
            .pending_edits
            .lock()
            .iter()
            .all(|p| matches!(
                p,
                crate::render::model::PendingLayout::TemporaryBlocks { .. }
            ));
        assert!(
            has_only_temp_blocks,
            "queue should only have TemporaryBlocks (no Edit items)"
        );

        // 3. 新语义(无损分区 + 覆盖即插):树完整 → 第二批立即以替换语义
        //    插入(清旧插新),队列长度不变;推回只发生在树未覆盖其边界时
        //    (见 test_layout_temporary_blocks_never_destroys_uncovered_spacers
        //    ——旧实现无条件推回,正是 spacer 压队尾、等整棵树排空才出现的根源)。
        app.read(|ctx| {
            render_state.layout_temporary_blocks(
                vec![TemporaryBlock {
                    content: "spacer2".to_string(),
                    insert_before: LineCount::from(1),
                    line_decoration: None,
                    inline_text_decorations: Vec::new(),
                }],
                true,
                ctx,
            );
        });
        assert_eq!(
            render_state.pending_edits.lock().len(),
            queue_before,
            "树已覆盖时立即插入,不推回队列"
        );

        // 4. Drain the queue; the deferred batches are applied once it empties.
        loop {
            let more = app.read(|ctx| render_state.try_layout_pending_edits(ctx));
            if !more {
                break;
            }
        }

        // 5. After queue is drained, the spacer must be in the tree at line 1.
        let spacer_at_1 = {
            let content = render_state.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            let mut found = false;
            while let Some(item) = cursor.item() {
                if matches!(item, BlockItem::TemporaryBlock { .. })
                    && cursor.end_seek_position() == LineCount::from(1)
                {
                    found = true;
                    break;
                }
                cursor.next();
            }
            found
        };
        assert!(
            spacer_at_1,
            "spacer must be at line 1 once queue is fully drained"
        );
    });
}

/// Regression test: queued edits that arrive AFTER a split (chunked) Edit delta must not
/// be applied in the same flush pass. A split pushes its remainder chunk back onto the
/// pending queue and the content tree is then incomplete; a later edit's CharOffsets are
/// relative to the buffer state after the FULL earlier edit, so applying it against the
/// partial tree inserts its rows at the wrong position (or loses them entirely).
///
/// Trigger: queue `[Edit(600 lines → splits), Edit(insert at row 500)]`, then flush via
/// `preload_pending_edits` + `try_layout_pending_edits`. The inserted line must land at
/// row 500 once the queue is fully drained.
#[test]
#[cfg(target_os = "macos")]
fn test_pending_edit_after_split_delta_waits_for_tree_completion() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };
    use vec1::vec1;

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, BufferEditAction::ReplaceWith, EditOrigin, InitialBufferState},
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            LineCount, PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles,
            TableStyle, WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });
        // 600 uniform 5-char lines ("0000\n".."0599\n"): 0-indexed line i starts at
        // 1-indexed char 1 + 5*i.
        buffer_handle.update(&mut app, |buffer, ctx| {
            let mut text = String::new();
            for i in 0..600 {
                text.push_str(&format!("{i:04}\n"));
            }
            buffer.update_content(
                ReplaceWith(InitialBufferState::plain_text(&text)),
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });

        // Delta 1: full-content replacement — 600 rows > ROWS_PER_FLUSH (300) → will split.
        let delta_big = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());

        // Insert "TAIL_A\n" at the start of line 500 (char 2501) — inside the region the
        // first flush does NOT apply (rows 300..600) — then take a line-aligned delta over
        // the tail region, mirroring a real buffer edit queued after the big one.
        buffer_handle.update(&mut app, |buffer, ctx| {
            let tail_at = CharOffset::from(1 + 5 * 500);
            let edits = vec1![("TAIL_A\n".to_string(), tail_at..tail_at)];
            buffer.update_content(
                BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );
        });
        let delta_tail = app.read(|ctx| {
            let buffer = buffer_handle.as_ref(ctx);
            buffer.invalidate_layout_for_range(
                CharOffset::from(1 + 5 * 500)..buffer.max_charoffset(),
            )
        });

        let mut render_state = RenderState::new_for_test(
            styles.clone(),
            1000.0.into_pixels(),
            600.0.into_pixels(),
        );
        render_state.lazy_layout = true;
        render_state.width_setting = WidthSetting::InfiniteWidth;

        // Queue both edits (big first, tail second) and flush exactly like the UI does:
        // one synchronous preload, then per-frame budgeted flushes until drained.
        render_state.pending_edits.lock().push(
            crate::render::model::PendingLayout::Edit {
                delta: delta_big,
                hidden_ranges: None,
            },
        );
        render_state.pending_edits.lock().push(
            crate::render::model::PendingLayout::Edit {
                delta: delta_tail,
                hidden_ranges: None,
            },
        );

        app.read(|ctx| {
            render_state.preload_pending_edits(ctx);
        });
        loop {
            let more = app.read(|ctx| render_state.try_layout_pending_edits(ctx));
            if !more {
                break;
            }
        }

        // The inserted TAIL line is the only 7-char paragraph; it must sit at row 500.
        let tail_row = {
            let content = render_state.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            let mut found = None;
            while let Some(item) = cursor.item() {
                if let BlockItem::Paragraph(paragraph) = item
                    && paragraph.content_length == CharOffset::from(7)
                {
                    found = Some(cursor.end_seek_position() - LineCount::from(1));
                    break;
                }
                cursor.next();
            }
            found
        };
        assert_eq!(
            tail_row,
            Some(LineCount::from(500)),
            "TAIL must land at row 500 once the queue is drained — a later edit must not \
             be applied against the partial tree left by a split delta"
        );
    });
}

/// 端到端静态对齐验证(side-by-side 两列"对得上"的最终判据):
/// 左列(old 内容 + `compute_spacers` 左 spacer)与右列(new 内容 + 右 spacer)各自
/// 构建真实内容树后,任何未变行 o↔n 必须满足 y_left(row o) == y_right(row n),
/// 且两列总高度相等。diff、spacer 计算、树插入任何一环出错都会以精确像素差暴露。
///
/// 对照 Zed 的 `check_invariants`(split.rs):每次重算后断言两列行数对齐。
#[test]
#[cfg(target_os = "macos")]
fn test_side_by_side_columns_end_to_end_row_alignment() {
    use std::collections::HashMap;

    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            edit::TemporaryBlock,
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            LineCount, PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles,
            TableStyle, WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };
    use crate::content::diff::{compute_spacers, diff_lines};

    // 混合增/删/改的真实感内容(< 300 行,不触发分批 chunk 布局)。
    let old_lines: Vec<String> = (0..220).map(|i| format!("let value{i} = {i};")).collect();
    let mut new_lines = old_lines.clone();
    new_lines.splice(5..5, (0..7).map(|i| format!("header {i}")).collect::<Vec<_>>());
    new_lines.drain(120..124);
    for line in new_lines.iter_mut().skip(150).take(3) {
        *line += " // changed";
    }
    new_lines.splice(153..153, (0..5).map(|i| format!("extra {i}")).collect::<Vec<_>>());
    let cut = new_lines.len() - 9;
    new_lines.truncate(cut);

    let old_text: String = old_lines.iter().map(|l| format!("{l}\n")).collect();
    let new_text: String = new_lines.iter().map(|l| format!("{l}\n")).collect();

    let hunks = diff_lines(&old_text, &new_text);
    let (left_blocks, right_blocks) = compute_spacers(&hunks);
    assert!(!hunks.is_empty(), "内容必须有变更");
    assert!(
        !left_blocks.is_empty() || !right_blocks.is_empty(),
        "行数不等替换/纯增删必须产生 spacer,否则测试没有覆盖对齐路径"
    );

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let build_column =
            |app: &mut App, text: &str, blocks: Vec<TemporaryBlock>| -> RenderState {
                let buffer_handle =
                    app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
                let selection_handle =
                    app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
                buffer_handle.update(app, |buffer, ctx| {
                    buffer.update_content(
                        BufferEditAction::Insert {
                            text: "x",
                            style: Default::default(),
                            override_text_style: None,
                        },
                        EditOrigin::SystemEdit,
                        selection_handle.clone(),
                        ctx,
                    );
                });
                buffer_handle.update(app, |buffer, ctx| {
                    buffer.update_content(
                        BufferEditAction::ReplaceWith(
                            crate::content::buffer::InitialBufferState::plain_text(text),
                        ),
                        EditOrigin::SystemEdit,
                        selection_handle.clone(),
                        ctx,
                    );
                });
                let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
                let mut render_state = RenderState::new_for_test(
                    styles.clone(),
                    1000.0.into_pixels(),
                    600.0.into_pixels(),
                );
                render_state.lazy_layout = true;
                render_state.width_setting = WidthSetting::InfiniteWidth;
                let empty_hidden = Some(rangemap::RangeSet::new());
                app.read(|ctx| {
                    render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
                });
                render_state.add_temporary_blocks(blocks, false);
                app.read(|ctx| {
                    render_state.preload_pending_edits(ctx);
                });
                loop {
                    let had_pending =
                        app.read(|ctx| render_state.try_layout_pending_edits(ctx));
                    if !had_pending {
                        break;
                    }
                }
                render_state
            };

        let left_column = build_column(&mut app, &old_text, left_blocks);
        let right_column = build_column(&mut app, &new_text, right_blocks);

        // boundary(累计行数)→ 该边界的累计像素高度(含挂在该边界之前的 spacer)。
        // 树顺序为 ... row_(k-1), spacer@k, row_k ...,零行 spacer 不推进行号但推进
        // 高度,因此后写覆盖即可得到"row k 视觉顶部 y(含上方 spacer)"。
        let boundary_heights = |column: &RenderState| -> (HashMap<usize, f32>, f32) {
            let content = column.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            let mut y = 0.0f32;
            let mut total = 0.0f32;
            let mut map = HashMap::new();
            map.insert(0usize, 0.0f32);
            while let Some(item) = cursor.item() {
                let height = item.height().as_f32();
                y += height;
                total += height;
                map.insert(cursor.end_seek_position().as_usize(), y);
                cursor.next();
            }
            (map, total)
        };

        let (left_y, left_total) = boundary_heights(&left_column);
        let (right_y, right_total) = boundary_heights(&right_column);

        assert!(
            (left_total - right_total).abs() < 0.01,
            "两列总高度必须相等: left={left_total} right={right_total}"
        );

        let assert_row_aligned = |left_y: &HashMap<usize, f32>,
                                 right_y: &HashMap<usize, f32>,
                                 o: usize,
                                 n: usize,
                                 what: &str| {
            let ly = *left_y.get(&o).unwrap_or_else(|| panic!("左列缺 row {o} 边界"));
            let ry = *right_y.get(&n).unwrap_or_else(|| panic!("右列缺 row {n} 边界"));
            assert!(
                (ly - ry).abs() < 0.01,
                "{what}: old row {o} (y={ly}) ↔ new row {n} (y={ry}) 错位"
            );
        };

        let mut old_consumed = 0usize;
        let mut new_consumed = 0usize;
        for hunk in &hunks {
            // hunk 前未变区逐行对齐
            for k in 0..(hunk.old_rows.start - old_consumed) {
                assert_row_aligned(
                    &left_y,
                    &right_y,
                    old_consumed + k,
                    new_consumed + k,
                    "未变行",
                );
            }
            // 替换 hunk 内前 min(old, new) 行 1:1 配对
            for k in 0..hunk.old_rows.len().min(hunk.new_rows.len()) {
                assert_row_aligned(
                    &left_y,
                    &right_y,
                    hunk.old_rows.start + k,
                    hunk.new_rows.start + k,
                    "替换行",
                );
            }
            old_consumed = hunk.old_rows.end;
            new_consumed = hunk.new_rows.end;
        }
        // 尾部未变区逐行对齐
        for k in 0..(old_lines.len() - old_consumed) {
            assert_row_aligned(
                &left_y,
                &right_y,
                old_consumed + k,
                new_consumed + k,
                "尾部未变行",
            );
        }
    });
}

/// 大文件(>1000 行)走创建路径时触发 split-delta 分批排空,spacer 边界横跨
/// head 预渲染窗口内外。无论 flush 的拆分/押后策略如何,排空完成后所有
/// spacer 都必须上树——某列总高等于「纯内容行数 × 行高」即该列 spacer
/// 全部丢失的量化证据(实测曾出现两列各丢 292 行的回归)。
#[test]
fn test_side_by_side_spacers_survive_lazy_chunked_layout() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            LineCount, ParagraphStyles, RenderState, RichTextStyles, TableStyle, WidthSetting,
            test_utils::TEST_BASELINE_OFFSET,
        },
    };
    use crate::content::diff::{compute_spacers, diff_lines};

    let old_lines: Vec<String> = (0..1200).map(|i| format!("let v{i} = {i};")).collect();
    let mut new_lines = old_lines.clone();
    // 中部插入(head 窗口 1000 行之外)、尾部插入(更深处)、文件头小 hunk
    // (head 窗口内)——左列三块 spacer;中部删除——右列一块 spacer。
    new_lines.splice(400..400, (0..50).map(|i| format!("ins {i}")).collect::<Vec<_>>());
    new_lines.splice(1100..1103, (0..9).map(|i| format!("tail {i}")).collect::<Vec<_>>());
    new_lines.splice(5..5, (0..3).map(|i| format!("head {i}")).collect::<Vec<_>>());
    new_lines.drain(600..605);

    let old_text: String = old_lines.iter().map(|l| format!("{l}\n")).collect();
    let new_text: String = new_lines.iter().map(|l| format!("{l}\n")).collect();

    let hunks = diff_lines(&old_text, &new_text);
    let (left_blocks, right_blocks) = compute_spacers(&hunks);
    let spacer_rows = |blocks: &[crate::content::edit::TemporaryBlock]| -> usize {
        blocks.iter().map(|b| b.content.lines().count()).sum()
    };
    let (left_spacer_rows, right_spacer_rows) =
        (spacer_rows(&left_blocks), spacer_rows(&right_blocks));
    assert!(
        left_spacer_rows >= 55 && right_spacer_rows >= 5,
        "spacer 行数不符: left={left_spacer_rows} right={right_spacer_rows} \
         (left_blocks={} right_blocks={})",
        left_blocks.len(),
        right_blocks.len()
    );

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache.load_system_font("Arial").expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: None,
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::black(),
                scrollbar_active_thumb_color: ColorU::black(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let build_column =
            |app: &mut App, text: &str, blocks: Vec<crate::content::edit::TemporaryBlock>| -> RenderState {
                let buffer_handle =
                    app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
                let selection_handle =
                    app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
                buffer_handle.update(app, |buffer, ctx| {
                    buffer.update_content(
                        BufferEditAction::ReplaceWith(
                            crate::content::buffer::InitialBufferState::plain_text(text),
                        ),
                        EditOrigin::SystemEdit,
                        selection_handle.clone(),
                        ctx,
                    );
                });
                let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
                let mut render_state = RenderState::new_for_test(
                    styles.clone(),
                    1000.0.into_pixels(),
                    600.0.into_pixels(),
                );
                render_state.lazy_layout = true;
                render_state.width_setting = WidthSetting::InfiniteWidth;
                let empty_hidden = Some(rangemap::RangeSet::new());
                // 与创建路径一致:先排内容(>300 行触发 split-delta 余量推回),
                // 再入队 spacer,随后无预算排空 + 逐帧排空到队列干净。
                app.read(|ctx| {
                    render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
                });
                // 渐进插入断言:懒加载每帧之后,「当前树已覆盖」的 spacer 必须
                // 已经全部在树上——覆盖即插一旦退化为「排空后一次性插入」,
                // 懒加载期间视口区域就会缺失对齐空行。
                let boundaries: Vec<usize> =
                    blocks.iter().map(|b| b.insert_before.as_usize()).collect();
                render_state.add_temporary_blocks(blocks, false);
                app.read(|ctx| {
                    render_state.preload_pending_edits(ctx);
                });
                loop {
                    let had_pending =
                        app.read(|ctx| render_state.try_layout_pending_edits(ctx));
                    let tree_rows = render_state.content_row_count();
                    let expected_covered =
                        boundaries.iter().filter(|b| **b <= tree_rows).count();
                    let (n, _) = render_state.spacer_stats();
                    assert!(
                        n >= expected_covered,
                        "懒加载中途 spacer 缺失: 树 {tree_rows} 行,树上 {n} 块, \
                         应 ≥ {expected_covered} 块"
                    );
                    if !had_pending {
                        break;
                    }
                }
                render_state
            };

        let left_column = build_column(&mut app, &old_text, left_blocks);
        let right_column = build_column(&mut app, &new_text, right_blocks);

        let left_total = left_column.content.borrow().summary().height;
        let right_total = right_column.content.borrow().summary().height;

        let spacer_stats = |column: &RenderState| -> (usize, f32) {
            let content = column.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();
            cursor.descend_to_first_item(&content, |_| true);
            let (mut n, mut h) = (0usize, 0.0f32);
            while let Some(item) = cursor.item() {
                if matches!(item, BlockItem::TemporaryBlock { .. }) {
                    n += 1;
                    h += item.height().as_f32();
                }
                cursor.next();
            }
            (n, h)
        };

        let (left_n, left_h) = spacer_stats(&left_column);
        let (right_n, right_h) = spacer_stats(&right_column);
        assert!(
            (left_total - right_total).abs() < 0.01,
            "两列总高必须相等: left={left_total} right={right_total}"
        );
        assert!(
            left_h > 0.0 && right_h > 0.0,
            "spacer 丢失: 左列 {left_n} 块/{left_h}px,右列 {right_n} 块/{right_h}px \
             (期望左 ≥2 块、右 ≥1 块且高度大于 0)"
        );
    });
}

/// 复刻 crates/editor/src/render/element/temporary_block.rs 的实际 diff(纯删除型:
/// 头部 7→4、中部 29→23,spacer 全部落在右列,第一块紧贴文件头)。创建路径下
/// 右列两块曾互相销毁(head covered 先插、mark_lazy rest 以 replace 语义 reset
/// 清旧)——回归断言:两列 spacer 数据完整、逐边界 y 对齐、总高相等。
#[test]
fn test_side_by_side_deletion_diff_right_column_spacers_survive() {
    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, units::IntoPixels,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            edit::TemporaryBlock,
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::model::{
            BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
            PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderState, RichTextStyles,
            TableStyle, WidthSetting, test_utils::TEST_BASELINE_OFFSET,
        },
    };
    use crate::content::diff::{compute_spacers, diff_lines};

    // 复刻 crates/editor/src/render/element/temporary_block.rs 的实际 diff 形态:
    // 头部替换(旧 7 行 → 新 4 行)+ 中部替换(旧 29 行 → 新 23 行)——纯删除型
    // diff,spacer 全部落在右列,且第一块紧贴文件头(insert_before 最小)。
    let old_lines: Vec<String> = (0..133).map(|i| format!("let old{i} = {i};")).collect();
    let mut new_lines: Vec<String> = Vec::new();
    new_lines.extend((0..4).map(|i| format!("let new_head{i} = {i};")));
    new_lines.extend((7..77).map(|i| format!("let old{i} = {i};")));
    new_lines.extend((0..23).map(|i| format!("let new_mid{i} = {i};")));
    new_lines.extend((106..133).map(|i| format!("let old{i} = {i};")));
    assert_eq!(old_lines.len(), 133);
    assert_eq!(new_lines.len(), 124);

    let old_text: String = old_lines.iter().map(|l| format!("{l}\n")).collect();
    let new_text: String = new_lines.iter().map(|l| format!("{l}\n")).collect();

    let hunks = diff_lines(&old_text, &new_text);
    let (left_blocks, right_blocks) = compute_spacers(&hunks);
    // 纯删除 diff:右列 spacer = 旧行数 - 新行数 = 9 行;左列 0 块。
    let right_rows: usize = right_blocks.iter().map(|b| b.content.lines().count()).sum();
    assert_eq!(left_blocks.len(), 0, "纯删除 diff 左列不应有 spacer");
    assert_eq!(right_rows, 9, "右列 spacer 总行数应等于净删除行数");

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache.load_system_font("Arial").expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles.clone(),
            code_text: paragraph_styles.clone(),
            embedding_text: paragraph_styles.clone(),
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: InlineCodeStyle {
                font_family: paragraph_styles.font_family,
                background: ColorU::black(),
                font_color: ColorU::white(),
            },
            check_box_style: CheckBoxStyle {
                border_color: ColorU::white(),
                border_width: 2.,
                icon_path: "bundled/svg/check-thick.svg",
                background: ColorU::black(),
                hover_background: ColorU::black(),
            },
            horizontal_rule_style: HorizontalRuleStyle {
                rule_height: 2.,
                color: ColorU::black(),
            },
            broken_link_style: BrokenLinkStyle {
                icon_path: "bundled/svg/link-broken-02.svg",
                icon_color: ColorU::black(),
            },
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        let build_column =
            |app: &mut App, text: &str, blocks: Vec<TemporaryBlock>| -> RenderState {
                let buffer_handle =
                    app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
                let selection_handle =
                    app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
                buffer_handle.update(app, |buffer, ctx| {
                    buffer.update_content(
                        BufferEditAction::ReplaceWith(
                            crate::content::buffer::InitialBufferState::plain_text(text),
                        ),
                        EditOrigin::SystemEdit,
                        selection_handle.clone(),
                        ctx,
                    );
                });
                let delta = app.read(|ctx| buffer_handle.as_ref(ctx).invalidate_layout());
                let mut render_state = RenderState::new_for_test(
                    styles.clone(),
                    1000.0.into_pixels(),
                    600.0.into_pixels(),
                );
                render_state.lazy_layout = true;
                render_state.width_setting = WidthSetting::InfiniteWidth;
                let empty_hidden = Some(rangemap::RangeSet::new());
                app.read(|ctx| {
                    render_state.layout_edit_delta(delta, empty_hidden.clone(), ctx);
                });
                render_state.add_temporary_blocks(blocks, false);
                app.read(|ctx| {
                    render_state.preload_pending_edits(ctx);
                });
                loop {
                    let had_pending =
                        app.read(|ctx| render_state.try_layout_pending_edits(ctx));
                    if !had_pending {
                        break;
                    }
                }
                render_state
            };

        let left_column = build_column(&mut app, &old_text, left_blocks);
        let right_column = build_column(&mut app, &new_text, right_blocks);

        let left_total = left_column.content.borrow().summary().height;
        let right_total = right_column.content.borrow().summary().height;
        assert!(
            (left_total - right_total).abs() < 0.01,
            "两列总高必须相等: left={left_total} right={right_total}"
        );
        assert_eq!(
            right_column.spacer_stats().0,
            2,
            "排空后右列两块 spacer 都必须在树上(回归:第二块曾以 replace 语义清掉第一块)"
        );
    });
}

