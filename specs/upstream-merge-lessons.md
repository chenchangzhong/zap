# 上游合并经验文档

> **⚠️ 历史归档（2026-08-13）**：逐轮移植/核验的详细记录（§0、§2–§6、§10、§12–§33）已移至 `specs/upstream-merge-history.md`，本文件只保留每轮必读的参考区（决策速查表 + 核心原则 + 验收清单 + 坑位速查 + 文件定位 + 经验总结）。文内 `history.md §X` 引用指向归档文件同名章节。新增轮次记录请追加到 history.md。

> **✅ 本轮移植（2026-09-12）**：2026-08-14 → `4143c09ff`（上游 HEAD，2026-09-11）区间同步——281 提交经三轮筛选得 89 候选，逐条核验后落地 **24 个上游提交**（P0 崩溃/真实 bug 9 + shell 脚本 5 + 构建 2 + 性能 4 + 功能 4）；12 个记录为「本轮不做」（冲突规模或前置不成立）。执行计划见 `upstream-merge-plan-2026-09-11.md`，完整记录见 history.md §35.5
> **✅ 本轮追加移植（2026-09-12 续）**：§35.5 判「不做」的 12 项经二次裁决后**再落地 3 项**——块列表右键「粘贴」`0a0fd3ae1`、切 markdown Raw/Rendered 保持滚动 `092c1dce9`、Agent Mode 用户提问时间戳 `b7ec0fc55`；剩余项终局裁决（`c25ac4070`+`18179177a` 缓做、其余 5 项不做）与 `511b952c2` 的 proto fork 调查见 history.md §36
> **同步边界（最新）**：`4143c09ff`（2026-09-11，上游 warpdotdev/warp master）
> **✅ 本轮移植（2026-09-06）**：两上游开放 PR 批次——warp 上游 13 PR（#15746 #15652 #15751 #15719 #15699 #15764 #15741 #15810 #15835 #15831 #15724 #15757 #15670）+ zerx 上游 2 PR（#338 #339）已全部落地（共 15 项）；等转正追踪 #15802/#15830/#15785/#15742/#15777/#15828/#15833/#15765（详见 history.md §34 与 upstream-merge-plan-2026-09.md）
> **✅ 遗漏修复（2026-08-05）**：terminal lifecycle recovery 栈 6 commit（#12853/#12854/#12855/#12856/#12858/#12859）已按方案 A 完整移植（详见 history.md §21.4/history.md §23）；history.md §22 重扫发现的 5 件遗漏已全部拣入（zsh glitch 剥离 #14166/#12438、尾点链接 #12965、Hermes BracketedPaste #14367、系统终止 #12480、O(1) 焦点 #13113，详见 history.md §23）
> **✅ 本轮移植（2026-08-13）第三波（盲区 59 候选）**：fork 点 `c325d146` → 最新区间 59 个漏合并修复已全部移植（P0 安全 4 / 崩溃死锁 5 / 竞态 4 + P1 19 + P2 27；5 个跳过：3fe061620 本地已等价、f6d8167f4 无 TabGroup、81cc895d1 部分已并入、429dbf2e3 已含本地、475fdb33e 设置页定制区无组件），详见 history.md §31。fork 点定位方法见下方 ⚠️ 段（merge-base = `c325d146`）
> **✅ 本轮移植（2026-08-13）**：性能优化 2 commit——imported-comments guard `922ba2584`（#13114）+ async blocklist find `fb5ad384a`（#9618，含后续 `cd745fac9` #11205 消费端）（详见 history.md §29）
> **✅ 本轮移植（2026-08-13）第二波**：`02c042063` → `5fb3144db` 区间筛出 6 个遗漏修复已移植——Core Text style runs 合并 `12e455c56`（#15043）、workflow 截断 panic `87a4e4b34`（#14933）、citation 降级 `80a203474`（#14915）、SSH wrapper RCS `f919f8935`（#13407）、code review renamed `724579a87`（#14655）、generator 进程组取消 `a4769955f`（#14853）；`46c0b5136`（#13405 Windows kaspersky 蓝屏）只记录待 Windows 版；`aa9f3a436`（#14854 context chips 活跃面）经核验本地 `FeatureFlag::AgentView` 默认关、无此 bug，**跳过**（详见 history.md §30）
> **✅ 本轮移植（2026-08-12）**：AI file-outline 内存上限 `3fe6e17ad`（APP-4794）已移植——累计 outline 堆超 256MiB/repo 预算即跳过剩余文件，降级为有界部分 outline（提交 `942d8b609`，详见 history.md §32）
> **✅ 本轮移植（2026-08-11）**：Cmd-Up 导航 `da4da09f8`（#14685）+ conversation_export 抽离 `a77348c67`（#13603 GUI 侧）+ framework 补移植 `Container::with_foreground_border`（#13056），3 commit（详见 history.md §27）
> **✅ queued prompts 移植（2026-08-10）**：#11439（`98af7b654`）+ 其后 12 个演进 commit + `098c307c7`（LRC 交回守卫）已移植，本地 7 commit 提交链（详见 history.md §28）
> Zap 分支：`ed3bb76af`
> 最后核验：2026-08-13，`cargo check -p warp`（默认）与 `--features async_find` 均通过；find 31 测试（含 21 async）+ blocklist 268 通过（8 个既有环境失败与改动无关）；4 reviewer 并行评审（2 P1 已修：mark 位置、cd745fac9 消费端 2 处）
>
> 历轮边界：
>
> | 轮次 | 区间 | commits | 日期 | 记录章节 |
> |------|------|---------|------|----------|
> | 一 | `89f742fa6` → `ddba1684e` | 73 | 2026-07-30 | history.md §2–§11 |
> | 二 | `ddba1684e` → `7cbb22d5c` | 60 | 2026-08-01 | history.md §0 |
> | 三 | 21 个 agent commit 核对 | — | 2026-08-01 | history.md §12 |
> | 四 | 遗漏候选核对 | — | 2026-08-01 | history.md §13 |
> | 五 | CLI agent 事件链路 5 项 | — | 2026-08-03 | history.md §14 |
> | — | DeepSeek 集成移除 | — | 2026-08-03 | history.md §15 |
> | 六 | `7cbb22d5c` → `02c042063` | 5 | 2026-08-03 | history.md §16（全不合并） |
> | 七 | fork 点全量对账 21 个缺失 | — | 2026-08-03 | history.md §17（落地 9 个） |
> | 八 | `f7e298027` 移植偏差回查 | — | 2026-08-04 | history.md §18（修 3 处偏差） |
> | 九 | `7cbb22d5c` 后 34 个拣入 | 34 | 2026-08-05 | history.md §19 |
> | 十 | lifecycle 栈 6 commit + 5 件遗漏 | 11 | 2026-08-05 | history.md §21–§23（提交 `81bbcf869`） |
| 十一 | Cmd-Up #14685 + export #13603 + foreground_border #13056 | 3 | 2026-08-11 | history.md §27（提交 `2d0942210`） |
| 十零 | queued prompts：#11439 + 12 演进 + 098c307c7 | 13 | 2026-08-10 | history.md §28（提交 `4b2d5b855`→`9fb3abb`） |
| 十二 | 性能优化：imported-comments guard #13114 + async find #9618(+#11205) | 2 | 2026-08-13 | history.md §29（提交 `40d94c395`→`ed3bb76af`） |
| 十三 | 遗漏修复 6 commit（Core Text / workflow panic / citation / SSH RCS / code review renamed / 进程组取消） | 6 | 2026-08-13 | history.md §30（未提交，工作区） |
| 十四 | 盲区 59 候选（fork 点 `c325d146` 起：P0 13 + P1 19 + P2 27） | 54 commit | 2026-08-13 | history.md §31 |
| 十五 | file-outline 内存上限 `3fe6e17ad`（APP-4794） | 1 | 2026-08-12 | history.md §32（提交 `942d8b609`） |
>
> **⚠️ 待评估区间定位（2026-08-13 已修正）**：本 fork 早期是浅克隆、与上游「无 merge-base」，
> 只能用手工边界 hash。**2026-08-13 已 `git fetch --unshallow upstream`**，本地不再是浅仓库，
> 上游完整历史（2076 commit，从 `0dbd3d567` 根起）已拉入。**现在可用 merge-base 精确定位 fork 点**：
>
> ```bash
> git merge-base HEAD upstream/master   # = c325d146（2026-04-28 #9329，真实分叉点）
> git log $(git merge-base HEAD upstream/master)..upstream/master   # fork 后全部待对账 commit（2044）
> ```
>
> 真实 fork 点 = `c325d146`（本地与上游最后共同祖先），非 `0dbd3d567`（上游根）。今后每轮
> 以 `git merge-base HEAD upstream/master` 为起点，彻底消除盲区；不要再用历史手工边界 hash。
> 旧记录：本 fork 曾浅克隆，`git rev-list HEAD..upstream/master` 会算错全部历史（曾得 1795，
> 真实待评估只有 4 个），现因 unshallow 已失效。

