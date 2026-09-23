use super::*;
use std::path::PathBuf;

/// 构造 `zap.switch_project` 的 IPC payload(与 zap-bridge-client.js 一致:
/// `"method\nid\nparams_json"`)。
fn switch_project_payload(path: &str, id: u64) -> String {
    format!(
        "zap.switch_project\n{id}\n{{\"path\":{}}}",
        serde_json::to_string(path).unwrap()
    )
}

/// 构造 `zap.notify` 的 IPC payload。
fn notify_payload(title: &str, body: &str, category: Option<&str>, id: u64) -> String {
    notify_payload_with_session(title, body, category, None, id)
}

/// 构造 `zap.notify` 的 IPC payload(可携带 session_id)。
fn notify_payload_with_session(
    title: &str,
    body: &str,
    category: Option<&str>,
    session_id: Option<&str>,
    id: u64,
) -> String {
    let mut params = serde_json::Map::new();
    params.insert("title".into(), serde_json::json!(title));
    params.insert("body".into(), serde_json::json!(body));
    if let Some(c) = category {
        params.insert("category".into(), serde_json::json!(c));
    }
    if let Some(s) = session_id {
        params.insert("session_id".into(), serde_json::json!(s));
    }
    format!(
        "zap.notify\n{id}\n{}",
        serde_json::to_string(&serde_json::Value::Object(params)).unwrap()
    )
}

/// payload 缺 method → None。
#[test]
fn missing_method_returns_none() {
    assert!(handle_zap_ipc("").is_none());
    assert!(handle_zap_ipc("\n1\n{}").is_none());
}

/// payload 缺 params → 视为空对象;switch_project 无 path → None。
#[test]
fn missing_params_defaults_empty() {
    assert!(handle_zap_ipc("zap.switch_project\n1\n").is_none());
    assert!(handle_zap_ipc("zap.notify\n1\n").is_none());
}

/// 未知方法 → None,不 panic。
#[test]
fn unknown_method_returns_none() {
    assert!(handle_zap_ipc("zap.nope\n1\n{}").is_none());
}

/// zap.switch_project:正常路径 → 产生 SwitchProject 事件(canonical 后)。
#[test]
fn switch_project_ok() {
    let dir = std::env::temp_dir().join(format!("zap-switch-ok-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    PENDING_EVENTS.lock().clear();

    let event = handle_zap_ipc(&switch_project_payload(dir.to_str().unwrap(), 1)).unwrap();
    match event {
        BridgeEvent::SwitchProject { path } => {
            assert_eq!(path, dir.canonicalize().unwrap());
        }
        other => panic!("expected SwitchProject, got {other:?}"),
    }
    // 事件也进入 PENDING_EVENTS 供 on_frame_drawn drain。
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], BridgeEvent::SwitchProject { .. }));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.switch_project:路径不存在 → None(canonicalize 失败)。
#[test]
fn switch_project_nonexistent_path_rejected() {
    let missing = std::env::temp_dir().join(format!(
        "zap-switch-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    PENDING_EVENTS.lock().clear();
    assert!(
        handle_zap_ipc(&switch_project_payload(missing.to_str().unwrap(), 1)).is_none()
    );
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// zap.switch_project:路径是文件而非目录 → None。
#[test]
fn switch_project_file_path_rejected() {
    let file = std::env::temp_dir().join(format!("zap-switch-file-{}", std::process::id()));
    std::fs::write(&file, "x").unwrap();
    PENDING_EVENTS.lock().clear();
    assert!(
        handle_zap_ipc(&switch_project_payload(file.to_str().unwrap(), 1)).is_none()
    );
    assert!(PENDING_EVENTS.lock().is_empty());
    std::fs::remove_file(&file).unwrap();
}

/// zap.notify:正常调用 → Notify 事件(默认 Complete)。
#[test]
fn notify_ok() {
    PENDING_EVENTS.lock().clear();
    let event = handle_zap_ipc(&notify_payload("Task done", "Completed", None, 1)).unwrap();
    match event {
        BridgeEvent::Notify {
            title,
            body,
            category,
            session_id,
        } => {
            assert_eq!(title, "Task done");
            assert_eq!(body, "Completed");
            assert_eq!(category, NotificationCategory::Complete);
            assert_eq!(session_id, None);
        }
        other => panic!("expected Notify, got {other:?}"),
    }
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], BridgeEvent::Notify { .. }));
}

/// zap.notify:title 为空 → None。
#[test]
fn notify_empty_title_rejected() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc(&notify_payload("", "body", None, 1)).is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// zap.notify:category 映射 — "error" → Error,"confirm" → Request。
#[test]
fn notify_category_mapping() {
    PENDING_EVENTS.lock().clear();
    match handle_zap_ipc(&notify_payload("E", "b", Some("error"), 1)).unwrap() {
        BridgeEvent::Notify { category, .. } => {
            assert_eq!(category, NotificationCategory::Error);
        }
        other => panic!("expected Notify, got {other:?}"),
    }

    PENDING_EVENTS.lock().clear();
    match handle_zap_ipc(&notify_payload("C", "b", Some("confirm"), 1)).unwrap() {
        BridgeEvent::Notify { category, .. } => {
            assert_eq!(category, NotificationCategory::Request);
        }
        other => panic!("expected Notify, got {other:?}"),
    }
}

