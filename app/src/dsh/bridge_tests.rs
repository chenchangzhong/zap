use super::*;
use serde_json::json;

fn hello_req(id: Option<Value>, token: &str, protocol: Option<u32>) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".into(), json!("2.0"));
    if let Some(id) = id {
        obj.insert("id".into(), id);
    }
    obj.insert("method".into(), json!("bridge/hello"));
    let mut params = serde_json::Map::new();
    params.insert("token".into(), json!(token));
    params.insert("dshVersion".into(), json!("0.1.0-rc.6"));
    params.insert("capabilities".into(), json!(["tools", "workspace"]));
    if let Some(p) = protocol {
        params.insert("protocolVersion".into(), json!(p));
    }
    obj.insert("params".into(), Value::Object(params));
    serde_json::to_string(&Value::Object(obj)).unwrap()
}

/// token 生成:长度、随机性。
#[test]
fn generates_distinct_tokens() {
    let a = generate_token();
    let b = generate_token();
    assert_eq!(a.len(), TOKEN_LEN);
    assert_eq!(b.len(), TOKEN_LEN);
    assert_ne!(a, b, "tokens must differ across starts");
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
}

/// 畸形 JSON → PARSE_ERROR(-32700)。
#[test]
fn parse_error_on_malformed_json() {
    let err = handle_message("tok", false, "{not json").unwrap_err();
    assert_eq!(err.code, code::PARSE_ERROR);
    assert_eq!(err.id, None);
}

/// 非对象请求 → INVALID_REQUEST。
#[test]
fn invalid_request_on_non_object() {
    let err = handle_message("tok", false, "[1,2]").unwrap_err();
    assert_eq!(err.code, code::INVALID_REQUEST);
}

/// 缺 method → INVALID_REQUEST,且错误响应带请求 id。
#[test]
fn invalid_request_on_missing_method() {
    let raw = r#"{"jsonrpc":"2.0","id":7,"params":{}}"#;
    let err = handle_message("tok", false, raw).unwrap_err();
    assert_eq!(err.code, code::INVALID_REQUEST);
    assert_eq!(err.id, Some(json!(7)));
}

/// 未知方法 → METHOD_NOT_FOUND。
#[test]
fn method_not_found() {
    let raw = r#"{"jsonrpc":"2.0","id":1,"method":"zap.nope"}"#;
    let err = handle_message("tok", false, raw).unwrap_err();
    assert_eq!(err.code, code::METHOD_NOT_FOUND);
    assert_eq!(err.id, Some(json!(1)));
}

/// 握手缺 token → UNAUTHORIZED。
#[test]
fn handshake_missing_token_rejected() {
    let raw = r#"{"jsonrpc":"2.0","id":1,"method":"bridge/hello","params":{"dshVersion":"0.1.0"}}"#;
    let err = handle_message("tok", false, raw).unwrap_err();
    assert_eq!(err.code, code::UNAUTHORIZED);
}

/// 握手 token 错误 → UNAUTHORIZED。
#[test]
fn handshake_wrong_token_rejected() {
    let raw = hello_req(Some(json!(1)), "wrong", Some(1));
    let err = handle_message("tok", false, &raw).unwrap_err();
    assert_eq!(err.code, code::UNAUTHORIZED);
    assert_eq!(err.id, Some(json!(1)));
}

/// 握手 token 正确 → 返回 protocolVersion + Connected 事件。
#[test]
fn handshake_success() {
    let raw = hello_req(Some(json!(5)), "tok", Some(1));
    let outcome = handle_message("tok", false, &raw).unwrap();

    // 事件:Connected 携带 dsh 版本与能力。
    let ev = outcome.events.first().expect("has Connected event");
    let BridgeEvent::Connected {
        dsh_version,
        capabilities,
    } = ev
    else {
        panic!("expected Connected, got {ev:?}");
    };
    assert_eq!(dsh_version, "0.1.0-rc.6");
    assert_eq!(capabilities, &vec!["tools".to_string(), "workspace".to_string()]);

    // 响应:protocolVersion + zap 能力。
    let resp = outcome.response.expect("has response");
    assert_eq!(resp["id"], json!(5));
    assert_eq!(resp["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
    assert!(resp["result"]["zapVersion"].is_string());
}

/// 握手 protocolVersion 不匹配 → PROTOCOL_MISMATCH。
#[test]
fn handshake_protocol_mismatch() {
    let raw = hello_req(Some(json!(2)), "tok", Some(999));
    let err = handle_message("tok", false, &raw).unwrap_err();
    assert_eq!(err.code, code::PROTOCOL_MISMATCH);
}

/// 未握手时调用 zap.ping → UNAUTHORIZED。
#[test]
fn ping_before_handshake_unauthorized() {
    let raw = r#"{"jsonrpc":"2.0","id":1,"method":"zap.ping"}"#;
    let err = handle_message("tok", false, raw).unwrap_err();
    assert_eq!(err.code, code::UNAUTHORIZED);
}

/// 已握手后 zap.ping → ok,且不产生事件。
#[test]
fn ping_after_handshake_ok() {
    let raw = r#"{"jsonrpc":"2.0","id":3,"method":"zap.ping"}"#;
    let outcome = handle_message("tok", true, raw).unwrap();
    assert!(outcome.events.is_empty());
    let resp = outcome.response.unwrap();
    assert_eq!(resp["id"], json!(3));
    assert_eq!(resp["result"], json!({ "ok": true }));
}

/// 通知(无 id):握手成功但 response 为 None。
#[test]
fn handshake_as_notification_no_response() {
    let raw = hello_req(None, "tok", Some(1));
    let outcome = handle_message("tok", false, &raw).unwrap();
    assert!(outcome.response.is_none(), "notification must not get a response");
    assert_eq!(outcome.events.len(), 1);
}