---

## 决策速查表

每轮同步先查这张表；命中即按裁决执行，未命中再读详细章节。

| 上游改动主题 | 裁决 | 依据 |
|--------------|------|------|
| `crates/warp_tui` / `app/src/tui/` 任何改动 | 跳过 | history.md §2 阶段③（crate 已删） |
| 云计费 / GraphQL credit / workspace teams | 跳过 | §1 核心原则 |
| 共享会话 / Drive / Notebook sync | 跳过 | §1 核心原则 |
| `ThirdPartyHarness` / `HarnessRunner` / `FeatureFlag::AgentHarness` | 跳过 | history.md §10 |
| `CLIAgent::DeepSeek` 相关（注意区分 BYOP 的 `AgentProviderApiType::DeepSeek`） | 跳过 | history.md §15 |
| `OmpModelSelector` / `CLIAgent::OhMyPi` / CLI agent session | 取本地 | §1（Zap 自研禁止覆盖） |
| `Stop` 分支字段清理（`clear_permission_scoped_state` 类） | 跳过 | history.md §14 第 2 项 |
| rich status 判定机制（latch vs `session_id`） | 保持本地 | history.md §14 第 4 项 |
| Codex OSC777/OSC9 双通道去重 | 跳过 | history.md §14 第 3 项 |
| `settings_view/` 定制区 | 逐项判 | history.md §0（冲突密集，低价值） |
| `FeatureFlag` promote 类（加 variant + 列表） | 可拣 | history.md §0 教训（需手动补 Cargo feature 定义） |
| 纯加法的事件/枚举变体 | 可拣 | history.md §14 第 1 项（`StopFailure` 先例） |
| 来自**未合并分支**的 commit | 查分支 tip | history.md §18.1（初版可能已被 review 否决） |
| Windows 专属修复（如 `46c0b5136` kaspersky 蓝屏） | **只记录，待发 Windows 版再合** | history.md §30（用户决定：当前只处理 macOS） |
| `aa9f3a436` #14854 context chips 活跃面 | **跳过**（本地 `FeatureFlag::AgentView` 默认关，无此 bug） | history.md §30.4；未来 AgentView 默认开再移植 |
| 上游 `elements/gui/` 路径下的 framework 改动（如 `Container::with_foreground_border`） | 按平铺路径适配移植 | history.md §27.2（本地未跟随 #12633 目录重构，gui/ 文件本地天然缺失） |