/// zap.notify:session_id 透传为 Notify.session_id;空串视为缺失。
#[test]
fn notify_session_id_passthrough() {
    PENDING_EVENTS.lock().clear();
    match handle_zap_ipc(&notify_payload_with_session("T", "b", None, Some("sess-1"), 1)).unwrap() {
        BridgeEvent::Notify { session_id, .. } => {
            assert_eq!(session_id.as_deref(), Some("sess-1"));
        }
        other => panic!("expected Notify, got {other:?}"),
    }

    PENDING_EVENTS.lock().clear();
    match handle_zap_ipc(&notify_payload_with_session("T", "b", None, Some(""), 1)).unwrap() {
        BridgeEvent::Notify { session_id, .. } => {
            assert_eq!(session_id, None);
        }
        other => panic!("expected Notify, got {other:?}"),
    }
}

/// payload params 不是合法 JSON → 视为空对象;switch_project 无 path → None。
#[test]
fn malformed_params_returns_none() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc("zap.switch_project\n1\n{not json").is_none());
    assert!(handle_zap_ipc("zap.notify\n1\n{not json").is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// path 字段不是字符串 → None。
#[test]
fn switch_project_non_string_path_rejected() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc("zap.switch_project\n1\n{\"path\":123}").is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// 构造 `zap.open_file` 的 IPC payload(与 zap-bridge-client.js 一致)。
fn open_file_payload(path: &str, id: u64) -> String {
    format!(
        "zap.open_file\n{id}\n{{\"path\":{}}}",
        serde_json::to_string(path).unwrap()
    )
}

/// zap.open_file:存在的文件 → OpenFile 事件(canonical 后,允许文件)。
#[test]
fn open_file_ok() {
    let file = std::env::temp_dir().join(format!("zap-openfile-ok-{}", std::process::id()));
    std::fs::write(&file, "x").unwrap();
    PENDING_EVENTS.lock().clear();

    let event = handle_zap_ipc(&open_file_payload(file.to_str().unwrap(), 1)).unwrap();
    match event {
        BridgeEvent::OpenFile { path } => {
            assert_eq!(path, file.canonicalize().unwrap());
        }
        other => panic!("expected OpenFile, got {other:?}"),
    }
    let events = PENDING_EVENTS.lock().clone();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], BridgeEvent::OpenFile { .. }));

    std::fs::remove_file(&file).unwrap();
}

/// zap.open_file:存在的目录 → 同样产生 OpenFile 事件(目录开 session)。
#[test]
fn open_file_directory_ok() {
    let dir = std::env::temp_dir().join(format!("zap-openfile-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    PENDING_EVENTS.lock().clear();

    let event = handle_zap_ipc(&open_file_payload(dir.to_str().unwrap(), 1)).unwrap();
    match event {
        BridgeEvent::OpenFile { path } => {
            assert_eq!(path, dir.canonicalize().unwrap());
        }
        other => panic!("expected OpenFile, got {other:?}"),
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// zap.open_file:路径不存在 → None(canonicalize 失败)。
#[test]
fn open_file_nonexistent_path_rejected() {
    let missing = std::env::temp_dir().join(format!(
        "zap-openfile-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc(&open_file_payload(missing.to_str().unwrap(), 1)).is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// zap.open_file:path 字段缺失/非字符串 → None。
#[test]
fn open_file_malformed_params_rejected() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc("zap.open_file\n1\n").is_none());
    assert!(handle_zap_ipc("zap.open_file\n1\n{}").is_none());
    assert!(handle_zap_ipc("zap.open_file\n1\n{\"path\":123}").is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// 构造 `zap.open_code_review` 的 IPC payload(与 zap-bridge-client.js 一致)。
fn open_code_review_payload(path: &str, id: u64) -> String {
    format!(
        "zap.open_code_review\n{id}\n{{\"path\":{}}}",
        serde_json::to_string(path).unwrap()
    )
}

/// zap.open_code_review:绝对路径 → OpenCodeReview 事件。
/// 不要求文件存在:改动行指向的文件可能已被删除/重命名,面板仍应打开。
#[test]
fn open_code_review_absolute_path_ok() {
    PENDING_EVENTS.lock().clear();
    let path = "/tmp/zap-open-code-review-missing.rs";
    let event = handle_zap_ipc(&open_code_review_payload(path, 1)).unwrap();
    match event {
        BridgeEvent::OpenCodeReview { path: got } => {
            assert_eq!(got, Some(PathBuf::from(path)));
        }
        other => panic!("expected OpenCodeReview, got {other:?}"),
    }
    PENDING_EVENTS.lock().clear();
}

/// zap.open_code_review:无 path(改动卡片表头按钮)→ 只开面板,path 为 None。
/// 缺 params 段与 `{}` 等价(handle_zap_ipc 对空 params 回退空对象)。
#[test]
fn open_code_review_without_path_ok() {
    PENDING_EVENTS.lock().clear();
    for payload in ["zap.open_code_review\n1\n{}", "zap.open_code_review\n1\n"] {
        let event = handle_zap_ipc(payload).unwrap();
        match event {
            BridgeEvent::OpenCodeReview { path } => assert_eq!(path, None),
            other => panic!("expected OpenCodeReview, got {other:?}"),
        }
    }
    PENDING_EVENTS.lock().clear();
}

/// zap.open_code_review:相对路径 → None(只接受本机绝对路径)。
#[test]
fn open_code_review_relative_path_rejected() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc(&open_code_review_payload("src/main.rs", 1)).is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}

/// zap.open_code_review:path 非字符串 → None(缺 path 合法,见上一个用例)。
#[test]
fn open_code_review_malformed_params_rejected() {
    PENDING_EVENTS.lock().clear();
    assert!(handle_zap_ipc("zap.open_code_review\n1\n{\"path\":123}").is_none());
    assert!(PENDING_EVENTS.lock().is_empty());
}
