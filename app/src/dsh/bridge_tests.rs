use super::*;
use futures_util::StreamExt as _;
use serde_json::json;

/// 读一条 WS 响应帧并解析为 JSON。泛型避免命名具体流类型(connect_async
/// 返回 `WebSocketStream<ClientStream<TcpStream>>`,路径易随版本变动)。
async fn read_ws_resp<S>(ws: &mut S) -> Value
where
    S: futures_util::Stream<
            Item = Result<
                async_tungstenite::tungstenite::Message,
                async_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let msg = ws.next().await.unwrap().unwrap();
    let text = msg.into_text().unwrap();
    serde_json::from_str(&text).unwrap()
}

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

/// gitignore 规则祖先传递:根 /target 应影响其子目录内容(进入 target 也过滤)。
#[test]
fn zap_list_files_respects_ancestor_gitignore() {
    let dir = std::env::temp_dir().join(format!("zap-ig-anc-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("target").join("debug")).unwrap();
    std::fs::write(dir.join("target").join("x.o"), "x").unwrap();
    std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();

    // 根目录:target/ 被过滤。
    let out = handle_zap_method("zap.list_files", &json!({ "path": "" }), &dir).unwrap();
    let names: Vec<&str> = out["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"target"), "root-level target filtered: {names:?}");

    // 进入 target:祖先规则(target/)应过滤其内容。
    let out = handle_zap_method("zap.list_files", &json!({ "path": "target" }), &dir).unwrap();
    let entries = out["entries"].as_array().unwrap();
    assert!(
        entries.is_empty(),
        "target content should be filtered by ancestor rule: {entries:?}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 连接循环路由级测试:真实拉起 accept_loop + handle_connection,经 WS 客户端
/// 走完整消息路由(握手 → zap.ping / zap.terminal_context → 未知方法),验证
/// fast path 与 handle_message 的分发。防止「握手后 zap.ping 被 fast path
/// 劫持返回 METHOD_NOT_FOUND」「握手后错误误关连接」这类纯函数单测漏网的
/// 集成缺陷。
#[test]
fn connection_loop_routes_zap_methods() {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let token: Arc<str> = Arc::from("route-test-token");

    let rt = tokio::runtime::Runtime::new().unwrap();
    // 服务端:与真实桥一致地起 accept loop。
    rt.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        accept_loop(listener, token).await;
    });

    rt.block_on(async {
        let (mut ws, _) = async_tungstenite::tokio::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        use async_tungstenite::tungstenite::Message;
        use futures_util::{SinkExt as _, StreamExt as _};

        // 1. 握手成功(返回 protocolVersion + zap 能力)。
        ws.send(Message::Text(hello_req(Some(json!(1)), "route-test-token", Some(1)).into()))
            .await
            .unwrap();
        let resp = read_ws_resp(&mut ws).await;
        assert_eq!(resp["id"], json!(1));
        assert_eq!(resp["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(
            resp["result"]["capabilities"],
            json!(ZAP_CAPABILITIES),
            "握手应宣告实际能力清单"
        );

        // 2. 握手后 zap.ping → ok:true(核心回归:此前被 fast path 劫持为 -32601)。
        ws.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":2,"method":"zap.ping"}"#.into(),
        ))
        .await
        .unwrap();
        let resp = read_ws_resp(&mut ws).await;
        assert_eq!(resp["id"], json!(2));
        assert_eq!(resp["result"], json!({ "ok": true }), "zap.ping must route through");

        // 3. 握手后 zap.terminal_context → 返回 commands(隐私默认关 → 空数组)。
        ws.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":3,"method":"zap.terminal_context"}"#.into(),
        ))
        .await
        .unwrap();
        let resp = read_ws_resp(&mut ws).await;
        assert_eq!(resp["id"], json!(3));
        assert!(
            resp["result"]["commands"].is_array(),
            "terminal_context should return commands array"
        );

        // 4. 握手后未知方法 → 错误响应,且连接保持(不误报 HandshakeFailed/关闭)。
        ws.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":50,"method":"foo.bar"}"#.into(),
        ))
        .await
        .unwrap();
        let resp = read_ws_resp(&mut ws).await;
        assert_eq!(resp["id"], json!(50));
        assert_eq!(resp["error"]["code"], code::METHOD_NOT_FOUND);

        // 连接仍存活:再 ping 一次应成功。
        ws.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":4,"method":"zap.ping"}"#.into(),
        ))
        .await
        .unwrap();
        let resp = read_ws_resp(&mut ws).await;
        assert_eq!(resp["result"], json!({ "ok": true }), "connection must survive an error");
    });
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

// 注意:以下测试操作全局 PENDING_EVENTS,依赖 nextest(每测试独立进程)。
// 若改用 cargo test(多线程同进程),需加 #[serial] 或改验证返回值。

/// zap.notify:正常调用 → 返回 ok,产生 Notify 事件。
#[test]
fn zap_notify_ok() {
    let dir = std::env::temp_dir().join(format!("zap-notify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // 清空全局事件缓冲。
    PENDING_EVENTS.lock().clear();

    let out = handle_zap_method(
        "zap.notify",
        &json!({ "title": "Task done", "body": "Completed successfully" }),
        &dir,
    )
    .unwrap();
    assert_eq!(out, json!({ "ok": true }));

    // 验证事件已入队。
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    match &events[0] {
        BridgeEvent::Notify { title, body, category } => {
            assert_eq!(title, "Task done");
            assert_eq!(body, "Completed successfully");
            assert_eq!(*category, NotificationCategory::Complete);
        }
        other => panic!("expected Notify event, got {other:?}"),
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.notify:title 为空 → INVALID_PARAMS。
#[test]
fn zap_notify_empty_title_rejected() {
    let dir = std::env::temp_dir().join(format!("zap-notify-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let err = handle_zap_method("zap.notify", &json!({ "title": "" }), &dir).unwrap_err();
    assert_eq!(err.code, code::INVALID_PARAMS);
    assert!(err.message.contains("title"));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.notify:category 映射: "error" → Error, "confirm" → Request。
#[test]
fn zap_notify_category_mapping() {
    let dir = std::env::temp_dir().join(format!("zap-notify-cat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    PENDING_EVENTS.lock().clear();

    // error
    handle_zap_method(
        "zap.notify",
        &json!({ "title": "Error", "category": "error" }),
        &dir,
    )
    .unwrap();
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    match &events[0] {
        BridgeEvent::Notify { category, .. } => {
            assert_eq!(*category, NotificationCategory::Error);
        }
        other => panic!("expected Notify, got {other:?}"),
    }

    PENDING_EVENTS.lock().clear();

    // confirm
    handle_zap_method(
        "zap.notify",
        &json!({ "title": "Confirm", "category": "confirm" }),
        &dir,
    )
    .unwrap();
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    match &events[0] {
        BridgeEvent::Notify { category, .. } => {
            assert_eq!(*category, NotificationCategory::Request);
        }
        other => panic!("expected Notify, got {other:?}"),
    }

    std::fs::remove_dir_all(&dir).unwrap();
}
