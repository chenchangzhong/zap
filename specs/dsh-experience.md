# dsh 集成全量经验（Zap）

> 截至 2026-08-20，分支 `feat/dsh-webui-integration`，验证：dsh 切项目 badge/面板实时刷新、非 git 不误报。

## 1. 总览

- dsh 独立 web pane（webview），内多项目随意切，`sessions/workspaces` 快照权威 `cwd`。
- Zap 侧单 pane 窗口，`Workspace.dsh_git_status: Option<ModelHandle<GitRepoStatusModel>>` 自持数据源，不依赖 terminal。
- 已去 `local_fs` 门控（`RightPanelView/dsh_repo` 全量开放），`crates/warpui` 合成 B 探已定点 `notify→present` 自动。

## 2. 通信（IPC-only，WebSocket 已删）

- 客户端：`@zap/zap-bridge-client`（目录 `@zap/`，`package.json name zap-bridge-client` 无 scope，`cordis.patch.yml name: '@zap/zap-bridge-client'` 包名、`require.resolve('<name>/package.json')`）。
- `window.__ModuleLoader__.load({id:"@zap/zap-bridge-client", inject:["sessions","workspaces"]})`，`subscribe reportCurrentPath -> currentWorkspacePath(cwd) -> zapRpc('zap.switch_project')`，`lastReportedPath` 去重，`pendingRequests Map`。
- Rust：`browser_web_view.rs` `DSH_WEBVIEW_IDS` 白名单，`body.strip_prefix("zap:")` 验 id -> `handle_zap_ipc(payload method\nid\nparams_json)`，`canonicalize+is_dir` 校验 -> `set_workspace_dir` -> `push_event(SwitchProject)` -> `request_redraw_all_windows()`（首跳排帧，push 需触发 drain）。
- `bridge.rs` `PENDING_EVENTS LazyLock<Mutex<Vec<BridgeEvent>>> {Notify,SwitchProject,Ready,Updating,Restarted,Failed}`，`lib.rs on_frame_drawn drain_events -> ModelContext::emit`，`has_pending_events` fallback `window.request_redraw()`。
- `runtime.rs install_client_plugin(dsh_home)` 写 `node_modules/@zap/zap-bridge-client/{index.js,client.js,package.json}` + `profiles/web/cordis.patch.yml`。
- 坑：`name: '.../index.js'` 文件路径 `require.resolve FAIL` 静默不入表；`id` 必须 `@zap/zap-bridge-client`；`evaluate_script_on` wry borrow 闪退 -> 静写 cordis。
- 启动链：`Workspace::new` 3 订阅 `DshRuntime`（Ready/Restarted/Updating, Notify->NotificationsModel::add_dsh_notification, SwitchProject->handle_dsh_switch_project），`DshRuntime dsh_data_dir` + `ensure_dsh_installed` + `dsh web --port 0` + `wait_until_ready poll child / lsof 127.0.0.1`。

## 3. badge 实时刷新

- 基线 `render_right_panel_button:16148` 原只读 `active_session_view.current_diff_line_changes`，dsh 无 terminal -> None。
- 本改：`Workspace.dsh_git_status` + `update_dsh_git_status_subscription`，`GitStatusUpdateModel::subscribe(&dir)` + `canonicalize(repo_path)==dir` 防父 repo 残留，`subscribe ok -> refresh_metadata`（缓存命中补刷）-> `subscribe_to_model MetadataChanged -> ctx.notify`（`notify` 即 `present`，B 已去冗余 `request_redraw`）。
- 判定：`is_dsh_active = active_tab_pane_group().dsh_panes().next().is_some()` pane-group 级（分栏 dsh+terminal 焦点 terminal 也跟 dsh），`dsh_line_changes.filter>0` 独占，不 `or(tv)`（防非 git 串 terminal 数）。
- 订阅生命周期：`clear_dsh_git_status` helper `take old -> unsubscribe_to_model(&old) -> set_dsh_repo(None)`，`repo_matches` 前 `take` 去重（缓存同 handle 多次 subscribe 累加，多次 notify）。
- 初：`on_frame_drawn` 首帧 `request_redraw_all_windows` 被 drain 消耗，第二帧 `MetadataChanged notify` 再 `request_redraw` 补，否则空档 `notify != present` hover 才 `CATransaction`。B 后第二跳无需补，`notify`自动`present`。
- 验证：`zap.log MetadataChanged×4` `subscribe ok 3192` `Set available_branches 4`，切 `zap <-> dsh-plugins(非git) <-> zap` 无 hover 即刷。

