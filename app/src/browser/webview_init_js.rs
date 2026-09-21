//! 嵌入式 webview 的**后端无关**注入脚本。
//!
//! 同一段脚本供两条路径使用,保证行为一致:
//! - wry/WKWebView:`with_initialization_script_for_main_only`(文档开始前);
//! - CEF/Chromium:render 进程 `RenderProcessHandler::on_context_created`(文档开始前)。
//!
//! 脚本本身不含会话 token(render 进程拿不到 browser 进程的 token):带 token 的
//! 回环 shim 由 `dsh::loopback_ipc::shim_script()` 在 load_end 注入,并通过
//! `__zapIpcQueue` 回放此前排队的消息。

/// 文档开始前注入的引导脚本(见模块文档)。
/// 文档开始前注入的引导脚本(见模块文档)。放在 `app/assets/` 下以便
/// `tools/cef-helper`(render 进程)用同一个文件,避免两份脚本漂移。
pub(crate) const WEBVIEW_INIT_JS: &str = include_str!("../../assets/webview_init.js");

#[cfg(test)]
mod tests {
    use super::WEBVIEW_INIT_JS;

    /// N1 回归:引导脚本必须带上 dsh 客户端插件用来判断"是否在 Zap 环境"的标记,
    /// 否则 `zap-bridge-client.js` 的 apply() 会在第一行直接 return(整条桥全废)。
    #[test]
    fn init_script_marks_zap_bridge_and_queues_early_messages() {
        assert!(
            WEBVIEW_INIT_JS.contains("window.__ZAP_BRIDGE__ = true;"),
            "缺少 __ZAP_BRIDGE__ 标记:dsh 插件会自我禁用"
        );
        assert!(
            WEBVIEW_INIT_JS.contains("__zapIpcQueue"),
            "缺少消息排队占位:文档开始到 shim 就位之间的 zapRpc 会永久丢失"
        );
        assert!(
            WEBVIEW_INIT_JS.contains("warp:open-external:"),
            "缺少外链拦截:CEF pane 内导航后无法返回"
        );
        assert!(
            !WEBVIEW_INIT_JS.contains("?token=") && !WEBVIEW_INIT_JS.contains("x-zap-webview-token"),
            "引导脚本不得携带 token(它由 render 进程注入,拿不到 browser 进程的 token)"
        );
    }
}
