# 上游合并状态总表（唯一入口）

> **本文件是「上游同步」这件事的唯一状态入口**：一眼看清哪些上游提交已合并、哪些没合并、为什么。
> 逐轮过程记录在 `specs/upstream-merge-history.md`（§35–§40，追加式，不回改），
> 教训沉淀在 `specs/upstream-merge-lessons.md`，原始计划在 `specs/upstream-merge-plan-2026-09-11.md`。
>
> **维护规则**：每轮同步收尾时，只更新本文件的三张表（已合并 / 未合并 / 本地补齐）+ 未决事项；
> 过程细节继续往 `history.md` 追加新章节，两处不互相复制。

| 项 | 值 |
|---|---|
| 上游 | `warpdotdev/warp` master = **`4143c09ff`**（2026-09-11）|
| 本地基线 | `main`（本轮起点 `99989c8bf`；真实 fork 点 `c325d146`）|
| 区间 | 2026-08-14 → `4143c09ff`，**281** 提交 |
| 本文件更新 | 2026-09-12 晚 |

---

## 一、总览

| 阶段 | 数量 |
|---|---|
| 区间内上游提交 | 281 |
| 三轮筛选后候选 | 89 |
| 计划可做（34 项任务）| **36 个上游提交** |
| ✅ **已合并** | **31 个上游提交** |
| ❌ **未合并** | **5 个上游提交**（1 个移植后回滚）|
| 规划期直接淘汰 | 53 条（见 §五）|
| 额外本地改动（非上游）| 4 项 + 3 个配套 commit（见 §四）|
| 未推送提交 | 见 §六 |

---

## 二、✅ 已合并的 31 个上游提交