## 4. 面板非 git 提示

- `RightPanelView dsh_repo_path: Option<PathBuf>`，`set_dsh_repo(Some)` 先 `set_selected_repo`，`get_or_create_diff_state_model` -> `open_code_review(None,true)` `is_dsh=true` 跳 `has_active_repos`，`render_panel_content filter available.contains || dsh==dsh_repo` 放行。
- `set_dsh_repo(None)` 仅当 `selected == old_dsh` 才 `close+None`（防后台订阅失败误伤手选普通 repo）。
- `setup_code_review_panel` 原 `context_data None -> close_code_review`（dsh 无 terminal），现回退 `dsh_repo_path` 建 view，`diff_state None` 保留 selected 不 close。
- `create_code_review_view is_dsh` 跳 `has_active_repos`，`terminal_view Option<WeakViewHandle<TerminalView>>`。
- `open_dsh_pane` 后补 `update_dsh_git_status_subscription` 解首次无 SwitchProject。
- 去门控：`RightPanelView` 字段/构造 `local_fs` 去 `cfg`，dsh 全量开放；`setup/setup_code_review` 为 `feature=local_fs` 但 dsh 自己开放已够，`WorkingDirectoriesModel` 底座 `local_fs` 空桩不影响 dsh 侧差分。

## 5. 合成层（warpui）

- `ctx.notify()` dirty Workspace 已 render，但 `CAMetalLayer presentDrawable` 未提交，空闲无 `DisplayLink` 便车。
- 已试全败回滚：`postEvent/JS动画/全量view失效/host_view displayLayer/setNeedsDisplay dispatch_async/browser动画/focus递归/update_windows_called/[self displayIfNeeded]->NSWindow drawRect空+DuringViewResize+Metal` 闪退。
- 本轮初 `ipc后 request_redraw + MetadataChanged后 request_redraw` 二跳，B 根治后 `MetadataChanged` 仅 `notify`，首跳 `request_redraw` 保留以触发 `drain_events`，`has_pending` 兜底可删。
- `crash_recovery` 原 `user_is_logged_in` 分支注册 `on_frame_drawn`（覆盖式单回调），合并到 `BrowserPane||DshPane` 门控致登录态丢失 -> 改无条件注册单回调，内分支 `if user_is_logged_in { crash_recovery }` + `if BrowserPane||DshPane { webview/dsh }`，`move` 捕获 `user_is_logged_in bool`。

## 6. 上游与门控

- 上游无 dsh，`2fe9d43ca stale chip -> set_git_repo_status clear_abort+clear_cache+update_tx+emit TerminalViewStateChanged` 与 `99a8e5090 harness header update_pane_configuration` 仅作参考，未完全对齐时仍不刷说明缺口在合成。
- `is_dsh_pane => IPaneType::DeepSeek` 单 pane 深 seek 类型同源，`should_enable_file_tree || is_dsh_pane()` 放行文件树/搜索。
- 构建：`cargo check default/local_fs` 0 error，`cargo build -p warp --bin zap-oss` 产 debug，重启 `target/debug/zap-oss`（日志 02:26 新 window），旧 `/Applications/Zap.app` 仍旧版。

## 7. 已验证链

- dsh 切 `zap(git, +N/-N)` -> `zap-bridge-client js subscribe -> IPC SwitchProject /Users/zhong/project/zap -> subscribe ok 3192 -> MetadataChanged×4 -> Branches 4 -> wrap_files subscribe -> badge + 面板 diff` 实时。
- 切 `Users/zhong(非git) subscribe failed No watched repository -> badge 无 -> 面板空态不 Cannot detect`。
- 单 pane 约束：关 pane 不清 watcher 视为 P3（Workspace 销毁清），无需 close hook。

## 8. 遇到的全部错误与最终修复

