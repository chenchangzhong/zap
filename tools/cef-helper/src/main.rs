//! CEF 子进程 helper(Chromium 的 GPU/Renderer/Plugin/Utility 进程)。
//!
//! 为什么不复用 zap 主二进制:
//! 1. **体积**:CEF 需要 5 个 helper.app(`Helper`/`Helper (GPU)`/`Helper (Renderer)`/
//!    `Helper (Plugin)`/`Helper (Alerts)`),每个内含一份可执行文件。用主二进制时
//!    debug 构建实测包体 3833MB(5 × 589MB app 二进制),用本 helper 后只剩几 MB。
//! 2. **启动语义**:helper 由 CEF 以 `--type=<process>` 拉起,不应进入 zap 的初始化;
//!    helper bundle 的 identifier 带后缀,主二进制里的 AppId 解析会 panic。
//!
//! 本 crate 只依赖 `cef`,不链接 zap 的任何代码,故二进制很小。
//! 构建:`CEF_PATH=... cargo build --manifest-path tools/cef-helper/Cargo.toml --release`

use cef::*;

/// 文档开始前注入的引导脚本。与 app 侧(`app/src/browser/webview_init_js.rs`)读的是
/// **同一个文件**,避免两份脚本漂移。
const WEBVIEW_INIT_JS: &str = include_str!("../../../app/assets/webview_init.js");

wrap_render_process_handler! {
    struct WebviewRenderProcessHandler;

    impl RenderProcessHandler {
        /// 在页面脚本执行前注入引导脚本(等价于 wry 的 WKUserScript document-start)。
        /// 只注入主 frame:与 wry 的 `with_initialization_script_for_main_only` 对齐。
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _context: Option<&mut V8Context>,
        ) {
            let Some(frame) = frame else {
                return;
            };
            if frame.is_main() == 0 {
                return;
            }
            let script = CefString::from(WEBVIEW_INIT_JS);
            frame.execute_java_script(Some(&script), None, 0);
        }
    }
}

wrap_app! {
    struct HelperApp;

    impl App {
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(WebviewRenderProcessHandler::new())
        }
    }
}

fn main() {
    // helper 模式:framework 在外层 app 的 Contents/Frameworks 下,非 helper 模式会在
    // helper 自己的 bundle 内查找而失败。先显式检查路径,便于脱离 bundle 运行时给出
    // 可读报错(否则 LibraryLoader 内部 canonicalize 会直接 unwrap panic)。
    let exe = std::env::current_exe().expect("current exe");
    let framework = exe
        .parent()
        .expect("exe has parent")
        .join("../../..")
        .join("Chromium Embedded Framework.framework");
    if !framework.exists() {
        eprintln!(
            "[zap_cef_helper] 未找到 {}:本 helper 必须在 .app bundle 内运行",
            framework.display()
        );
        std::process::exit(1);
    }
    let loader = library_loader::LibraryLoader::new(&exe, true);
    if !loader.load() {
        eprintln!("[zap_cef_helper] failed to load Chromium Embedded Framework");
        std::process::exit(1);
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    // 必须传 App:render 进程的注入(render_process_handler)只有在子进程也提供
    // CefApp 时才会被调用 —— 传 None 会让"文档开始前注入"整条路径失效。
    let mut app = HelperApp::new();
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    // loader 需活到进程结束(卸载会带着 CEF 的运行时代码一起走);exit 不跑析构,
    // 故先 forget 再退出。
    std::mem::forget(loader);
    // CEF 文档要求子进程以 execute_process 的返回值退出(它承载 renderer/GPU 的失败码)。
    std::process::exit(ret);
}
