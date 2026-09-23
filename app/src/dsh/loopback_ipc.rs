//! dsh(CEF 后端)回环 IPC:把浏览器端 shim 的 `zap.*` 消息从 webview IPC 通道
//! 迁到 Zap 本地回环 HTTP 端点。
//!
//! 背景:WKWebView 下 dsh 客户端插件用 `window.webkit.messageHandlers.ipc` 发消息;
//! CEF 没有该桥(见 specs/cef-webview-minimal/TECH.md 阶段 1.3)。方案是注入一段
//! shim(`shim_script()`),把同一份 `"method\nid\nparams"` 字符串 POST 到本端点,
//! 服务端转发给 `dsh::bridge::handle_zap_ipc`——**协议与消息格式完全不变,dsh 插件零改动**。
//!
//! 安全边界(端点只在 `FeatureFlag::CefWebview` 启用时注册):
//! - 监听地址由 `crates/http_server` 固定为 `127.0.0.1`(非回环不可达);
//! - 每次进程启动生成随机 token,随 shim 注入页面;请求必须携带匹配 token;
//! - token 比较为常量时间;请求体上限 64 KiB;未知方法由 bridge 侧忽略。

use std::collections::HashMap;
use std::sync::LazyLock;

use axum::extract::{DefaultBodyLimit, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;

/// token 的查询参数名(与 dsh 的 `?token=` 惯例一致,便于前端复用)。
const TOKEN_QUERY: &str = "token";
/// token 的请求头名(仅原生调用方兜底;页面侧 no-cors 请求会把它丢掉)。
const TOKEN_HEADER: &str = "x-zap-webview-token";
/// 单条 IPC 消息上限(正常消息为数百字节;留足 Session 日志类通知的余量)。
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 本次进程会话的回环 IPC token。每次启动重新生成,不落盘、不写日志。
static SESSION_TOKEN: LazyLock<String> = LazyLock::new(|| {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
});

/// 当前会话的回环 IPC token(供 shim 注入使用)。
pub(crate) fn session_token() -> &'static str {
    &SESSION_TOKEN
}