> 按时间顺序，含编译期/运行时/合并审查错误。

### 8.1 编译期

| # | 现象 | 根因 | 修复 | 影响范围 |
|---|------|------|------|----------|
| E1 | `use tab_configs::{SidecarItemKind,...}` 误删 8 行 → 37 error | `dsh_git_status` 字面量重建时误删 import | 恢复该 8 行，`cargo check 0 error, 29 warnings` | `view.rs` |
| E2 | `unresolved import` after `local_fs` 去 `cfg` → 1 error | `RightPanelView` 字段仍 `#[cfg(feature="local_fs")]` | 去字段/构造 `cfg`，全量开放 | `right_panel.rs` |
| E3 | `unexpected closing delimiter: } at lib.rs:1824` → 1 error | `on_frame_drawn` 合并时 `});}` 大括号缺一 | 补 `}`→`});`，并改无条件注册单回调 | `lib.rs` |
| E4 | `E0599: no method named unsubscribe_from_model` 2处 | warpui 用 `unsubscribe_to_model`，非 `from` | `replaceAll('unsubscribe_from_model','unsubscribe_to_model')` | `view.rs` |
| E5 | `E0061: takes 6 args but 5 supplied at right_panel.rs:1675` | `create_code_review_view` 加 `is_dsh` 未补旧调用 | `Some(terminal_view.downgrade()), false` 补参 | `right_panel.rs` |
| E6 | `set_available_repos_preserving` 无调用点 死代码 | 中间补丁遗留，`git checkout --` 后重建未删 | 删除该方法 | `right_panel.rs` |
| E7 | `missing open ( for delimiter at lib.rs:1528` | 同 E3 | 同修 | `lib.rs` |

### 8.2 运行时 / 功能

| # | 现象 | 根因 | 修复 | 验证 |
|---|------|------|------|------|
| R1 | badge 无数据：dsh pane group 无 terminal → `None` | `render_right_panel_button` 只读 terminal | 新增 `Workspace.dsh_git_status` 独占数据源 | 终端 100% 跟随，dsh 同链 |
| R2 | badge 误串：dsh 非 git 仍 `+N/-N` 另一仓库 | `dsh.or(tv)` fallback 退 terminal | 改 `is_dsh_active` 独占分支（group 级 `dsh_panes().next()`） | 切 `dsh-plugins` 无 badge |
| R3 | 面板 `Cannot detect`：`get_or_create None` → return 未选 | `set_dsh_repo None` 回退未 `set_selected` | 设 `dsh_repo_path` + `render filter` 放行 + `is_dsh` 跳 `has_active_repos` | 切 zap 面板正常 |
| R4 | 切非 git 误清用户手选普通 repo | `set_dsh_repo(None)` 无条件 `selected=None` | 仅 `selected==old_dsh` 才清 | 后台失败不误伤 |
| R5 | badge 不实时：数据通但不上屏，hover 才刷 | `notify != present`，第二帧无 `request_redraw` | **workaround**：`MetadataChanged ctx.notify+request_redraw_all_windows`+`ipc后 request_redraw`二跳通；**B 根治**：`warpui_core ViewContext::notify→pending_effects→notify_view_observers→window_invalidations→flush_effects→update_windows→on_window_invalidated→window.request_redraw`自动，删回调内手工 `request_redraw_all_windows`冗余，仅`ctx.notify`即`present` | 无 hover 即刷 |
| R6 | 面板点按钮仍 `非git`：`setup_code_review_panel` 无 terminal → `close` | `context_data None -> close_code_review` | 回退 `dsh_repo_path` 建 view，`diff_state None` 不 close | 点按钮面板正常 |
| R7 | 首次打开 dsh 无 SwitchProject → 无订阅 | `open_dsh_pane` 闭包后未补 | 补 `update_dsh_git_status_subscription` | 首载即有 |
| R8 | 父 repo 污染：subscribe 返回父仓库 model | `subscribe` 可返回父 git 仓库 | `canonicalize(repo_path)==dir` 校验，不等则 `clear` | 非 git 失败符合预期 |

### 8.3 合成层尝试（全部闪退/无效，已回滚）

