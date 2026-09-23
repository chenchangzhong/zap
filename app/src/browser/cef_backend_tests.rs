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

/// OSR 模式开关解析:只有显式 `1`/`true` 才启用 OSR;未设置/空串/其他值一律
/// windowed —— 保证默认路径逐字节不变、可回滚(OSR-PLAN.md T4/T8)。
#[test]
fn parse_render_mode_defaults_to_windowed() {
    assert_eq!(parse_render_mode(None), RenderMode::Windowed);
    assert_eq!(parse_render_mode(Some("")), RenderMode::Windowed);
    assert_eq!(parse_render_mode(Some("0")), RenderMode::Windowed);
    assert_eq!(parse_render_mode(Some("yes")), RenderMode::Windowed);
    assert_eq!(parse_render_mode(Some("1")), RenderMode::Osr);
    assert_eq!(parse_render_mode(Some(" true ")), RenderMode::Osr);
}

/// OSR 视图尺寸必须是 **DIP(逻辑点)**:CEF 自己乘 device_scale_factor。
/// T1 实测:这里若返回像素、`screen_info` 又报 scale=2,surface 会被缩放两次
/// (2400x1600 而非 1200x800)。0 尺寸要夹到 1(CEF 不接受 0 宽高)。
#[test]
fn osr_view_size_is_dip_and_clamped() {
    assert_eq!(osr_view_size(600.0, 400.0), (600, 400));
    assert_eq!(osr_view_size(600.4, 399.6), (600, 400));
    assert_eq!(osr_view_size(0.0, 0.0), (1, 1));
    assert_eq!(osr_view_size(-5.0, 3.0), (1, 3));
}

/// windowed 是默认模式:未设开关时 render_mode() 必须与现状一致
/// (测试进程里没设 ZAP_CEF_OSR;若外部设了则跳过,避免误判)。
#[test]
fn render_mode_is_windowed_without_switch() {
    if std::env::var_os("ZAP_CEF_OSR").is_some() {
        return;
    }
    assert_eq!(render_mode(), RenderMode::Windowed);
    assert!(!external_begin_frame());
    assert!(!cpu_paint());
}

/// NSEventModifierFlags → cef_event_flags_t 位(鼠标/键盘共用)。
/// 位值取自 cef_types.h:CAPS_LOCK_ON=1<<0、SHIFT_DOWN=1<<1、CONTROL_DOWN=1<<2、
/// ALT_DOWN=1<<3、COMMAND_DOWN=1<<7。
#[test]
fn cef_modifiers_maps_appkit_flags() {
    assert_eq!(cef_modifiers(0), 0);
    assert_eq!(cef_modifiers(NS_MOD_CAPS_LOCK), 1 << 0);
    assert_eq!(cef_modifiers(NS_MOD_SHIFT), 1 << 1);
    assert_eq!(cef_modifiers(NS_MOD_CONTROL), 1 << 2);
    assert_eq!(cef_modifiers(NS_MOD_OPTION), 1 << 3);
    assert_eq!(cef_modifiers(NS_MOD_COMMAND), 1 << 7);
    // 组合:Shift+Cmd 是"另存/新开"类快捷键的常见组合。
    assert_eq!(
        cef_modifiers(NS_MOD_SHIFT | NS_MOD_COMMAND),
        (1 << 1) | (1 << 7)
    );
    // NumericPad 只用于 IS_KEY_PAD 判定,不进通用修饰键位。
    assert_eq!(cef_modifiers(NS_MOD_NUMERIC_PAD), 0);
}

/// mac 虚拟键码 → Windows VK:字符无关的键查表,普通键用大写码点。
#[test]
fn windows_key_code_matches_cef_convention() {
    // 字母/数字:VK_A..VK_Z / VK_0..VK_9 就是大写码点。
    assert_eq!(windows_key_code(0, Some("a")), i32::from(b'A'));
    assert_eq!(windows_key_code(0, Some("A")), i32::from(b'A'));
    assert_eq!(windows_key_code(29, Some("0")), i32::from(b'0'));
    // 字符无关的键必须查表(不能退化成字符码)。
    assert_eq!(windows_key_code(0x24, Some("\r")), 0x0D); // Return
    assert_eq!(windows_key_code(0x30, Some("\t")), 0x09); // Tab
    assert_eq!(windows_key_code(0x33, Some("\u{8}")), 0x08); // 退格
    assert_eq!(windows_key_code(0x7B, None), 0x25); // 左方向键
    assert_eq!(windows_key_code(0x7E, None), 0x26); // 上方向键
    assert_eq!(windows_key_code(0x35, Some("\u{1b}")), 0x1B); // Escape
    assert_eq!(windows_key_code(0x37, None), 0x5B); // Command
    assert_eq!(windows_key_code(0x7A, None), 0x70); // F1
    assert_eq!(windows_key_code(0x6F, None), 0x7B); // F12
    // 非 ASCII 字符没有 VK,返回 0(字符由 character 字段承载)。
    assert_eq!(windows_key_code(0, Some("中")), 0);
    assert_eq!(windows_key_code(0, None), 0);
}