### 移植前四问（history.md §14 / history.md §18 教训提炼）

1. **前置假设成立吗**——上游修复常依赖上游自己的实现前提。Zap 改过那个前提，
   同步修复反而引入退化（history.md §14 第 2 项是典型）。
2. **本地已有等效解法吗**——同一问题两套解法不要合并。同形化看似便于未来
   cherry-pick，实际是删除已验证的本地功能换假想便利（history.md §14 第 4 项）。
3. **字段有消费点吗**——零消费点字段改动无风险；在回落链上的字段改动高风险。
   同一提交里不同字段的风险可以完全不同，`grep` 消费点是必要步骤。
4. **这是上游的最终版本吗**——主题 grep 命中的常是初版。必查
   `git branch -a --contains <commit>` 与该分支后续 commit；未合并 master 的
   分支尤其要查有无返工（history.md §18.1）。

## 1. 核心原则

| 原则 | 说明 |
|------|------|
| **品牌字符串替换** | 所有用户可见 "Warp" → "Zap"、"Oz" → "Zap"、"Oz CLI" → "Zap Agent CLI" |
| **Zap 本地功能禁止覆盖** | OMP 集成（`CLIAgent::OhMyPi`、CLI agent session）、OhMyPi 模型选择器（`OmpModelSelector`）等 Zap 自研功能，上游 cherry-pick **不得覆盖**。合并时若冲突，取本地版本 |
| **DeepSeek CLI agent 已移除** | `CLIAgent::DeepSeek` 整条集成于 2026-08-03 删除（上游改名 CodeWhale 且 v0.9.0 移除 `deepseek`/`deepseek-tui` shim）。上游若新增 DeepSeek CLI agent 相关改动**一律跳过**，详见 history.md §15。BYOP 侧 `AgentProviderApiType::DeepSeek`（模型提供商）**保留**，两者是独立实体 |
| **第三方 Harness 已删除** | `ThirdPartyHarness`、`HarnessRunner`、`FeatureFlag::AgentHarness`、Claude/Gemini harness 执行路径已全部清理。上游 `Harness` 枚举变体保留（序列化兼容），`ClaudeHarness`/`GeminiHarness` struct 已删 |
| **云服务代码不合并** | 上游 workspace/team/云同步/Drive/Notebook sync 等 SaaS 功能 Zap 不需要，仅保留 struct 兼容字段 |
| **无 merge-base** | 采用分阶段手动 cherry-pick，非标准 git merge |

