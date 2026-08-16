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

/// zap.list_files:列目录条目,排序且区分 dir/file。
#[test]
fn zap_list_files_lists_entries() {
    let dir = std::env::temp_dir().join(format!("zap-list-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "hi").unwrap();

    let out = handle_zap_method("zap.list_files", &json!({ "path": "" }), &dir).unwrap();
    let entries = out["entries"].as_array().expect("entries");
    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["a.txt", "sub"], "sorted dir/file listing");
    assert_eq!(entries[0]["kind"], "file");
    assert_eq!(entries[1]["kind"], "dir");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.list_files:相对子目录路径。
#[test]
fn zap_list_files_relative_subdir() {
    let dir = std::env::temp_dir().join(format!("zap-list-sub-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("inner.txt"), "x").unwrap();

    let out = handle_zap_method("zap.list_files", &json!({ "path": "sub" }), &dir).unwrap();
    let entries = out["entries"].as_array().unwrap();
    assert_eq!(entries[0]["name"], "inner.txt");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.list_files:路径非目录 → NOT_A_DIR。
#[test]
fn zap_list_files_not_a_dir() {
    let dir = std::env::temp_dir().join(format!("zap-list-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "x").unwrap();

    let err = handle_zap_method("zap.list_files", &json!({ "path": "f.txt" }), &dir)
        .unwrap_err();
    assert_eq!(err.code, code::NOT_A_DIR);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.read_file:读取文件内容。
#[test]
fn zap_read_file_reads_content() {
    let dir = std::env::temp_dir().join(format!("zap-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "hello zap").unwrap();

    let out = handle_zap_method("zap.read_file", &json!({ "path": "a.txt" }), &dir).unwrap();
    assert_eq!(out["content"], "hello zap");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.read_file:缺 path → INVALID_PARAMS。
#[test]
fn zap_read_file_missing_path() {
    let dir = std::env::temp_dir().join(format!("zap-read-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let err = handle_zap_method("zap.read_file", &json!({}), &dir).unwrap_err();
    assert_eq!(err.code, code::INVALID_PARAMS);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.*:未知方法 → METHOD_NOT_FOUND。
#[test]
fn zap_unknown_method() {
    let dir = std::env::temp_dir().join(format!("zap-unk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = handle_zap_method("zap.nope", &json!({}), &dir).unwrap_err();
    assert_eq!(err.code, code::METHOD_NOT_FOUND);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.* 路径穿越防护:拒绝 `..` 逃逸与绝对路径。
#[test]
fn zap_path_traversal_rejected() {
    let dir = std::env::temp_dir().join(format!("zap-traversal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // ".." 逃逸。
    let err = handle_zap_method("zap.read_file", &json!({ "path": "../secret" }), &dir)
        .unwrap_err();
    assert_eq!(err.code, code::PATH_INVALID);
    // 绝对路径。
    let err = handle_zap_method("zap.list_files", &json!({ "path": "/etc" }), &dir).unwrap_err();
    assert_eq!(err.code, code::PATH_INVALID);
    // 嵌套 ".."。
    let err = handle_zap_method("zap.read_file", &json!({ "path": "a/../../x" }), &dir)
        .unwrap_err();
    assert_eq!(err.code, code::PATH_INVALID);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.list_files 尊重 .gitignore(与 Zap 文件树语义一致)。
#[test]
fn zap_list_files_respects_gitignore() {
    let dir = std::env::temp_dir().join(format!("zap-ig-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("node_modules")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join(".gitignore"), "node_modules/\n").unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "fn main() {}").unwrap();

    let out = handle_zap_method("zap.list_files", &json!({ "path": "" }), &dir).unwrap();
    let names: Vec<&str> = out["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.contains(&"node_modules"),
        "gitignored entry must be filtered: {names:?}"
    );
    assert!(names.contains(&"src"), "non-ignored dir listed: {names:?}");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.search:按文件名子串匹配,返回相对项目根的路径。
#[test]
fn zap_search_matches_filenames() {
    let dir = std::env::temp_dir().join(format!("zap-search-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src").join("foo_utils.rs"), "x").unwrap();
    std::fs::write(dir.join("src").join("bar.rs"), "x").unwrap();
    std::fs::write(dir.join("README.md"), "x").unwrap();

    let out = handle_zap_method("zap.search", &json!({ "pattern": "foo" }), &dir).unwrap();
    let matches: Vec<&str> = out["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap())
        .collect();
    assert_eq!(matches, vec!["src/foo_utils.rs"]);

    // 无匹配。
    let out = handle_zap_method("zap.search", &json!({ "pattern": "zzz" }), &dir).unwrap();
    assert!(out["matches"].as_array().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}
