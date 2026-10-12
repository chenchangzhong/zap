use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::icons::Icon as WarpIcon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill as WarpThemeFill, WarpTheme};
use warpui::{
    assets::asset_cache::AssetSource,
    elements::{
        CacheOption, ChildAnchor, ConstrainedBox, Container, CornerRadius, Element,
        Fill as ElementFill, Image, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Radius, Stack,
    },
};

use crate::ai::agent::conversation::ConversationStatus;
use crate::terminal::CLIAgent;
use crate::themes::theme::Fill as ThemeFill;

/// Sizing configuration for the icon circle and its status badge.
pub(crate) struct IconWithStatusSizing {
    pub(crate) icon_size: f32,
    pub(crate) padding: f32,
    pub(crate) badge_icon_size: f32,
    pub(crate) badge_padding: f32,
    /// The overall constrained size for the stack.
    /// When set, overrides the default `icon_size + padding * 2`.
    pub(crate) overall_size_override: Option<f32>,
    /// Offset of the status badge from the bottom-right corner of the circle.
    /// Positive x pushes right, positive y pushes down.
    pub(crate) badge_offset: (f32, f32),
}

// ===== 自上游移植：以单一 `total_size` 按比例推导全部子组件几何。
// orchestration pill bar 需要 `CIRCLE_RATIO` / `circle_size` / `StatusBadgeStyle` /
// `BadgeInnerShape` 以及 `render_icon_with_status_with_badge_style`。
// 本地既有的 `IconWithStatusSizing` 路径保持不变。

// 子组件尺寸比例，均以 `total_size` 为基准：品牌圆约占 76% 宽，状态徽章约占 57% 宽，
// 徽章右下角锚定在整盒右下角。`CIRCLE_RATIO` 为 `pub(crate)`，便于调用方自行预渲染头像时对齐尺寸。
pub(crate) const CIRCLE_RATIO: f32 = 0.76;
const ICON_RATIO: f32 = 0.43;
const DEFAULT_BADGE_RATIO: f32 = 0.57;
const DEFAULT_BADGE_ICON_RATIO: f32 = 0.34;
const CLOUD_RATIO: f32 = 0.57;
const STATUS_IN_CLOUD_RATIO: f32 = 0.285;

/// 状态徽章几何覆盖项。沿用默认观感时传 [`StatusBadgeStyle::DEFAULT`]。
#[derive(Clone, Copy)]
pub(crate) struct StatusBadgeStyle {
    /// 挖孔环直径占 `total_size` 的比例。
    pub ring_ratio: f32,
    /// 状态图标字形直径占 `total_size` 的比例。
    pub icon_ratio: f32,
    pub inner_shape: BadgeInnerShape,
}

#[derive(Clone, Copy)]
pub(crate) enum BadgeInnerShape {
    Circle,
    RoundedSquare { radius_px: f32 },
}

impl StatusBadgeStyle {
    pub(crate) const DEFAULT: Self = Self {
        ring_ratio: DEFAULT_BADGE_RATIO,
        icon_ratio: DEFAULT_BADGE_ICON_RATIO,
        inner_shape: BadgeInnerShape::Circle,
    };
}

// Neutral 变体没有叠加层，因此填满整个 `total_size` 包围盒。
// 内部字形占 `NEUTRAL_GLYPH_RATIO * total_size`，与旧尺寸（24px 容器装 16px 字形）一致。
const NEUTRAL_GLYPH_RATIO: f32 = 16.0 / 24.0;

/// 返回给定 `total_size` 下的品牌圆直径。
pub(crate) fn circle_size(total: f32) -> f32 {
    total * CIRCLE_RATIO
}

fn icon_size(total: f32) -> f32 {
    total * ICON_RATIO
}

fn circle_padding(total: f32) -> f32 {
    (circle_size(total) - icon_size(total)) / 2.
}

/// 状态徽章挖孔环直径，即徽章实际绘制的足迹。
fn badge_size(total: f32, style: StatusBadgeStyle) -> f32 {
    total * style.ring_ratio
}

