//! `AgentProviderSecrets::set` 未变化短路行为的回归测试。
//!
//! 短路规则:同值重复设置、空串删除不存在的键,不再 emit 也不再落盘;
//! 首次设置、改值、空串删除已存在的键仍走完整路径(删除语义不短路)。
//! emit 与落盘在同一分支,这里用订阅计数断言 emit,配合 `get` 断言内存语义。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use warpui::{App, SingletonEntity};

use crate::ai::agent_providers::AgentProviderSecrets;
use crate::settings::AISettings;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn set_short_circuits_unchanged_values_but_keeps_delete_semantics() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(AgentProviderSecrets::new);

        let counter = Arc::new(AtomicUsize::new(0));
        app.update(|ctx| {
            let counter = counter.clone();
            // 自订阅被禁止(debug_assert),挂在 AISettings 实体上订阅 secrets。
            AISettings::handle(ctx).update(ctx, |_, ctx| {
                ctx.subscribe_to_model(
                    &AgentProviderSecrets::handle(ctx),
                    move |_, _, _event, _| {
                        counter.fetch_add(1, Ordering::SeqCst);
                    },
                );
            });
        });

        app.update(|ctx| {
            AgentProviderSecrets::handle(ctx).update(ctx, |secrets, ctx| {
                secrets.set("p1", "k1".to_owned(), ctx); // 首次设置:+1
                assert_eq!(secrets.get("p1"), Some("k1"));
                secrets.set("p1", "k1".to_owned(), ctx); // 同值重设:短路
                secrets.set("p1", "k2".to_owned(), ctx); // 改值:+1
                assert_eq!(secrets.get("p1"), Some("k2"));
                secrets.set("p1", String::new(), ctx); // 空串删除已存在的键:+1
                assert_eq!(secrets.get("p1"), None);
                secrets.set("p1", String::new(), ctx); // 空串删除不存在的键:短路
                secrets.set("p2", String::new(), ctx); // 为不存在的键设空串:短路
                secrets.set("p2", "k9".to_owned(), ctx); // 第二个键首次设置:+1
            });
        });

        assert_eq!(counter.load(Ordering::SeqCst), 4);
    });
}
