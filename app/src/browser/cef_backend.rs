//! CEF(Chromium)后端骨架:仅在 `cef_webview` feature 下编译(macOS)。
//!
//! 目标(specs/cef-webview-minimal/TECH.md 阶段 1):用 CEF 承载 dsh pane 的 webview,
//! 替代 wry/WKWebView。本文件当前只提供**运行时骨架**——初始化、消息泵、以子视图方式
//! 创建浏览器;尚未接入 `BrowserWebViewManager`/`DshPane`(下一步)。
//!
//! 与 zap 事件循环的关系(集成要求 #4,见 TECH.md):
//! zap 已有自己的事件循环,故 CEF 走 `external_message_pump = 1` + 主线程稳定周期
//! 调用 [`pump`];**不能**只依赖"有帧才跑"的 on_frame_drawn。
//! 宿主契约(集成要求 #3):CEF 要求进程的 NSApplication 实现 `CefAppProtocol`
//! (`isHandlingSendEvent`/`setHandlingSendEvent:`),zap 目前没有 —— 需在启用本后端时
//! 用运行时 category/swizzle 补齐(spec 已记录,未实现)。

use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::ffi::c_void;
use std::sync::OnceLock;

use cef::*;

use crate::features::FeatureFlag;

/// CEF 泵间隔:60Hz。CEF 官方无固定值;过密浪费 CPU,过疏会拖慢流式更新。
const PUMP_INTERVAL_SECONDS: f64 = 1.0 / 60.0;

/// OSR(windowless)的绘制帧率上限。CEF 默认 30;60 与显示器刷新对齐
/// (specs/cef-webview-minimal/OSR-PLAN.md §1)。
const OSR_FRAME_RATE: i32 = 60;

/// 渲染模式:windowed(默认,现状)与 osr(windowless,真透明路线)。
///
/// OSR 是**进程级**能力(见 `initialize_inner`),故模式必须在 CefInitialize 之前
/// 就能判定 —— 这也是它先用环境变量而非设置项的原因(设置项见 OSR-PLAN.md T8)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RenderMode {
    /// CEF 自建原生子视图(windowed):**做不到真透明**,是当前默认路径。
    Windowed,
    /// windowless(OSR):宿主自建视图 + IOSurface,唯一能真透明的路径。
    Osr,
}

/// 解析模式开关(纯函数,便于单测):只有显式 `1`/`true` 才启用 OSR,
/// 未设置、空串、其他值一律 windowed —— 保证默认行为不变、可回滚。
/// **注意优先级**:在 `resolve_render_mode` 里,环境变量只要**非空**就算"显式覆盖"
/// (包括 `ZAP_CEF_OSR=0`/`=yes` 这类非真值 —— 它们会覆盖设置项、回落到 windowed),
/// 只有**空/空白**才视为"没设"、回落到设置项。
fn parse_render_mode(value: Option<&str>) -> RenderMode {
    match value.map(str::trim) {
        Some("1") | Some("true") => RenderMode::Osr,
        _ => RenderMode::Windowed,
    }
}

/// 启动期登记的"尽早初始化 CEF"请求(环境开关/flag 显式要求时,见 lib.rs)。
///
/// **为什么不在启动期直接初始化**:渲染模式(OSR/windowed)现在是设置项,而设置要等 app
/// 起来才读得到;`windowless_rendering_enabled` 又是 `CefSettings` 上的**进程级**开关,
/// `render_mode()` 用 OnceLock 首读即定死 ⇒ 若在读到设置之前初始化,设置项会被静默忽略
/// (审查 C1)。故启动期只登记请求,真正初始化推到"设置已推入之后"的首帧。
static EAGER_INIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 启动期登记 eager init 请求(幂等)。
pub(crate) fn request_eager_init() {
    EAGER_INIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 取出并清零 eager init 请求(首帧调用一次,返回是否曾被请求)。
pub(crate) fn take_eager_init_request() -> bool {
    EAGER_INIT_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 设置项推入的"是否用 OSR"(见 app/src/settings/cef_webview.rs 的 use_osr_rendering,
/// 由 app 每帧推入,与 freeze_after_secs 同一模式)。
static OSR_SETTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 由 app 推入设置值(每帧调用,代价是一次原子写)。
pub(crate) fn set_use_osr_rendering(enabled: bool) {
    OSR_SETTING.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// 当前渲染模式。
///
/// **进程内固定**:`windowless_rendering_enabled` 是 `CefSettings` 上的进程级开关,
/// 一旦 CefInitialize 就不能改,故这里缓存一次(因此设置项"下次启动生效")。
/// 优先级:环境变量 `ZAP_CEF_OSR`(显式设了就听它的,dev 排查用)→ 设置项 → windowed。
pub(crate) fn render_mode() -> RenderMode {
    static MODE: OnceLock<RenderMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        resolve_render_mode(
            std::env::var("ZAP_CEF_OSR").ok().as_deref(),
            OSR_SETTING.load(std::sync::atomic::Ordering::Relaxed),
        )
    })
}

/// 渲染模式的解析规则(**纯函数,单测覆盖**):
/// 环境变量显式非空 ⇒ 听它的(dev 排查/灰度);否则听设置项(`use_osr_rendering`,
/// 默认 windowed)。
fn resolve_render_mode(env_override: Option<&str>, setting_enabled: bool) -> RenderMode {
    match env_override {
        Some(value) if !value.trim().is_empty() => parse_render_mode(Some(value)),
        _ => {
            if setting_enabled {
                RenderMode::Osr
            } else {
                RenderMode::Windowed
            }
        }
    }
}

/// 是否由宿主驱动外部 begin-frame。默认关闭:T1 实测 CEF 内部 60fps 节奏已可用,
/// 而外部驱动要求宿主严格按显示刷新喂帧(见 OSR-PLAN.md §5 性能风险)。
fn external_begin_frame() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    // 口径与 `parse_render_mode` 一致:只有显式 "1"/"true" 才是开;设 `=0`/`=false`
    // 不能被当成开启(否则"关掉"这个开关反而生效)。
    *ENABLED.get_or_init(|| env_flag("ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME"))
}

/// 环境变量按"真值"解析:口径与 `parse_render_mode` 完全一致(只有 "1"/"true" 为真)。
fn env_flag(name: &str) -> bool {
    matches!(std::env::var(name).ok().as_deref(), Some("1") | Some("true"))
}

/// 关掉共享纹理,强制走 `on_paint`(CPU 位图)兜底路径:用于验证兜底实现
/// (GPU 进程异常时 CEF 也会自动回落到这条路径)。
fn cpu_paint() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_flag("ZAP_CEF_OSR_CPU_PAINT"))
}

/// OSR 视图尺寸(**DIP / 逻辑点**,不是像素)。
///
/// CEF 会自己乘 `device_scale_factor`:T1 实测若这里返回像素、`screen_info` 又报
/// scale=2,surface 会被缩放两次(2400x1600 而非 1200x800,浪费显存且可能模糊)。
/// CEF 不接受 0 宽高,故下限夹到 1。
fn osr_view_size(width: f64, height: f64) -> (i32, i32) {
    (
        (width.round() as i32).max(1),
        (height.round() as i32).max(1),
    )
}

/// OSR 的每-webview 原生状态(宿主视图指针 + 几何),由 webview 状态与 render handler
/// **共享**(`Rc`,UI 线程内单线程使用)。
///
/// 为什么不直接读 `WEBVIEWS`:CEF 的 `was_resized()` / `invalidate()` 会**同步**回调
/// render handler(GetViewRect/GetScreenInfo),而调用点通常正持有 `WEBVIEWS` 的
/// `borrow_mut()` —— 回调里再 `borrow()` 就是 `already mutably borrowed` panic
/// (T4 首次真机跑必崩,已实测)。故几何/视图指针走这个独立单元,render handler 只碰它。
struct OsrState {
    view: Cell<*mut c_void>,
    /// 视图尺寸(DIP):每帧 set_bounds 写入,CEF 同步回调时读到的就是最新值。
    size: Cell<(i32, i32)>,
    /// backing scale(2.0 = retina)。
    scale: Cell<f64>,
    /// 最近贴上去的 surface 像素尺寸(仅用于"surface = 点数 × scale"的证据日志)。
    surface: Cell<(i32, i32)>,
}

impl OsrState {
    fn new(view: *mut c_void, size: (i32, i32), scale: f64) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self {
            view: Cell::new(view),
            size: Cell::new(size),
            scale: Cell::new(scale),
            surface: Cell::new((0, 0)),
        })
    }

    fn view(&self) -> *mut c_void {
        self.view.get()
    }
}

// ===================== OSR 输入转发(T5) =====================
//
// windowless 下 CEF 没有自己的原生视图,AppKit 事件不会自动进渲染器,必须由宿主
// 采集后转成 CEF 事件。采集在 ObjC(见 cef_support.m),这里负责:修饰键位映射、
// mac→Windows 键码表、构造 cef_key_event_t / cef_mouse_event_t、编辑命令路由到
// focused frame。纯逻辑(键码表/修饰键位)都有单测。

/// 输入事件类型(与 ObjC 侧 `WARP_CEF_OSR_EVENT_*` 一一对应)。
const OSR_EVENT_MOUSE_MOVE: i32 = 0;
const OSR_EVENT_MOUSE_CLICK: i32 = 1;
const OSR_EVENT_MOUSE_WHEEL: i32 = 2;
const OSR_EVENT_KEY: i32 = 3;

/// 标准编辑动作(与 ObjC 侧 `WARP_CEF_OSR_EDIT_*` 一一对应)。
const OSR_EDIT_COPY: i32 = 0;
const OSR_EDIT_CUT: i32 = 1;
const OSR_EDIT_PASTE: i32 = 2;
const OSR_EDIT_SELECT_ALL: i32 = 3;
const OSR_EDIT_UNDO: i32 = 4;
const OSR_EDIT_REDO: i32 = 5;

/// AppKit 修饰键位(`NSEventModifierFlags`)。这些是稳定的 ABI 常量,与
/// `NSEventModifierFlag*` 一致;这里显式列出,避免为一个常量表引入 objc2 的
/// NSEvent feature(采集侧在 ObjC,本来就有这些常量)。
const NS_MOD_CAPS_LOCK: u32 = 1 << 16;
const NS_MOD_SHIFT: u32 = 1 << 17;
const NS_MOD_CONTROL: u32 = 1 << 18;
const NS_MOD_OPTION: u32 = 1 << 19;
const NS_MOD_COMMAND: u32 = 1 << 20;
const NS_MOD_NUMERIC_PAD: u32 = 1 << 21;

/// 与 ObjC `WarpCefOsrInputEvent` **逐字段对应**(顺序/类型不可改)。
#[repr(C)]
struct WarpCefOsrInputEvent {
    type_: i32,
    x: f64,
    y: f64,
    modifier_flags: u32,
    click_count: i32,
    button: i32,
    mouse_up: i32,
    mouse_leave: i32,
    delta_x: i32,
    delta_y: i32,
    key_type: i32,
    key_code: u16,
    is_repeat: i32,
    chars: *const c_char,
    chars_ignoring_modifiers: *const c_char,
}

/// 宿主右键菜单的命令(与 ObjC 侧 `WARP_CEF_OSR_MENU_*` 一一对应)。
const OSR_MENU_RELOAD: i32 = 0;
const OSR_MENU_INSPECT: i32 = 1;

/// 输入法动作(与 ObjC 侧 `WARP_CEF_OSR_IME_*` 一一对应)。
const OSR_IME_SET_COMPOSITION: i32 = 0;
const OSR_IME_COMMIT: i32 = 1;
const OSR_IME_FINISH: i32 = 2;
const OSR_IME_CANCEL: i32 = 3;

/// 一次按键的完整上下文(与 ObjC `WarpCefOsrKeyInput` 逐字段对应)。
/// 决策(普通按键 vs 组合 vs 上屏)在 [`deferred_key_plan`] 里做,便于单测。
#[repr(C)]
struct WarpCefOsrKeyInput {
    modifier_flags: u32,
    key_code: u16,
    is_repeat: i32,
    chars: *const c_char,
    chars_ignoring_modifiers: *const c_char,
    text_to_insert: *const c_char,
    marked_text: *const c_char,
    marked_sel_from: i32,
    marked_sel_to: i32,
    replacement_from: i32,
    replacement_to: i32,
    old_has_marked: i32,
    has_marked: i32,
    unmark_called: i32,
}

/// 输入法在按键之外直接发起的调用(与 ObjC `WarpCefOsrImeCommand` 逐字段对应)。
#[repr(C)]
struct WarpCefOsrImeCommand {
    action: i32,
    text: *const c_char,
    sel_from: i32,
    sel_to: i32,
    replacement_from: i32,
    replacement_to: i32,
    keep_selection: i32,
}

#[repr(C)]
struct WarpCefOsrInputCallbacks {
    handle_event: extern "C" fn(u64, *const WarpCefOsrInputEvent),
    handle_edit_command: extern "C" fn(u64, i32),
    handle_focus: extern "C" fn(u64, i32),
    handle_key: extern "C" fn(u64, *const WarpCefOsrKeyInput),
    handle_ime: extern "C" fn(u64, *const WarpCefOsrImeCommand),
    handle_menu_command: extern "C" fn(u64, i32, f64, f64),
}

static OSR_INPUT_CALLBACKS: WarpCefOsrInputCallbacks = WarpCefOsrInputCallbacks {
    handle_event: osr_input_event_trampoline,
    handle_edit_command: osr_edit_command_trampoline,
    handle_focus: osr_focus_trampoline,
    handle_key: osr_key_trampoline,
    handle_ime: osr_ime_trampoline,
    handle_menu_command: osr_menu_command_trampoline,
};

