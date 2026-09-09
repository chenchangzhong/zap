//! AISettingsPageAction 的 Debug 脱敏测试。
//!
//! action 会经框架层 dispatch 以 Info 级打印完整 Debug 并落盘日志,
//! 携带 api_key / 请求头值的变体必须只输出 `[redacted]`(见 ai_page.rs
//! 的 `Redacted`),键名与其它字段保留可读。

use super::*;

#[test]
fn save_action_debug_redacts_api_key_and_header_values() {
    let action = AISettingsPageAction::SaveAgentProviderEditsThen {
        provider_id: "p1".into(),
        name: "My Provider".into(),
        base_url: "https://api.example.com".into(),
        api_key: Redacted("sk-SUPER-SECRET".into()),
        headers: vec![("Authorization".into(), Redacted("Bearer token".into()))],
        models: vec![],
        action: Box::new(AISettingsPageAction::AddAgentProvider),
    };

    let dbg = format!("{action:?}");
    assert!(dbg.contains("[redacted]"), "脱敏占位应出现: {dbg}");
    assert!(
        !dbg.contains("sk-SUPER-SECRET"),
        "api_key 明文不得出现: {dbg}"
    );
    assert!(!dbg.contains("Bearer token"), "请求头值明文不得出现: {dbg}");
    assert!(dbg.contains("Authorization"), "请求头键名保留可读: {dbg}");
    assert!(
        dbg.contains("https://api.example.com"),
        "非敏感字段保留可读: {dbg}"
    );
}

#[test]
fn update_api_key_action_debug_redacts_api_key() {
    let action = AISettingsPageAction::UpdateAgentProviderApiKey {
        provider_id: "p1".into(),
        api_key: Redacted("sk-SUPER-SECRET".into()),
    };

    let dbg = format!("{action:?}");
    assert!(dbg.contains("[redacted]"), "脱敏占位应出现: {dbg}");
    assert!(
        !dbg.contains("sk-SUPER-SECRET"),
        "api_key 明文不得出现: {dbg}"
    );
    assert!(dbg.contains("p1"), "provider_id 保留可读: {dbg}");
}

#[test]
fn save_edits_action_debug_redacts_api_key_and_header_values() {
    let action = AISettingsPageAction::SaveAgentProviderEdits {
        provider_id: "p1".into(),
        name: "My Provider".into(),
        base_url: "https://api.example.com".into(),
        api_key: Redacted("sk-SUPER-SECRET".into()),
        headers: vec![("Authorization".into(), Redacted("Bearer token".into()))],
        models: vec![],
    };

    let dbg = format!("{action:?}");
    assert!(dbg.contains("[redacted]"), "脱敏占位应出现: {dbg}");
    assert!(
        !dbg.contains("sk-SUPER-SECRET"),
        "api_key 明文不得出现: {dbg}"
    );
    assert!(!dbg.contains("Bearer token"), "请求头值明文不得出现: {dbg}");
    assert!(dbg.contains("Authorization"), "请求头键名保留可读: {dbg}");
    assert!(
        dbg.contains("https://api.example.com"),
        "非敏感字段保留可读: {dbg}"
    );
}

#[test]
fn update_header_action_debug_redacts_header_value() {
    let action = AISettingsPageAction::UpdateAgentProviderHeader {
        provider_id: "p1".into(),
        header_index: 0,
        key: "Authorization".into(),
        value: Redacted("Bearer token".into()),
    };

    let dbg = format!("{action:?}");
    assert!(!dbg.contains("Bearer token"), "请求头值明文不得出现: {dbg}");
    assert!(dbg.contains("Authorization"), "请求头键名保留可读: {dbg}");
}
