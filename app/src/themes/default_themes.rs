use pathfinder_color::ColorU;
use warp_core::ui::{
    color::OPAQUE,
    theme::{AnsiColor, AnsiColors, Details, Fill, TerminalColors, WarpTheme},
};
use warp_core::ui::theme::ui_colors::UiColors;

const DARK_MODE_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x616161FF),
    AnsiColor::from_u32(0xFF8272FF),
    AnsiColor::from_u32(0xB4FA72FF),
    AnsiColor::from_u32(0xFEFDC2FF),
    AnsiColor::from_u32(0xA5D5FEFF),
    AnsiColor::from_u32(0xFF8FFDFF),
    AnsiColor::from_u32(0xD0D1FEFF),
    AnsiColor::from_u32(0xF1F1F1FF),
);
const DARK_MODE_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x8E8E8EFF),
    AnsiColor::from_u32(0xFFC4BDFF),
    AnsiColor::from_u32(0xD6FCB9FF),
    AnsiColor::from_u32(0xFEFDD5FF),
    AnsiColor::from_u32(0xC1E3FEFF),
    AnsiColor::from_u32(0xFFB1FEFF),
    AnsiColor::from_u32(0xE5E6FEFF),
    AnsiColor::from_u32(0xFEFFFFFF),
);

const LIGHT_MODE_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x212121FF),
    AnsiColor::from_u32(0xC30771FF),
    AnsiColor::from_u32(0x10A778FF),
    AnsiColor::from_u32(0xA89C14FF),
    AnsiColor::from_u32(0x008EC4FF),
    AnsiColor::from_u32(0x523C79FF),
    AnsiColor::from_u32(0x20A5BAFF),
    AnsiColor::from_u32(0xE0E0E0FF),
);
const LIGHT_MODE_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x212121FF),
    AnsiColor::from_u32(0xFB007AFF),
    AnsiColor::from_u32(0x5FD7AFFF),
    AnsiColor::from_u32(0xF3E430FF),
    AnsiColor::from_u32(0x20BBFCFF),
    AnsiColor::from_u32(0x6855DEFF),
    AnsiColor::from_u32(0x4FB8CCFF),
    AnsiColor::from_u32(0xF1F1F1FF),
);

// 16 色 ANSI 配色源: vscode/extensions/theme-defaults/themes/2026-dark.json
const VSCODE_2026_DARK_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x000000FF),
    AnsiColor::from_u32(0xCD3131FF),
    AnsiColor::from_u32(0x0DBC79FF),
    AnsiColor::from_u32(0xE5E510FF),
    AnsiColor::from_u32(0x2472C8FF),
    AnsiColor::from_u32(0xBC3FBCFF),
    AnsiColor::from_u32(0x11A8CDFF),
    AnsiColor::from_u32(0xE5E5E5FF),
);
const VSCODE_2026_DARK_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x666666FF),
    AnsiColor::from_u32(0xF14C4CFF),
    AnsiColor::from_u32(0x23D18BFF),
    AnsiColor::from_u32(0xF5F543FF),
    AnsiColor::from_u32(0x3B8EEAFF),
    AnsiColor::from_u32(0xD670D6FF),
    AnsiColor::from_u32(0x29B8DBFF),
    AnsiColor::from_u32(0xE5E5E5FF),
);

/// 返回 VS Code 2026 Dark 主题的 16 色 ANSI 终端颜色。
pub(super) fn vscode_2026_dark_colors() -> TerminalColors {
    TerminalColors::new(VSCODE_2026_DARK_NORMAL_COLORS, VSCODE_2026_DARK_BRIGHT_COLORS)
}

/// VS Code 2026 Dark 内置主题；配色源: vscode/extensions/theme-defaults/themes/2026-dark.json。
/// 包含完整 UiColors 覆盖，将 VS Code 的 editor/panel 颜色映射到 Zap UI 组件。
pub(super) fn vscode_2026_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x191A1BFF)),
        ColorU::from_u32(0xCCCCCCFF),
        Fill::Solid(ColorU::from_u32(0x3994BCFF)),
        Some(Fill::Solid(ColorU::from_u32(0xBFBFBFFF))),
        Some(Details::Darker),
        vscode_2026_dark_colors(),
        None,
        Some("VS Code 2026 Dark".to_string()),
        Some(UiColors {
            surface_1: Some(ColorU { r: 0x20, g: 0x21, b: 0x22, a: 255 }),
            surface_2: Some(ColorU { r: 0x24, g: 0x25, b: 0x26, a: 255 }),
            surface_3: Some(ColorU { r: 0x2A, g: 0x2B, b: 0x2C, a: 255 }),
            border: Some(ColorU { r: 0x33, g: 0x35, b: 0x36, a: 255 }),
            focus_border: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0xB3 }),
            split_pane_border: Some(ColorU { r: 0x2A, g: 0x2B, b: 0x2C, a: 255 }),
            main_text: Some(ColorU { r: 0xED, g: 0xED, b: 0xED, a: 255 }),
            sub_text: Some(ColorU { r: 0x8C, g: 0x8C, b: 0x8C, a: 255 }),
            hint_text: Some(ColorU { r: 0x55, g: 0x55, b: 0x55, a: 255 }),
            disabled_text: Some(ColorU { r: 0x55, g: 0x55, b: 0x55, a: 255 }),
            selection: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0x33 }),
            text_selection: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0x33 }),
            hover: Some(ColorU { r: 0xFF, g: 0xFF, b: 0xFF, a: 0x0D }),
            active: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 255 }),
            warning: Some(ColorU { r: 0xE5, g: 0xBA, b: 0x7D, a: 255 }),
            error: Some(ColorU { r: 0xF4, g: 0x87, b: 0x71, a: 255 }),
            success: Some(ColorU { r: 0x72, g: 0xC8, b: 0x92, a: 255 }),
            link: Some(ColorU { r: 0x48, g: 0xA0, b: 0xC7, a: 255 }),
        }),
    )
}

pub(super) fn light_mode_colors() -> TerminalColors {
    TerminalColors::new(LIGHT_MODE_NORMAL_COLORS, LIGHT_MODE_BRIGHT_COLORS)
}

pub(super) fn dark_mode_colors() -> TerminalColors {
    TerminalColors::new(DARK_MODE_NORMAL_COLORS, DARK_MODE_BRIGHT_COLORS)
}

/// Default bundled themes
pub fn dark_theme() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x000000FF)),
        ColorU::from_u32(0xffffffff),
        Fill::Solid(ColorU::from_u32(0x19AAD8FF)),
        None,
        Some(Details::Darker),
        dark_mode_colors(),
        None,
        Some("Dark".to_string()),
        None,
    )
}

pub fn light_theme() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::white()),
        ColorU::new(17, 17, 17, OPAQUE),
        Fill::Solid(ColorU::from_u32(0x00c2ffff)),
        None,
        Some(Details::Lighter),
        light_mode_colors(),
        None,
        Some("Light".to_string()),
        None,
    )
}