/// 注册输入回调(幂等;必须在创建 OSR 宿主视图之前)。
fn install_osr_input_callbacks() {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        unsafe { warp_cef_osr_view_set_input_callbacks(&OSR_INPUT_CALLBACKS) };
        true
    });
}

/// 取浏览器句柄副本:**不持有 `WEBVIEWS` 借用再调 CEF**(CEF 可能同步回调
/// render handler,那条路径不得再借 `WEBVIEWS`,见 OsrState 注释)。
fn browser_snapshot(id: u64) -> Option<Browser> {
    if is_shutting_down() {
        return None;
    }
    // 与 with_browser 同口径:所有 extern "C" 回调入口都可能被同步重入,
    // 借不到(调用方正持借用)就跳过,不要 panic(在 C 回调里 panic = abort)。
    WEBVIEWS.with(|map| {
        let map = map.try_borrow().ok()?;
        map.get(&id).and_then(|state| state.browser.clone())
    })
}

/// 视图坐标(DIP,可能是小数)→ CEF 的整数坐标。
/// cefclient 的做法是"先乘 scale 取整、再除回 scale",对 scale ≥ 1 等价于截断。
fn to_dip_coord(value: f64) -> i32 {
    value.trunc() as i32
}

/// `NSEventModifierFlags` → `cef_event_flags_t` 位(鼠标/键盘共用)。
fn cef_modifiers(flags: u32) -> u32 {
    let mut out = 0;
    if flags & NS_MOD_SHIFT != 0 {
        out |= sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0;
    }
    if flags & NS_MOD_CONTROL != 0 {
        out |= sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0;
    }
    if flags & NS_MOD_OPTION != 0 {
        out |= sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0;
    }
    if flags & NS_MOD_COMMAND != 0 {
        out |= sys::cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0;
    }
    if flags & NS_MOD_CAPS_LOCK != 0 {
        out |= sys::cef_event_flags_t::EVENTFLAG_CAPS_LOCK_ON.0;
    }
    out
}

/// 小键盘判定:`NSEventModifierFlagNumericPad` 覆盖绝大多数情况,再补一份键码表
/// (对齐 cefclient 的 `isKeyPadEvent:`)。
fn is_key_pad_event(mac_key_code: u16, flags: u32) -> bool {
    if flags & NS_MOD_NUMERIC_PAD != 0 {
        return true;
    }
    matches!(
        mac_key_code,
        // Clear, =, /, *, -, +, Enter, ., 数字 0-9
        71 | 81 | 75 | 67 | 78 | 69 | 76 | 65 | 82 | 83 | 84 | 85 | 86 | 87 | 88 | 89 | 91 | 92
    )
}

/// macOS 虚拟键码 → Windows VK(`cef_key_event_t.windows_key_code`)。
///
/// CEF/Chromium 用 Windows 的 VK_* 编号;普通可打印键用**大写码点**(字母/数字正好
/// 等于 VK_A..VK_Z / VK_0..VK_9),其余查表(对齐参考实现与 Chromium 的 mac 转换)。
fn windows_key_code(mac_key_code: u16, chars_ignoring_modifiers: Option<&str>) -> i32 {
    if let Some(code) = special_windows_key_code(mac_key_code) {
        return code;
    }
    match chars_ignoring_modifiers.and_then(|chars| chars.chars().next()) {
        Some(first) => {
            // 参考实现用 `uppercased()`(对非 ASCII 也可能变长),这里只认 ASCII:
            // 非 ASCII 字符没有对应 VK,交给 character 字段承载。
            let code = u32::from(first.to_ascii_uppercase());
            if code < 128 {
                code as i32
            } else {
                0
            }
        }
        None => 0,
    }
}

/// 与字符无关的键(导航/功能/修饰键)的 VK 表。
/// mac 键码 → Windows VK。
///
/// **注意:本平台(CEF macOS OSR)这个值不会被 CEF 采用** ——
/// `CefBrowserPlatformDelegateNativeMac::TranslateWebKeyEvent` 会用我们给的字符/键码
/// **合成一个 NSEvent**,再由 Chromium 的 `NativeWebKeyboardEvent(NSEvent*)` 反推
/// windowsKeyCode(CEF 源码注释直言这是"唯一无法直接翻译的成员")。保留此表是为了与
/// 参考实现(cefclient/CefSwift,跨平台共用同一份 KeyEvent 构造)保持一致,不要把它
/// 当作 mac 上的生效行为(证据文档曾误把它写成交付项,已更正)。
fn special_windows_key_code(mac_key_code: u16) -> Option<i32> {
    let code = match mac_key_code {
        0x24 => 0x0D,       // Return
        0x4C => 0x0D,       // 小键盘 Enter
        0x30 => 0x09,       // Tab
        0x31 => 0x20,       // Space
        0x33 => 0x08,       // Delete(退格)→ VK_BACK
        0x75 => 0x2E,       // Forward Delete
        0x35 => 0x1B,       // Escape
        0x7B => 0x25,       // Left
        0x7C => 0x27,       // Right
        0x7D => 0x28,       // Down
        0x7E => 0x26,       // Up
        0x73 => 0x24,       // Home
        0x77 => 0x23,       // End
        0x74 => 0x21,       // PageUp
        0x79 => 0x22,       // PageDown
        0x38 | 0x3C => 0x10, // Shift / RightShift
        0x3B | 0x3E => 0x11, // Control / RightControl
        0x3A | 0x3D => 0x12, // Option / RightOption
        0x37 => 0x5B,       // Command → VK_LWIN
        0x36 => 0x5C,       // RightCommand
        0x39 => 0x14,       // CapsLock
        0x7A => 0x70,       // F1
        0x78 => 0x71,       // F2
        0x63 => 0x72,       // F3
        0x76 => 0x73,       // F4
        0x60 => 0x74,       // F5
        0x61 => 0x75,       // F6
        0x62 => 0x76,       // F7
        0x64 => 0x77,       // F8
        0x65 => 0x78,       // F9
        0x6D => 0x79,       // F10
        0x67 => 0x7A,       // F11
        0x6F => 0x7B,       // F12
        _ => return None,
    };
    Some(code)
}

/// `flagsChanged` 时该修饰键现在是按下还是松开。
fn is_modifier_pressed(mac_key_code: u16, flags: u32) -> bool {
    match mac_key_code {
        56 | 60 => flags & NS_MOD_SHIFT != 0,
        59 | 62 => flags & NS_MOD_CONTROL != 0,
        58 | 61 => flags & NS_MOD_OPTION != 0,
        55 | 54 => flags & NS_MOD_COMMAND != 0,
        57 => flags & NS_MOD_CAPS_LOCK != 0,
        _ => true,
    }
}

/// CEF 光标类型 → 语义 id(与 ObjC 侧 `WARP_CEF_OSR_CURSOR_*` 对应)。
/// 未映射的一律给箭头,避免出现"卡住不动"的怪光标(自定义光标位图不支持)。
fn cursor_semantic(cursor_type: CursorType) -> i32 {
    if cursor_type == CursorType::HAND {
        1 // 手型(链接)
    } else if cursor_type == CursorType::IBEAM || cursor_type == CursorType::VERTICALTEXT {
        2 // 文本输入
    } else if cursor_type == CursorType::CROSS {
        3
    } else if cursor_type == CursorType::EASTRESIZE
        || cursor_type == CursorType::WESTRESIZE
        || cursor_type == CursorType::EASTWESTRESIZE
        || cursor_type == CursorType::COLUMNRESIZE
        || cursor_type == CursorType::EASTPANNING
        || cursor_type == CursorType::WESTPANNING
    {
        4 // 水平缩放
    } else if cursor_type == CursorType::NORTHRESIZE
        || cursor_type == CursorType::SOUTHRESIZE
        || cursor_type == CursorType::NORTHSOUTHRESIZE
        || cursor_type == CursorType::ROWRESIZE
        || cursor_type == CursorType::NORTHPANNING
        || cursor_type == CursorType::SOUTHPANNING
    {
        5 // 垂直缩放
    } else if cursor_type == CursorType::GRAB {
        6
    } else if cursor_type == CursorType::GRABBING
        || cursor_type == CursorType::MIDDLEPANNING
        || cursor_type == CursorType::MIDDLE_PANNING_VERTICAL
    {
        7
    } else if cursor_type == CursorType::NOTALLOWED || cursor_type == CursorType::NODROP {
        8
    } else if cursor_type == CursorType::ZOOMIN {
        9
    } else if cursor_type == CursorType::ZOOMOUT {
        10
    } else if cursor_type == CursorType::MOVE {
        11
    } else {
        0 // 箭头
    }
}

/// 鼠标键位 → CEF 枚举。
fn mouse_button(button: i32) -> MouseButtonType {
    match button {
        1 => MouseButtonType::RIGHT,
        2 => MouseButtonType::MIDDLE,
        _ => MouseButtonType::LEFT,
    }
}

/// 按住的是哪个键(用于拖拽/点击时的 `EVENTFLAG_*_MOUSE_BUTTON`;cefclient 对
/// 按下与抬起都会置位)。
fn mouse_button_flag(button: i32) -> u32 {
    match button {
        1 => sys::cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON.0,
        2 => sys::cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0,
        _ => sys::cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0,
    }
}

/// C 字符串 → String(空指针/空串 → None)。
fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    (!text.is_empty()).then_some(text)
}

/// 取首个 UTF-16 码元(CEF 的 `character`/`unmodified_character` 口径)。
fn first_utf16(text: &str) -> Option<u16> {
    text.encode_utf16().next()
}

extern "C" fn osr_input_event_trampoline(id: u64, event: *const WarpCefOsrInputEvent) {
    if event.is_null() {
        return;
    }
    let event = unsafe { &*event };
    let Some(browser) = browser_snapshot(id) else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };
    // 转发日志:鼠标移动按 trace(否则每次移动都刷屏),其余按 debug。
    // `RUST_LOG=warp::browser::cef_backend=debug` 可打开 —— 这是"send_mouse_*/send_key_event
    // 确实被调用"的证据通道(OSR-PLAN.md T5 的验证要求)。
    match event.type_ {
        OSR_EVENT_MOUSE_MOVE => {
            log::trace!(
                "[cef] osr {id}: send_mouse_move ({:.0},{:.0}) leave={}",
                event.x,
                event.y,
                event.mouse_leave
            );
            send_osr_mouse(&host, event);
        }
        OSR_EVENT_MOUSE_CLICK => {
            log::debug!(
                "[cef] osr {id}: send_mouse_click button={} up={} count={} ({:.0},{:.0})",
                event.button,
                event.mouse_up,
                event.click_count,
                event.x,
                event.y
            );
            send_osr_mouse(&host, event);
        }
        OSR_EVENT_MOUSE_WHEEL => {
            log::debug!(
                "[cef] osr {id}: send_mouse_wheel ({},{}) at ({:.0},{:.0})",
                event.delta_x,
                event.delta_y,
                event.x,
                event.y
            );
            send_osr_mouse(&host, event);
        }
        // 注意:**按键按下(key_type=0)不走这条路径** —— T7 起宿主视图的 keyDown
        // 走 deferred 模型(handle_key → osr_key_trampoline),因为要先把事件交给输入法。
        // 这里只剩抬起(key_type=1)与修饰键变化(key_type=2)。
        OSR_EVENT_KEY => {
            log::debug!(
                "[cef] osr {id}: send_key_event key_type={} code={} ns_flags=0x{:X} cef_flags=0x{:X} \
                 chars={:?}",
                event.key_type,
                event.key_code,
                event.modifier_flags,
                cef_modifiers(event.modifier_flags),
                cstr_to_string(event.chars)
            );
            send_osr_key(&host, event);
        }
        _ => {}
    }
}

/// 鼠标/滚轮转发。坐标已是 DIP(左上原点)。
fn send_osr_mouse(host: &BrowserHost, event: &WarpCefOsrInputEvent) {
    let mut modifiers = cef_modifiers(event.modifier_flags);
    if event.button >= 0 && event.type_ != OSR_EVENT_MOUSE_WHEEL {
        modifiers |= mouse_button_flag(event.button);
    }
    let point = MouseEvent {
        x: to_dip_coord(event.x),
        y: to_dip_coord(event.y),
        modifiers,
    };
    match event.type_ {
        OSR_EVENT_MOUSE_MOVE => host.send_mouse_move_event(Some(&point), event.mouse_leave),
        OSR_EVENT_MOUSE_CLICK => host.send_mouse_click_event(
            Some(&point),
            mouse_button(event.button),
            event.mouse_up,
            event.click_count,
        ),
        OSR_EVENT_MOUSE_WHEEL => {
            host.send_mouse_wheel_event(Some(&point), event.delta_x, event.delta_y)
        }
        _ => {}
    }
}

