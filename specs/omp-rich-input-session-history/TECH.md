# TECH.md — Rich Input 历史菜单读取 omp 会话用户消息

## 概述

rich input 的 ↑ 历史菜单(`InlineHistoryMenuDataSource`)在 CLI agent 为 omp(`CLIAgent::OhMyPi`)时,改为展示当前 omp 会话的用户消息;输入 `/` 时 slash 菜单优先接管。全部改动在客户端本地,只读 omp 磁盘数据,Zap 零持久化。

## 数据源:omp 会话文件

omp 把每个会话写为 `~/.omp/agent/sessions/-<cwd>/<ts>_<session_id>.jsonl`,每行一个 JSON 事件。用户消息形如:

```json
{"type":"message", ..., "message":{"role":"user","content":[{"type":"text","text":"..."}]}}
```

`-<cwd>` 目录名由会话 cwd 派生(如 `/Users/zhong/project/zap` → `-project-zap`);文件名 `<ts>_<id>.jsonl` 中的 id 是**会话存储 id**。

### 关键事实:两套 id 体系

omp 的 OSC777 `session_start` 上报的 `session_id` 是**运行实例 id**(每次启动新值),与 jsonl 文件名中的会话存储 id **并非同一体系**;resume 旧会话时 omp 继续写历史 jsonl 但上报新 id。因此不能靠 session_id 直接定位文件。

### 定位策略(三级)

`omp_session_history::read_omp_user_messages(session_id: Option<&str>, cwd: Option<&str>) -> Option<Vec<OmpUserMessage>>`:

1. **session_id glob**:匹配 `~/.omp/agent/sessions/*/*_<id>.jsonl`(id 体系一致时最精确);
2. **tty 映射**:`~/.omp/agent/terminal-sessions/ttys*` 文件内容为 `cwd\njsonl路径\n状态`,omp 在新建/恢复会话时更新;
   - cwd 匹配命中多个时,按**映射文件 mtime** 最新排名(不是 jsonl mtime——新会话 jsonl 惰性落盘,mtime 不可用);
   - cwd 无匹配时,取映射文件 mtime 最新(最近切换过会话的终端大概率是当前操作的);
3. 全部 miss → `None`。

返回语义:
- `Some(vec)` = 定位到 omp 会话(新会话映射指向未落盘 jsonl 时为空 vec);
- `None` = 非 omp / 无法定位 / 文件存在但读取失败(损坏)→ 调用方回退其它历史。

### 解析

`parse_omp_session_file` 按行处理,快速跳过非 `{"type":"message"` 前缀的行(assistant/tool 事件是大头,避免全量 serde 解析),只提取 `role == "user"` 的 text 块,trim 非空,时间戳 RFC3339 解析失败时沿用上一条已解析消息的时间戳(排序稳定)。文件存在但 `read_to_string` 失败返回 `None`(回退),与"新会话未落盘 → `Some(空)`"严格区分。

## 菜单集成

`app/src/terminal/input/inline_history/data_source.rs`:

- `MenuItem` 新增 `AIPrompt { query_text, display_timestamp, prefix_match_len }`,渲染复用既有 `InlineHistoryItem::ai_prompt`(Select 回填 / Enter 提交链路已有);
- `build_omp_prompt_entries(trimmed_query, prefix_match_len, app) -> Option<Vec<MenuEntry>>`:
  - session 不存在 / `agent != CLIAgent::OhMyPi` → `None`;
  - cwd 取 `session_context.cwd`(OSC777 启动 cwd,与 tty 映射同源)优先,`ActiveSession::current_working_directory()`(shell 实时 cwd,随 cd 漂移)兜底;
  - 前缀过滤后构造 `MenuItem::AIPrompt`;
- `run_query`:
  - `omp_entries = if filters 允许 PromptHistory { build(...) } else { None }`;
  - `omp_only = omp_entries.is_some()`:omp 定位成功时命令历史与会话条目全部让位(即使消息为空);
  - `None` 时原逻辑不变(命令历史 + conversations + interleave 时间序合并)。

WASM 分支(`#[cfg(target_family = "wasm")]`)返回 `None`,无本地文件。

## Slash 优先级

`app/src/terminal/input/slash_commands/mod.rs` 的 `Composing` 分支(输入 `/` 且其它菜单打开时):

1. **菜单预览写入抑制**:↑ 历史菜单浏览 / 开头项时,`Select*` 事件通过 `set_buffer_text_ignoring_undo` 回填 buffer,也会进入 Composing——但这**不是用户键入**。`Input::with_menu_driven_buffer_write` 标志包住 inline history 的 SelectCommand/SelectAIPrompt/SelectConversation 回填,Composing 分支见标志即不接管;
2. **守卫谓词**:`can_open_slash_commands_menu`(长运行命令且非 CLI agent 输入时 false)。打不开时回退旧语义(`slash_command_model.disable()`,不动当前菜单),避免"菜单已关但 slash 开不了"的空窗;
3. 可打开时:`close_input_suggestions(false, ctx)` 关闭其它菜单(不恢复 buffer、不抢焦点)再 `open_slash_commands_menu`。

副作用说明:close 会触发一次 `ai_input_detection` 后台任务;输入类型锁定时(rich input 恒锁定)`detect_and_set_input_type` 首行即 return,无实际影响。

## 文件清单

|文件|改动|
|---|---|
|`app/src/terminal/cli_agent_sessions/omp_session_history.rs`|新增:定位 + 解析,`Option<Vec<OmpUserMessage>>` 语义|
|`app/src/terminal/cli_agent_sessions/omp_session_history_tests.rs`|新增:6 个单测(解析/容错/id 定位/tty 映射/cwd 排名/未落盘)|
|`app/src/terminal/cli_agent_sessions/mod.rs`|注册模块(非 wasm)|
|`app/src/terminal/input/inline_history/data_source.rs`|AIPrompt 条目 + omp_only 让位 + filter 门控|
|`app/src/terminal/input/inline_history/data_source_tests.rs`|interleave 测试补 AIPrompt 分支|
|`app/src/terminal/input/slash_commands/mod.rs`|Composing 接管:菜单预览抑制 + 守卫谓词 + 关闭其它菜单|
|`app/src/terminal/input.rs`|`is_menu_driven_buffer_write` 字段/辅助、`can_open_slash_commands_menu` 谓词、inline history Select 回填包标志|

## 已知边界与决策

- **多终端同 cwd / 无 cwd 匹配**:按映射文件 mtime 最新定位,并发切换会话时可能选到其它终端的会话(可接受降级;比显示旧历史或空白更符合直觉)。
- **不缓存**:每次 ↑/输入都重读磁盘(当前会话 jsonl 毫秒级);后续若卡顿,可加按 `(session_id, mtime)` 的会话内内存缓存(不落盘)。
- **omp 目录/格式变更**:依赖 omp 的会话文件布局与 jsonl 格式;omp 是 Zap 配套项目,变更可控;格式变化时自然降级为 `None`(回退命令历史)。
- **session_id 体系差异**是本次实现的核心发现(实测确认:OSC777 上报 id 与文件名 id 不同),若 omp 未来对齐 id,第一级定位即可覆盖,后续两级自动失效但无害。
