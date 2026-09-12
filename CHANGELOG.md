# Changelog

本文档记录 Zap 各个发布版本的关键变更。仅收录功能性 commit,省略 dev / stable 等内部滚动 tag。

## [Unreleased]

- **主题**:内置主题由 27 个精简为仅保留 Dark、Light、VS Code 2026 Dark 三个,其余主题的配色定义、背景图与引导页预览图一并删除;新用户引导的主题选择器同步收敛为 Dark / Light 两项;另清理 8 个已无引用的 referral 图标与已失效的 `default_adeberry_theme` 构建开关

- **修复（DeepSeek Harness）**:dsh 启动失败现在**立即**在面板上显示真实报错——此前只有一句「已停止」,且要空转到 120s 就绪超时才报「超时」,而真实原因(插件加载失败等)几秒前就已写进 `~/.dsh/zap-dsh-web.log`;失败面板新增「修复错误」按钮(复制错误并附加到终端 Agent 输入框,当前标签页无终端时新开一个 Agent 标签页),「重新启动」点击后立即回到「启动中」界面;并去掉此前那句对启动失败有误导的「连续崩溃已停止」toast

- **修复（智能体）**:历史会话(历史对话)里点击命令行现在会正确展开命令内容块——此前恢复流程把命令块插在错误位置(被一个隐藏的空输入块隔开),展开状态翻转后没有可渲染内容,表现为点击无反应(#10423);同时补上本地智能体消息的时间戳,并加旧会话兼容回退,使修复前保存的历史对话同样生效

- **上游同步收尾（2026-09-12）**:新增命令面板动作「向 Agent 对话附加文件」(默认不绑键,可在键位设置里自行绑定);代码评审右栏最大化/新建按钮的 tooltip 补上键位提示

- **i18n 补齐（2026-09-12）**:消除中文界面里的英文残留——「右键行为」下拉项、Agent 提问时间戳的 tooltip 与显示格式（中文改为「提问时间：9月12日 15:04」），以及既存的「Ctrl+Tab 行为」「全局热键」下拉项

- **上游同步追加（2026-09-12 三波）**:新增「右键行为」设置(#15365)——终端裸右键可选「打开上下文菜单」(默认)或「直接从剪贴板粘贴」,粘贴模式按住 Shift 右键仍打开菜单(并显示该提示 #15392);块列表、alt screen、输入行等 6 处右键面统一接入,鼠标被运行中全屏程序接管时仍转发原始右键


- **上游同步追加（2026-09-12 续）**:二次裁决后再落地 3 项——块列表右键菜单新增「粘贴」(#15346)、切换 markdown Raw/Rendered 保持滚动位置(#13967,editor 新增 `ScrollPosition::Fraction` + 延迟到 element layout 的 `ScrollToFraction`)、Agent Mode 用户提问显示时间戳并支持复制(#15605)。其余候选终局:右键行为设置(菜单/直接粘贴 #15365)缓做;原生 agent File explorer chip / Attach file 调色板命令 / 模型名提示 / 按住修饰键 tab 快捷键提示 / 编译期 serde 优化 不做;`create_file allow_overwrite` 因本地不发 proto `Request` 而不适用(详见 `specs/upstream-merge-history.md` §36)

- **上游同步（2026-09-12）**:2026-08-14 → `4143c09ff`（上游 master 2026-09-11）区间同步,落地 24 个上游提交——崩溃/真实 bug:宽字符提升崩溃(#15763)、空流式 Agent 文档崩溃(#15720)、glyph 无包围盒 em_width panic(#15705)、已结束后台块双光标(#15322)、重复确认丢弃文件越界(#15884)、grep 含冒号路径 ParseIntError(#15422)、「新建窗口」可重绑(#15771)、移除不可重绑的 Alt+1 固定绑定(#15310)、静默 in-band 重置警告(#15779);shell 集成:bash 丢 shell_plugins(#15518)、zsh compadd 丢 `-ld` 描述(#15313)、zsh kill-buffer 绑定全部 keymap 修 bootstrap 残留回显(#15118)、四个 shell-integration bug(#15428)、honor PS1 二次展开(#15792);构建:删除 12 个孤儿 cargo feature(#15694)、cosmic-text pin 禁止 Hack 作 fallback donor(#15569);性能:大文件加载不再整份克隆 styled blocks(#13508)、布局 fan-out 与单行 shaping 上限(#15128)、SignatureCache 有界 miss 缓存(#15181)、gitignore matcher 共享缓存(#15240);功能:首次搜索前不绘制空 category 标题(#15376)、completer 选项参数按值位置解析(#15475)、AgentSource 增加 Orchestration 变体(#15164)、AI plan 文档编辑器延迟布局(#15579)。另有 12 个上游提交记为「本轮不做」(冲突规模/前置不成立,详见 `specs/upstream-merge-history.md` §35.5)

- **上游同步（2026-09-06）**:两个上游开放 PR 批次拣入 15 项——warp 上游:CoreText autorelease 排水(#15746)、bash 首个 precmd 丢失死锁(#15652)、终端 resize 每行深拷贝(#15751)、macOS History/Up 菜单 Circular update 崩溃(#15719)、APFS case-only rename 文件树残留(#15699)、AppContext 订阅泄漏(#15764)、连续 ViewNotification 去重(#15741)、EditDelta precise_deltas Arc 化(#15810)、Code editor 文件读 100MiB 守卫(#15835)、编辑器文本绕过 LayoutCache(#15831)、rich input 打开时光标下字形丢失(#15724)、启动 block 恢复 SQL LIMIT(#15757,新增 blocks_restore_order 索引迁移)、后台 pane SSH 提示不再抢焦点(#15670);zerx 上游:BYOP 模型名 `-max` 后缀被剥修复(#338)、硬编码中文接入 Fluent(#339,新增 91 key)

- **AI / BYOP**:port opencode `applyCaching`,启用 prompt caching;`write_to_long_running_shell_command` 在 line 模式下拒绝嵌入 LF;BYOP LRC monitor fallback 改走 silent subtask;`cancel_execution` 50ms 窗口内 sender 泄漏修复(#134 follow-up,#137)
- **对话历史**:对话列表面板新增多选批量删除(选择 → 勾选 → 删除选中 N)与"删除全部"功能;进行中/ambient 对话受保护不可删;确认弹窗接入 en/ja/zh-CN 多语言
- **CLI agent rich input**:↑ 历史菜单读取 omp 当前会话的用户消息(仅 omp agent;有消息时只显示 omp 消息,新会话为空,非 omp 回退命令历史;Zap 不持久化,按需读 `~/.omp/agent/` 磁盘记录);输入 `/` 优先级最高,其它菜单打开时自动关闭并打开 slash 命令菜单
- **云端剥离 Phase 1–2**:增加 `cloud-disabled` channel 谓词;清理 billing/pricing、referral/reward、cloud sharing dialog UI;退订 RTC UpdateManager;退役 notebook/folder sync queue
- **平台**:修复 Spotlight/Finder/Launchpad 启动 macOS 时的 panic;`run_shell_command` stdout 兜底回退至 command grid
- **基建**:`.gitattributes` 强制 LF;新增 stale bot 与 Claude Code GitHub workflow
- **编辑器**:代码/Markdown 查看器新增 15 种语言语法高亮(Dart、Zig、SCSS、R、Julia、OCaml、Erlang、Nix、Groovy、Solidity、GraphQL、Protobuf、Clojure、Elm、CMake)
-
- **上游同步 (`89f742fa6` → `ddba1684e`)**:73 commit 合并
  - Phase 1: 构建基础设施 — workspace/profile/feature 同步
  - Phase 2: 低风险模块 — Settings、Editor、Vim、Persistence、TUI、Scripts
  - Phase 3: 新增 `crates/warp_tui/` TUI crate
  - Phase 4: 高风险模块 — Terminal 基础设施、SSH RemoteServerSupport、AI MCP/MOD、CLIAgent Hermes+Vibe 支持
  - 保留本地 OMP 实现，仅 cherry-pick 上游非冲突增量
  - 预存测试编译错误（cloud/teams 代码）待后续清理
- **上游同步 (第八/九轮拣选)**:34 commit 合并（边界 `7cbb22d5c` 之后逐条拣入）
  - 安全:禁用 iTerm 文件下载、仅支持 inline 文件（#25261）;cd 转义 + 非本地会话不 cd（#25383）
  - 终端/协议:DCS hook session ID 完整性校验（#25395）;normal-screen focus events（#11946）;PTY spawn 快速失败（#12663）;CRLF 粘贴规范化（#12446）;PS1 复制（#13076）;启动期 inline 图片（#10478）;PowerShell bootstrap 延迟加载（#12764）
  - CLI agent:rich input 方向键修复（#10556）;拖拽图片（#9553）;CLI subagent 交互缩范围（#12384）;cmd-enter 新会话（#12540）;cmd-k 取消在途（#12555）;Oz run failure 上报（#13210）;多 pane 会话 AI 块（#11494）
  - 焦点/导航:block 上下导航（#10095）;命令完成焦点返回（#12583）;code diff 导航不抢焦点（#12107）;focus_ai_block 清理（#12286）
  - 设置/基建:复用已有 control master（#12465）;amd64 arch 映射与遥测（#10534）;ModelContext 订阅 emitter 参数（#12767）
  - 死代码清理:删 TMUX SSH warpification 流程（#12478）;删 /pr-comments 斜杠命令（#13621）;删 WelcomePalette（#12614）;删 Agent Mode 背景叠加（#13495）
  - 其他 UI:tab 分割线对比度（#13200）;分屏 footer 溢出（#11099）;vertical tab Summary PR chip（#12945）;completions banner 永久 dismiss（#12969）等
  - 品牌:PTY spawn 错误消息 "Warp logs" → "Zap logs"

## [v2026.05.06.preview] — 2026-05-06

- **AI**
  - 提升 LSP 安装可靠性
  - LSP 改为全局 `enabled_lsp_servers` setting,移除 `/index` 命令与 codebase indexing runtime
  - `/plan` 真实复刻 Plan Mode(system prompt + 工具硬护栏)
  - Agent dynamic tool whitelist、`persist_conversations` setting、auto-approve 下 `ask_user_question` 始终询问
  - BYOP 支持 provider extra headers
- **修复**
  - `apply_file_diffs` schema 从 `const` 改为 `enum` 适配 Gemini
  - SSE 卡顿根因——genai gzip 默认关闭 + workflow 拆分
  - 无云端环境下计划文件夹笔记本立即创建
- **品牌**:logo 与图标改用白色背景;BYOP 模式隐藏 credits/billing UI

## [v2026.05.04.preview] — 2026-05-04

- **SSH Manager**:数据层 + 持久化 + keychain 落地;UI/UX 完整接入(面板 + 中央 Pane + 拖拽 + 折叠 + Connect + Command Palette)
- **AI**:区分模型"无建议"输出并完善提示系统;BYOP 历史多模态扩展到 PDF/audio,opencode 风格 ERROR 替换;UserQuery.context.images 全链路保活
- **UI**:标题栏搜索框可隐藏开关;键位设置编辑态与快捷键徽章对比度修复
- **i18n**:剩余主要界面固定文案汉化;`/model` 默认绑定 `alt-shift-/`
- **修复**:Anthropic adapter 默认带 1M context beta header;BYOP ToolCall 首帧即 emit 占位卡;OpenAI-strict provider 禁回传 `reasoning_content`
- **基建**:CI 修复 `.deb` 构建并启用 PR 测试

## [v2026.05.03.preview(.2/.3/.4)] — 2026-05-03

- **上游同步**:合入大批 warp-upstream commit(tab 跨窗口拖拽、shell 脚本识别、IME cursor、远程服务器初始化重构、SSH remote-server 自动升级、跨窗口 tab drag 等);建立 rerere + `zap-ours` 合并驱动;新增黑名单文档
- **AI / BYOP**:工具参数 type-mismatched 输出的 coerce 层;suspicious backslash 扫描收紧消除 ls/diff 误报
- **i18n**:中文国际化补齐(设置面板等)
- **网站**:GitHub 地址统一为 `zerx-lab/warp`;移动端横向溢出修复
- **修复**:Windows 任务栏 ICO 与上游格式对齐;NLD in terminal 默认 true 恢复中文输入自动入 AI

## [v2026.05.02.preview] — 2026-05-02

- **AI / BYOP**
  - 完成会话压缩闭环——`byop_compaction` 模块、settings 持久化、auto prune、overflow 透传,1:1 复刻 opencode
  - reasoning effort 从 provider settings 迁移到输入框 picker
  - 多模态附件能力接入 BYOP 路径
  - 本地 BYOP webfetch / websearch 与 Exa 集成
  - 按模型标识选择系统提示模板,新增多份模板
- **隐私 / 云端剥离**
  - 物理删 P4 易剥离死代码(anonymous_id / EXPERIMENT_ID_HEADER / settings 同步 / app_focus)
  - 切断闭源遥测、Sentry、anonymous_id、Settings 同步四条外发链路
  - 三个隐私开关默认值 true → false
  - `cloud_conversations` 两波清理(UI / 隐私 / FeatureFlag / AIClient / cargo feature)
- **重构**:移除 blocklist 人工智能响应评分及埋点;移除 `agent_attribution` 与 Oz changelog toggle
- **CI**:周构建改为正式发布并规范 tag

## [v2026.05.01.preview] — 2026-05-01

- **云端剥离**:物理删 6 个云端 LLM tool + child_agent + orchestration;物理删 share modal 三件套与 billing denied modal;website 换单色 logo
- **AI**
  - Workflow Autofill 接入 BYOP one-shot
  - BYOP LRC 后续轮持续注入上下文 + sanitize 强化 + 控制键 token
  - 聊天流增加远程登录会话提示与推理回传
  - genai 错误映射细化为 Stream / Other variants
  - chat stream adapter,修复 ToolCall None 处理
- **平台**:`warpui_core` 避免重复扫描系统字体;同步命令无条件禁用 pager,改用 `PAGER=cat` 保留真实退出码
- **网站**:全站组件与 i18n 重构,Tailwind 与全局样式同步

## [v2026.04.30.oss] — 2026-04-30

- **CI**:CHANNEL `preview` → `oss`,修 Windows / macOS 构建失败
- **重构**:删除 cloud_mode 残留代码与设置

## [v2026.04.30.preview] — 2026-04-30

Zap 社区分支首个预览版本。

- **品牌与定位**:Zap 改名 + logo 重制 + 社区分支 README
- **BYOP**
  - `async-openai` → `genai`,支持 5 种原生协议显式绑定
  - Providers 子页 + models.dev 数据源 + 快速添加搜索框
  - prompt 模板精简
- **去中心化清理**:移除 `UseComputer` / `RequestComputerUse` 工具、Drive `Create team` / `Join team` 入口、referral 相关代码
- **i18n**:Fluent 基础设施 + 12 个 settings_view 文件翻译;ai / features / teams 三页 i18n 补全
- **网站**:新增 BYOP 落地页(Astro + Tailwind, 中英双语);响应式优化
- **AI**:CJK 输入分类、reasoning 拆分、BYOP tool_call 诊断、LRC tag-in 合成虚拟 subagent + 浮窗 spawn 链路
- **CI**:Release 显式声明 `contents: write` 权限修 403

[Unreleased]: https://github.com/zerx-lab/warp/compare/v2026.05.06.preview...HEAD
[v2026.05.06.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.04.preview...v2026.05.06.preview
[v2026.05.04.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.03.preview.4...v2026.05.04.preview
[v2026.05.03.preview(.2/.3/.4)]: https://github.com/zerx-lab/warp/compare/v2026.05.02.preview...v2026.05.03.preview.4
[v2026.05.02.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.01.preview...v2026.05.02.preview
[v2026.05.01.preview]: https://github.com/zerx-lab/warp/compare/v2026.04.30.oss...v2026.05.01.preview
[v2026.04.30.oss]: https://github.com/zerx-lab/warp/compare/v2026.04.30.preview...v2026.04.30.oss
[v2026.04.30.preview]: https://github.com/zerx-lab/warp/releases/tag/v2026.04.30.preview