/// 键盘转发:按下走 **KEYDOWN + CHAR 两段式**。
///
/// 只发 KEYDOWN 的话页面 JS `keydown` 会触发但字符进不去、光标不动;只发 CHAR 则
/// 快捷键/组合键不生效(cefclient 的 deferred 模型结论一致)。
fn send_osr_key(host: &BrowserHost, event: &WarpCefOsrInputEvent) {
    let chars = cstr_to_string(event.chars);
    let unmodified = cstr_to_string(event.chars_ignoring_modifiers);
    let mut key = KeyEvent::default();
    key.modifiers = cef_modifiers(event.modifier_flags);
    if is_key_pad_event(event.key_code, event.modifier_flags) {
        key.modifiers |= sys::cef_event_flags_t::EVENTFLAG_IS_KEY_PAD.0;
    }
    if event.is_repeat != 0 {
        key.modifiers |= sys::cef_event_flags_t::EVENTFLAG_IS_REPEAT.0;
    }
    key.native_key_code = i32::from(event.key_code);
    key.windows_key_code = windows_key_code(event.key_code, unmodified.as_deref());
    key.is_system_key = 0;
    if let Some(character) = chars.as_deref().and_then(first_utf16) {
        key.character = character;
    }
    if let Some(character) = unmodified.as_deref().and_then(first_utf16) {
        key.unmodified_character = character;
    }
    match event.key_type {
        1 => {
            key.type_ = KeyEventType::KEYUP;
            host.send_key_event(Some(&key));
        }
        2 => {
            // flagsChanged:按下发 KEYDOWN、松开发 KEYUP。
            key.type_ = if is_modifier_pressed(event.key_code, event.modifier_flags) {
                KeyEventType::KEYDOWN
            } else {
                KeyEventType::KEYUP
            };
            host.send_key_event(Some(&key));
        }
        // 按下不在这里:KEYDOWN+CHAR 的两段式由 deferred 计划决定
        // (见 osr_key_trampoline),因为要先看输入法有没有组合/上屏。
        _ => {}
    }
}

/// 编辑命令 → focused frame(取不到时退回主 frame,与参考实现一致)。
extern "C" fn osr_edit_command_trampoline(id: u64, command: i32) {
    // 证据通道(与 send_mouse_*/send_key_event 同口径)。两条入口都会到这里:
    // 1. 视图级 `performKeyEquivalent:` 拦下的 Cmd+A/C/V/X/Z(见 cef_support.m);
    // 2. Cmd+C/V/X/A 由 WarpWindow::performKeyEquivalent: 的嵌入视图分支直接发到
    //    本视图的 copy:/paste:/cut:/selectAll:(以及菜单经响应者链的 undo:/redo:)。
    log::debug!("[cef] osr {id}: edit command {command}");
    let Some(browser) = browser_snapshot(id) else {
        return;
    };
    let Some(frame) = browser.focused_frame().or_else(|| browser.main_frame()) else {
        return;
    };
    match command {
        OSR_EDIT_COPY => frame.copy(),
        OSR_EDIT_CUT => frame.cut(),
        OSR_EDIT_PASTE => frame.paste(),
        OSR_EDIT_SELECT_ALL => frame.select_all(),
        OSR_EDIT_UNDO => frame.undo(),
        OSR_EDIT_REDO => frame.redo(),
        _ => {}
    }
}

/// 宿主视图成为/失去 first responder → 同步给 CEF(页面光标、IME、键盘都依赖它)。
extern "C" fn osr_focus_trampoline(id: u64, focused: i32) {
    log::debug!("[cef] osr {id}: focus={focused}");
    focus(id, focused != 0);
}

// ---------------------- T7:输入法(IME) ----------------------
//
// 决策模型照抄参考实现/CefClient 的 deferred 模型(HandleKeyEventBefore/AfterTextInputClient):
// 输入法在 `interpretKeyEvents:` 里只**累积**状态,keyDown 结束时一次性决定该发什么。
// 这样普通按键仍是 KEYDOWN+CHAR(保住页面 JS keydown 与光标移动),而 composition/上屏
// 走 ime_set_composition / ime_commit_text。

/// 一次按键要执行的动作(顺序:普通键 → 提交 → 组合上报 → 收尾)。
#[derive(Debug, Default, PartialEq, Eq)]
struct DeferredKeyPlan {
    /// 普通按键:KEYDOWN + CHAR。
    send_plain_key: bool,
    /// 提交文本(粘贴、输入法上屏)。
    commit_text: Option<String>,
    /// composition 文本更新。
    set_composition: Option<String>,
    /// composition 以 finish 收尾(输入法主动 unmarkText)。
    finish_composition: bool,
    /// composition 以 cancel 收尾(组合被丢弃)。
    cancel_composition: bool,
}

/// deferred 决策(纯逻辑,单测覆盖):
/// 1. 前后都没有 composition 且最多插入 1 个字符 ⇒ 按普通按键发 KEYDOWN+CHAR;
/// 2. 待插入文本长度超过阈值(有 composition 时阈值 0,否则 1)⇒ 提交;
/// 3. 有 composition 且非空 ⇒ 上报组合;组合从"有"变"无" ⇒ finish 或 cancel。
fn deferred_key_plan(
    text_to_insert: Option<&str>,
    marked_text: Option<&str>,
    old_has_marked: bool,
    has_marked: bool,
    unmark_called: bool,
) -> DeferredKeyPlan {
    let text_len = text_to_insert.map_or(0, |text| text.encode_utf16().count());
    let mut plan = DeferredKeyPlan::default();
    plan.send_plain_key = !has_marked && !old_has_marked && text_len <= 1;
    let commit_threshold = usize::from(!(has_marked || old_has_marked));
    if text_len > commit_threshold {
        plan.commit_text = text_to_insert.map(ToString::to_string);
    }
    if has_marked && marked_text.is_some() {
        plan.set_composition = marked_text.map(ToString::to_string);
    } else if old_has_marked && !has_marked {
        plan.finish_composition = unmark_called;
        plan.cancel_composition = !unmark_called;
    }
    plan
}

/// UTF-16 选区 → CEF `Range`;**没有替换目标时不能传 NULL**。
///
/// CEF 的 capi 会把可空指针退化成 `(0,0)`,渲染器据此 `SelectRange` 打断
/// `<textarea>` 焦点,整条 composition 被静默丢弃(T2 spike 实测,见 OSR-SPIKE-B.md)。
fn ime_replacement_range(from: i32, to: i32) -> Range {
    if from < 0 {
        Range {
            from: u32::MAX,
            to: u32::MAX,
        }
    } else {
        Range {
            from: from as u32,
            to: to.max(from) as u32,
        }
    }
}

/// composition 下划线:Blink 至少需要一条(参考实现同款全串实线)。
fn ime_underline(utf16_len: u32) -> CompositionUnderline {
    CompositionUnderline {
        size: std::mem::size_of::<CompositionUnderline>(),
        range: Range {
            from: 0,
            to: utf16_len,
        },
        color: 0xFF00_0000,
        background_color: 0,
        thick: 0,
        style: CompositionUnderlineStyle::SOLID,
    }
}

/// 一次按键的 deferred 决策 → 实际下发。
extern "C" fn osr_key_trampoline(id: u64, key: *const WarpCefOsrKeyInput) {
    if key.is_null() {
        return;
    }
    let key = unsafe { &*key };
    // **这里不再拦截 Cmd+A/C/V/X/Z**:那条路径已被视图级 `performKeyEquivalent:`
    // (cef_support.m,排在 AppKit 菜单之前、覆盖 CapsLock 形态)完整接管并消费,
    // keyDown 根本收不到这组键 ⇒ 原来这层兜底不可达(评审判定为重复 owner,已删)。
    let unmodified = cstr_to_string(key.chars_ignoring_modifiers);
    let text_to_insert = cstr_to_string(key.text_to_insert);
    let marked_text = cstr_to_string(key.marked_text);
    let plan = deferred_key_plan(
        text_to_insert.as_deref(),
        marked_text.as_deref(),
        key.old_has_marked != 0,
        key.has_marked != 0,
        key.unmark_called != 0,
    );
    let Some(browser) = browser_snapshot(id) else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };

    // 基础键事件(普通按键与 CHAR 共用)。
    let mut base = KeyEvent::default();
    base.modifiers = cef_modifiers(key.modifier_flags);
    if is_key_pad_event(key.key_code, key.modifier_flags) {
        base.modifiers |= sys::cef_event_flags_t::EVENTFLAG_IS_KEY_PAD.0;
    }
    if key.is_repeat != 0 {
        base.modifiers |= sys::cef_event_flags_t::EVENTFLAG_IS_REPEAT.0;
    }
    base.native_key_code = i32::from(key.key_code);
    base.windows_key_code = windows_key_code(key.key_code, unmodified.as_deref());
    let chars = cstr_to_string(key.chars);
    if let Some(character) = chars.as_deref().and_then(first_utf16) {
        base.character = character;
    }
    if let Some(character) = unmodified.as_deref().and_then(first_utf16) {
        base.unmodified_character = character;
    }

    if plan.send_plain_key {
        // 小键盘 Clear(键码 71)只发 KEYDOWN、**不发 CHAR**:cefclient 在此直接 return
        // (见 text_input_client_osr_mac.mm 的 `native_key_code == 71`),因为它的
        // characters 是 ESC(0x1B),当文本发出去会往页面插控制字符。
        if is_key_pad_event(key.key_code, key.modifier_flags) && key.key_code == 71 {
            log::debug!("[cef] osr {id}: send_key_event(KEYDOWN only, 小键盘 Clear)");
            base.type_ = KeyEventType::KEYDOWN;
            host.send_key_event(Some(&base));
            return;
        }
        // 证据通道(与 T5 的 send_mouse_*/send_key_event 同口径):英文/功能键走这条路径。
        log::debug!(
            "[cef] osr {id}: send_key_event(KEYDOWN+CHAR) code={} chars={:?}",
            key.key_code,
            text_to_insert.as_deref().or(chars.as_deref())
        );
        base.type_ = KeyEventType::KEYDOWN;
        host.send_key_event(Some(&base));
        // CHAR 用输入法实际插入的字符(死键/组合键时与 event.characters 不同)。
        if let Some(character) = text_to_insert.as_deref().and_then(first_utf16) {
            base.character = character;
        }
        base.type_ = KeyEventType::CHAR;
        host.send_key_event(Some(&base));
    }
    if let Some(text) = plan.commit_text.as_deref() {
        if !text.is_empty() {
            let cef_text = CefString::from(text);
            // **提交时不带 replacement range**(对齐参考实现 `imeCommitText(text, replacementRange: nil)`):
            // 组装的文本已经在 composition 阶段进了文档,再给一个"有效"的替换区间会让
            // 渲染器走 SelectRange —— 在 `<textarea>` 上这正是 T2 记录过的、会把焦点打断、
            // composition 静默丢弃的路径(见 OSR-SPIKE-B.md)。
            let replacement = ime_replacement_range(-1, -1);
            log::debug!("[cef] osr {id}: ime_commit_text {text:?}");
            // relative_cursor_pos=0 ⇒ 光标落在提交文本末尾(Chromium 的
            // InputMethodController::ComputeAbsoluteCaretPosition = 起点 + 长度 + relative)。
            host.ime_commit_text(Some(&cef_text), Some(&replacement), 0);
        }
    }
    if let Some(text) = plan.set_composition.as_deref() {
        let utf16_len = text.encode_utf16().count() as u32;
        let cef_text = CefString::from(text);
        let underline = ime_underline(utf16_len);
        let selection = Range {
            from: key.marked_sel_from.max(0) as u32,
            to: key.marked_sel_to.max(0) as u32,
        };
        let replacement = ime_replacement_range(key.replacement_from, key.replacement_to);
        log::debug!("[cef] osr {id}: ime_set_composition {text:?} sel={selection:?}");
        host.ime_set_composition(
            Some(&cef_text),
            Some(&[underline]),
            Some(&replacement),
            Some(&selection),
        );
    }
    if plan.finish_composition {
        log::debug!("[cef] osr {id}: ime_finish_composing_text");
        host.ime_finish_composing_text(0);
    }
    if plan.cancel_composition {
        log::debug!("[cef] osr {id}: ime_cancel_composition");
        host.ime_cancel_composition();
    }
}

/// 宿主右键菜单的命令(见 cef_support.m 的 `warpShowContextMenu:`)。
///
/// 为什么是宿主菜单而不是 CEF 原生菜单:`menu_runner_mac.mm` 的 windowless 分支要求
/// `browser->GetWindowHandle()` 非空,而 windowless 的 handle 来自 `WindowInfo.parent_view`
/// —— 本项目的 OSR 路径从不设它(只有 windowed 路径 `set_as_child`)⇒ CEF 原生菜单在 OSR
/// **结构性不可用**(这也是实测里 `on_before_context_menu` 一次都没被调用的原因)。
/// 故按 OSR-PLAN.md T6 由宿主弹 NSMenu,命令回这里走 CEF API。
extern "C" fn osr_menu_command_trampoline(id: u64, command: i32, x: f64, y: f64) {
    let Some(browser) = browser_snapshot(id) else {
        return;
    };
    match command {
        OSR_MENU_RELOAD => {
            log::info!("[cef] webview {id} 右键重新加载");
            browser.reload();
        }
        OSR_MENU_INSPECT => {
            log::info!("[cef] webview {id} 右键检查元素 ({x},{y})");
            if let Some(host) = browser.host() {
                let point = Point {
                    x: x as i32,
                    y: y as i32,
                };
                host.show_dev_tools(None, None, None, Some(&point));
            }
        }
        _ => {}
    }
}