/// flagsChanged:按修饰键的当前状态决定 KEYDOWN/KEYUP。
#[test]
fn is_modifier_pressed_reads_flag_state() {
    assert!(is_modifier_pressed(56, NS_MOD_SHIFT)); // 左 Shift
    assert!(!is_modifier_pressed(56, 0));
    assert!(is_modifier_pressed(60, NS_MOD_SHIFT)); // 右 Shift
    assert!(is_modifier_pressed(55, NS_MOD_COMMAND)); // 左 Cmd
    assert!(!is_modifier_pressed(55, NS_MOD_SHIFT));
    assert!(is_modifier_pressed(62, NS_MOD_CONTROL)); // 右 Control
    assert!(is_modifier_pressed(61, NS_MOD_OPTION)); // 右 Option
    assert!(is_modifier_pressed(57, NS_MOD_CAPS_LOCK)); // CapsLock
}

/// 小键盘:既有 modifier flag 覆盖,也有键码兜底。
#[test]
fn is_key_pad_event_covers_flag_and_keycodes() {
    assert!(is_key_pad_event(0, NS_MOD_NUMERIC_PAD));
    assert!(is_key_pad_event(65, 0)); // 小键盘 0
    assert!(is_key_pad_event(76, 0)); // 小键盘 Enter
    assert!(!is_key_pad_event(0, 0)); // 主键盘 a
    assert!(!is_key_pad_event(0x24, 0)); // 主键盘 Return
}

/// 视图坐标(DIP,可能小数)→ CEF 整数坐标:截断(与 cefclient 的
/// "先乘 scale 取整再除回" 在 scale ≥ 1 时等价)。
#[test]
fn to_dip_coord_truncates() {
    assert_eq!(to_dip_coord(0.0), 0);
    assert_eq!(to_dip_coord(100.7), 100);
    assert_eq!(to_dip_coord(100.2), 100);
    assert_eq!(to_dip_coord(-3.4), -3);
}

/// deferred 按键决策(T7 输入法,照 cefclient/CefSwift 的
/// HandleKeyEventBefore/AfterTextInputClient):普通按键必须仍是 KEYDOWN+CHAR,
/// 组合中上报 composition,上屏走 commit —— 三者不能混。
#[test]
fn deferred_key_plan_covers_plain_composition_and_commit() {
    // 1) 普通字母:前后无 composition、只插入 1 个字符 ⇒ KEYDOWN+CHAR。
    let plain = deferred_key_plan(Some("a"), None, false, false, false);
    assert!(plain.send_plain_key);
    assert_eq!(plain.commit_text, None);
    assert_eq!(plain.set_composition, None);
    assert!(!plain.finish_composition && !plain.cancel_composition);

    // 2) 功能键(没有插入文本,如方向键/退格)同样按普通按键发。
    let function_key = deferred_key_plan(None, None, false, false, false);
    assert!(function_key.send_plain_key);
    assert_eq!(function_key.commit_text, None);

    // 3) 拼音组合更新:marked text 非空、没有插入文本 ⇒ 只上报 composition。
    let composing = deferred_key_plan(None, Some("ni'hao"), false, true, false);
    assert!(!composing.send_plain_key, "组合期间不得发普通按键");
    assert_eq!(composing.set_composition.as_deref(), Some("ni'hao"));
    assert_eq!(composing.commit_text, None);

    // 4) 候选上屏:组合消失 + 插入多字符 ⇒ commit + cancel(未 unmark)。
    let commit = deferred_key_plan(Some("你好"), None, true, false, false);
    assert!(!commit.send_plain_key);
    assert_eq!(commit.commit_text.as_deref(), Some("你好"));
    assert!(commit.cancel_composition);
    assert!(!commit.finish_composition);

    // 5) 输入法主动 unmarkText ⇒ 以 finish 收尾(而不是 cancel)。
    let finished = deferred_key_plan(Some("你"), None, true, false, true);
    assert!(finished.finish_composition && !finished.cancel_composition);
    assert_eq!(finished.commit_text.as_deref(), Some("你"));
}

