# 已知问题(KNOWN ISSUES)

> 记录在案、暂未修复的问题。修复前相关测试一律 `#[ignore]` 跳过,不纳入日常测试结果。
> 修复完成后:移除 `#[ignore]` 标记,更新本文件状态。

---

## 1. `test_deserialize_corrupted_guests` 永远失败(TEAM owner 语义被上游删除)

- **状态**:已修复(2026-07-31,本轮测试修复提交)
- **现象**:`cargo test -p warp --lib 'persistence::sqlite'` → 11 过 1 挂。`to_cloud_object_permissions()` 返回 `None`,测试期望 `Some(...)`。
- **根因**:上游同步提交 `9486252d4`(7/30 上游同步 73 提交)删除了 `owner_for_permissions` 中的 `"TEAM"` 分支(`Owner::Team`),TEAM 一律返回 `None`;但 `sqlite_tests.rs::test_deserialize_corrupted_guests` 仍使用 `subject_type: "TEAM"` 并断言旧语义(期望 `Owner::User`),测试未同步更新。
- **修复方式**:测试改为 `subject_type: "USER"`(上游删 TEAM 语义后 TEAM 场景不可能发生),断言 `Owner::User { user_uid: … }`,并移除 `#[ignore]`。
- **涉及文件**:`app/src/persistence/sqlite_tests.rs`(`test_deserialize_corrupted_guests`)

---

## 2. `workspace::view` 43 个测试全挂(AgentProviderSecrets 单例未注册)

- **状态**:已修复(2026-07-31,本轮测试修复提交)
- **现象**:`cargo test -p warp --lib 'workspace::view'` → 70 过 43 挂,全部同一 panic:
  ```
  Cannot get singleton model of type "warp::ai::agent_providers::secrets::AgentProviderSecrets" that was never registered
  ```
  (panic 于 `crates/warpui_core/src/core/app.rs:4597`)
- **根因**:本地 OMP 集成新增 `AgentProviderSecrets` 单例(自建模型/BYOP API key 存储),挂在 `App` 单例注册表;`app/src/workspace/view_test.rs` 的 `initialize_app` helper 未注册该单例。WarpUI 要求取单例前必须 `register_model`。
- **修复方式**(逐层补齐初始化,全部在测试基建内,不动产品代码):
  1. `view_test.rs::initialize_app`:`AgentProviderSecrets` 注册移到 `LLMPreferences::new`(构造时查询 secrets)之前。
  2. `test_util/settings.rs::initialize_settings_for_tests` 补注册缺失的 settings group:`LanguageSettings`、`NetworkSettings`、`CloudSyncSettings`、`AutoupdateSettings`、`UserAppInstallDetectionSettings`、`WarpDrivePrivacySettings`、`WorkflowAliases`。
  3. `view_test.rs` 补注册 `CLIAgentInstallModel`、`ProxyCredentials`、`CloudSyncTokenStore`。
  4. `view_test.rs` 补 `warp_ssh_manager::set_database_path`(SSH 管理器 DB 路径)。
  5. `initialize_settings_for_tests` 统一 `crate::i18n::init(Some("en"))` —— 顺带根治问题 3 的 flaky。
- **顺带修复**(同批暴露的测试过时断言):
  - `test_unified_new_session_menu_uses_new_worktree_config_label_and_order`:Zap 在新建会话菜单加入 Agent / Coding Agents / Docker 段,原断言"第一个分隔符后即 worktree config"失效;改为断言 worktree config 紧跟 new tab config(意图不变)。
- **结果**:`workspace::view` 104 passed / 0 failed(14 ignored 含问题 4 的 8 个 + 既有 6 个)。
- **涉及文件**:`app/src/workspace/view_test.rs`、`app/src/test_util/settings.rs`

---

## 3. `terminal_primary_line_falls_back_to_new_session` 单独跑必挂(i18n 未初始化)

- **状态**:已修复(2026-07-31,本轮测试修复提交)
- **现象**:`cargo test -p warp --lib 'workspace::view::vertical_tabs'` → 55 过 1 挂:
  ```
  left: "vertical-tabs-new-session"    (t!() 返回 key 本身)
  right: "New session"
  ```
  前置警告:`[i18n] t!(...) called before init(); returning key as-is`
- **根因**:Zap i18n 翻译表(`t!` 宏)需先初始化才能翻译;该测试单独跑(或模块过滤跑)时 i18n 从未初始化 → 返回 key 原样。全量跑时依赖其他测试碰巧初始化(全局状态),即**测试顺序依赖,flaky**。
- **修复方式**:测试内显式 `crate::i18n::init(Some("en"));`;同时 `initialize_settings_for_tests` 统一初始化(见问题 2 修复 5),根治同类 flaky。
- **涉及文件**:`app/src/workspace/view/vertical_tabs_tests.rs`、`app/src/test_util/settings.rs`

---

## 4. 8 个共享会话测试挂(Zap 切断共享会话网络链路)

- **状态**:已解决(2026-07-31)—— 采纳方案 (b),8 个测试及共享会话 mock helper 已删除
  (`app/src/workspace/view_test.rs`,commit 见本轮 `test: 删除共享会话遗留测试`);
  `workspace::view` 恢复 105 passed / 0 failed / 6 ignored(仅剩既有 ignore)
- **现象**:`workspace::view` 修复问题 2 后暴露,8 个测试失败:
  - `test_close_last_tab_skip_confirmation`、`test_close_other_tabs_confirmation_dialog`、`test_close_pane_confirmation_dialog`、`test_close_tab_confirmation_dialog`、`test_close_tabs_right_confirmation_dialog`、`test_confirmation_dialog_dont_show_again`、`test_reopen_closed_shared_tab`(均在 `setup_session_sharing_test` 的 `number_of_shared_sessions_in_tab == 0` 断言,view_test.rs:930)
  - `test_view_only_session`(terminal/input.rs:10191,`shared_session_status().is_viewer()` 断言)
- **根因**:Zap 本地化切断了 Shared Session 网络入口,`TerminalView::attempt_to_share_session`(app/src/terminal/view/shared_session/view_impl.rs:211)整体 no-op;测试 setup 调用它后不产生共享会话,后续断言无法成立。测试是上游共享会话功能的遗留,功能切断后未同步删除/重写。
- **影响**:低。仅测试失效;共享会话 UI 入口已随功能下线。
- **处理**:选 (b) 删除(功能已下线,回归价值低;mock 链路依赖 no-op 入口,重写成本高收益低)。
- **涉及文件**:`app/src/workspace/view_test.rs`(8 个被 ignore 的测试)、`app/src/terminal/view/shared_session/view_impl.rs`(`attempt_to_share_session`)