/// 输入法在按键之外的直接动作(候选点击上屏、unmark 等)。
extern "C" fn osr_ime_trampoline(id: u64, command: *const WarpCefOsrImeCommand) {
    if command.is_null() {
        return;
    }
    let command = unsafe { &*command };
    let text = cstr_to_string(command.text);
    let Some(browser) = browser_snapshot(id) else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };
    match command.action {
        OSR_IME_SET_COMPOSITION => {
            let Some(text) = text.as_deref() else {
                return;
            };
            let utf16_len = text.encode_utf16().count() as u32;
            let cef_text = CefString::from(text);
            let underline = ime_underline(utf16_len);
            let selection = Range {
                from: command.sel_from.max(0) as u32,
                to: command.sel_to.max(0) as u32,
            };
            let replacement = ime_replacement_range(command.replacement_from, command.replacement_to);
            log::debug!("[cef] osr {id}: ime_set_composition(直接) {text:?}");
            host.ime_set_composition(
                Some(&cef_text),
                Some(&[underline]),
                Some(&replacement),
                Some(&selection),
            );
        }
        OSR_IME_COMMIT => {
            let Some(text) = text.as_deref() else {
                return;
            };
            let cef_text = CefString::from(text);
            let replacement = ime_replacement_range(command.replacement_from, command.replacement_to);
            log::debug!("[cef] osr {id}: ime_commit_text(直接) {text:?}");
            host.ime_commit_text(Some(&cef_text), Some(&replacement), 0);
        }
        OSR_IME_FINISH => {
            log::debug!("[cef] osr {id}: ime_finish_composing_text(直接)");
            host.ime_finish_composing_text(command.keep_selection);
        }
        OSR_IME_CANCEL => {
            log::debug!("[cef] osr {id}: ime_cancel_composition(直接)");
            host.ime_cancel_composition();
        }
        _ => {}
    }
}

extern "C" {
    /// 见 app/src/platform/mac/objc/cef_support.m(仅 cef_webview feature 下编译)。
    fn warp_cef_start_periodic_main_timer(interval: f64, callback: extern "C" fn());
    fn warp_cef_install_app_protocol_support();

    // OSR(windowless)宿主视图:生命周期由本模块显式持有(+1),销毁时 release。
    fn warp_cef_osr_view_new(
        container: *mut c_void,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        webview_id: u64,
    ) -> *mut c_void;
    fn warp_cef_osr_view_set_input_callbacks(callbacks: *const WarpCefOsrInputCallbacks);
    fn warp_cef_osr_view_set_cursor(view: *mut c_void, semantic: i32);
    fn warp_cef_osr_view_focus(view: *mut c_void, focused: i32);
    fn warp_cef_osr_view_is_first_responder(view: *mut c_void) -> i32;
    fn warp_cef_osr_view_set_ime_bounds(
        view: *mut c_void,
        sel_from: i32,
        sel_to: i32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        has_bounds: i32,
    );
    fn warp_cef_osr_view_set_surface(view: *mut c_void, surface: *mut c_void);
    fn warp_cef_osr_view_set_popup_surface(view: *mut c_void, surface: *mut c_void);
    fn warp_cef_osr_view_set_popup_bitmap(
        view: *mut c_void,
        buffer: *const u8,
        width: i32,
        height: i32,
        bytes_per_row: i32,
    );
    fn warp_cef_osr_view_show_popup(view: *mut c_void, visible: i32);
    fn warp_cef_osr_view_screen_point(
        view: *mut c_void,
        view_x: f64,
        view_y: f64,
        out_x: *mut f64,
        out_y: *mut f64,
    ) -> i32;
    fn warp_cef_osr_view_set_popup_rect(view: *mut c_void, x: f64, y: f64, w: f64, h: f64);
    fn warp_cef_osr_view_set_bitmap(
        view: *mut c_void,
        buffer: *const u8,
        width: i32,
        height: i32,
        bytes_per_row: i32,
    );
    fn warp_cef_osr_view_set_frame(view: *mut c_void, x: f64, y: f64, w: f64, h: f64);
    fn warp_cef_osr_view_set_hidden(view: *mut c_void, hidden: i32);
    fn warp_cef_osr_view_scale(view: *mut c_void) -> f64;
    fn warp_cef_osr_view_surface_size(view: *mut c_void, out_width: *mut i32, out_height: *mut i32);
    fn warp_cef_osr_view_release(view: *mut c_void);
}

extern "C" fn pump_trampoline() {
    pump();
}

/// 安装 NSApplication 协议桥(集成要求 #3)。须在创建任何浏览器前、主线程调用。
pub(crate) fn install_app_protocol_support() {
    unsafe { warp_cef_install_app_protocol_support() };
}

/// 启动主线程周期泵(集成要求 #4):zap 事件循环空闲时没有帧,必须靠独立定时器
/// 推进 CEF 的消息循环。
pub(crate) fn start_pump() {
    unsafe { warp_cef_start_periodic_main_timer(PUMP_INTERVAL_SECONDS, pump_trampoline) };
}

/// CEF 是否已完成初始化(进程内一次性)。
static INITIALIZED: OnceLock<bool> = OnceLock::new();

/// 已加载的 CEF 动态库(进程生命周期持有;macOS 从 .app 内 framework 加载)。
/// 必须在 `execute_process` / `initialize` **之前**完成加载,否则 CEF 的 C API
/// 是空指针表 —— 实测在 `execute_process` 处直接 SIGSEGV。
static LIBRARY: OnceLock<cef::library_loader::LibraryLoader> = OnceLock::new();
/// 动态库是否加载成功(OnceLock 里存的是 loader,不带状态查询 API)。
static LIBRARY_LOADED: OnceLock<bool> = OnceLock::new();

/// 当前进程是否是 CEF 子进程(以 `--type=<process>` 启动)。
///
/// 必须在**加载 CEF 库之前**判断:子进程要用 helper 模式加载 framework
/// (helper 的 framework 在**外层 app** 的 Contents/Frameworks 下,非 helper 模式
/// 会在 helper 自己的 bundle 里找 → 找不到 → GPU/Renderer 启动即失败)。
fn is_cef_subprocess() -> bool {
    std::env::args().any(|arg| arg.starts_with("--type="))
}

/// 加载 CEF 动态库 + 初始化 API hash(幂等)。返回是否成功。
///
/// `helper=true` 用于子进程:库加载按"外层 app"解析(CefScopedLibraryLoader 的
/// helper 路径),这是 CEF 多进程布局的硬要求。
fn load_library(helper: bool) -> bool {
    let loaded = *LIBRARY_LOADED.get_or_init(|| {
        let loader = cef::library_loader::LibraryLoader::new(
            &std::env::current_exe().expect("current exe"),
            helper,
        );
        let loaded = loader.load();
        if !loaded {
            log::error!("[cef] failed to load Chromium Embedded Framework (需以 .app 形态运行)");
        }
        let _ = LIBRARY.set(loader);
        loaded
    });
    if !loaded {
        return false;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    true
}

/// 子进程启动前置:CEF 的多进程模型要求每个子进程在这里分流执行。
/// 返回值:`true` 表示当前是 browser 进程,应继续正常启动流程;
/// `false` 表示这是 CEF 子进程(已处理完毕,调用方应立即退出)。
pub(crate) fn handle_subprocess_or_continue() -> bool {
    // 顺序关键:先加载库并初始化 API hash,再 execute_process(spike 已验证;
    // 反过来会因 C API 表为空而崩溃)。子进程判定只能靠命令行,不能靠 CEF API。
    let is_subprocess = is_cef_subprocess();
    if !load_library(is_subprocess) {
        // 子进程分支不能"继续正常启动":那会走到 ChannelState::new,而 helper 的
        // 带后缀 bundle id 会命中 AppId 三段断言而 **panic**(评审 F5)。
        if is_subprocess {
            log::error!("[cef] helper 子进程无法加载 CEF framework,直接退出");
            std::process::exit(1);
        }
        return true;
    }
    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        log::error!("[cef] failed to parse command line arguments");
        if is_subprocess {
            std::process::exit(1);
        }
        return true;
    };
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;
    let ret = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if is_browser_process {
        debug_assert_eq!(ret, -1, "browser process must not be a CEF subprocess");
        true
    } else {
        false
    }
}

/// 初始化 CEF(browser 进程内只做一次)。返回是否可用。
///
/// 注意:`library_loader` 在 macOS 上从当前可执行文件所在的 .app 内找
/// `Chromium Embedded Framework.framework`,因此**必须**以 .app 形态运行
/// (见 TECH.md 阶段 1 的打包要求)。
pub(crate) fn initialize_runtime() -> bool {
    *INITIALIZED.get_or_init(|| initialize_inner())
}

/// CEF 的缓存目录(每实例独立,避免 Chromium ProcessSingleton 冲突)。
fn cef_cache_paths() -> (String, String) {
    let base = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zap-cef");
    let root = base.join("root-cache");
    // CEF 要求 cache_path 必须是 root_cache_path 的子目录(否则报错并回退到内存存储)。
    let cache = root.join("cache");
    for dir in [&root, &cache] {
        if let Err(err) = std::fs::create_dir_all(dir) {
            log::warn!("[cef] 创建缓存目录 {} 失败: {err}", dir.display());
        }
    }
    (
        cache.to_string_lossy().into_owned(),
        root.to_string_lossy().into_owned(),
    )
}

fn initialize_inner() -> bool {
    if !load_library(false) {
        let _ = INIT_STATUS.set("framework 加载失败(需以 .app 形态运行且 bundle 内含 CEF)".into());
        return false;
    }

    let args = cef::args::Args::new();
    let mut app = CefApp::new();
    // 每实例独立的缓存/根缓存目录。**必须显式设置**:CEF 默认用共享目录时 Chromium 的
    // ProcessSingleton 会与其他 CEF 进程(本 app 之前的实例、spike 等)冲突 ——
    // 实测表现为新实例启动后静默消失(CEF 自身也会警告 "Please customize
    // CefSettings.root_cache_path ... unintended process singleton behavior")。
    let (cache_path, root_cache_path) = cef_cache_paths();
    // no_sandbox:zap 走 Developer ID 直发,不进 MAS(spec 决策)。
    // external_message_pump:消息泵由 [`pump`] 驱动(zap 已有自己的事件循环)。
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        // OSR 是进程级能力:必须在 CefInitialize 之前置位,浏览器级 WindowInfo 再逐个开。
        // 反过来(libcef 的告警)会"降低性能或运行时报错",见 browser_host_create.cc。
        windowless_rendering_enabled: i32::from(render_mode() == RenderMode::Osr),
        cache_path: CefString::from(cache_path.as_str()),
        root_cache_path: CefString::from(root_cache_path.as_str()),
        ..Default::default()
    };
    // 注意:这里调用的是 cef::initialize(bindings 的 re-export);本模块自己的
    // 初始化入口叫 initialize_runtime,避免同名遮蔽。
    let ok = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ok != 1 {
        let _ = INIT_STATUS.set(format!("CefInitialize 返回 {ok}"));
        return false;
    }
    let _ = INIT_STATUS.set("ok".into());
    true
}

/// 初始化结果(供后续在 logger 就绪后打印:启动早期的日志会丢)。
static INIT_STATUS: OnceLock<String> = OnceLock::new();

/// 是否正在关闭(关掉后消息泵必须停:继续 `do_message_loop_work` 会触碰已释放的
/// CEF 状态)。
fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
}

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// CEF 上下文是否已初始化(`OnContextInitialized` 回调置位)。
static CONTEXT_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// CEF 有序关闭(幂等)。CEF 要求进程退出前调用 `CefShutdown`,否则 Chromium 的
/// 线程/atexit 清理可能崩溃或挂起(评审 F6)。未初始化时是 no-op。
pub(crate) fn shutdown() {
    if !INITIALIZED.get().copied().unwrap_or(false)
        || SHUTTING_DOWN.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    // 先逐个关闭浏览器并清空注册表,再 CefShutdown:CEF 文档明确"调用本函数后不得再
    // 调用任何 CEF 函数",而我们持有的 `Browser` 是引用计数对象 —— 若留到 thread_local
    // 析构时 drop,就会在关机后调用 release()(评审 N3)。
    let (osr_views, browsers): (Vec<std::rc::Rc<OsrState>>, Vec<Browser>) = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        let mut views = Vec::new();
        let mut browsers = Vec::new();
        for state in map.values_mut() {
            // 只**收集**,关闭同样放到借用之外:`close_browser` 的完成时机 CEF 文档说的是
            // "may complete either synchronously or asynchronously",若它同步回调
            // `on_before_close`(那里会 `borrow_mut`)就是借着重入 ⇒ panic ⇒ 在关机路径上
            // abort。与本文件"借用内只取指针/句柄,借用外再调外部"的既有模式一致。
            if let Some(browser) = state.browser.take() {
                browsers.push(browser);
            }
            // 自建视图必须在 CefShutdown 前拆掉(否则留下指向已销毁 CEF 内容的 layer);
            // 这里只**收集**,释放放到借用之外 —— 释放会让视图从响应者链上退下来,可能
            // 同步回调 focus(),那条路径会再借 WEBVIEWS。
            if let Some(osr) = state.osr.take() {
                views.push(osr);
            }
        }
        map.clear();
        (views, browsers)
    });
    for browser in browsers {
        close_and_detach(&browser);
    }
    for osr in osr_views {
        take_and_release_osr_view(osr);
    }
    log::info!("[cef] 关闭 CEF(CefShutdown)");
    cef::shutdown();
}