---

## 2. 每轮同步验收清单

这是**每轮同步都要重跑**的模板，不是一次性待办。下方「上次核验」列记录
2026-08-03 第五轮结束时的实测结果。

### 2.1 必做

| 检查项 | 上次核验（2026-08-03） |
|--------|------------------------|
| `cargo check -p warp` 通过 | ✅ 通过 |
| `cargo build -p warp` 生成二进制 | ✅ `target/debug` 已产出 |
| 二进制冒烟（`--version` / `whoami`） | ✅ 通过 |
| `CHANGELOG.md` 记录同步边界 | ✅ 已记录 |
| 本文档头部「同步边界（最新）」已更新 | ✅ `7cbb22d5c` |

### 2.2 残留引用核验

已删功能不得有残留；BYOP 同名项不得误删。

| 检查项 | 期望 | 上次核验（2026-08-03） |
|--------|------|------------------------|
| `warp_tui` / `warp_search_core` / `warp_errors` | 0 引用 | ✅ 均 0 |
| `CLIAgent::DeepSeek` / `DeepSeekLogo` / `deepseek.svg` | 0 引用 | ✅ 均 0 |
| `AgentProviderApiType::DeepSeek`（BYOP，**保留项**） | >0 引用 | ✅ 3 处 |

核验命令：

```bash
for s in warp_tui warp_search_core warp_errors \
         'CLIAgent::DeepSeek' DeepSeekLogo deepseek.svg; do
  echo "$s: $(grep -rl "$s" app crates script Cargo.toml 2>/dev/null | wc -l)"
done
grep -rl 'AgentProviderApiType::DeepSeek' app crates | wc -l   # 应 > 0
```

### 2.3 已知未做项（明确不做，非待办）

| 项 | 结论 |
|----|------|
| `cargo nextest run` 全量测试 | **未跑**——nextest 未安装。`cargo check` 是本仓交付门槛（见 AGENTS.md §5.1），全量测试非必需 |
| 云服务死代码清理（`admin.rs` workspace 字段、`cloud_environments` 残留） | **不做**——保留 struct 兼容字段是 §1 既定原则，删除会破坏服务端反序列化 |
| 补全 `warp_core/src/async` 模块声明 | **不做**——仅 `warp_search_core` 需要，该 crate 已删且不会回来 |
| Phase 4 遗留 cherry-pick（history.md §3.2 六项） | **不做**——见 history.md §3.2 各项已更新为终局结论 |

---

## 3. 常见坑位速查

