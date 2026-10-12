//! 子代理模型解析的单测。

use super::resolve_subagent_model;
use ai::LLMId;

fn id(value: &str) -> LLMId {
    LLMId::from(value.to_owned())
}

/// **默认行为**:profile 里没配「子代理模型」时,与父会话完全一致。
#[test]
fn falls_back_to_parent_model_when_profile_has_no_subagent_model() {
    assert_eq!(
        resolve_subagent_model(None, None, Some(id("parent-model"))),
        Some(id("parent-model"))
    );
}

/// profile 里配了就用它,不再跟随父会话。
#[test]
fn configured_profile_model_overrides_parent_model() {
    assert_eq!(
        resolve_subagent_model(None, Some(id("subagent-model")), Some(id("parent-model"))),
        Some(id("subagent-model"))
    );
}

/// 工具调用里显式给的 `model` 优先级最高 —— 模型自己指定某个子任务用别的模型时,
/// profile 设置不能把它覆盖掉。
#[test]
fn explicit_model_wins_over_profile_setting() {
    assert_eq!(
        resolve_subagent_model(
            Some("explicit-model".to_owned()),
            Some(id("subagent-model")),
            Some(id("parent-model"))
        ),
        Some(id("explicit-model"))
    );
    // 空白串的显式 model 视为「没给」,继续往下走 profile/父模型。
    assert_eq!(
        resolve_subagent_model(
            Some("   ".to_owned()),
            Some(id("subagent-model")),
            Some(id("parent-model"))
        ),
        Some(id("subagent-model"))
    );
    assert_eq!(
        resolve_subagent_model(Some("   ".to_owned()), None, Some(id("parent-model"))),
        Some(id("parent-model"))
    );
}

/// 显式 model 要裁掉前后空白,否则会把带空格的脏 id 传下去。
#[test]
fn explicit_model_is_trimmed() {
    assert_eq!(
        resolve_subagent_model(Some("  explicit-model  ".to_owned()), None, None),
        Some(id("explicit-model"))
    );
}

/// 父会话模型也拿不到时返回 None(调用方据此跳过模型覆盖,沿用子 pane 的默认)。
#[test]
fn returns_none_when_no_source_available() {
    assert_eq!(resolve_subagent_model(None, None, None), None);
    assert_eq!(
        resolve_subagent_model(Some("   ".to_owned()), None, None),
        None
    );
}

/// 跨 provider 的模型 id 要原样透传(`LLMId` 字符串自带 provider 维度)。
#[test]
fn passes_through_provider_qualified_ids() {
    assert_eq!(
        resolve_subagent_model(
            None,
            Some(id("commandCode/deepseek/deepseek-v4-flash")),
            None
        ),
        Some(id("commandCode/deepseek/deepseek-v4-flash"))
    );
}
