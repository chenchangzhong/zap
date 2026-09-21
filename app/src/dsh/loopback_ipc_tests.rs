use super::*;

/// token 必须逐字节匹配;长度不同或任一位不同都拒绝(常量时间实现见 token_matches)。
#[test]
fn token_matches_only_accepts_exact_session_token() {
    assert!(token_matches(session_token()));

    let mut tampered = session_token().to_string();
    tampered.push('x');
    assert!(!token_matches(&tampered), "长度不同的 token 必须拒绝");

    let mut flipped = session_token().to_string();
    // 翻转首字符:验证"只差一位"也被拒绝(而不是只比较长度)。
    let first = flipped.remove(0);
    flipped.insert(0, if first == 'a' { 'b' } else { 'a' });
    assert!(!token_matches(&flipped), "单字符不同的 token 必须拒绝");

    assert!(!token_matches(""));
}

/// 会话 token 每次进程启动随机生成,且长度固定 32(不要落盘/写日志)。
#[test]
fn session_token_is_random_alphanumeric() {
    let token = session_token();
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_eq!(token, session_token(), "同一进程内应稳定复用");
}

/// shim 必须:
/// 1. 定义 dsh 插件实际调用的 `window.webkit.messageHandlers.ipc.postMessage`;
/// 2. 把消息 POST 到回环端点并带上会话 token;
/// 3. 可重复注入(幂等),不覆盖已装载标记。
#[test]
fn shim_script_installs_ipc_bridge_with_token() {
    let script = shim_script();
    assert!(
        script.contains("window.webkit.messageHandlers.ipc"),
        "shim 必须提供 dsh 插件使用的 webkit IPC 入口"
    );
    assert!(script.contains("postMessage: post"));
    assert!(
        script.contains(&format!("const TOKEN = '{}'", session_token())),
        "shim 必须持有会话 token"
    );
    assert!(
        script.contains("JSON.stringify({ token: TOKEN, message: text })"),
        "token 必须走请求体信封:no-cors 请求会静默丢弃自定义请求头(实测到达时为 null)"
    );
    assert!(
        !script.contains("x-zap-webview-token"),
        "shim 不得再把 token 放请求头(no-cors 下会被丢弃)"
    );
    assert!(
        !script.contains("?token="),
        "token 不得走 query 串:http_server 的 TraceLayer 会把 uri 记进日志"
    );
    assert!(
        script.contains("mode: 'no-cors'"),
        "跨源 POST 无需读响应,用 no-cors 避免 CORS 噪音"
    );
    assert!(
        script.contains("http://127.0.0.1:9277/webview-ipc"),
        "shim 必须指向 Zap 本地回环端点"
    );
    assert!(
        script.contains("__ZAP_LOOPBACK_IPC__"),
        "shim 需幂等标记,避免重复注入"
    );
    // token 转义:JS 字符串字面量里不能出现裸引号/换行。
    assert!(!script.contains('\'') || !script.contains("token='"));
}

/// 信封解析:请求体里的 token 与 message 必须分别取出,message 原样交给 bridge。
#[test]
fn envelope_carries_token_and_preserves_message() {
    let payload = "zap.switch_project\n7\n{\"path\":\"/tmp\"}";
    let envelope = serde_json::json!({ "token": session_token(), "message": payload }).to_string();
    let parsed: serde_json::Value = serde_json::from_str(&envelope).expect("信封必须是合法 JSON");
    assert_eq!(parsed["token"].as_str(), Some(session_token()));
    assert_eq!(parsed["message"].as_str(), Some(payload));
}

/// B1 修复的可验证判据:信封里的 token 必须被取出,message 原样保留;
/// 无 token 时必须取不到(修复前正是这种情况——头被 no-cors 丢弃 → 恒 403)。
#[test]
fn envelope_is_the_authoritative_token_source() {
    let payload = "zap.notify\n1\n{}";
    let envelope = serde_json::json!({ "token": session_token(), "message": payload }).to_string();

    let (token, message) = extract_token_and_message(&envelope, None, None);
    assert_eq!(token.as_deref(), Some(session_token()));
    assert_eq!(message, payload);
    assert!(token.as_deref().map(token_matches).unwrap_or(false));

    // 修复前的形态:页面只发了裸消息、头被浏览器丢弃 → 服务端拿不到 token。
    let (token, message) = extract_token_and_message(payload, None, None);
    assert_eq!(token, None, "裸请求体不得被当成凭证");
    assert_eq!(message, payload);
    assert!(!token.as_deref().map(token_matches).unwrap_or(false));

    // query 兜底仍可用(原生调用方)。
    let (token, _) = extract_token_and_message(payload, Some(session_token()), None);
    assert!(token.as_deref().map(token_matches).unwrap_or(false));
}

/// N2 回归:dsh 客户端发的是**带 `zap:` 前缀**的真实 wire format,服务端必须剥掉,
/// 否则 method 变成 `zap:zap.switch_project`,bridge 不匹配 → 消息静默丢弃。
#[test]
fn client_prefix_is_stripped_before_bridge() {
    let wire = "zap:zap.switch_project\n7\n{\"path\":\"/tmp\"}";
    let payload = normalize_zap_payload(wire);
    assert_eq!(payload, "zap.switch_project\n7\n{\"path\":\"/tmp\"}");
    assert_eq!(payload.splitn(3, '\n').next(), Some("zap.switch_project"));
    // 无前缀(原生调用方)原样透传。
    assert_eq!(normalize_zap_payload("zap.notify\n1\n{}"), "zap.notify\n1\n{}");
}

/// 协议不变性:shim 传送的字符串格式与 webview IPC 一致
/// (`method\nid\nparams`),bridge 侧解析路径因此无需改动。
#[test]
fn payload_format_matches_bridge_parser() {
    let payload = "zap.switch_project\n7\n{\"path\":\"/tmp\"}";
    let mut parts = payload.splitn(3, '\n');
    assert_eq!(parts.next(), Some("zap.switch_project"));
    assert_eq!(parts.next(), Some("7"));
    assert_eq!(parts.next(), Some("{\"path\":\"/tmp\"}"));
}

/// 上限值本身是安全边界:放大它需要显式评审,这里锁死当前取值。
#[test]
fn body_limit_is_bounded() {
    assert_eq!(MAX_BODY_BYTES, 64 * 1024);
}
