//! Web preview:嵌入式 WebView(WKWebView)与 BrowserPane。

mod browser_pane_view;
mod browser_web_view;

pub use browser_pane_view::{BrowserPane, BrowserPaneAction, BrowserPaneView};
pub use browser_web_view::{BrowserWebViewEvent, BrowserWebViewManager};