fn badge_icon_size(total: f32, style: StatusBadgeStyle) -> f32 {
    total * style.icon_ratio
}

fn badge_padding(total: f32, style: StatusBadgeStyle) -> f32 {
    (badge_size(total, style) - badge_icon_size(total, style)) / 4.
}

fn cloud_icon_size(total: f32) -> f32 {
    total * CLOUD_RATIO
}

fn status_in_cloud_size(total: f32) -> f32 {
    total * STATUS_IN_CLOUD_RATIO
}

/// 叠加层右下角默认越过圆右下边缘的外伸量（占 `total_size` 比例），
/// 已折进 `corner_overlay_offset`，因此多数界面只要传 `0.0`。
const DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE: f32 = 0.19;

/// 返回叠加层 `BottomRight → BottomRight` 锚点所用的像素偏移。
/// 偏移量以包围盒右下角为基准，因此在叠加层位于盒子右上方（当前唯一渲染情形）时返回负值。
/// `overlay_extra_overhang_ratio` 是叠加到默认外伸量上的带符号比例：
/// * `0.0` —— 叠加层右下角越过圆右下边缘 `DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE * total`。
/// * 正数 —— 进一步推向包围盒右下角。
/// * 负数 —— 向圆心方向收回。
fn corner_overlay_offset(total: f32, overlay_extra_overhang_ratio: f32) -> f32 {
    let total_overhang = DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE + overlay_extra_overhang_ratio;
    -((1.0 - CIRCLE_RATIO) - total_overhang) * total
}

const ANTIGRAVITY_LOGO_PATH: &str = "bundled/svg/antigravity.svg";
const OH_MY_PI_LOGO_PATH: &str = "bundled/svg/oh_my_pi.svg";

pub(crate) fn render_cli_agent_logo(
    agent: CLIAgent,
    icon_color: WarpThemeFill,
    fallback_icon_color: WarpThemeFill,
) -> Box<dyn Element> {
    let multi_color_logo_path = match agent {
        CLIAgent::Antigravity => Some(ANTIGRAVITY_LOGO_PATH),
        CLIAgent::OhMyPi => Some(OH_MY_PI_LOGO_PATH),
        _ => None,
    };
    if let Some(path) = multi_color_logo_path {
        Image::new(
            AssetSource::Bundled { path },
            CacheOption::BySize,
        )
        .finish()
    } else {
        agent
            .icon()
            .map(|icon| icon.to_warpui_icon(icon_color).finish())
            .unwrap_or_else(|| {
                WarpIcon::Terminal
                    .to_warpui_icon(fallback_icon_color)
                    .finish()
            })
    }
}

/// What to render inside the circle.
pub(crate) enum IconWithStatusVariant {
    /// A generic icon with a given color on an overlay background.
    Neutral {
        icon: WarpIcon,
        icon_color: WarpThemeFill,
    },
    /// A pre-built icon element on an overlay background.
    NeutralElement { icon_element: Box<dyn Element> },
    /// An Oz agent icon on the theme background.
    OzAgent {
        status: Option<ConversationStatus>,
        is_ambient: bool,
    },
    /// A CLI agent icon on the agent's brand color background.
    CLIAgent {
        agent: CLIAgent,
        status: Option<ConversationStatus>,
    },
    /// DSH pane 图标:品牌 logo 画在主题背景上,背景色与图标色和 Oz 智能体一致。
    Dsh {
        icon: WarpIcon,
        status: Option<ConversationStatus>,
    },
    /// 预渲染好的头像 + 可选状态叠加层（ambient 时显示云朵徽章）。
    /// 叠加层锚定在 `total_size` 盒的右下角，与 `avatar` 本身多大无关；因此请传入
    /// 按 [`circle_size`] 预渲染的头像，或本身就填满 `total_size` 盒的元素。
    CustomAvatar {
        avatar: Box<dyn Element>,
        status: Option<ConversationStatus>,
        is_ambient: bool,
    },
}

