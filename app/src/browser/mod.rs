//! Web preview:嵌入式 WebView(WKWebView)与 BrowserPane。
//!
//! `cef_backend`:CEF(Chromium)后端骨架,仅 `cef_webview` feature 下编译
//! (specs/cef-webview-minimal/TECH.md 阶段 1)。

mod browser_pane_view;
mod browser_web_view;

pub use browser_pane_view::{BrowserPane, BrowserPaneAction, BrowserPaneView};
pub use browser_web_view::{BrowserWebViewEvent, BrowserWebViewManager};

#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) mod cef_backend;

/// CEF 内核版本检查(设置页「CEF 内核更新」;只比版本,不下载、不改设置)。
/// 升级流程见 `specs/cef-webview-minimal/CEF-UPGRADE.md`。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) mod cef_update;

/// 后端无关的 webview 注入脚本(wry 与 CEF 共用)。
pub(crate) mod webview_init_js;