/// 确保 CEF 已初始化(幂等):用户在设置里打开"使用 Chromium 内核"后,无需重启
/// 即可在下一个 dsh pane 上生效。失败时返回 false,调用方回退 wry。
pub(crate) fn ensure_initialized() -> bool {
    if is_enabled() {
        return true;
    }
    if !initialize_runtime() {
        return false;
    }
    // CEF 的 OnContextInitialized 在 CefInitialize() 期间同步回调,故此处应已置位;
    // 未置位说明 CEF 行为变化(那时同栈建浏览器是危险的),显式记录以便排查。
    if !CONTEXT_INITIALIZED.load(std::sync::atomic::Ordering::SeqCst) {
        // 不 assert:CEF 行为若变化,应回退 wry 而不是把 debug 构建打挂(评审 N19)。
        log::warn!("[cef] CefInitialize 返回但 context 未标记初始化,回退 wry");
        return false;
    }
    install_app_protocol_support();
    start_pump();
    log::info!("[cef] 由设置项触发,已初始化 CEF 后端");
    true
}

/// 初始化结果描述("ok" 或失败原因)。
pub(crate) fn init_status() -> &'static str {
    INIT_STATUS.get().map(String::as_str).unwrap_or("未初始化")
}

/// 是否是 zap 的崩溃恢复 watchdog 子进程(以 `--crash-recovery-mechanism` 启动)。
///
/// 该子进程同样会走到 `warp::run()`;若不跳过 CEF 初始化,两个进程会共用缓存目录并
/// 触发 Chromium 的 ProcessSingleton 冲突(实测:探针日志里 `CefInitialize` 出现两次)。
/// 这里只认命令行标记,不依赖 `crash_recovery` 模块(它本身受 cfg 门控)。
pub(crate) fn is_crash_recovery_process() -> bool {
    std::env::args().any(|arg| arg.starts_with("--crash-recovery-mechanism"))
}

/// 运行时是否**请求**使用 CEF 后端:`ZAP_CEF_WEBVIEW` 环境开关(显式强制)或 feature
/// flag 任一为真。
///
/// 为什么要环境开关作为强制项:flag 会经 `USER_PREFERENCE_MAP`(settings 里的用户偏好)
/// 覆盖 —— 实测实例里 `ZAP_CEF_WEBVIEW=1` 已注入进程环境、二进制也含 CEF 代码,但
/// `FeatureFlag::CefWebview.is_enabled()` 仍为 false,导致 CEF 完全不初始化。
/// 开关只影响"是否尝试初始化",不改变默认(wry)路径。
pub(crate) fn is_requested() -> bool {
    let by_env = std::env::var_os("ZAP_CEF_WEBVIEW").is_some();
    let by_flag = FeatureFlag::CefWebview.is_enabled();
    if by_env && !by_flag {
        log::info!("[cef] 由 ZAP_CEF_WEBVIEW 环境开关强制启用(flag 未开启)");
    }
    by_env || by_flag
}

/// CEF 是否已成功初始化。为 false 时调用方(wry 路径)应回退,保证默认行为可用。
pub(crate) fn is_enabled() -> bool {
    INITIALIZED.get().copied().unwrap_or(false)
}

/// 驱动 CEF 消息泵。必须在主线程**稳定周期**调用(空闲也要),否则 CEF 内部
/// IPC/渲染任务不推进。
pub(crate) fn pump() {
    if is_shutting_down() {
        return;
    }
    if INITIALIZED.get().copied().unwrap_or(false) {
        do_message_loop_work();
        // 给受控实跑一个"泵确实在跑"的可观察信号。注意:**不能**在第一拍就记:那时
        // 文件 logger 可能还没就绪(实测首拍日志会丢),故延迟到第 60 拍(约 1s)。
        static PUMP_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let ticks = PUMP_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if ticks == 60 {
            log::info!("[cef] message pump active (60 ticks)");
        }
        // 每秒扫一次"隐藏超时"的页面并冻结(见 freeze_hidden_overdue)。
        if ticks % 60 == 0 {
            freeze_hidden_overdue();
        }
        // OSR 外部 begin-frame:CEF 只在宿主喂帧后才绘制(默认关闭,见 external_begin_frame)。
        if external_begin_frame() {
            drive_external_begin_frame();
        }
    }
}

/// 给所有 OSR 浏览器喂一帧(仅在启用外部 begin-frame 时有意义)。
fn drive_external_begin_frame() {
    WEBVIEWS.with(|map| {
        for state in map.borrow().values() {
            if state.osr.is_none() {
                continue;
            }
            if let Some(host) = state.browser.as_ref().and_then(|browser| browser.host()) {
                host.send_external_begin_frame();
            }
        }
    });
}

wrap_app! {
    struct CefApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(CefBrowserProcessHandler::new(RefCell::new(None)))
        }

        /// 引导脚本必须在**渲染进程的上下文创建时**注入(见
        /// [webview_init_js](../../browser/webview_init_js.rs) 的文件头注释):
        /// 它定义 `window.__restoreFocused`(切回页面/重载后恢复输入框 DOM 焦点)与
        /// 早期 IPC 队列 `window.__zapIpcQueue`(由 load_end 注入的真实 shim 回放)。
        ///
        /// 以前只在 load_end 注入 loopback shim,导致这个引导脚本在 CEF 模式下**从未存在**:
        /// ① 打开 pane 后不点页面直接打字没有反应(没有 DOM 焦点);
        /// ② 文档开始到 shim 就位之间的 zapRpc 永久丢失;
        /// ③ `warp:webview-focusin`/`warp:webview-mousedown` 不上报(地址栏与页面双光标);
        /// ④ `window.open` 外链拦截失效。
        /// 与 wry 一致:注入**所有** frame(`with_initialization_script_for_main_only(js, false)`)。
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(CefRenderProcessHandler::new())
        }
    }
}

wrap_render_process_handler! {
    struct CefRenderProcessHandler;

    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _context: Option<&mut V8Context>,
        ) {
            let Some(frame) = frame else {
                return;
            };
            let code = CefString::from(crate::browser::webview_init_js::WEBVIEW_INIT_JS);
            let url = CefString::from("zap://webview-init.js");
            // 与浏览器的 execute_java_script 不同:这里跑在渲染进程、文档脚本之前,
            // 脚本自带幂等守卫(window.webkit || {} 等),重复注入无副作用。
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
}

wrap_browser_process_handler! {
    struct CefBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            CONTEXT_INITIALIZED.store(true, std::sync::atomic::Ordering::SeqCst);
            // 阶段 1 下一步:在这里按 dsh pane 的请求创建子视图浏览器,
            // 并订阅加载完成/崩溃事件转发给 BrowserWebViewManager。
            log::info!("[cef] context initialized");
        }
    }
}

// ===================== dsh pane 的 CEF webview(阶段 1) =====================
//
// CEF 的 Browser/Frame 句柄只能在 UI 线程使用,故状态放在 thread_local;
// `BrowserWebViewManager` 只持有 id,CEF 分支按 id 调本模块(见 browser_web_view.rs)。

use std::collections::HashMap;

use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pathfinder_geometry::rect::RectF;

/// 每次创建浏览器递增的代际号:用于丢弃"迟到"的旧浏览器回调
/// (同一 id 会被 pane 重建复用,旧实例的 on_before_close 晚到会误清新句柄)。
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct CefWebview {
    browser: Option<Browser>,
    /// 当前句柄属于哪一代。
    generation: u64,
    /// AppKit frame 坐标(父视图内、底部原点)。
    rect: NSRect,
    visible: bool,
    init_js: String,
    /// 隐藏即销毁后重建所需:容器视图指针与当前 URL。
    parent_view: *mut c_void,
    url: String,
    /// 最近一次收到的逻辑坐标(挂起期间也更新;重建时据此换算)。
    pending_rect: Option<RectF>,
    /// 隐藏起始时刻(用于"隐藏超过 N 秒后冻结")。
    hidden_since: Option<std::time::Instant>,
    /// 页面是否已被冻结(CDP Page.setWebLifecycleState=frozen)。
    frozen: bool,
    /// 浏览器背景色(0xAARRGGBB,opaque)。重建时复用。
    background: u32,
    /// OSR 共享状态(仅 OSR 模式;windowed 下为 None)。宿主视图由本模块 +1 持有,
    /// 销毁时交给 `warp_cef_osr_view_release` 配平。
    osr: Option<std::rc::Rc<OsrState>>,
}

/// 隐藏多久后冻结页面。来源:`CefWebviewSettings::freeze_after_secs`(默认 300s,
/// 见 app/src/settings/cef_webview.rs),由 app 每帧推入;
/// `ZAP_CEF_FREEZE_AFTER_SECS` 可覆盖(dev 排查用)。0 = 不冻结。
static FREEZE_AFTER_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(300);

/// 由 app 推入设置值(每帧调用,代价是一次原子写)。
pub(crate) fn set_freeze_after_secs(secs: u32) {
    FREEZE_AFTER_SECS.store(u64::from(secs), std::sync::atomic::Ordering::Relaxed);
}