| 尝试 | 结果 | 结论 |
|------|------|------|
| `postEvent` 合成事件 | 无 | — |
| JS 动画 / 全量 view 失效 / `host_view displayLayer` / `setNeedsDisplay:YES` / `dispatch_async` / browser 动画 / focus 递归 / `update_windows_called` | 无 | — |
| `[self displayIfNeeded]` → `NSWindow drawRect 空 + DuringViewResize + Metal` | **闪退**，`python /tmp/revert_inject.py` 收敛 | 不碰 `WarpWindow displayIfNeeded` |
| 结论 | 全闪退，用户确认 `crates/warpui/**` 不在计划，数据正确为前提，合成另案 | 本轮先`request_redraw`二跳+`has_pending`兜底通，B探后确认`notify→present`自动，删workaround，`notify`即`present` |

### 8.4 通信链路

| 尝试 | 结果 | 修复 |
|------|------|------|
| BridgeServer WebSocket + `BRIDGE_INFO/token` + `zap-bridge.ts` host | 端口随机不稳 | 删 WebSocket，重写 `bridge.rs` IPC-only |
| `evaluate_script_on` 在 `pane.rs UrlChanged` 同步注入 `__ModuleLoader__` | **闪退**（wry borrow + 未就绪） | 回滚，改静写 cordis patch |
| patch `name:'.../index.js'` 文件路径 vs 包名 | `require.resolve FAIL` 不入表 | 改 `name: '@zap/zap-bridge-client'` 包名 OK |
| `client.js id:"zap-bridge-client"` vs graph id | `bundle loaded without registering` | 改 `id:"@zap/zap-bridge-client"` 后通 |

### 8.5 合并审查发现（7 reviewer 并行，P1 必修）

| 审查项 | 级别 | 现象 | 修复 |
|--------|------|------|------|
| `lib.rs crash_recovery` 被 `BrowserPane\|\|DshPane` 门控包裹 | **P1** | 登录态崩溃恢复丢失 | `on_frame_drawn` 无条件注册，内分支 `if user_is_logged_in` |
| `MetadataChanged` 订阅重复累加 | **P1** | 同 handle 缓存命中多次 `subscribe_to_model` → 多次 `notify/request_redraw` | `clear_dsh_git_status` 前 `take+unsubscribe_to_model` 去重 |
| `set_dsh_repo(None)` 无条件清 selected | **P1** | 误伤用户手选普通 repo | `selected==old_dsh` 守卫 |
| `has_pending_events` fallback 冗余 | P2 | 与 `handle_zap_ipc request_redraw` 重复，但无死循环 | 保留，无害兜底 |
| `clear` 分支双 `notify` | P3 | `clear_dsh_git_status` 内 `notify` + 函数末 `notify` 重绘两次 | 去末尾冗余，仅 `repo_matches` 分支 `notify` |
| 关 DshPane 未清 watcher | P2 | handle 残留至下次 SwitchProject | 单 pane 约束（仅1窗），随 Workspace 销毁清，**不修** |
| badge `focused_pane_id` 分栏不一致 | P2 | focus terminal 回落 terminal 数 | 改 pane-group 级 `dsh_panes().next()` |
| `setup_code_review None` 回退 close | P2 | `diff_state None` 误清已选 | `diff_state None` 不 close，保留 selected |

### 8.6 更早记忆（手工上下文）

- `diff_layout.rs` 纯内存，不引 schemars
- 通知：`TECH-notify.md` 未提交，`BridgeEvent::Notify` + `NotificationSourceAgent::Dsh`
- `git checkout -- .` 丢 working tree（无 stash），含 dsh badge 整段，177 行 PLAN 重建
- Watcher 验证缺口：从未 `touch` 文件看 badge 上屏
- `zap-bridge.ts` 旧 56/-187 行批改在 topic 外

## 9. B 探根治

- 定点 `warpui_core/src/core/view/context.rs:452 notify→app.pending_effects ViewNotification`→`app.rs:3826 notify_view_observers→window_invalidations`→`3205 update_windows→invalidation_callbacks→platform window.request_redraw()`。`notify`已自动排帧，无需手工。
- 已删 `MetadataChanged`内`request_redraw_all_windows`冗余；`lib.rs has_pending`兜底可删，或保留无害。
- 改动：`view.rs` 1行删 + `lib.rs crash_recovery`无条件单回调（`move`捕获`user_is_logged_in`）。