/// 没有替换目标时必须传**显式 InvalidRange**(两个 u32::MAX),不能传 NULL:
/// CEF 的 capi 会把 NULL 退化成 (0,0),渲染器据此 SelectRange 打断 `<textarea>`
/// 焦点,整条 composition 被静默丢弃(T2 spike 实测,见 OSR-SPIKE-B.md)。
#[test]
fn ime_replacement_range_never_null() {
    let invalid = ime_replacement_range(-1, -1);
    assert_eq!((invalid.from, invalid.to), (u32::MAX, u32::MAX));

    let normal = ime_replacement_range(3, 5);
    assert_eq!((normal.from, normal.to), (3, 5));

    // 反向区间也收敛成合法值(否则渲染器侧 checked_cast 会失败)。
    let reversed = ime_replacement_range(5, 2);
    assert_eq!((reversed.from, reversed.to), (5, 5));
}

/// 渲染模式解析:环境变量优先于设置项;都"没给值"时回落到 windowed。
///
/// 注意:**设置项的默认值是 true**(见 settings/cef_webview.rs),这里传的 `false` 表示
/// "设置项为 false"(用户关掉或旧配置),不是"未设置"。
#[test]
fn resolve_render_mode_env_wins_then_setting() {
    // 设置项 false 且无 env ⇒ windowed(回滚入口)。
    assert_eq!(resolve_render_mode(None, false), RenderMode::Windowed);
    // 只有设置项 ⇒ 跟随设置。
    assert_eq!(resolve_render_mode(None, true), RenderMode::Osr);
    // 环境变量显式给值 ⇒ 覆盖设置项(两个方向都覆盖)。
    assert_eq!(resolve_render_mode(Some("1"), false), RenderMode::Osr);
    assert_eq!(resolve_render_mode(Some("0"), true), RenderMode::Windowed);
    // 空/空白环境变量视为"没设",回落到设置项(避免 `ZAP_CEF_OSR=` 静默推翻设置)。
    assert_eq!(resolve_render_mode(Some(""), true), RenderMode::Osr);
    assert_eq!(resolve_render_mode(Some("  "), true), RenderMode::Osr);
}

/// 页面请求的光标 → 宿主语义 id(链接/文本/缩放等)。
#[test]
fn cursor_semantic_maps_common_types() {
    assert_eq!(cursor_semantic(CursorType::HAND), 1);
    assert_eq!(cursor_semantic(CursorType::IBEAM), 2);
    assert_eq!(cursor_semantic(CursorType::CROSS), 3);
    assert_eq!(cursor_semantic(CursorType::EASTWESTRESIZE), 4);
    assert_eq!(cursor_semantic(CursorType::NORTHSOUTHRESIZE), 5);
    assert_eq!(cursor_semantic(CursorType::GRAB), 6);
    assert_eq!(cursor_semantic(CursorType::GRABBING), 7);
    assert_eq!(cursor_semantic(CursorType::NOTALLOWED), 8);
    assert_eq!(cursor_semantic(CursorType::ZOOMIN), 9);
    assert_eq!(cursor_semantic(CursorType::ZOOMOUT), 10);
    assert_eq!(cursor_semantic(CursorType::MOVE), 11);
    // 未映射(含 CT_NONE/CUSTOM)一律箭头,避免出现卡住的怪光标。
    assert_eq!(cursor_semantic(CursorType::POINTER), 0);
    assert_eq!(cursor_semantic(CursorType::NONE), 0);
    assert_eq!(cursor_semantic(CursorType::CUSTOM), 0);
}

/// 拖放掩码契约(P0-3):AppKit 的 `NSDragOperation` 与 CEF 的
/// `cef_drag_operations_mask_t` 逐位相同(copy=1、link=2、generic=4、private=8、
/// move=16、delete=32、every=UINT_MAX),因此 ObjC ↔ Rust 之间直接透传、不做映射表。
///
/// 两侧都断言:CEF 侧来自 `sys::cef_drag_operations_mask_t`,AppKit 侧来自
/// `objc2_app_kit::NSDragOperation`(需 `NSDragging` feature,已在 `app/Cargo.toml` 启用)——
/// 任一侧改号都会在这里红,这才真正钉住"直接透传"的前提。
#[test]
fn drag_operation_mask_matches_appkit_bits() {
    use objc2_app_kit::NSDragOperation as appkit;
    use sys::cef_drag_operations_mask_t as mask;
    // 注:`Every` 两侧宽度不同(AppKit `NSUIntegerMax` vs CEF `UINT_MAX`),`as u32` 恰好把差异
    // 截平 —— 这是**良性**的(生产侧 ObjC 边界同样按 `uint32_t` 传),但断言到 `Every` 时
    // 它证明的是"低 32 位相同",不是"类型宽度相同"。
    assert_eq!(mask::DRAG_OPERATION_NONE.0, appkit::None.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_COPY.0, appkit::Copy.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_LINK.0, appkit::Link.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_GENERIC.0, appkit::Generic.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_PRIVATE.0, appkit::Private.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_MOVE.0, appkit::Move.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_DELETE.0, appkit::Delete.0 as u32);
    assert_eq!(mask::DRAG_OPERATION_EVERY.0, appkit::Every.0 as u32);
}

