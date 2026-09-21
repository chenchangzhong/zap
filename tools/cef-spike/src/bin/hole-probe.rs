//! 阶段 1 第一生死项:CEF 子视图嵌入 zap 式挖洞/分层窗口
//! (specs/cef-webview-minimal/TECH.md,探针实现见 probes/hole_probe.m)
//!
//! 用法:
//!   hole-probe --url=<url> [--routing=plain|zap] [--hole=x,y,w,h]
//!              [--click-after=6] [--exit-after=12]
//!
//! 对照实验:--routing=plain 走平台默认事件分发;--routing=zap 复刻
//! WarpWindow::sendEvent 的 macOS 27 分流(含 WKWebView 类名特判)。

#[path = "../mac/mod.rs"]
mod mac;
#[path = "../shared/mod.rs"]
pub mod shared;

use cef::*;
use std::cell::RefCell;
use std::ffi::c_void;

use shared::simple_handler::{SimpleHandler, SimpleHandlerClient};

// 链接 build.rs 用 cc 编译出的 ObjC 宿主探针静态库。
// 注:本 crate 的 build script `rustc-link-lib` 未进入 bin 链接行(实测),
// 故在源码侧用 #[link] 显式声明;search path 仍由 build script 的 -L 提供。
#[link(name = "hole_probe_host", kind = "static")]
extern "C" {
    fn probe_create_window(hx: f64, hy: f64, hw: f64, hh: f64);
    fn probe_container_ptr() -> *mut c_void;
    fn probe_set_routing(mode: i32);
    fn probe_schedule_click(delay: f64);
    fn probe_schedule_exit(delay: f64);
    fn probe_window_number() -> i64;
    fn probe_redirect_stdout(path: *const std::ffi::c_char);
    fn probe_schedule_resize(delay: f64, dw: f64, dh: f64);
}

/// 洞的位置与目标 URL:on_context_initialized 在 CEF 回调里取用,故用全局量传递。
struct ProbeConfig {
    url: String,
    hole: (f64, f64, f64, f64),
}
static CONFIG: std::sync::OnceLock<ProbeConfig> = std::sync::OnceLock::new();

wrap_app! {
    struct ProbeApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(ProbeBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct ProbeBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            // Alloy 运行时的浏览器才能挂在自备 NSView 上(Views/Chrome 风格自建顶层窗口)。
            let client = SimpleHandlerClient::new(SimpleHandler::new(true));
            let mut client = Some(client);
            *self.client.borrow_mut() = client.clone();

            let config = CONFIG.get().expect("config not set");
            let (hx, hy, hw, hh) = config.hole;
            let bounds = Rect {
                x: hx as i32,
                y: hy as i32,
                width: hw as i32,
                height: hh as i32,
            };
            let parent = unsafe { probe_container_ptr() } as cef::sys::cef_window_handle_t;
            let window_info = WindowInfo {
                runtime_style: RuntimeStyle::ALLOY,
                ..WindowInfo::default().set_as_child(parent, &bounds)
            };
            // 透明实验:浏览器背景 alpha 透明(0)。CEF 文档称 windowed 下会退化成白底,
            // 但视图层若被清成非不透明则可能透出下层 —— 本实验就是验证这一点。
            let settings = BrowserSettings {
                background_color: if transparent_probe() {
                    0
                } else {
                    BrowserSettings::default().background_color
                },
                ..Default::default()
            };
            let url = CefString::from(config.url.as_str());
            browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
            );
            println!(
                "[probe] browser created as child of {:p} rect=({hx},{hy},{hw},{hh})",
                parent
            );
        }
    }
}

/// 透明实验开关:环境变量或落文件标记(`open` 启动不经环境变量)。
fn transparent_probe() -> bool {
    std::env::var_os("PROBE_TRANSPARENT").is_some()
        || std::path::Path::new("/tmp/probe_transparent").exists()
}

fn arg_value(name: &str) -> Option<String> {
    std::env::args()
        .find_map(|a| a.strip_prefix(&format!("{name}=")).map(ToString::to_string))
}

fn main() {
    let url = arg_value("--url").unwrap_or_else(|| "file:///tmp/hole/index.html".to_string());
    let routing = arg_value("--routing").unwrap_or_else(|| "plain".to_string());
    let hole = arg_value("--hole").unwrap_or_else(|| "100,100,600,400".to_string());
    let click_after: f64 = arg_value("--click-after")
        .and_then(|v| v.parse().ok())
        .unwrap_or(6.0);
    let exit_after: f64 = arg_value("--exit-after")
        .and_then(|v| v.parse().ok())
        .unwrap_or(12.0);
    let parts: Vec<f64> = hole.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let (hx, hy, hw, hh) = match parts.as_slice() {
        [x, y, w, h] => (*x, *y, *w, *h),
        _ => panic!("--hole 需为 x,y,w,h"),
    };
    CONFIG
        .set(ProbeConfig {
            url,
            hole: (hx, hy, hw, hh),
        })
        .ok();

    if let Some(log) = arg_value("--log") {
        let c = std::ffi::CString::new(log).expect("log path");
        unsafe { probe_redirect_stdout(c.as_ptr()) };
    }

    // 顺序约束:load_cef 会断言 NSApp 之前未被触碰(cef-rs mac 模块),故必须最先执行。
    let _library = shared::load_cef();
    unsafe {
        probe_create_window(hx, hy, hw, hh);
        let mode = match routing.as_str() {
            "plain" => 0,
            "zap" => 1,
            "embedded" => 2,
            other => panic!("--routing 需为 plain|zap|embedded,收到 {other}"),
        };
        probe_set_routing(mode);
    }
    println!("[probe] WINDOW_NUMBER={}", unsafe { probe_window_number() });

    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        panic!("Failed to parse command line arguments");
    };
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;
    let ret = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if is_browser_process {
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        return;
    }

    let mut app = ProbeApp::new();
    // 探针不启用 CEF sandbox(与 zap 的 Developer ID 分发形态一致,spec 决策)。
    let settings = Settings {
        no_sandbox: 1,
        background_color: if transparent_probe() {
            0
        } else {
            Settings::default().background_color
        },
        ..Default::default()
    };
    assert_eq!(
        initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        ),
        1
    );

    let resize_test = std::env::args().any(|a| a == "--resize-test");
    unsafe {
        if resize_test {
            probe_schedule_resize((click_after - 2.0).max(0.5), 240.0, 160.0);
        }
        probe_schedule_click(click_after);
        probe_schedule_exit(exit_after);
    }
    run_message_loop();
    shutdown();
}