/// Renders an icon inside a circle with an optional status badge overlay.
pub(crate) fn render_icon_with_status(
    variant: IconWithStatusVariant,
    sizing: &IconWithStatusSizing,
    theme: &WarpTheme,
    badge_ring_background: WarpThemeFill,
) -> Box<dyn Element> {
    let sub_text = theme.sub_text_color(theme.background());

    match variant {
        IconWithStatusVariant::Neutral { icon, icon_color } => {
            let inner = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                .with_width(sizing.icon_size)
                .with_height(sizing.icon_size)
                .finish();
            Container::new(inner)
                .with_uniform_padding(sizing.padding)
                .with_background(internal_colors::fg_overlay_2(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    (sizing.icon_size + sizing.padding * 2.) / 2.,
                )))
                .finish()
        }
        IconWithStatusVariant::NeutralElement { icon_element } => {
            let inner = ConstrainedBox::new(icon_element)
                .with_width(sizing.icon_size)
                .with_height(sizing.icon_size)
                .finish();
            Container::new(inner)
                .with_uniform_padding(sizing.padding)
                .with_background(internal_colors::fg_overlay_2(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    (sizing.icon_size + sizing.padding * 2.) / 2.,
                )))
                .finish()
        }
        IconWithStatusVariant::OzAgent { status, is_ambient } => {
            let icon = if is_ambient {
                WarpIcon::OzCloud
            } else {
                WarpIcon::Oz
            };
            let inner = ConstrainedBox::new(
                icon.to_warpui_icon(theme.main_text_color(theme.background()))
                    .finish(),
            )
            .with_width(sizing.icon_size)
            .with_height(sizing.icon_size)
            .finish();
            let circle = Container::new(inner)
                .with_uniform_padding(sizing.padding)
                .with_background(theme.background())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    (sizing.icon_size + sizing.padding * 2.) / 2.,
                )))
                .finish();
            render_with_optional_status_badge(
                circle,
                status.as_ref(),
                sizing,
                theme,
                badge_ring_background,
            )
        }
        IconWithStatusVariant::CLIAgent { agent, status } => {
            let brand_color = agent
                .brand_color()
                .unwrap_or(ColorU::new(100, 100, 100, 255));
            let icon_color = agent.brand_icon_color();
            let icon_element =
                render_cli_agent_logo(agent, WarpThemeFill::Solid(icon_color), sub_text);
            let inner = ConstrainedBox::new(icon_element)
                .with_width(sizing.icon_size)
                .with_height(sizing.icon_size)
                .finish();
            let background: ElementFill =
                if matches!(agent, CLIAgent::Antigravity | CLIAgent::OhMyPi) {
                    theme.background().into()
                } else {
                    ThemeFill::Solid(brand_color).into()
                };
            let circle = Container::new(inner)
                .with_uniform_padding(sizing.padding)
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    (sizing.icon_size + sizing.padding * 2.) / 2.,
                )))
                .finish();
            render_with_optional_status_badge(
                circle,
                status.as_ref(),
                sizing,
                theme,
                badge_ring_background,
            )
        }
        IconWithStatusVariant::Dsh { icon, status } => {
            let inner = ConstrainedBox::new(
                icon.to_warpui_icon(theme.main_text_color(theme.background()))
                    .finish(),
            )
            .with_width(sizing.icon_size)
            .with_height(sizing.icon_size)
            .finish();
            let circle = Container::new(inner)
                .with_uniform_padding(sizing.padding)
                .with_background(theme.background())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    (sizing.icon_size + sizing.padding * 2.) / 2.,
                )))
                .finish();
            render_with_optional_status_badge(
                circle,
                status.as_ref(),
                sizing,
                theme,
                badge_ring_background,
            )
        }
        IconWithStatusVariant::CustomAvatar { avatar, status, .. } => {
            // 本地 sizing 路径：调用方传入已预渲染的头像，这里仅按圆的整体直径约束后
            // 叠加本地状态徽章。ambient 云朵徽章只在新函数
            // `render_icon_with_status_with_badge_style` 中实现。
            let circle_diameter = sizing.icon_size + sizing.padding * 2.;
            let circle = ConstrainedBox::new(avatar)
                .with_width(circle_diameter)
                .with_height(circle_diameter)
                .finish();
            render_with_optional_status_badge(
                circle,
                status.as_ref(),
                sizing,
                theme,
                badge_ring_background,
            )
        }
    }
}

