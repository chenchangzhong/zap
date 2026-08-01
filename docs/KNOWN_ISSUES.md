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

## 4. 共享会话测试挂(Zap 切断共享会话网络链路)

- **状态**:已解决(2026-07-31)—— 云服务相关测试全部删除(用户拍板:Zap 无云服务功能)
- **首批删除**(commit `89040c1f6`):`workspace::view_test.rs` 8 个共享会话测试 + 4 个 mock helper;
  `workspace::view` 恢复 105 passed / 0 failed / 6 ignored。
- **根因**:Zap 本地化切断了 Shared Session 网络入口,`TerminalView::attempt_to_share_session`
  (app/src/terminal/view/shared_session/view_impl.rs:211)整体 no-op;共享会话测试 setup 依赖它
  建立共享会话,功能切断后断言无法成立。
- **本批删除(commit 见本轮)**:`terminal::shared_session` 与 `terminal::view::shared_session`
  全部测试文件(5 个:mod_test / view_impl_test / presence_manager_test / selections_test /
  test_utils),30+ 测试:
  - web URL → shared session intent 改写 3 个(web_intent_parser 已无 shared_session 分支)
  - viewer UI 5 个(banner / resize / context menu / tombstone;viewer 链路生产不可达,
    且 on_session_share_ended 的 tombstone 插入已随 CloudModeSetupV2 退役删除)
  - presence manager / selections 测试(共享会话组件,云功能)
  - 同步清理:`max_session_size` 的 `#[cfg(test)]` 版本与 `pub use tests::MAX_BYTES_SHAREABLE`(死桥)
- **云相关单测清理**:
  - 删 `cloud_object::model::persistence::tests::test_shared_team_object`(Owner::mock_current_user
    断言 Shared,与 Zap 语义矛盾:本地 Owner 归 Personal;团队空间语义 has_teams=false 已死)
  - 适配 `drive::index::tests::test_retry_menu_item_visibility`(Zap 删 drive-share 菜单项、加
    Trash 项;断言改为 Edit / Copy workflow text / Duplicate / Export / Trash,Retry 可见性逻辑保留)
- **涉及文件**:`app/src/terminal/shared_session/*`(5 个测试文件删除 + 3 处声明清理)、
  `app/src/terminal/view/shared_session/*`、`app/src/cloud_object/model/model_test.rs`、
  `app/src/drive/index_test.rs`
