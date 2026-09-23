use super::*;

/// 构建脚本必须从 `Cargo.lock` 推出 CEF 版本:取到 `unknown` 或解析不出,说明
/// `app/build.rs` 的推导失效 —— 设置页会显示不出"当前版本",版本比较也就无从谈起。
#[test]
fn installed_version_is_resolved_from_build_script() {
    assert_ne!(INSTALLED_VERSION, "unknown");
    assert!(
        parse_version(INSTALLED_VERSION).is_some(),
        "INSTALLED_VERSION 不可解析:{INSTALLED_VERSION}"
    );
}

/// 版本号比较:`+` 后的构建标识不参与;任一侧不可解析时返回 None(不谎报有新版本)。
#[test]
fn compare_cef_versions_ignores_build_suffix() {
    assert_eq!(compare("152.0.8+g1ce985c+chromium-152.0.7977.134", "152.0.8"), Some(false));
    assert_eq!(compare("153.0.0+gabc+chromium-153.0.1", "152.0.8"), Some(true));
    assert_eq!(compare("152.1.0", "152.0.8"), Some(true));
    assert_eq!(compare("152.0.7", "152.0.8"), Some(false));
    // 缺 patch 段按 0 处理(索引里出现过 `152.0` 形态时不该误判成"更新")。
    assert_eq!(compare("152.0", "152.0.8"), Some(false));
    assert_eq!(compare("152.0", "152.0.0"), Some(false));
    // 不可解析 ⇒ None(调用方按"无法判定"处理)。
    assert_eq!(compare("not-a-version", "152.0.8"), None);
    assert_eq!(compare("152.0.8", "unknown"), None);
}

/// 从索引 JSON 里取最新版本:忽略没有 `cef_version` 的条目,取版本最大的那个。
#[test]
fn latest_from_index_picks_highest_version() {
    let body = serde_json::json!({
        "macosarm64": {
            "versions": [
                { "cef_version": "152.0.8+g1ce985c+chromium-152.0.7977.134" },
                { "cef_version": "153.0.1+gabc+chromium-153.0.8000.0" },
                { "note": "没有 cef_version 的条目" },
            ]
        },
        "macosx64": { "versions": [] }
    });
    assert_eq!(
        latest_from_index(&body, "macosarm64").unwrap(),
        "153.0.1+gabc+chromium-153.0.8000.0"
    );
    // 平台不存在 / 列表为空 ⇒ 报错而不是 panic。
    assert!(latest_from_index(&body, "linux64").is_err());
    assert!(latest_from_index(&body, "macosx64").is_err());
}