| 现象 | 可能原因 | 对策 |
|------|----------|------|
| `cargo check` 报 `unresolved import` | 上游测试依赖本地无 helper | 删除测试而非补全 |
| `cargo check` 报 `use of unresolved module` | 同上 | 同上 |
| `warp_tui` 编译失败 | 缺 `warp::tui_export`（已删） | 删除 crate（Zap 不用） |
| `warp_search_core` 编译失败 | 缺 `warp_core::r#async` | 删除 crate——该 crate 已于第一轮删除且不再引入 |
| `sentry`/`unicode-segmentation` 未使用 | 仅被已删 crate 依赖 | 同步删除 workspace dep |
| `FeatureFlag` 编译错误 | 上游新增 flag 本地无 | 在 `crates/warp_core/src/features.rs` 加 variant + 对应 FLAGS 列表 |
| `FeatureFlag::AgentHarness` 引用 | 上游 cherry-pick 引用此 flag | **跳过**，Zap 已删除此 flag 及其 6 处门控 |
| `ThirdPartyHarness` / `HarnessRunner` | 上游改动 driver/harness 模块 | **跳过**，Zap 已清理全部第三方 harness 执行路径 |
| `CLIAgent::DeepSeek` 引用 | 上游改动 DeepSeek CLI agent 路径 | **跳过**，Zap 已删除整条集成（枚举变体、handler、plugin manager、logo）。注意区分 BYOP 的 `AgentProviderApiType::DeepSeek`（保留） |
| 终端死锁 | `TerminalModel::lock()` 重入 | 检查调用栈，传已锁引用而非再次加锁 |

---

## 4. 关键文件定位

| 类别 | 路径 |
|------|------|
| Feature Flag 定义 | `crates/warp_core/src/features.rs` |
| Cargo workspace deps | `Cargo.toml` `[workspace.dependencies]` |
| App features | `app/Cargo.toml` `[features]` |
| 同步边界记录 | **本文档头部**（原 `specs/upstream-merge-plan.md` 已删） |
| 详细变更记录 | **history.md §0 / §12–§15**（原 `specs/upstream-changes-detailed.md` 已删） |
| 跳过项终局结论 | **history.md §3.2 / 本文档决策速查表**（原 `specs/remaining-cherry-pick.md` 已删） |
| 验证纪律 | `AGENTS.md` §5.6.1 |
| 上游源码对照（**常驻**） | `/Users/zhong/project/.worktrees/upstream-master` |

> **上游 worktree 是常驻设施，不要删除。** 它挂在 `upstream/master` 的 detached HEAD 上，
> 并有**独立**的 codegraph 索引（主仓 299 MB / 上游 427 MB，互不覆盖），
> 用于对照上游实现与查上游侧调用方。
>
> ```bash
> # 每轮同步前更新
> git fetch upstream master
> cd /Users/zhong/project/.worktrees/upstream-master
> git checkout --detach upstream/master
> codegraph sync
> ```
>
> 若意外删除，完整重建（**两步，缺一不可**）：
>
> ```bash
> git worktree add /Users/zhong/project/.worktrees/upstream-master \
>   upstream/master --detach
> cd /Users/zhong/project/.worktrees/upstream-master
> codegraph init          # 不是 sync——见下方坑位
> ```
>
> **坑位一：新 worktree 必须先 `init`。** 直接跑 `codegraph sync` 会正常输出
> 「Indexed 4,101 files / 133,484 nodes」然后报 `CodeGraph not initialized`——
> 索引算完了但没落盘，`.codegraph/` 不会创建。先 `init` 才写库。
>
> **坑位二：`.codegraph/` 需写进 `.git/info/exclude`。** 上游 `.gitignore` 不含
> 该条目（那是 Zap 本地加的），且 `.codegraph/.gitignore` 只忽略目录内部文件、
> 不忽略目录本身，所以会污染 worktree 的 `git status`。已写入
> `.git/info/exclude`（worktree 与主仓共享该文件），重建后无需重做。

---

## 5. 经验总结

1. **大胆删除**：上游新增的完整 crate（`warp_tui` 等）若依赖本地没有的胶水层，直接删——别试图修补。
2. **测试优先删**：Cherry-pick 带来的测试编译失败 90% 是上游专有 helper 缺失，删测试最快。
3. **品牌替换要全**：不仅代码字符串，`Cargo.toml` description、二进制名、帮助文本都要改。
4. **Zap 本地功能禁止覆盖**：OMP 集成、OhMyPi 模型选择器、CLI agent session 等 Zap 自研代码，
   上游 cherry-pick **不得覆盖**。合并时若冲突，取本地版本。已在核心原则中统一声明。
   （`oh_my_pi.rs` 已删除，是未被编译的死代码）
5. **记录同步边界**：每次合并必须在 `specs/upstream-merge-lessons.md` 头部记录 commit hash，方便下次 diff。
6. **分阶段验证**：每阶段结束跑 `cargo check -p warp`，别攒到最后才发现基础设施坏了。

---