/// 当前生效的冻结阈值;`None` 表示不冻结(设置为 0)。
fn freeze_after() -> Option<std::time::Duration> {
    let secs = std::env::var("ZAP_CEF_FREEZE_AFTER_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| FREEZE_AFTER_SECS.load(std::sync::atomic::Ordering::Relaxed));
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// 向页面发一条 CDP 命令(冻结/解冻用)。
fn send_cdp(browser: &Browser, method: &str, params: &str) -> bool {
    let Some(host) = browser.host() else {
        return false;
    };
    let message = format!(r#"{{"id":1,"method":"{method}","params":{params}}}"#);
    let ok = host.send_dev_tools_message(Some(message.as_bytes())) == 1;
    if !ok {
        log::warn!("[cef] CDP {method} 下发失败(浏览器可能尚未就绪)");
    }
    ok
}

/// 隐藏超时后冻结页面(由 pump 周期调用)。
fn freeze_hidden_overdue() {
    let Some(threshold) = freeze_after() else {
        return; // 设置为 0:不冻结
    };
    // 先只挑出"确实有浏览器句柄"的候选,再在下发成功后置 frozen:否则 browser 尚未
    // 创建就被标记冻结,该实例此后永远不会被冻结(且解冻时会发多余的 active)。
    let candidates: Vec<(u64, Browser)> = WEBVIEWS.with(|map| {
        map.borrow()
            .iter()
            .filter(|(_, state)| {
                !state.visible
                    && !state.frozen
                    && state
                        .hidden_since
                        .is_some_and(|since| since.elapsed() >= threshold)
            })
            .filter_map(|(id, state)| state.browser.clone().map(|browser| (*id, browser)))
            .collect()
    });
    for (id, browser) in candidates {
        if send_cdp(&browser, "Page.setWebLifecycleState", r#"{"state":"frozen"}"#) {
            log::info!("[cef] webview {id}: 隐藏超时,已冻结页面(CDP setWebLifecycleState=frozen)");
            WEBVIEWS.with(|map| {
                if let Some(state) = map.borrow_mut().get_mut(&id) {
                    state.frozen = true;
                }
            });
        }
    }
}

/// 解冻(重新可见时调用)。
/// 解冻;返回是否真正下发成功(失败则保留 frozen,下一帧重试)。
fn thaw(browser: &Browser) -> bool {
    send_cdp(browser, "Page.setWebLifecycleState", r#"{"state":"active"}"#)
}

thread_local! {
    static WEBVIEWS: RefCell<HashMap<u64, CefWebview>> = RefCell::new(HashMap::new());
}

/// 该 id 是否由 CEF 承载(manager 据此区分后端)。
pub(crate) fn webview_exists(id: u64) -> bool {
    WEBVIEWS.with(|map| map.borrow().contains_key(&id))
}

/// 场景逻辑坐标(左上原点)→ 父视图 AppKit 坐标(底部原点)。
/// 翻转公式与 wry 的 `window_position` 对齐(`parentHeight - y - h`),见
/// TECH.md 集成要求 #2。
fn to_appkit_rect(parent_view: *mut c_void, rect: RectF) -> NSRect {
    let parent_height = unsafe { (*(parent_view as *const NSView)).frame().size.height };
    flip_rect_to_appkit(rect, parent_height)
}

/// 场景逻辑坐标(左上原点)→ AppKit frame 坐标(底部原点)的纯计算。
/// 公式与 wry 的 `window_position` 对齐(`parentHeight - y - h`),见集成要求 #2。
/// 抽成纯函数以便单测(窗口几何在测试环境里无法构造)。
fn flip_rect_to_appkit(rect: RectF, parent_height: f64) -> NSRect {
    NSRect {
        origin: NSPoint {
            x: rect.origin_x() as f64,
            y: parent_height - rect.origin_y() as f64 - rect.height() as f64,
        },
        size: NSSize {
            width: rect.width() as f64,
            height: rect.height() as f64,
        },
    }
}

fn cef_rect(ns_rect: NSRect) -> Rect {
    Rect {
        x: ns_rect.origin.x as i32,
        y: ns_rect.origin.y as i32,
        width: ns_rect.size.width as i32,
        height: ns_rect.size.height as i32,
    }
}

/// 创建 CEF 子视图 webview(挂在 `parent_view` 内,即 warpui 的 WebViewContainerView)。
/// `init_js` 在主 frame 加载完成后注入一次(回环 IPC shim)。
pub(crate) fn create_webview(
    id: u64,
    parent_view: *mut c_void,
    rect: RectF,
    url: &str,
    init_js: &str,
    background: u32,
) {
    let ns_rect = to_appkit_rect(parent_view, rect);
    // OSR:宿主视图必须**先于浏览器**存在 —— on_accelerated_paint 随时可能到来,
    // 且 view_rect/screen_info 要靠它读 backing scale(T4)。
    let osr = if render_mode() == RenderMode::Osr {
        install_osr_input_callbacks();
        let view = unsafe {
            warp_cef_osr_view_new(
                parent_view,
                ns_rect.origin.x,
                ns_rect.origin.y,
                ns_rect.size.width,
                ns_rect.size.height,
                id,
            )
        };
        Some(OsrState::new(
            view,
            osr_view_size(ns_rect.size.width, ns_rect.size.height),
            unsafe { warp_cef_osr_view_scale(view) },
        ))
    } else {
        None
    };
    // 诊断:父视图必须已在某个窗口内,否则 CEF 行为不可预期(排查"多出一个窗口")。
    {
        let parent = unsafe { &*(parent_view as *const NSView) };
        let parent_window = parent.window();
        log::info!(
            "[cef] webview {id} parent={:p} in_window={} parent_frame={:?} rect={:?} mode={:?}",
            parent_view,
            parent_window.is_some(),
            parent.frame(),
            ns_rect,
            render_mode()
        );
    }
    WEBVIEWS.with(|map| {
        map.borrow_mut().insert(
            id,
            CefWebview {
                browser: None,
                generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                rect: ns_rect,
                visible: true,
                init_js: init_js.to_owned(),
                parent_view,
                url: url.to_owned(),
                pending_rect: Some(rect),
                hidden_since: None,
                frozen: false,
                background,
                osr,
            },
        );
    });
    let generation = WEBVIEWS.with(|map| map.borrow().get(&id).map(|s| s.generation));
    if let Some(generation) = generation {
        spawn_browser(id, generation, parent_view, ns_rect, url, background);
    }
}

/// 创建 CEF 浏览器(首次创建与"隐藏后重现"共用)。
fn spawn_browser(
    id: u64,
    generation: u64,
    parent_view: *mut c_void,
    ns_rect: NSRect,
    url: &str,
    background: u32,
) {
    let mut window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..WindowInfo::default()
    };
    let settings = match render_mode() {
        RenderMode::Windowed => {
            window_info = window_info
                .set_as_child(parent_view as sys::cef_window_handle_t, &cef_rect(ns_rect));
            // **CEF 的 windowed 浏览器做不到真透明**:按 cef_types.h,浏览器背景 alpha 透明时
            // 会回退到 CefSettings.background_color,而那个再透明就退化成"不透明白"。wry 那套
            // (WKWebView 私有 KVC `drawsBackground`)在 CEF 里没有对应键 —— 真透明只有 windowless
            // (OSR)渲染一条路。所以这里显式填 zap 的工作区底色,避免白底。
            BrowserSettings {
                background_color: background,
                ..Default::default()
            }
        }
        RenderMode::Osr => {
            // windowless 三开关 + DIP bounds(OSR-PLAN.md §1)。
            window_info.windowless_rendering_enabled = 1;
            window_info.shared_texture_enabled = i32::from(!cpu_paint());
            window_info.external_begin_frame_enabled = i32::from(external_begin_frame());
            let (width, height) = osr_view_size(ns_rect.size.width, ns_rect.size.height);
            window_info.bounds = Rect {
                x: 0,
                y: 0,
                width,
                height,
            };
            // windowless 下 `background_color` 的 alpha=0 即"启用透明绘制"(cef_types.h:701-708):
            // 这正是走 OSR 的唯一理由 —— windowed 填什么都不透明。
            BrowserSettings {
                windowless_frame_rate: OSR_FRAME_RATE,
                background_color: 0,
                ..Default::default()
            }
        }
    };
    // render handler 需要 OSR 共享状态;此处**没有**持有 WEBVIEWS 借用(浏览器创建本身
    // 会同步回调 render handler,不能带借用进去)。
    let osr = WEBVIEWS.with(|map| map.borrow().get(&id).and_then(|state| state.osr.clone()));
    let mut client = WebviewClient::new(id, generation, osr);
    let url = CefString::from(url);
    let created = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url),
        Some(&settings),
        None,
        None,
    );
    if created != 1 {
        // 创建失败会留下 has_webview=true 但 browser=None 的 pane(不会重建、也无 wry
        // 回退),故移除条目让对方下次 attach 时重试(评审 N10)。
        log::error!("[cef] webview {id}: browser 创建失败(返回 {created}),移除条目以便重试");
        crate::browser::browser_web_view::notify_webview_create_failed(id);
    }
}

/// 把浏览器对应的原生视图从父视图里摘掉。
///
/// **必须做**:`do_close` 返回 1 表示"由客户端负责完成关闭",CEF 因此不会自己
/// 拆视图/发关闭通知;若我们只调 `close_browser` 而不管视图,浏览器会成为僵尸
/// ——实测表现为切到别的 tab 后底下仍显示着旧 webview,且重建时叠加第二层。
fn detach_view(browser: &Browser) {
    if render_mode() == RenderMode::Osr {
        // windowless 下 `GetWindowHandle()` 返回的是**父容器**
        // (browser_platform_delegate_osr_mac.mm:GetHostWindowHandle ⇒
        // window_info.parent_view),拿它 removeFromSuperview 会把 warpui 的
        // WebViewContainerView 整个摘掉。OSR 的原生视图是我们自建的,走
        // `release_osr_view`。
        return;
    }
    if let Some(host) = browser.host() {
        let view = host.window_handle() as *mut NSView;
        if !view.is_null() {
            unsafe { (*view).removeFromSuperview() };
        }
    }
}

/// 摘掉一个 OSR 宿主视图:**先置空共享指针,再释放 NSView**。
///
/// 顺序不能反:`close_browser` 是**异步**的(CEF 文档明确),关闭完成前 CEF 仍可能投递
/// `on_accelerated_paint`/`on_paint`,而 render handler 持有同一个 `OsrState` ——
/// 指针不清空就会对已释放的 NSView 发消息(use-after-free)。置空后所有回调都会
/// 早返回(null 检查),渲染/光标/几何安全降级。
fn take_and_release_osr_view(osr: std::rc::Rc<OsrState>) {
    let view = osr.view();
    osr.view.set(std::ptr::null_mut());
    if !view.is_null() {
        unsafe { warp_cef_osr_view_release(view) };
    }
}

/// 从注册表取出并释放 OSR 宿主视图(幂等:状态被 take 走后不会二次释放)。
fn release_osr_view(id: u64) {
    let osr = WEBVIEWS.with(|map| map.borrow_mut().get_mut(&id).and_then(|state| state.osr.take()));
    if let Some(osr) = osr {
        take_and_release_osr_view(osr);
    }
}

/// 关闭并拆除一个浏览器(隐藏即销毁 / 销毁 / 代际淘汰共用)。
fn close_and_detach(browser: &Browser) {
    detach_view(browser);
    if let Some(host) = browser.host() {
        host.close_browser(1);
    }
}

/// 取浏览器句柄执行操作;浏览器尚未创建(`on_after_created` 未到)时静默跳过。
fn with_browser<R>(id: u64, f: impl FnOnce(&Browser, &mut CefWebview) -> R) -> Option<R> {
    // 关机后不得再调用任何 CEF 函数(cef_app_capi.h 明示)。这里是最集中的入口,
    // 统一早返回,避免各调用点漏加门控(评审 N3)。
    if is_shutting_down() {
        return None;
    }
    WEBVIEWS.with(|map| {
        // **必须用 try_borrow_mut**:本模块的对外调用(ObjC/CEF)会**同步**回调进 Rust
        // —— 最典型的是 `set_hidden:` 让 first responder 让位 → `resignFirstResponder`
        // → focus 回调 → 又回到这里,而调用方此时正持着借用。用 `borrow_mut` 会 panic,
        // 而 panic 发生在 `extern "C"` 回调里会**直接 abort**(实机崩溃:切 tab 隐藏 pane)。
        // 借不到就说明是这种重入,跳过这次操作即可(调用方仍在完成它自己的语义)。
        let mut map = map.try_borrow_mut().ok()?;
        let state = map.get_mut(&id)?;
        let browser = state.browser.clone()?;
        Some(f(&browser, state))
    })
}

pub(crate) fn navigate(id: u64, url: &str) {
    WEBVIEWS.with(|map| {
        if let Some(state) = map.borrow_mut().get_mut(&id) {
            state.url = url.to_owned();
        }
    });
    with_browser(id, |browser, _| {
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&CefString::from(url)));
        }
    });
}

/// 当前 URL(主 frame 最近一次加载/导航后的地址)。
pub(crate) fn current_url(id: u64) -> Option<String> {
    WEBVIEWS.with(|map| map.borrow().get(&id).map(|state| state.url.clone()))
}

pub(crate) fn reload(id: u64) {
    with_browser(id, |browser, _| browser.reload());
}

pub(crate) fn evaluate(id: u64, js: &str) {
    with_browser(id, |browser, _| {
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&CefString::from(js)), None, 0);
        }
    });
}

pub(crate) fn focus(id: u64, focused: bool) {
    // OSR 下 CEF 的 SetFocus 不会把自建视图设为 first responder(native mac 委托只对
    // content native view 这么做,windowless 下它是 null)⇒ 键盘根本到不了页面。
    // **必须在借用之外调用**:makeFirstResponder 会同步回调 becomeFirstResponder,
    // 那条路径会再进 focus()(见 warp_cef_osr_view_focus 的 `_syncingFocus`)。
    let view = WEBVIEWS.with(|map| {
        // 同 with_browser:这是 AppKit 会同步回调进来的路径,借不到就跳过。
        let map = map.try_borrow().ok()?;
        map.get(&id)
            .and_then(|state| state.osr.as_ref().map(|osr| osr.view()))
    });
    let was_first_responder = view.is_some_and(|view| {
        !view.is_null() && unsafe { warp_cef_osr_view_is_first_responder(view) } == 1
    });
    if let Some(view) = view {
        if !view.is_null() {
            unsafe { warp_cef_osr_view_focus(view, i32::from(focused)) };
        }
    }
    // 焦点**首次**落到页面上时,按 T2 spike 实测通过的一组补一次 CEF 焦点同步
    // (`was_hidden(0)+set_focus(0)+set_focus(1)`,见 OSR-SPIKE-B.md 硬约束 2;
    // 只 set_focus(1) 不够,CEF 导航后会静默丢焦点)。**只在"之前不是 first responder"
    // 时做**:页面内每次点击也会走这里,若无条件来一遍 0→1,会产生多余的 blur/focus
    // 抖动,而页面菜单的 onBlur 会因此误收起。
    let first_responder = view.is_some_and(|view| {
        !view.is_null() && unsafe { warp_cef_osr_view_is_first_responder(view) } == 1
    });
    with_browser(id, |browser, _| {
        if let Some(host) = browser.host() {
            if focused && first_responder && !was_first_responder {
                host.was_hidden(0);
                host.set_focus(0);
                host.set_focus(1);
            } else {
                host.set_focus(i32::from(focused));
            }
        }
    });
}

/// 断言当前在 CEF UI 线程(= 主线程)。`WEBVIEWS` 是 thread_local:从别的线程访问
/// 会拿到另一个空 map 而静默 no-op(比 panic 更难查,评审 F18)。
fn debug_assert_ui_thread() {
    debug_assert_ne!(
        cef::currently_on(cef::ThreadId::UI),
        0,
        "CEF 后端的状态访问必须在 UI(主)线程"
    );
}

/// 可见性:**仅隐藏原生视图,不销毁浏览器**(2026-09-21 按用户要求取消"隐藏即销毁")。
///
/// 隐藏必须真的把 NSView 藏起来(`setHidden:`):CEF 的 `was_hidden` 不足以让视图从
/// 屏幕上消失 —— 隐藏没生效时,切到别的 tab 会在底下看到残留的 webview(实测)。
/// renderer 保活的好处:切回不需要重载,页面状态/输入内容不丢。
pub(crate) fn set_visible(id: u64, visible: bool) {
    debug_assert_ui_thread();
    let known = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        match map.get_mut(&id) {
            Some(state) => {
                state.visible = visible;
                true
            }
            None => false,
        }
    });
    if !known {
        return;
    }
    // 隐藏:记开始时刻(供 pump 判超时冻结);可见:清计时并解冻。
    let was_frozen = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        match map.get_mut(&id) {
            Some(state) => {
                if visible {
                    // 只清计时;`frozen` 是否清掉取决于解冻是否下发成功(见下)。
                    state.hidden_since = None;
                    state.frozen
                } else {
                    state.hidden_since.get_or_insert_with(std::time::Instant::now);
                    false
                }
            }
            None => false,
        }
    });
    // **借用内只取句柄/指针,所有对外调用(ObjC/CEF)放到借用之外** —— 这是本模块的硬纪律:
    // 例如 `setHidden:` 会让 first responder 让位,AppKit 随即同步回调我们的
    // `resignFirstResponder` → `focus()` → 又要借 WEBVIEWS;持借期间调它必然重入
    // (曾经用 `borrow_mut` 时直接 abort,见 OSR-T8 的崩溃报告)。
    let Some((browser, osr_view)) = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        let state = map.get_mut(&id)?;
        state.browser
            .clone()
            .map(|browser| (browser, state.osr.as_ref().map(|osr| osr.view())))
    }) else {
        return;
    };
    if visible && was_frozen {
        log::info!("[cef] webview {id}: 重新可见,解冻页面");
        // 解冻失败则保留 frozen:下一帧(仍不可见时的冻结扫描不会再碰它,
        // 但下次可见时会再次尝试)重试,避免"永久冻结"(评审 N6)。
        if thaw(&browser) {
            // 写回用**独立**的短借用(不与任何对外调用重叠)。
            WEBVIEWS.with(|map| {
                if let Some(state) = map.borrow_mut().get_mut(&id) {
                    state.frozen = false;
                }
            });
        }
    }
    if let Some(host) = browser.host() {
        match osr_view {
            None => {
                let view = host.window_handle() as *mut NSView;
                if !view.is_null() {
                    unsafe { (*view).setHidden(!visible) };
                }
            }
            Some(osr_view) => {
                // OSR:windowless 的 `window_handle()` 是父容器(见 detach_view),
                // 必须操作自建宿主视图;隐藏期间 CEF 不绘制,重新可见时主动请求一帧,
                // 否则切回来看到的是旧画面。
                unsafe { warp_cef_osr_view_set_hidden(osr_view, i32::from(!visible)) };
                if visible {
                    host.invalidate(PaintElementType::VIEW);
                }
            }
        }
        host.was_hidden(i32::from(!visible));
    }
}