### 批次 1 — P0 崩溃 / 真实 bug（9）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 1.1 | `a7326f8fe` (#15763) | 行尾 cell 提升为宽字符时崩溃 | `e1336315f` |
| 1.2 | `92a98662f` (#15720) | 空流式 Agent 文档更新崩溃 | `53edb7083` |
| 1.3 | `83e270f1d` (#15705) | glyph 无包围盒时 `em_width` panic | `26a693ff0` |
| 1.4 | `ee95ac0fd` (#15322) | 已结束后台块出现双光标 | `f904ff55a` |
| 1.5 | `33c3bf6b7` (#15884) | 重复确认「丢弃文件」时 `[0]` 越界 panic | `68bc478db` |
| 1.6 | `fbbfc41f3` (#15422) | grep 工具在含冒号路径上 `ParseIntError` | `38d56a649` |
| 1.7 | `53b502c8e` (#15771) | 「新建窗口」成为可重绑快捷键 | `6b0dafeec` |
| 1.8 | `90c2484dc` (#15310) | 移除隐藏且不可重绑的 Alt+1 Project Explorer | `7d3a409be` |
| 1.9 | `5cd24ed1b` (#15779) | 静默预期的 in-band 命令重置警告（仅日志）| `acd5b04b4` |
| — | （批次 1 编译修复）| 本地能力差异适配 | `9355f32a7` |

### 批次 2 — shell / bootstrap 脚本（5）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 2.1 | `e722ebeda` (#15428) | 四个 shell-integration 潜在 bug | `c1f05db98` |
| 2.2 | `607be8c26` (#15518) | bash bootstrap 丢 `shell_plugins` | `a0b6fe01d` |
| 2.3 | `0140af045` (#15313) | zsh compadd shim 丢 `_describe` 的 `-ld` 描述 | `8f495c2e8` |
| 2.4 | `294033bb1` (#15118) | zsh kill-buffer 绑定全部 keymap（修 bootstrap 残留回显）| `bdcf99200` |
| 2.5 | `17f432027` (#15792) | 「honor PS1」模式下 Bash PS1 二次展开 | `9e8502c48` |

### 批次 3 — 构建 / 依赖（2）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 3.1 | `ccf683193` (#15694) | 删除 12 个孤儿 cargo feature | `f05c2f13c` |
| 3.2 | `542683634` (#15569) | cosmic-text pin 到「禁止 Hack 作 fallback donor」 | `6beea36c7` |

### 批次 4 — 性能（4）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 4.1 | `d89e78385` | 加载大文件时不再整份克隆 styled blocks | `ce78c1fce` |
| 4.2 | `1c925e333` | 限制布局任务并行 fan-out 与单行 shaping 上限 | `424a943f7` |
| 4.3 | `213c9b32e` | SignatureCache 加 key 长度上限与有界 FIFO miss 缓存 | `9fc4e15f1` |
| 4.4 | `c6609ef23` | 共享并缓存 gitignore matcher | `941448bfe` |
| — | （测试适配）| `EditDelta::new_lines` / `Gitignore` Arc 化连带 | `8f5be7b7d` `d854a3466` |

### 批次 5 — 功能（6）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 5.1 | `3a7a4a5b3` | 首次搜索前不绘制空 category 标题 | `e5fed477f` |
| 5.2 | `79a9cb721` (#15475) | completer 选项参数按「值位置」解析 | `bffda6744` |
| 5.3 | `0a0fd3ae1` (#15346) | 块列表右键菜单加「粘贴」 | `f36dc8613` |
| 5.5 | `3a6f05512` | AI plan 文档编辑器延迟布局 | `bccb62839` |
| 5.6 | `142b87102` (#15762) | Attach file 成为**默认不绑键**的命令面板动作 | `6d9e859f8` |
| 5.7 | `d15645c77` | 客户端 `AgentSource` 增加 Orchestration 变体 | `0f32aa94e` |

### 批次 6 — 中大成本功能（5）

| 任务 | 上游 | 功能 | 本地 commit |
|---|---|---|---|
| 6.1 | `092c1dce9` (#13967) | 切 markdown Raw/Rendered 保持滚动位置 | `ee4c41d39` |
| 6.3 | `c25ac4070` (#15365) + `18179177a` (#15392) | **右键行为设置**（上下文菜单 / 直接粘贴）+ Shift+右键提示 | `76ba85fd7` |
| 6.4 | `b7ec0fc55` (#15605) | Agent 提问显示时间戳（悬停 tooltip）| `c1330a62e` |
| 6.6 | `4b894db80` (#15455) | 编译期 serde 优化（**仅阶段 1**）| `e11deb51e` |

**小计（可自查）**：9 + 5 + 2 + 4 + 6 + 5 = **31 个上游提交**，与 §三 的 5 个未合并合计 36 ✓

> ⚠️ `4b894db80` 实测**净收益为负**（`warp` crate 编译 98.6s → 100.7s，+2.1%；rlib 仅 −0.2%），
> 已按用户决定保留，`git revert e11deb51e` 可一键回滚。测量方法与数据见 `history.md §38`。

---

## 三、❌ 未合并的 5 个上游提交

| 任务 | 上游 | 功能 | 状态与原因 |
|---|---|---|---|
| 6.5 | `5e7030db7` | warping 行显示当前模型名 | **移植后回滚**（`73843d81e` → `ea3b6fadf`）。根因：本仓**从不构造** proto 消息 `Message::ModelUsed`（全仓仅 3 处读取，BYOP 在 `chat_stream.rs:804/967` 把它归入 `=> {}` 忽略分支）→ `AIAgentOutput::model_info` 恒为 `None` → 功能不可能生效。详见 `history.md §40.7` |
| 6.2 | `8b88df987` (#15221) + `40e397170` (#15496) | 按住修饰键显示 tab 切换快捷键提示 | **不做**：29 hunk / 6 文件，本地完全无该机制（`TabShortcutModifierState` 等 0 命中），`tab.rs`/`vertical_tabs.rs` 是自研度最高区域，收益/成本比最差 |
| 5.4 | `4cd1c77c4` (#15007) | 原生 agent 输入工具条加 File explorer chip | **不做**：本地已有 `AgentToolbarItemKind::FileExplorer`（现仅 CLI agent）属部分重复；10 hunk / 3 文件 |
| 3.3 | `511b952c2` (#15380) | `create_file` 工具 `allow_overwrite` | **不合**：本地 proto 走 `zerx-lab/warp-proto-apis` fork（rev `14ab9a71`），上游该提交依赖 `warpdotdev/warp-proto-apis` 自有 rev 的 `supports_create_file_overwrite` |

---

## 四、额外本地改动（非上游移植）

| 内容 | 提交 | 说明 |
|---|---|---|
| **i18n 中文化 4 处** | `2e0604f55` `7fbb3b105` | 右键行为下拉项、提问时间戳 tooltip 与显示格式（中文改 24 小时制）、Ctrl+Tab 行为 / 全局热键下拉项 |
| **「复制时间戳」菜单入口** | `b4fd53118` | 上游只加了 action + 处理函数、**无任何构造点**（悬空动作）；本地在右键行菜单与三点溢出菜单都补上入口 |
| **删除死代码** | `43417ffc8` | `AIBlock::output_model_display_name`（全仓无调用，23 行）|
| （编译修复 / 清理）| `9355f32a7` `bbe0ba371` | 批次 1 本地能力差异适配；4.4 遗留的未使用参数与闲置 import |

---

## 五、规划期直接淘汰的 53 条（8 类，逐条依据见计划 §4 与 `history.md §35`）

| 类别 | 条数 |
|---|---|
| `agent_sdk/driver/` 新模块本地不存在 | 11 |
| Sentry / `warp_errors` 链（本地无该链路）| 10 |
| 依赖缺失符号（本地 0 命中）| 9 |
| 设置页 / onboarding 定制区 | 6 |
| 纯重构 / 编译期 / 高风险 / pin bump | 6 |
| native shell completions / widget handoff 整条未跟 | 4 |
| promote 无对象（本地无对应 Cargo feature）| 4 |
| 已含等效实现 | 3 |

> **附：筛选之外新发现 1 条（2026-09-12 盘查编译期提交时发现）**：`1e4b86a81` (#15517)
> "Bump release-cli codegen-units from 1 to 4"（2026-08-25，属本区间）**不在 89/53 名单内**
> （只动根 `Cargo.toml`，疑被路径过滤漏掉）。本地对应 `Cargo.toml` 的 `[profile.release-cli]`
> 仍是 `codegen-units = 1`。纯构建配置、无行为风险，列为候选（见 §六）。

---

## 六、未决事项

1. **推送**：本地 `main` 领先 `origin/main` **52 个提交**，全部未推送。可选：一次推 / 分两批（上游移植 · 文档）/ 暂不推。
2. **`4b894db80` 阶段 1 去留**：保留中（实测负收益）；若要回滚：`git revert e11deb51e`。
3. **`5e7030db7` 若将来仍要该效果**：正确路径是**本地自行命名**（客户端本就知道请求的模型），
   即已删除的 `AIBlock::output_model_display_name` 的思路，而非移植上游（它等不到数据）。
4. **编译期栈的其余部分是否补做**（见 §八）：栈内 #15453 / #15454 未做；`4b894db80` 自身还剩
   「阶段 2」（2 文件 / 4 处冲突）；栈外的 `1e4b86a81` (#15517) 是 1 行配置。建议：除 #15517 外都不做，
   理由是 §38 实测 stage 1 已是 **+2.1%（更慢）**、stage 2 天花板低。

---

## 七、文档地图

| 文件 | 职责 | 何时更新 |
|---|---|---|
| `specs/upstream-merge-status.md`（本文件）| **唯一状态入口**：已合并 / 未合并 / 本地补齐 / 未决 | 每轮同步收尾 |
| `specs/upstream-merge-history.md` | 逐轮过程记录（§35 首轮 24 提交、§36 追加 3 + P3 裁决、§37 右键行为、§38 编译期实测、§39 i18n、§40 收尾三项）| 每轮追加新章节，不回改旧章 |
| `specs/upstream-merge-lessons.md` | 教训速查（头部含同步边界与每轮要点）| 每轮追加行 |
| `specs/upstream-merge-plan-2026-09-11.md` | 原始计划（批次、锚点、适配要点、淘汰依据）| 仅计划期，不再改 |
| `CHANGELOG.md` | 用户可见变化 | 有用户可见变化时 |

## 八、编译期（compile time）优化专项

上游那条栈是 **3 个 PR，`4b894db80` 就是最后一个**（提交正文自述 "PR 3 of a stack"，`6a96a72d8` 为 "PR 2 of 3"）：

| 栈序 | 上游 | 内容 | 本地状态 |
|---|---|---|---|
| PR 1/3 | `dc1077845` (#15453) | Reduce monomorphization in `warpui_core` update/spawn paths | ❌ 未做（规划期按「纯重构 / 编译期」淘汰）|
| PR 2/3 | `6a96a72d8` (#15454) | share settings registration code across settings | ❌ 未做（规划期按「依赖缺失符号」淘汰：落点 `crates/settings/src/registration.rs` 本地不存在）|
| PR 3/3 | `4b894db80` (#15455) | serde `Content` 缓冲 → JSON-value 反序列化 | ⚠️ **仅阶段 1**（`dcs_hooks.rs` 的 `DProtoHook`/`BootstrappedValue`，`e11deb51e`）；**阶段 2 未做** |

**`4b894db80` 自身剩余（＝阶段 2）**，共 2 个文件、今日 HEAD 上 **4 处冲突（2+2）**：

| 文件 | 内容 |
|---|---|
| `app/src/ai/agent/mod.rs` | `AIAgentContext` / `AIAgentAttachment`（外部标签 + `#[serde(untagged)] Block(Box<BlockContext>)` 混用，上游称「最贵的模式」）|
| `app/src/ai/artifacts/mod.rs` | `Artifact`（相邻标签 `artifact_type`/`data`，由 `ArtifactEnvelope` 取代 `ArtifactHelper`）|

**实测与天花板**（详见 `history.md §38`）：stage 1 后 `warp` crate 编译 **98.6s → 100.7s（+2.1%）**，
**体积仅 −958 KB（rlib，−0.098%）/ −1.0 MB（调试二进制，−0.167%）**（`ContentDeserializer` 实例化 −62%、`__DeserializeWith` −41%，但占比过小）；
stage 2 的目标 `BlockContext × ContentRefDeserializer` 在符号层面只有 **13 个**。

**栈外同期的编译期提交**：`1e4b86a81` (#15517，见 §五 附)、`21f413b79` (#14875 终端 grid 大搬运，已在淘汰 53)、
`2a183d552` (#15462 抽 `secret_redaction` crate，已在淘汰 53)。

**结论**：这条栈在本仓的收益基本测不出（stage 1 反向），除 `1e4b86a81`（1 行配置、纯构建期）外不建议继续；
若将来要做栈内其余部分，先按 `history.md §38.3` 的方法（`RUSTC_WRAPPER=` 绕过 sccache + 同态两次）测基线再决定。

### 附：验证基线（判断「失败是否既存」的对照）

| 测试目标 | 基线 | 备注 |
|---|---|---|
| `cargo check -p warp` | 0 error | 每批必过 |
| `cargo test -p warp_editor` | 456~457 过 / **10 败** | 10 个失败为既存环境干扰 |
| `cargo test -p warp_completer` | 125 过 / **26 败** | 26 个失败为既存环境干扰 |
| `cargo test -p repo_metadata` | 55 过 / 0 败 | |
| `cargo test -p warpui text_layout` | 33/33 | cosmic-text pin 后 |
| `cargo test -p warpui_core` | 290 过 / 0 败 | 右键行为设置（事件层）后复核 |
| `cargo test -p warp --lib i18n` | **单线程** 4/4 | 并行时 `fallback_chain_works` 失败属既存干扰（全局 `init()` 竞争）|
