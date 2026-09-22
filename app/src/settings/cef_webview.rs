//! CEF(Chromium)dsh pane 后端的设置。
//!
//! 目前两项:是否用 Chromium 内核、隐藏多久后冻结页面。冻结用 CDP
//! `Page.setWebLifecycleState=frozen`(见 app/src/browser/cef_backend.rs):
//! 保住 DOM 与会话,只停页面 JS 与渲染 —— dsh 的 Node 服务端与 agent 任务不受影响。
//! 代价是隐藏期间页面发不出消息(如"任务完成"通知会延后到切回)。

use settings::{
    macros::define_settings_group, SupportedPlatforms, SyncToCloud,
};

define_settings_group!(CefWebviewSettings, settings: [
    // macOS 上是否用 Chromium(CEF)内核承载 dsh pane。其他平台内置 webview 已是
    // Chromium 内核,无需此项(故 supported_platforms 只给 MAC)。
    use_chromium: UseChromiumWebview {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.webview.use_chromium",
        description: "Whether the dsh pane uses the Chromium (CEF) engine on macOS.",
    },
    // 是否用 OSR(windowless)渲染 dsh pane 的 webview。
    //
    // 背景:windowed 模式下 CEF 自建的原生子视图在半透明窗口里**做不到真透明**
    // (浏览器背景 alpha 透明会退化成不透明白);OSR 由宿主自建视图 + IOSurface,
    // 才能让带 alpha 的网页像素与下层 Metal 背景合成。代价是实现面更大(输入/IME/
    // 弹层都要宿主转接),故默认仍走 windowed。
    // 生效时机:进程级开关(`CefSettings.windowless_rendering_enabled`),**下次启动
    // 生效**;`ZAP_CEF_OSR` 可临时覆盖(dev 排查用,见 `cef_backend::render_mode`)。
    // **默认 true**(2026-09-22 用户决定):透明是这条分支的唯一目的,默认关着等于多数人
    // 拿不到;开关保留作为**回滚入口**(改成 false + 重启即退回 windowed)。
    use_osr_rendering: UseOsrRendering {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.webview.use_osr_rendering",
        description: "Whether the dsh pane uses windowless (OSR) rendering instead of a native child view on macOS (enabled by default; required for a transparent background). Takes effect after restarting Zap.",
    },
    // 隐藏超过该秒数后冻结页面;0 = 不冻结(仅隐藏,renderer 保活)。
    freeze_after_secs: CefWebviewFreezeAfterSecs {
        type: u32,
        default: 300u32,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.webview.freeze_after_secs",
        description: "Seconds a hidden embedded web page (dsh pane) may stay idle before it is frozen; 0 disables freezing.",
    },
]);