/// 每帧布局驱动(集成要求 #2):CEF 子视图不跟随洞 rect,必须显式设 frame 并
/// 通知宿主 `was_resized()`。
pub(crate) fn set_bounds(id: u64, rect: RectF) {
    debug_assert_ui_thread();
    // 先把最新逻辑坐标记进状态:挂起(无浏览器)期间也要更新,否则"隐藏即销毁"
    // 重建时会用旧几何落位。
    WEBVIEWS.with(|map| {
        if let Some(state) = map.borrow_mut().get_mut(&id) {
            state.pending_rect = Some(rect);
        }
    });
    with_browser(id, |browser, state| {
        let Some(host) = browser.host() else {
            return;
        };
        let is_osr = state.osr.is_some();
        // 父视图高度:windowed 从 CEF 子视图的 superview 取;OSR 没有 CEF 子视图,
        // 用创建时记下的容器(同一个 WebViewContainerView)。
        let parent_height = if is_osr {
            unsafe { (*(state.parent_view as *const NSView)).frame().size.height }
        } else {
            let view = host.window_handle() as *mut NSView;
            if view.is_null() {
                return;
            }
            let Some(superview) = (unsafe { (*view).superview() }) else {
                return;
            };
            superview.frame().size.height
        };
        let ns_rect = flip_rect_to_appkit(rect, parent_height);
        // **跨屏检查必须在几何早返回之前**:窗口在 1x/2x 屏之间拖动时逻辑几何不变,
        // 若跟着几何一起早返回,backing scale 变化就永远检测不到,CEF 会一直按旧 DPI
        // 出图(糊/浪费)。这里只读一次窗口的 backing scale,不产生 CEF 调用。
        let scale_changed = match state.osr.as_ref() {
            Some(osr) => {
                let view = osr.view();
                if view.is_null() {
                    false
                } else {
                    let scale = unsafe { warp_cef_osr_view_scale(view) };
                    if (scale - osr.scale.get()).abs() > f64::EPSILON {
                        osr.scale.set(scale);
                        true
                    } else {
                        false
                    }
                }
            }
            None => false,
        };
        // 调用点每帧都会上报几何;wry 分支同样"先比较再下发"。每帧无条件
        // setFrame + was_resized 会让 Chromium 做无意义重排(评审 F8)。
        if ns_rect == state.rect && !scale_changed {
            return;
        }
        state.rect = ns_rect;
        match state.osr.as_ref() {
            Some(osr) => {
                unsafe {
                    warp_cef_osr_view_set_frame(
                        osr.view(),
                        ns_rect.origin.x,
                        ns_rect.origin.y,
                        ns_rect.size.width,
                        ns_rect.size.height,
                    )
                };
                // 先更新共享几何再通知 CEF:was_resized 会**同步**回调
                // GetViewRect/GetScreenInfo,那时读到的必须是新尺寸。
                osr.size
                    .set(osr_view_size(ns_rect.size.width, ns_rect.size.height));
                host.was_resized();
                // 跨屏(backing scale 变化)时补一次屏幕信息,否则沿用旧 DPI 渲染会糊。
                // scale 已在函数开头写好(见那里的注释:必须在几何早返回之前检测)。
                if scale_changed {
                    host.notify_screen_info_changed();
                }
            }
            None => {
                let view = host.window_handle() as *mut NSView;
                unsafe { (*view).setFrame(ns_rect) };
                host.was_resized();
            }
        }
    });
}

/// 销毁 webview。CEF 的 `on_before_close` 会再清一次(幂等)。
pub(crate) fn destroy(id: u64) {
    debug_assert_ui_thread();
    let browser = WEBVIEWS.with(|map| map.borrow().get(&id).and_then(|state| state.browser.clone()));
    if let Some(browser) = browser {
        close_and_detach(&browser);
    }
    // OSR 宿主视图由我们持有:先释放(幂等),再从注册表移除。
    release_osr_view(id);
    WEBVIEWS.with(|map| {
        map.borrow_mut().remove(&id);
    });
}

/// `on_after_created` 后把几何/可见性落到刚创建的浏览器上。
fn apply_geometry(id: u64) {
    let pending = WEBVIEWS.with(|map| {
        map.borrow().get(&id).map(|state| (state.rect, state.pending_rect))
    });
    let Some((stored_rect, pending_rect)) = pending else {
        return;
    };
    // 有更新的逻辑坐标就以它为准:创建用的 rect 常是 RectF::default(),真实几何由
    // 首帧 set_bounds 给出(那时浏览器可能还没建好)。
    let rect = match pending_rect {
        Some(logical) => {
            let parent = WEBVIEWS.with(|map| map.borrow().get(&id).map(|state| state.parent_view));
            match parent {
                Some(parent_view) => flip_rect_to_appkit(
                    logical,
                    unsafe { (*(parent_view as *const NSView)).frame().size.height },
                ),
                None => stored_rect,
            }
        }
        None => stored_rect,
    };
    with_browser(id, |browser, state| {
        if let Some(host) = browser.host() {
            match state.osr.as_ref() {
                None => {
                    let view = host.window_handle() as *mut NSView;
                    if !view.is_null() {
                        unsafe { (*view).setFrame(rect) };
                    }
                }
                Some(osr) => {
                    unsafe {
                        warp_cef_osr_view_set_frame(
                            osr.view(),
                            rect.origin.x,
                            rect.origin.y,
                            rect.size.width,
                            rect.size.height,
                        )
                    };
                    osr.size.set(osr_view_size(rect.size.width, rect.size.height));
                }
            }
            host.was_hidden(i32::from(!state.visible));
            host.was_resized();
        }
    });
}

wrap_client! {
    struct WebviewClient {
        id: u64,
        generation: u64,
        // OSR 共享状态(仅 OSR 模式):render handler 靠它读视图指针/几何,不碰 WEBVIEWS。
        // (宏内字段不支持 /// 文档注释,故用行注释。)
        osr: Option<std::rc::Rc<OsrState>>,
    }

    impl Client {
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(WebviewContextMenu::new(self.id))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(WebviewRequest::new(self.id, self.generation))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(WebviewLifeSpan::new(self.id, self.generation))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(WebviewLoad::new(self.id, self.generation))
        }

        /// OSR 必须提供 RenderHandler:libcef 在创建 windowless 浏览器时**硬性要求**
        /// 它非空(browser_host_create.cc: "Windowless rendering requires a
        /// CefRenderHandler implementation"),否则创建直接失败。windowed 模式不提供
        /// (CEF 也不会调用)。
        fn render_handler(&self) -> Option<RenderHandler> {
            self.osr
                .clone()
                .map(|osr| WebviewRenderHandler::new(osr))
        }

        /// OSR 下 CEF 不会自己设 NSCursor,必须由宿主转发(见 on_cursor_change)。
        fn display_handler(&self) -> Option<DisplayHandler> {
            self.osr
                .clone()
                .map(|osr| WebviewDisplayHandler::new(osr))
        }

        /// 认领"页面未消费"的按键,阻止 CEF 把它递给应用菜单(见 WebviewKeyboardHandler)。
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            self.osr.clone().map(|_| WebviewKeyboardHandler::new())
        }
    }
}

wrap_keyboard_handler! {
    struct WebviewKeyboardHandler;

    impl KeyboardHandler {
        /// 返回 1 = 客户端已处理,**不再走 CEF 的平台回退**。
        ///
        /// 为什么必须认领(源码链路,实测踩到"在页面里打 f 会把 zap 窗口切全屏"):
        /// 1. 页面没有 `preventDefault()` 的按键,渲染器会回报"未处理";
        /// 2. `AlloyBrowserHostImpl::HandleKeyboardEvent` 先问客户端
        ///    `CefKeyboardHandler::OnKeyEvent`,没人认领就调
        ///    `platform_delegate_->HandleKeyboardEvent`;
        /// 3. mac 的 OSR 委托实现是
        ///    `CefBrowserPlatformDelegateNativeMac::HandleKeyboardEvent`:
        ///    `[[NSApp mainMenu] performKeyEquivalent:合成 NSEvent]`;
        /// 4. 而 zap 的窗口菜单经 `NSApplication::setWindowsMenu:` 让 AppKit 自动加了
        ///    "Enter Full Screen" 项,它的 keyEquivalent 就是**裸 `f`**(Fn+F,掩码是
        ///    `NSEventModifierFlagFunction`)—— 于是页面里的 `f` 命中了菜单的全屏项。
        ///
        /// 这些键我们已经转发给渲染器了(见 send_osr_key),页面才是它们的主人;
        /// AppKit 的快捷键(含 Cmd+Ctrl+F 全屏、Cmd+T 新标签)在**窗口的
        /// key equivalent 阶段**就已经被 zap 处理,根本到不了这里,故不受影响。
        fn on_key_event(
            &self,
            _browser: Option<&mut Browser>,
            _event: Option<&KeyEvent>,
            _os_event: *mut u8,
        ) -> i32 {
            1
        }
    }
}

wrap_display_handler! {
    struct WebviewDisplayHandler {
        osr: std::rc::Rc<OsrState>,
    }

    impl DisplayHandler {
        /// 页面请求光标形状(链接/文本/缩放…)。返回 1 = 客户端已处理。
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: *mut u8,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> i32 {
            let view = self.osr.view();
            if view.is_null() {
                return 0;
            }
            unsafe { warp_cef_osr_view_set_cursor(view, cursor_semantic(type_)) };
            1
        }
    }
}

wrap_render_handler! {
    struct WebviewRenderHandler {
        osr: std::rc::Rc<OsrState>,
    }

    impl RenderHandler {
        /// OSR 视图尺寸:**DIP(逻辑点)**,不是像素 —— CEF 自己会乘
        /// `device_scale_factor`(见 osr_view_size 的注释)。
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let Some(rect) = rect else {
                return;
            };
            let (width, height) = self.view_size_dip();
            rect.x = 0;
            rect.y = 0;
            rect.width = width;
            rect.height = height;
        }

        /// 屏幕信息:retina 缩放、色深、可用区(同样全部是 DIP)。
        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> i32 {
            let Some(info) = screen_info else {
                return 0;
            };
            let (width, height) = self.view_size_dip();
            info.device_scale_factor = self.scale() as f32;
            info.depth = 32;
            info.depth_per_component = 8;
            info.is_monochrome = 0;
            info.rect = Rect {
                x: 0,
                y: 0,
                width,
                height,
            };
            info.available_rect = info.rect.clone();
            1
        }

        /// 加速绘制:macOS 上给的是 IOSurface 指针,直接贴进自建视图的
        /// CALayer.contents(零拷贝;句柄每帧会变,故每帧都贴)。
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else {
                return;
            };
            let view = self.osr_view();
            if type_ == PaintElementType::POPUP {
                // 页面内弹层(`<select>` 等)画到独立弹层,尺寸由 on_popup_size 定。
                unsafe { warp_cef_osr_view_set_popup_surface(view, info.shared_texture_io_surface) };
                return;
            }
            unsafe { warp_cef_osr_view_set_surface(view, info.shared_texture_io_surface) };
            self.log_surface_size_once(view);
        }

        /// view DIP → 屏幕坐标。CEF 的每一次鼠标事件翻译(`TranslateWebMouseEvent`)
        /// 都要用它填 `screenX/screenY`,拖动/DevTools 等原生 UI 也需要,故必须实现
        /// (默认返回 false)。mac 交给 AppKit 换算:屏幕坐标即 DIP、原点在左下
        /// —— 与 CEF 自家 mac 实现一致(`menu_runner_mac.mm` 把它直接交给
        /// `popUpMenuPositioningItem:…inView:nil`,AppKit 要的就是左下原点)。
        /// **注意**:它**不是** OSR 右键菜单之前不出现的原因(那是 `WindowInfo.parent_view`
        /// 未设 ⇒ `GetWindowHandle()`=0 ⇒ `CefMenuRunnerMac` 拒绝,见 cef_support.m)。
        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: i32,
            view_y: i32,
            screen_x: Option<&mut i32>,
            screen_y: Option<&mut i32>,
        ) -> i32 {
            let (Some(screen_x), Some(screen_y)) = (screen_x, screen_y) else {
                return 0;
            };
            let view = self.osr_view();
            if view.is_null() {
                return 0;
            }
            let mut x = 0.0_f64;
            let mut y = 0.0_f64;
            let ok = unsafe {
                warp_cef_osr_view_screen_point(
                    view,
                    f64::from(view_x),
                    f64::from(view_y),
                    &mut x,
                    &mut y,
                )
            };
            if ok == 0 {
                return 0;
            }
            // 鼠标移动/点击/滚轮每条事件都会调到这里,按 trace 记(与 send_mouse_move 同口径)。
            log::trace!("[cef] screen_point ({view_x},{view_y}) → ({x},{y})");
            *screen_x = x as i32;
            *screen_y = y as i32;
            1
        }

        /// 页面内弹层(`<select>` 下拉等)显隐。
        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: i32) {
            let view = self.osr_view();
            if view.is_null() {
                return;
            }
            log::debug!("[cef] on_popup_show show={show}");
            unsafe { warp_cef_osr_view_show_popup(view, show) };
        }

        /// 页面内弹层的位置与尺寸(**DIP、左上原点**,与 view_rect 同口径)。
        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let view = self.osr_view();
            let Some(rect) = rect else {
                return;
            };
            if view.is_null() {
                return;
            }
            log::debug!(
                "[cef] on_popup_size ({},{}) {}x{}",
                rect.x,
                rect.y,
                rect.width,
                rect.height
            );
            unsafe {
                warp_cef_osr_view_set_popup_rect(
                    view,
                    f64::from(rect.x),
                    f64::from(rect.y),
                    f64::from(rect.width),
                    f64::from(rect.height),
                )
            };
        }

        /// 输入法:CEF 回报 composition 的选区与**逐字位置**(DIP、左上原点)。
        /// 宿主视图据此给候选框定位(`firstRectForCharacterRange:`)。
        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            let view = self.osr_view();
            if view.is_null() {
                return;
            }
            let selected = selected_range.map(|range| (range.from, range.to));
            let first = character_bounds.and_then(|bounds| bounds.first()).cloned();
            let (sel_from, sel_to) = selected
                .map(|(from, to)| (from as i32, to as i32))
                .unwrap_or((-1, -1));
            let (x, y, w, h, has_bounds) = match &first {
                Some(rect) => (
                    f64::from(rect.x),
                    f64::from(rect.y),
                    f64::from(rect.width),
                    f64::from(rect.height),
                    1,
                ),
                None => (0.0, 0.0, 0.0, 0.0, 0),
            };
            log::debug!(
                "[cef] on_ime_composition_range_changed sel={selected:?} bounds={} {:?}",
                character_bounds.map(<[Rect]>::len).unwrap_or(0),
                first
            );
            unsafe { warp_cef_osr_view_set_ime_bounds(view, sel_from, sel_to, x, y, w, h, has_bounds) };
        }

        /// CPU 兜底:共享纹理不可用(如 GPU 进程异常、或 ZAP_CEF_OSR_CPU_PAINT=1)
        /// 时 CEF 走这条路径,把 BGRA 位图交给宿主。
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: i32,
            height: i32,
        ) {
            // CEF 的 on_paint 缓冲区是紧凑的 BGRA(width*4 字节/行)。
            let view = self.osr_view();
            if type_ == PaintElementType::POPUP {
                unsafe {
                    warp_cef_osr_view_set_popup_bitmap(view, buffer, width, height, width * 4)
                };
                return;
            }
            unsafe { warp_cef_osr_view_set_bitmap(view, buffer, width, height, width * 4) };
        }
    }
}