## 10. 风险与下一步

- 合成已根治：`notify`自动`present`，`request_redraw`仅首跳保留，无另案。
- 下一步：面板多项目历史/通知 `BridgeEvent::Notify` 入 `NotificationsModel` 已通，侧边多 dsh 扩展另案。

## 11. 启动失败可见性与早退检测（2026-09-12）

- 早退检测：`runtime.rs wait_until_ready(child, log_path)` 每轮（500ms）先 `child.try_status()`，子进程退出立即 bail，不再空转到 `STARTUP_TIMEOUT`（120s）后把真实原因掩盖成「就绪超时」。前提：`dsh web` 不 daemonize（常驻单进程，`ps` 实测），故「退出 = 服务已死」成立；若将来改成 fork+detach（父进程 exit 0），需加「父进程 exit 0 且日志已有 ready URL 时继续探测」的宽容分支。
- 原因提取：`extract_fatal_error` 取日志**首条**含 `Error: ` 的行（最外层，嵌套 cause 都在其后），单行截断 500 字符；日志每次启动 `File::create` 截断 ⇒ 不会串到上次运行的 Error 行。失败原因存 `DshRuntime.error`（`set_failed` 原子写原因+状态，`begin_start`/`begin_restart`/`request_stop` 清空），三条失败路径（启动失败 / 重启放弃 / 连续崩溃放弃）与「失败后新建/恢复 pane」的 attach 路径共用同一来源。崩溃放弃不再用 `repeated crashes` 占位串，`crash_failure_reason()` 取本次日志；取不到时给「无错误行 + 日志路径」。
- pane 侧：失败覆盖层展示错误原文 + 「修复错误」（复制到剪贴板并附加到终端 Agent 输入框；命中活动输入框复用 `insert_in_input`，无终端时新开 Agent 模式 tab）；「重新启动」用 `runtime_restarting` 态立即回「启动中」，并复位 `webview_loaded`/`load_started_at`/`webview_crashed`，避免露出已死实例的旧页面；Ready/Restarted 时重新计加载时钟，避免重启 >15s 被 `WEBVIEW_LOAD_TIMEOUT` 兜底提前揭页。失败 toast 已删（面板承接原因与重启/修复入口）。
- **测试夹具（验失败路径，验后必须还原）**：改 `~/.dsh/profiles/zap/node_modules/dsh-rewind-plugin/lib/index.js`——zap 专属 profile，**不影响终端自用 dsh 与 DSH Desktop**。
  - ✅ 有效：`import { __zapTestMissingExport } from "@deepseek-ai/dsh-llm";` —— **链接期**缺导出 → `Error: dsh: plugin tree failed to load: ... does not provide an export named ...` → node 退出（status 1），与真实故障（rewind 插件与 dsh 版本不匹配）同形。
  - ❌ 无效：顶层 `throw new Error(...)` —— 运行时异常被 cordis loader 吞掉，dsh 照常启动（实测 15s 内无报错、也没 ready URL，容易误判成夹具生效）。
  - 改法：先 `cp` 备份（记 sha256）再整份覆盖，**别原地追加**：pnpm 目录若是硬链接，原地写会污染 store（本例 `lib/index.js` links=1，安全）。还原后 `shasum -a 256` 必须与备份一致。
- 预存不对称（非本次引入）：`DshStartResult::Failed` 回调不校验 generation（`workspace/view.rs open_dsh_pane`、`lib.rs` 重启回调），慢失败的上一代启动可把新启动的 status 打成 `Failed`，改动后还会把上一代的 error 显示出来。

## 12. 修订

- 2026-08-20 订阅去重 + 关 pane 不误伤 + badge group 判定 + setup 回退 + 双 notify 去冗 + lib 合并 + 右面板去 local_fs。
- 2026-09-12 启动失败立即报真实原因（早退检测 + 原因提取 + 崩溃放弃取日志）＋面板「修复错误」/重启回启动中/删失败 toast；补插件夹具两形态经验。