/// Adds a status badge with a cutout ring to the bottom-right of the circle.
fn render_with_optional_status_badge(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    sizing: &IconWithStatusSizing,
    theme: &WarpTheme,
    badge_ring_background: WarpThemeFill,
) -> Box<dyn Element> {
    let Some(status) = status else {
        return circle;
    };
    let (icon, color) = status.status_icon_and_color(theme);
    let badge_icon = ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
        .with_width(sizing.badge_icon_size)
        .with_height(sizing.badge_icon_size)
        .finish();
    let badge = Container::new(badge_icon)
        .with_uniform_padding(sizing.badge_padding)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish();
    // Cutout ring that visually separates the badge from the circle.
    let badge_with_ring = Container::new(badge)
        .with_uniform_padding(sizing.badge_padding)
        .with_background(badge_ring_background)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish();

    let circle_size = sizing.icon_size + sizing.padding * 2.;
    let overall_size = sizing.overall_size_override.unwrap_or(circle_size);
    let mut stack = Stack::new().with_child(
        ConstrainedBox::new(circle)
            .with_width(overall_size)
            .with_height(overall_size)
            .finish(),
    );
    stack.add_positioned_child(
        badge_with_ring,
        OffsetPositioning::offset_from_parent(
            vec2f(sizing.badge_offset.0, sizing.badge_offset.1),
            ParentOffsetBounds::ParentBySize,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(overall_size)
        .with_height(overall_size)
        .finish()
}

// ===== 以下为上游 `render_icon_with_status_with_badge_style` 形状的独立实现，
// 与上面的 `render_icon_with_status`（本地 `IconWithStatusSizing` 语义）并存。

/// 与 [`render_icon_with_status`] 同用途，但几何完全由单一 `total_size` 按比例推导，
/// 徽章几何由 [`StatusBadgeStyle`] 决定。pill bar 需要它来把状态徽章收敛成圆角方块。
///
/// `overlay_extra_overhang_ratio` 是叠加到默认外伸量上的、以 `total_size` 为基准的带符号比例：
/// 多数界面传 `0.0` 取默认位置。
///
/// agent 变体上 `is_ambient` 置位时，状态徽章被替换为云朵（内含状态图标），
/// 该路径忽略 `badge_style`。
pub(crate) fn render_icon_with_status_with_badge_style(
    variant: IconWithStatusVariant,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    badge_style: StatusBadgeStyle,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    let sub_text = theme.sub_text_color(theme.background());

    match variant {
        IconWithStatusVariant::Neutral { icon, icon_color } => render_neutral_circle(
            icon.to_warpui_icon(icon_color).finish(),
            internal_colors::fg_overlay_2(theme),
            total_size,
        ),
        IconWithStatusVariant::NeutralElement { icon_element } => render_neutral_circle(
            icon_element,
            internal_colors::fg_overlay_2(theme),
            total_size,
        ),
        IconWithStatusVariant::OzAgent { status, is_ambient } => {
            // 沿用本地 Oz 语义：ambient 用 OzCloud 字形，底色与图标色取主题色
            //（上游此处改为 WarpIcon::Agent + 紫色 ambient 底色，见报告中的分叉说明）。
            let icon = if is_ambient {
                WarpIcon::OzCloud
            } else {
                WarpIcon::Oz
            };
            let circle = render_circle(
                icon.to_warpui_icon(theme.main_text_color(theme.background()))
                    .finish(),
                theme.background(),
                total_size,
            );
            attach_status_overlay(
                circle,
                status.as_ref(),
                is_ambient,
                total_size,
                overlay_extra_overhang_ratio,
                badge_style,
                theme,
                status_container_background,
            )
        }
        IconWithStatusVariant::CLIAgent { agent, status } => {
            // 本地 `CLIAgent` 变体没有 `is_ambient` 字段（加字段会破坏本文件外的构造点），
            // 因此这里固定按非 ambient 处理。
            let brand_color = agent
                .brand_color()
                .unwrap_or(ColorU::new(100, 100, 100, 255));
            let icon_color = agent.brand_icon_color();
            let icon_element =
                render_cli_agent_logo(agent, WarpThemeFill::Solid(icon_color), sub_text);
            let background: WarpThemeFill =
                if matches!(agent, CLIAgent::Antigravity | CLIAgent::OhMyPi) {
                    theme.background()
                } else {
                    ThemeFill::Solid(brand_color)
                };
            let circle = render_circle(icon_element, background, total_size);
            attach_status_overlay(
                circle,
                status.as_ref(),
                false,
                total_size,
                overlay_extra_overhang_ratio,
                badge_style,
                theme,
                status_container_background,
            )
        }
        IconWithStatusVariant::Dsh { icon, status } => {
            // 本地独有变体，上游没有对应分支：按本地语义（主题底色 + 主文字色图标）渲染，
            // 再叠加比例几何的状态徽章。
            let circle = render_circle(
                icon.to_warpui_icon(theme.main_text_color(theme.background()))
                    .finish(),
                theme.background(),
                total_size,
            );
            attach_status_overlay(
                circle,
                status.as_ref(),
                false,
                total_size,
                overlay_extra_overhang_ratio,
                badge_style,
                theme,
                status_container_background,
            )
        }
        IconWithStatusVariant::CustomAvatar {
            avatar,
            status,
            is_ambient,
        } => attach_status_overlay(
            avatar,
            status.as_ref(),
            is_ambient,
            total_size,
            overlay_extra_overhang_ratio,
            badge_style,
            theme,
            status_container_background,
        ),
    }
}

/// 构建品牌圆容器：圆直径 `circle_size(total)`，字形 `icon_size(total)`，
/// 其余空间作为字形四周的对称 padding。
/// 返回元素的宽度是 `circle_size(total)`；agent 调用方通过 `attach_status_overlay`
/// 让它占满整个 `total_size` 足迹。
fn render_circle(
    icon_element: Box<dyn Element>,
    background: WarpThemeFill,
    total_size: f32,
) -> Box<dyn Element> {
    let icon = icon_size(total_size);
    let padding = circle_padding(total_size);
    let inner = ConstrainedBox::new(icon_element)
        .with_width(icon)
        .with_height(icon)
        .finish();
    Container::new(inner)
        .with_uniform_padding(padding)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            circle_size(total_size) / 2.,
        )))
        .finish()
}

