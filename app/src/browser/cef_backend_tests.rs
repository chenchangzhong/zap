use super::*;
use pathfinder_geometry::rect::RectF;

/// 未初始化时 pump 必须是安全 no-op(避免早期调用点崩溃)。
#[test]
fn pump_before_initialize_is_noop() {
    pump();
}

/// 初始化前 `is_enabled()` 必须为 false —— manager/pane 据此回退 wry,保证默认路径可用。
#[test]
fn is_enabled_is_false_before_initialize() {
    assert!(!is_enabled());
}

/// 坐标翻转(集成要求 #2):场景逻辑坐标是左上原点,CEF 子视图要的是父视图
/// AppKit 坐标(底部原点),公式与 wry 的 `window_position` 一致
/// (`y_appkit = parentHeight - y - h`)。窗口几何在测试里无法构造,故对纯函数下桩:
///  - 顶部对齐的 rect(y=0,h=400,父高 600)→ y_appkit = 200
///  - 底部对齐的 rect(y=200,h=400,父高 600)→ y_appkit = 0
#[test]
fn flip_rect_to_appkit_matches_bottom_origin_convention() {
    let rect = RectF::new(
        pathfinder_geometry::vector::vec2f(100.0, 0.0),
        pathfinder_geometry::vector::vec2f(300.0, 400.0),
    );
    let flipped = flip_rect_to_appkit(rect, 600.0);
    assert_eq!(flipped.origin.x, 100.0);
    assert_eq!(flipped.origin.y, 200.0, "y 必须按父高翻转");
    assert_eq!(flipped.size.width, 300.0);
    assert_eq!(flipped.size.height, 400.0);

    let bottom = RectF::new(
        pathfinder_geometry::vector::vec2f(0.0, 200.0),
        pathfinder_geometry::vector::vec2f(300.0, 400.0),
    );
    assert_eq!(flip_rect_to_appkit(bottom, 600.0).origin.y, 0.0);

    // 父视图尚未布局(高度 0)时 y 为负:几何会在首帧布局后的 set_bounds 里纠正,
    // 这里只锁定"公式稳定、无 panic"。
    assert_eq!(flip_rect_to_appkit(bottom, 0.0).origin.y, -600.0);
}

// 注:不在这里测 `handle_subprocess_or_continue` —— 它调用进程级的 CEF
// `execute_process`(会修改 CEF 全局状态),在测试宿主进程里没有意义且不可靠。
// 该路径由端到端受控冒烟覆盖(script/macos/cef_smoke)。