impl WebviewRenderHandler {
    /// 宿主视图指针(CEF 可能同步回调,故只读共享单元,不碰 WEBVIEWS)。
    fn osr_view(&self) -> *mut c_void {
        self.osr.view()
    }

    /// 视图尺寸(DIP)。每帧 `set_bounds` 在通知 CEF **之前**写入,故 CEF 同步回调
    /// GetViewRect/GetScreenInfo 时读到的一定是最新值。
    fn view_size_dip(&self) -> (i32, i32) {
        self.osr.size.get()
    }

    /// 当前 backing scale(2.0 = retina;1.0 = 普通屏)。
    fn scale(&self) -> f64 {
        self.osr.scale.get()
    }

    /// surface 尺寸变化时记一次日志(首帧也会记):这是"retina 无模糊 / 没有重复缩放"
    /// 的硬证据 —— surface 应等于 `view_rect`(DIP)× device_scale_factor。
    fn log_surface_size_once(&self, view: *mut c_void) {
        if view.is_null() {
            return;
        }
        let (mut width, mut height) = (0i32, 0i32);
        unsafe { warp_cef_osr_view_surface_size(view, &mut width, &mut height) };
        if width <= 0 || height <= 0 {
            return;
        }
        if self.osr.surface.get() == (width, height) {
            return;
        }
        self.osr.surface.set((width, height));
        log::info!(
            "[cef] OSR surface {width}x{height}px (view_rect={:?} DIP, scale={:.1})",
            self.view_size_dip(),
            self.scale()
        );
    }
}

/// 上下文菜单自定义命令 id(CEF 约定用户 id 从 26500 起)。
const MENU_ID_RELOAD: i32 = 26500;
const MENU_ID_INSPECT_ELEMENT: i32 = 26501;

wrap_context_menu_handler! {
    struct WebviewContextMenu {
        id: u64,
    }

    impl ContextMenuHandler {
        /// 精简 CEF 默认菜单:清空后只放"重新加载"与"检查元素"。
        ///
        /// 实测 CEF(Alloy)默认项只有 Back/Forward/分隔符/Print…/View Page Source
        /// ——**没有**"自动填充",故无须按标题保留(标题匹配还会随语言失效,评审 F11)。
        /// 代价:右键菜单里不再有复制/粘贴,但 Cmd+C/V 等快捷键不受影响。
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else {
                return;
            };
            let removed = model.count();
            let labels: Vec<String> = (0..removed)
                .map(|index| CefString::from(&model.label_at(index)).to_string())
                .collect();
            model.clear();
            log::debug!("[cef] 右键菜单:清空 {removed} 个默认项 {labels:?}");
            model.add_item(MENU_ID_RELOAD, Some(&CefString::from("重新加载")));
            model.add_item(
                MENU_ID_INSPECT_ELEMENT,
                Some(&CefString::from("检查元素")),
            );
        }

        /// 处理"检查元素":打开 DevTools 并定位到右键点中的元素。
        /// 返回 1 = 已处理;返回 0 = 交回 CEF 处理默认命令。
        fn on_context_menu_command(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            command_id: i32,
            _event_flags: EventFlags,
        ) -> i32 {
            let Some(browser) = browser else {
                return 1;
            };
            if command_id == MENU_ID_RELOAD {
                log::info!("[cef] webview {} 右键重新加载", self.id);
                browser.reload();
                return 1;
            }
            if command_id != MENU_ID_INSPECT_ELEMENT {
                return 0;
            }
            let inspect_at = params.map(|params| Point {
                x: params.xcoord(),
                y: params.ycoord(),
            });
            if let Some(host) = browser.host() {
                log::info!("[cef] webview {} 右键检查元素", self.id);
                host.show_dev_tools(None, None, None, inspect_at.as_ref());
            }
            1
        }
    }
}

wrap_life_span_handler! {
    struct WebviewLifeSpan {
        id: u64,
        generation: u64,
    }

    impl LifeSpanHandler {
        /// 子视图浏览器的关闭契约:CEF 在 windowed 模式下的默认行为是向 browser 的
        /// **顶层父窗口**发标准关闭通知(macOS 即 `performClose:`),而我们的顶层父窗口
        /// 就是 Zap 主窗 —— 那会误关主窗。这里显式接管(true = 由我们完成关闭),
        /// 实际销毁由 `close_browser(1)` + `on_before_close` 里的视图移除完成。
        fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser.cloned() else {
                return;
            };
            let accepted = WEBVIEWS.with(|map| {
                let mut map = map.borrow_mut();
                match map.get_mut(&self.id) {
                    // 只接受当前代际:被取代的旧实例直接关掉。
                    Some(state) if state.generation == self.generation => {
                        state.browser = Some(browser.clone());
                        true
                    }
                    _ => false,
                }
            });
            if !accepted {
                close_and_detach(&browser);
                log::info!(
                    "[cef] webview {} 丢弃过期代际 {} 的浏览器",
                    self.id,
                    self.generation
                );
                return;
            }
            apply_geometry(self.id);
            log::info!("[cef] webview {} browser created", self.id);
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            // 防御性移除原生视图:CEF 的 child-view 归属由宿主负责,残留的
            // CefBrowserHostView 会在洞被别的 pane 复用时透出旧页面。
            // (OSR 下 window_handle() 是父容器,不能碰 —— 见 detach_view。)
            if let Some(browser) = browser {
                detach_view(browser);
            }
            // 只清句柄,不删状态:"隐藏即销毁"下同一 id 之后还会重建;
            // **只清当前代际**(否则迟到的旧回调会清掉新句柄/新视图)。
            // 顺带把 OSR 宿主视图一起摘掉:它属于这个浏览器实例,页面自行关闭
            // (window.close() 等)后不该继续显示最后一帧 —— windowed 路径在
            // detach_view 里已经这么做,两种模式保持一致。
            let osr = WEBVIEWS.with(|map| {
                let mut map = map.borrow_mut();
                match map.get_mut(&self.id) {
                    Some(state) if state.generation == self.generation => {
                        state.browser = None;
                        state.osr.take()
                    }
                    _ => None,
                }
            });
            // 借用外释放:不把 `WEBVIEWS` 借用跨过对外(ObjC/CEF)调用。
            if let Some(osr) = osr {
                take_and_release_osr_view(osr);
            }
            log::info!("[cef] webview {} closed (gen {})", self.id, self.generation);
        }
    }
}

wrap_request_handler! {
    struct WebviewRequest {
        id: u64,
        generation: u64,
    }

    impl RequestHandler {
        /// 渲染进程崩溃/被杀:与 wry 的 terminate handler 走**同一条**事件路径
        /// (pane 弹崩溃态 + 重新加载入口),否则 CEF pane 会停在死页且不再重建(评审 F7)。
        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: i32,
            _error_string: Option<&CefString>,
        ) {
            // 代际过滤:旧实例的终止事件不得影响新实例。
            let current = WEBVIEWS.with(|map| {
                map.borrow()
                    .get(&self.id)
                    .map(|state| state.generation)
            });
            if current != Some(self.generation) {
                return;
            }
            log::error!("[cef] webview {} renderer 终止 status={status:?} code={error_code}", self.id);
            crate::browser::browser_web_view::notify_webview_crashed(self.id);
        }
    }
}

wrap_load_handler! {
    struct WebviewLoad {
        id: u64,
        generation: u64,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: i32,
        ) {
            // 顺序敏感:先取 URL 与主 frame 判定(is_main 会移动 frame),再做后续操作。
            let loaded_url = frame
                .as_ref()
                .map(|frame| CefString::from(&frame.url()).to_string())
                .filter(|url| !url.is_empty());
            let is_main = frame
                .as_ref()
                .map(|frame| frame.is_main() != 0)
                .unwrap_or(true);
            if !is_main {
                return;
            }

            // 记录主 frame 的真实 URL(dsh 首载是 ?token=…,303 后变裸地址;pane 的
            // origin 比较与工具栏同步都依赖它)。
            WEBVIEWS.with(|map| {
                let mut map = map.borrow_mut();
                if let Some(state) = map.get_mut(&self.id) {
                    if state.generation == self.generation {
                        if let Some(url) = loaded_url {
                            state.url = url;
                        }
                    }
                }
            });

            // **每次**主 frame 加载完成都注入 shim:同一 browser 内的第二次文档
            // (reload / 服务端重定向后的硬跳转 / location.replace)是全新 DOM,
            // 一次性注入会让 IPC 在第二次加载后静默失效;shim 自带
            // `if (window.__ZAP_LOOPBACK_IPC__) return;` 幂等守卫,重复注入无副作用。
            let script = WEBVIEWS.with(|map| {
                let map = map.borrow();
                match map.get(&self.id) {
                    // 代际过滤:旧实例的 load_end 不得影响新实例。
                    Some(state) if state.generation == self.generation => {
                        Some(state.init_js.clone())
                    }
                    _ => None,
                }
            });
            if let Some(script) = script {
                log::info!("[cef] webview {} loaded (shim 注入)", self.id);
                evaluate(self.id, &script);

                // **关键**:wry 路径靠 `on_page_load_handler` 发 UrlChanged 事件,pane 据此
                // 结束"启动中"覆盖层;CEF 侧没有该回调,必须自己回传,否则页面已渲染但
                // 面板一直显示"启动中"(只能等 15s 超时兜底)——实测踩到。
                // 放在上面的代际判定内:旧实例的 load_end 不得影响新实例(评审 F9)。
                crate::browser::browser_web_view::notify_webview_page_loaded(self.id);

            }

            // CEF 导航后会**静默丢掉焦点**(chromiumembedded/cef#3870):只 set_focus(1)
            // 不够 —— T2 spike 实测通过的是 `was_hidden(0)+set_focus(0)+set_focus(1)`
            // 这一组(见 OSR-SPIKE-B.md 硬约束 2),否则页面里的光标与 IME 全部失效。
            // 仅在"本视图仍是 first responder"(输入本来就在页面上)时补,免得把用户
            // 在终端里的焦点抢走;pane 侧另有 focus_webview_restoring_input 负责 DOM 焦点。
            let osr_view = WEBVIEWS.with(|map| {
                let map = map.borrow();
                match map.get(&self.id) {
                    Some(state) if state.generation == self.generation && state.visible => {
                        state.osr.as_ref().map(|osr| osr.view())
                    }
                    _ => None,
                }
            });
            if let Some(view) = osr_view {
                if !view.is_null() && unsafe { warp_cef_osr_view_is_first_responder(view) } == 1 {
                    with_browser(self.id, |browser, _| {
                        if let Some(host) = browser.host() {
                            host.was_hidden(0);
                            host.set_focus(0);
                            host.set_focus(1);
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "cef_backend_tests.rs"]
mod tests;
