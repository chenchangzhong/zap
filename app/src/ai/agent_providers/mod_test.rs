//! Smoke tests for BYOP provider configuration and lookup, plus
//! {{session_id}} placeholder substitution and preset header seeding.

use ai::LLMId;
use settings::Setting;
use warpui::{App, SingletonEntity};

use crate::ai::agent_providers::models_dev::{
    filter_catalog, popular_only, preset_extra_headers, Catalog, Provider, POPULAR_PROVIDER_IDS,
};
use crate::ai::agent_providers::{
    llm_id, lookup_byop, substitute_session_id, AgentProviderSecrets, SESSION_ID_PLACEHOLDER,
};
use crate::ai::llms::{DisableReason, LLMPreferences};
use crate::auth::{AuthManager, AuthStateProvider};
use crate::network::NetworkStatus;
use crate::settings::{AISettings, AgentProvider, AgentProviderApiType, AgentProviderModel};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ObjectStoreModel;
use crate::ai::mcp::TemplatableMCPServerManager;

fn sample_provider(id: &str) -> AgentProvider {
    AgentProvider {
        id: id.to_owned(),
        name: "Test Ollama".to_owned(),
        kind: Default::default(),
        api_type: AgentProviderApiType::Ollama,
        base_url: "http://localhost:11434".to_owned(),
        models: vec![AgentProviderModel::from_id("llama3.2".to_owned())],
        extra_headers: Vec::new(),
    }
}

fn init_byop_test_app(app: &mut warpui::App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(AgentProviderSecrets::new);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|ctx| {
        crate::ai::execution_profiles::profiles::AIExecutionProfilesModel::new(
            &crate::LaunchMode::new_for_unit_test(),
            ctx,
        )
    });
    app.add_singleton_model(LLMPreferences::new);
}

#[test]
fn smoke_build_byop_models_by_feature_exposes_configured_models() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        let provider_id = "provider-smoke-1";
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider(provider_id)], ctx);
            });
        });

        app.read(|ctx| {
            let choices: Vec<_> = LLMPreferences::as_ref(ctx)
                .get_base_llm_choices_for_agent_mode()
                .collect();
            assert_eq!(choices.len(), 1, "expected one BYOP model in picker");
            assert!(
                choices[0].disable_reason.is_none(),
                "valid provider should not be disabled"
            );
            assert_eq!(
                choices[0].id.as_str(),
                llm_id::encode(provider_id, "llama3.2").as_str()
            );
        });
    });
}

#[test]
fn smoke_build_byop_models_by_feature_uses_placeholder_when_misconfigured() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        app.read(|ctx| {
            let default = LLMPreferences::as_ref(ctx).get_default_base_model();
            assert_eq!(
                default.disable_reason,
                Some(DisableReason::Unavailable),
                "empty config should surface placeholder entry"
            );
        });
    });
}

#[test]
fn smoke_build_byop_models_by_feature_skips_empty_base_url() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let mut broken = sample_provider("broken");
                broken.base_url.clear();
                let _ = settings.agent_providers.set_value(vec![broken], ctx);
            });
        });

        app.read(|ctx| {
            let default = LLMPreferences::as_ref(ctx).get_default_base_model();
            assert_eq!(
                default.disable_reason,
                Some(DisableReason::Unavailable),
                "provider with empty base_url must not appear as selectable model"
            );
        });
    });
}

#[test]
fn smoke_lookup_byop_resolves_provider_and_model_without_api_key() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        let provider_id = "provider-lookup-1";
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider(provider_id)], ctx);
            });
        });

        let encoded = llm_id::encode(provider_id, "llama3.2");
        app.read(|ctx| {
            let (provider, api_key, model_id) =
                lookup_byop(ctx, &encoded).expect("lookup_byop should resolve configured model");
            assert_eq!(provider.id, provider_id);
            assert_eq!(model_id, "llama3.2");
            assert!(api_key.is_empty(), "Ollama path allows empty API key");
        });
    });
}

#[test]
fn smoke_lookup_byop_returns_none_for_unknown_id() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        app.read(|ctx| {
            assert!(lookup_byop(ctx, &LLMId::from("byop:missing:model")).is_none());
            assert!(lookup_byop(ctx, &LLMId::from("not-byop")).is_none());
        });
    });
}

