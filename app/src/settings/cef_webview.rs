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