/// 拖放事件必须报"按住了左键":否则 CEF 会把拖动当成普通悬停,
/// `drag_target_drag_enter` 之后的 over/drop 判定会错(参考实现同款处理)。
#[test]
fn drag_event_modifiers_reports_left_button() {
    use sys::cef_event_flags_t as flags;
    assert_eq!(
        drag_event_modifiers(0),
        flags::EVENTFLAG_LEFT_MOUSE_BUTTON.0
    );
    assert_eq!(
        drag_event_modifiers(NS_MOD_OPTION | NS_MOD_SHIFT),
        flags::EVENTFLAG_ALT_DOWN.0
            | flags::EVENTFLAG_SHIFT_DOWN.0
            | flags::EVENTFLAG_LEFT_MOUSE_BUTTON.0
    );
}

/// 下载默认路径的文件名清洗:建议名来自服务端,带路径分隔符时只取最后一段,
/// 否则保存面板会预填到下载目录之外(参考实现 `CefDownloadDestination.resolve` 同款防护)。
#[test]
fn download_file_name_strips_path_components() {
    assert_eq!(
        download_file_name("dsh-session-abc.zip"),
        "dsh-session-abc.zip"
    );
    assert_eq!(download_file_name("../../evil.zip"), "evil.zip");
    assert_eq!(download_file_name("/etc/passwd"), "passwd");
    assert_eq!(download_file_name("sub/dir/report.md"), "report.md");
}

/// 空名 / 纯分隔符 / `..` 都不能产出空文件名(否则面板会预填一个目录路径)。
#[test]
fn download_file_name_falls_back_when_empty() {
    assert_eq!(download_file_name(""), "download");
    assert_eq!(download_file_name("/"), "download");
    assert_eq!(download_file_name(".."), "download");
}

/// 清洗后的默认路径必须落在下载目录内(`join` 不能被建议名带出去)。
///
/// 不用 `else { return }` 静默跳过:这是 macOS-only 模块,`download_dir()` 为 None 本身就是
/// 异常,静默"通过"会变成假绿(评审 M-7)。
#[test]
fn download_default_path_stays_in_download_dir() {
    let dir = dirs::download_dir().expect("macOS 上应有下载目录");
    let path = download_default_path("../../evil.zip").expect("download dir 可用");
    assert!(
        path.starts_with(&dir),
        "路径逃出下载目录:{}",
        path.display()
    );
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("evil.zip")
    );
}

/// 取消标记状态机(评审 M-7):进行中命中即消费且幂等;终态只清标记;未标记不取消。
/// 这条逻辑原本只由实机验证覆盖,抽成纯函数后可回归。
#[test]
fn take_cancel_request_is_consumed_once_and_cleared_on_terminal() {
    let mut canceled: HashSet<u32> = HashSet::new();

    // 未标记 + 进行中 ⇒ 不取消,集合不变。
    assert!(!take_cancel_request(&mut canceled, 7, true));
    assert!(canceled.is_empty());

    // 标记 + 进行中 ⇒ 取消,且只消费一次(后续 update 不再重复取消)。
    canceled.insert(7);
    assert!(take_cancel_request(&mut canceled, 7, true));
    assert!(!take_cancel_request(&mut canceled, 7, true));
    assert!(canceled.is_empty());

    // 标记 + 终态 ⇒ 不取消,但标记被清掉(防 id 复用后误取消)。
    canceled.insert(9);
    assert!(!take_cancel_request(&mut canceled, 9, false));
    assert!(canceled.is_empty());

    // 未标记 + 终态 ⇒ 无副作用。
    assert!(!take_cancel_request(&mut canceled, 11, false));
    assert!(canceled.is_empty());
}