#[test]
fn substitute_session_id_replaces_all_occurrences() {
    let headers = vec![
        (
            "x-opencode-session".to_owned(),
            SESSION_ID_PLACEHOLDER.to_owned(),
        ),
        ("x-prefixed".to_owned(), "zap-{{session_id}}-a".to_owned()),
        ("x-static".to_owned(), "static-value".to_owned()),
    ];
    assert_eq!(
        substitute_session_id(headers, "conv-1"),
        vec![
            ("x-opencode-session".to_owned(), "conv-1".to_owned()),
            ("x-prefixed".to_owned(), "zap-conv-1-a".to_owned()),
            ("x-static".to_owned(), "static-value".to_owned()),
        ]
    );
}

#[test]
fn substitute_session_id_keeps_header_name_untouched() {
    let headers = vec![("{{session_id}}".to_owned(), "{{session_id}}".to_owned())];
    assert_eq!(
        substitute_session_id(headers, "conv-1"),
        vec![("{{session_id}}".to_owned(), "conv-1".to_owned())]
    );
}

#[test]
fn preset_extra_headers_seeds_opencode_gateways_only() {
    let expected = vec![(
        "x-opencode-session".to_owned(),
        SESSION_ID_PLACEHOLDER.to_owned(),
    )];
    assert_eq!(preset_extra_headers("opencode"), expected);
    assert_eq!(preset_extra_headers("opencode-go"), expected);
    assert!(preset_extra_headers("deepseek").is_empty());
    assert!(preset_extra_headers("").is_empty());
}

fn catalog_with(id: &str) -> Catalog {
    let mut catalog = Catalog::default();
    let mut provider = Provider::default();
    provider.name = id.to_uppercase();
    catalog.insert(id.to_owned(), provider);
    catalog
}

#[test]
fn filter_catalog_only_returns_popular_providers() {
    let mut catalog = Catalog::default();
    for id in POPULAR_PROVIDER_IDS {
        catalog.extend(catalog_with(id));
    }
    // 混入若干非热门提供商,必须全部不可见。
    for id in ["moonshotai", "azure", "subconscious"] {
        catalog.extend(catalog_with(id));
    }

    // 空 query:只返回白名单条目,且按白名单声明顺序。
    let all = filter_catalog(&catalog, "");
    let ids: Vec<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, POPULAR_PROVIDER_IDS.to_vec());

    // 搜索命中非热门 id / name 也不返回(预设已删除,搜索只在白名单内)。
    assert!(filter_catalog(&catalog, "moonshot").is_empty());
    assert!(filter_catalog(&catalog, "SUBCONSCIOUS").is_empty());

    // 搜索按 id 与 name 匹配,仍只限白名单。
    let by_name = filter_catalog(&catalog, "DEEPSEEK");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].0, "deepseek");
    let by_id = filter_catalog(&catalog, "openrouter");
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].0, "openrouter");
}

#[test]
fn filter_catalog_skips_popular_ids_missing_from_catalog() {
    // catalog 缺失某热门条目时跳过而不是报错。
    assert!(filter_catalog(&Catalog::default(), "").is_empty());
    let by_name = filter_catalog(&catalog_with("openai"), "OpenAI");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].0, "openai");
}

#[test]
fn popular_only_drops_non_popular_providers() {
    let mut catalog = Catalog::default();
    for id in POPULAR_PROVIDER_IDS {
        catalog.extend(catalog_with(id));
    }
    for id in ["moonshotai", "azure", "subconscious"] {
        catalog.extend(catalog_with(id));
    }

    let filtered = popular_only(&catalog);
    assert_eq!(filtered.len(), POPULAR_PROVIDER_IDS.len());
    assert!(filtered.contains_key("openai"));
    assert!(filtered.contains_key("opencode-go"));
    // 非热门条目在数据入口即被剔除,lookup_caps / 模型同步对其不再可见。
    assert!(!filtered.contains_key("moonshotai"));
    assert!(!filtered.contains_key("azure"));
    assert!(!filtered.contains_key("subconscious"));

    // 空目录安全。
    assert!(popular_only(&Catalog::default()).is_empty());
}