/// 常量时间比较,避免按字节提前返回泄漏 token 前缀。
fn token_matches(candidate: &str) -> bool {
    let expected = SESSION_TOKEN.as_bytes();
    let candidate = candidate.as_bytes();
    if candidate.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(candidate.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// 注册 `/webview-ipc` 端点(仅在 CEF 后端启用时由 app 装配)。
pub fn make_router() -> axum::Router {
    axum::Router::new()
        .route("/webview-ipc", axum::routing::post(handle_webview_ipc))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

/// 请求体信封:shim 侧格式(页面无法用请求头携带 token,见 `shim_script`)。
#[derive(serde::Deserialize)]
struct WebviewIpcEnvelope {
    token: String,
    message: String,
}

/// 归一化客户端 payload:dsh 插件发的是 `zap:<method>\n<id>\n<params>`,
/// wry 路径在同一位置剥掉前缀;无前缀(原生调用方)则原样透传。
fn normalize_zap_payload(message: &str) -> &str {
    message.strip_prefix("zap:").unwrap_or(message)
}

/// 提取 (token, message):首选请求体信封(shim 走这条);query/header 仅作原生调用方兜底。
fn extract_token_and_message(
    body: &str,
    query_token: Option<&str>,
    header_token: Option<&str>,
) -> (Option<String>, String) {
    match serde_json::from_str::<WebviewIpcEnvelope>(body) {
        Ok(envelope) => (Some(envelope.token), envelope.message),
        Err(_) => (
            query_token
                .or(header_token)
                .map(ToString::to_string),
            body.to_string(),
        ),
    }
}

async fn handle_webview_ipc(
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let (provided, message) = extract_token_and_message(
        &body,
        params.get(TOKEN_QUERY).map(String::as_str),
        headers.get(TOKEN_HEADER).and_then(|value| value.to_str().ok()),
    );
    let authorized = provided.as_deref().map(token_matches).unwrap_or(false);
    if !authorized {
        log::warn!("[dsh-loopback] rejected /webview-ipc request with invalid token");
        return (StatusCode::FORBIDDEN, "forbidden");
    }

    // 与 wry 路径同一协议:客户端插件发的是 `zap:<method>\n<id>\n<params>`,
    // wry 侧在 handler 里剥掉 `zap:` 前缀后再交给 bridge;回环路径必须一致,
    // 否则 method 变成 `zap:zap.switch_project` → bridge 不匹配 → 消息静默丢弃
    // (日志是 debug 级,默认过滤在 Info,完全不可见;评审 N2)。
    if let Some(target) = message.strip_prefix("warp:open-external:") {
        // 链接/`window.open` 一律交给系统默认浏览器:CEF pane 没有地址栏与后退,
        // 在 pane 内导航就回不去了(与 wry 的 init_js 拦截行为对齐;评审 N4)。
        open_external_on_main(target.trim().to_string());
        return (StatusCode::OK, "ok");
    }
    let payload = normalize_zap_payload(&message);

    // 与 webview IPC 路径同一入口、同一协议;已知方法内部会推入待处理事件队列。
    let handled = crate::dsh::bridge::handle_zap_ipc(payload).is_some();
    if !handled {
        // 不能静默:method 不匹配(例如前缀没剥干净)时原先只有 debug 日志,而文件
        // logger 过滤在 Info,导致整条链路"看起来正常但什么都没发生"(评审 N2)。
        // `handle_zap_ipc` 返回 None 有两种原因:方法未知,或**已知方法但参数被拒**
        // (例如 zap.open_code_review 收到非绝对路径),日志文案要覆盖两者,否则会
        // 把「参数被拒」误读成「方法名写错」。
        let method = payload.split('\n').next().unwrap_or_default();
        log::warn!("[dsh-loopback] IPC 未被接受(方法未知或参数被拒,已丢弃): {method}");
    }
    if handled {
        // 事件在 on_frame_drawn 中 drain;空闲期没有帧就永远不会 drain,
        // 故在 main 上请求一次重绘(与 webview IPC handler 的做法一致)。
        request_redraw_on_main();
    }
    (StatusCode::OK, "ok")
}

/// 用系统默认浏览器打开 URL(须在 main 线程:触碰 AppKit/NSWorkspace)。
fn open_external_on_main(url: String) {
    #[cfg(target_os = "macos")]
    dispatch2::DispatchQueue::main().exec_async(move || {
        warpui::platform::mac::Window::open_url(&url);
    });
    #[cfg(not(target_os = "macos"))]
    let _ = url;
}

/// 在 main 线程请求重绘:HTTP handler 跑在 http_server 的 tokio 线程上,
/// 而 `request_redraw_all_windows` 触碰 AppKit,必须在 main 上调用。
fn request_redraw_on_main() {
    #[cfg(target_os = "macos")]
    dispatch2::DispatchQueue::main().exec_async(|| {
        warpui::platform::mac::Window::request_redraw_all_windows();
    });
}

/// 注入页面的 shim:把 `window.webkit.messageHandlers.ipc.postMessage` 重定向到
/// 本端点。dsh 客户端插件(wrap-webview-js 等)按原样调用,无需改动。
pub(crate) fn shim_script() -> String {
    let token = session_token();
    format!(
        r#"
(function () {{
  if (window.__ZAP_LOOPBACK_IPC__) return;
  window.__ZAP_LOOPBACK_IPC__ = true;
  const ENDPOINT = 'http://127.0.0.1:9277/webview-ipc';
  const TOKEN = '{token}';
  const post = (message) => {{
    const text = typeof message === 'string' ? message : JSON.stringify(message);
    try {{
      // token 放**请求体**的信封里,而不是请求头或 URL:
      // - 请求头:no-cors 请求会把非 CORS-safelisted 头**静默丢弃**(实测到达时
      //   为 null),等于没带 token → 服务端必然 403;
      // - URL:http_server 的 TraceLayer 默认在 DEBUG 级记录 uri(全局过滤在 Info,
      //   默认不落盘),但一旦有人开 DEBUG 就会把 token 写进日志 —— 体则不会。
      // 请求体不参与 CORS 预检也不进访问日志,故三者中唯一可行。
      const envelope = JSON.stringify({{ token: TOKEN, message: text }});
      fetch(ENDPOINT, {{
        method: 'POST',
        mode: 'no-cors',
        headers: {{ 'Content-Type': 'text/plain' }},
        body: envelope,
        keepalive: true,
      }}).catch((err) => console.warn('[zap] loopback ipc failed', err));
    }} catch (err) {{
      console.warn('[zap] loopback ipc threw', err);
    }}
  }};
  window.webkit = window.webkit || {{}};
  window.webkit.messageHandlers = window.webkit.messageHandlers || {{}};
  window.webkit.messageHandlers.ipc = {{ postMessage: post }};
  // 回放"文档开始 → 本 shim 就位"之间排队的消息(引导脚本里的占位实现)。
  const queued = window.__zapIpcQueue || [];
  window.__zapIpcQueue = [];
  queued.forEach(post);
}})();
"#
    )
}

#[cfg(test)]
#[path = "loopback_ipc_tests.rs"]
mod tests;