/// 构建 Neutral 圆：占满 `total_size` 的容器，字形为 `NEUTRAL_GLYPH_RATIO * total_size`。
/// 用于没有状态叠加层的非 agent 界面（普通终端、代码、文件标签等），
/// 它们应当填满调用方要求的包围盒，而不是缩到 `circle_size(total)`。
fn render_neutral_circle(
    icon_element: Box<dyn Element>,
    background: WarpThemeFill,
    total_size: f32,
) -> Box<dyn Element> {
    let glyph = total_size * NEUTRAL_GLYPH_RATIO;
    let padding = (total_size - glyph) / 2.;
    let inner = ConstrainedBox::new(icon_element)
        .with_width(glyph)
        .with_height(glyph)
        .finish();
    Container::new(inner)
        .with_uniform_padding(padding)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(total_size / 2.)))
        .finish()
}

/// 给品牌圆套上合适的状态叠加层：非 ambient 用徽章，ambient 用云朵。两者都按 `total_size` 推导。
#[allow(clippy::too_many_arguments)]
fn attach_status_overlay(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    is_ambient: bool,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    badge_style: StatusBadgeStyle,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    if is_ambient {
        render_with_cloud_status_badge(
            circle,
            status,
            total_size,
            overlay_extra_overhang_ratio,
            theme,
        )
    } else {
        render_with_optional_status_badge_by_ratio(
            circle,
            status,
            total_size,
            overlay_extra_overhang_ratio,
            badge_style,
            theme,
            status_container_background,
        )
    }
}

