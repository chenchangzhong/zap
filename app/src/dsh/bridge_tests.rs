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
    let mut params = serde_json::Map::new();
    params.insert("title".into(), serde_json::json!(title));
    params.insert("body".into(), serde_json::json!(body));
    if let Some(c) = category {
        params.insert("category".into(), serde_json::json!(c));
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
        } => {
            assert_eq!(title, "Task done");
            assert_eq!(body, "Completed");
            assert_eq!(category, NotificationCategory::Complete);
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
