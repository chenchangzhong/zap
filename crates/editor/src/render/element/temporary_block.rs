use warpui::{
    color::ColorU,
    geometry::{rect::RectF, vector::vec2f},
};
use warp_core::ui::theme::Fill;

use crate::render::model::{BlockItem, Decoration, RenderState, viewport::ViewportItem};

use super::{RenderableBlock, paint::RenderContext};

pub struct RenderableTemporaryBlock {
    viewport_item: ViewportItem,
    decoration: Option<Fill>,
    text_decoration: Vec<Decoration>,
    is_spacer: bool,
}

impl RenderableTemporaryBlock {
    pub fn new(
        viewport_item: ViewportItem,
        decoration: Option<Fill>,
        text_decoration: Vec<Decoration>,
        is_spacer: bool,
    ) -> Self {
        Self {
            viewport_item,
            decoration,
            text_decoration,
            is_spacer,
        }
    }
}

impl RenderableBlock for RenderableTemporaryBlock {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn overlay_decoration(&self) -> Option<Fill> {
        self.decoration
    }

    fn is_spacer(&self) -> bool {
        self.is_spacer
    }

    fn layout(
        &mut self,
        _model: &RenderState,
        _ctx: &mut warpui::LayoutContext,
        _app: &warpui::AppContext,
    ) {
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &warpui::AppContext) {
        // We cannot use `extract_block` macro here since we need to locate the viewport item by content height instead of charoffset
        // (temporary block has an offset of zero).
        let content = model.content();
        let paragraph_block = match content.block_at_height(self.viewport_item.height()) {
            Some(block) => match (&block, block.item) {
                (
                    block,
                    BlockItem::TemporaryBlock {
                        paragraph_block, ..
                    },
                ) => block.temporary_block(paragraph_block),
                other => {
                    log::warn!(
                        "Unexpected block {other:?} at {}",
                        self.viewport_item.block_offset
                    );
                    return;
                }
            },
            None => return,
        };

        // Side-by-side alignment spacers: paint a very light neutral background.
        // Diff line decorations are excluded from spacer rows by EditorWrapper, so
        // no red/green cover-up is needed here.
        if self.is_spacer {
            let background: Fill = ColorU::new(140, 140, 140, 26).into();
            let viewport_width = ctx.visible_bound().size().x();
            for paragraph in paragraph_block.paragraphs() {
                let line_height = paragraph.item.first_line_height();
                let mut y = paragraph.content_origin().y();
                for _ in paragraph.item.frame().lines() {
                    let screen_origin = ctx.content_to_screen(vec2f(0., y));
                    ctx.paint.scene.draw_rect_without_hit_recording(RectF::new(
                        screen_origin,
                        vec2f(viewport_width, line_height),
                    ))
                    .with_background(background);
                    y += line_height;
                }
            }
        }

        let start = paragraph_block.start_char_offset;
        let paragraph_styles = &model.styles().base_text;
        let mut decoration_index = 0;
        for paragraph in paragraph_block.paragraphs() {
            // We could draw text directly since temporary paragraph should have its own decoration and selection state.
            ctx.draw_text(
                paragraph.content_origin(),
                Default::default(),
                paragraph.item.frame(),
                paragraph_styles,
            );

            let paragraph_end = paragraph.end_char_offset();
            for (idx, decoration) in self.text_decoration[decoration_index..].iter().enumerate() {
                if decoration.start + start >= paragraph_end {
                    decoration_index += idx;
                    break;
                }
                if let Some(highlight) = decoration.background {
                    paragraph.draw_highlight(
                        decoration.start + start,
                        decoration.end + start,
                        highlight.into(),
                        ctx,
                        model.max_line(),
                    );
                }
            }
        }
    }

    fn is_temporary(&self) -> bool {
        true
    }
}