/// 在基础圆右下角叠加一朵云（若状态存在，状态图标居中画在云内）。用于 ambient/cloud 模式 agent。
fn render_with_cloud_status_badge(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let cloud_diameter = cloud_icon_size(total_size);
    let cloud = ConstrainedBox::new(
        WarpIcon::CloudFilled
            .to_warpui_icon(theme.foreground())
            .finish(),
    )
    .with_width(cloud_diameter)
    .with_height(cloud_diameter)
    .finish();

    let cloud_with_status: Box<dyn Element> = match status {
        Some(status) => {
            // 本地 `ConversationStatus::status_icon_and_color` 只接受 theme（没有上游的
            // `StatusColorStyle` 参数），因此这里只能取标准配色。
            let (icon, color) = status.status_icon_and_color(theme);
            let inner = status_in_cloud_size(total_size);
            let status_icon =
                ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
                    .with_width(inner)
                    .with_height(inner)
                    .finish();
            let mut stack = Stack::new().with_child(cloud);
            // CloudFilled SVG 的视觉重心低于容器几何中心（云下部比上部宽），
            // 因此把状态图标下移一点，使其在云形内看起来居中而不是在包围盒内居中。
            stack.add_positioned_child(
                status_icon,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 1.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
            stack.finish()
        }
        None => cloud,
    };

    let cloud_offset = corner_overlay_offset(total_size, overlay_extra_overhang_ratio);
    let mut stack = Stack::new().with_child(
        ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish(),
    );
    stack.add_positioned_child(
        cloud_with_status,
        OffsetPositioning::offset_from_parent(
            vec2f(cloud_offset, cloud_offset),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(total_size)
        .with_height(total_size)
        .finish()
}

/// 在圆右下角叠加带挖孔环的状态徽章；几何由 `badge_style` 与 `total_size` 决定。
/// 与本地 `render_with_optional_status_badge`（`IconWithStatusSizing` 版本）并存。
#[allow(clippy::too_many_arguments)]
fn render_with_optional_status_badge_by_ratio(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    badge_style: StatusBadgeStyle,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    let Some(status) = status else {
        // 没有状态徽章时仍占满调用方要求的 `total_size` 足迹，使有/无徽章的变体占据相同空间。
        // `ConstrainedBox` 只收紧约束、不居中，因此只有 `circle_size(total)` 宽的圆会画在盒子左上角。
        return ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish();
    };
    let (icon, color) = status.status_icon_and_color(theme);
    let badge_icon_diameter = badge_icon_size(total_size, badge_style);
    let pad = badge_padding(total_size, badge_style);
    let badge_icon = ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
        .with_width(badge_icon_diameter)
        .with_height(badge_icon_diameter)
        .finish();
    let inner_radius = match badge_style.inner_shape {
        BadgeInnerShape::Circle => Radius::Percentage(50.),
        BadgeInnerShape::RoundedSquare { radius_px } => Radius::Pixels(radius_px),
    };
    let badge = Container::new(badge_icon)
        .with_uniform_padding(pad)
        .with_corner_radius(CornerRadius::with_all(inner_radius))
        .finish();
    // 徽章外圈的挖孔环；始终是圆形（只有内层容器形状可变）。
    let badge_with_ring = Container::new(badge)
        .with_uniform_padding(pad)
        .with_background(status_container_background)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish();

    let badge_corner_offset = corner_overlay_offset(total_size, overlay_extra_overhang_ratio);
    let mut stack = Stack::new().with_child(
        ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish(),
    );
    stack.add_positioned_child(
        badge_with_ring,
        OffsetPositioning::offset_from_parent(
            vec2f(badge_corner_offset, badge_corner_offset),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(total_size)
        .with_height(total_size)
        .finish()
}
