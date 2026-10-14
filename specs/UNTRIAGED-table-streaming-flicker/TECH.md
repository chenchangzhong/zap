# UNTRIAGED-table-streaming-flicker: 开发计划

> 状态: **Task 0-4 已实施并验证(判据已 GREEN)**;Task 5 待用户肉眼验收;尚未提交。
> 症状: Zap 智能体页助手回复含 Markdown 表格时,表格"一闪一闪"。

## 1. 目标与验收

**目标**: 助手回复流式输出 Markdown 表格时,表格的 section 结构在整个流式过程中保持稳定 —— 既不回落到原始 markdown 文本,也不让未写完的尾行掉到表格外面。

**验收判据(唯一、可执行)**:

- 已落地的判据测试 `streaming_table_frames_never_fall_back_or_flip` 由 RED 转 GREEN(8 帧全部渲染成单一 `Table` section)。
- 最终输出(流结束后)与现状完全一致:同一份表格 markdown 得到相同的 行/列 数与单元格文本。
- 用户可见验收: 让助手流式输出一个 3 列表格,肉眼确认不再闪。

**Aegis Visibility**: 本次改动落在"表格行判定"这个被两处实现重复拥有的 owner 上,并外溢到共享 parser 的下游(编辑器/Notebook),所以需要一份计划固定 判据、兼容边界 与 回退点;不是"改一行就好"的局部修补。

## 2. 现状机制(已运行验证)

流式增量到达时的解析链路(每收到一个增量就重跑一遍,结果存进 model,渲染层直接用):

```
服务端流式增量
  → app/src/ai/agent/api/convert_from.rs:580-586
      impl From<api::message::AgentOutput> for AIAgentText   ← 助手正文的 section 在这里组装
      → app/src/ai/agent/util.rs:32 parse_markdown_into_text_and_code_sections
          → crates/ai/src/gfm_table.rs:70 maybe_collect_gfm_table_lines  (收集器: 决定哪些行属于表格)
          → app/src/ai/agent/util.rs:185 parse_agent_output_table
              → crates/markdown_parser: parse_markdown_with_gfm_tables  (解析器: 真正的 GFM 规则)
              → 要求结果 .exactly_one()
          → 任一步返回 None ⇒ util.rs:53-68 把整段当纯文本
  → AIAgentText 存入 model → AIBlock 渲染(渲染层不再重解析)
```

推论: 闪烁频率 == 服务端增量频率(不是帧率);且错误的分节结果会被写进 model,因此**修复必须在解析层,渲染层兜不住**。

**根因**: "表格行"的判定被两处各自实现,且规则不一致:

| 流式中间态 | 收集器 | 解析器 | 实测结果 |
|---|---|---|---|
| `\| 1 \| 2 \|` | 收(2 列) | 表格行 | 一致 ✅ |
| `\| 1 \| 2` | **收** | **表格外**(末列必须有尾竖线) | 冲突 → `exactly_one()` 失败 → **整表塌回原始文本** |
| `\| 1 \|` | **不收**(列数≠表头) | 表格行 | 冲突 → 尾行在"表格行/正文段落"之间翻转 |
| 纯文本行 | 不收 | 表格外 | 一致 ✅ |

即: 解析器违反 GFM 的"首尾竖线可省",收集器违反 GFM 的"数据行列数少于表头时补空单元格"。

## 3. 技术栈与基线

- Rust / WarpUI(自研 UI 框架,元素树每帧重建)/ nom(markdown parser)。
- **上游对照(逐行比对过,不是"看起来一样")**:
  - `crates/ai/src/gfm_table.rs`: 与 `upstream/master` **完全一致**(0 差异)。
  - `crates/markdown_parser/src/markdown_parser.rs`: **表格相关实现逐行一致**;该文件与上游的差异只有两处且与表格无关 —— (a) 上游有 HTML comment 处理(`parse_html_comment` / `parse_html_comment_block` / `InlineToken::Comment`),本地缺失;(b) 测试模块名 `_tests.rs` / `_test.rs`。
  - `app/src/ai/agent/util.rs`: 表格逻辑段(`parse_agent_output_table`、legacy 回落)逐行一致,差异仅为 `use` 排序。
  - 结论: **这是继承缺陷,不是移植漏行**;不存在"上游已修好、可以照搬"的实现 —— 上游同款解析器同样要求末列有尾竖线。
  - 附带观察(与本计划无关,需单独确认是同步落后还是有意移除): 本地缺少 HTML comment 的解析 → `<!-- ... -->` 会按字面渲染。另开条目处理。
- **规范依据**: [GFM §4.10 Tables](https://github.github.com/gfm/) —— "A leading and trailing pipe is also [optional]"; "If there are a number of cells fewer than the number of cells in the header row, empty cells are inserted"。

**基线引用**:

- `crates/markdown_parser/src/markdown_parser.rs:537-628`(表格解析)
- `crates/ai/src/gfm_table.rs:70-110`(收集器)
- `app/src/ai/agent/util.rs:43-69`、`:185-200`(section 组装与表格解析)
- `app/src/ai/agent/util_tests.rs`(判据测试落点)
- `crates/editor/src/content/buffer.rs:839-840`(共享 parser 的开关门控)

## 4. 兼容性边界

- **共享面**: `parse_markdown_with_gfm_tables` 同时服务编辑器/Notebook 的表格渲染,由 `FeatureFlag::MarkdownTables` 门控 → 必须跑下游 crate 测试(见 Task 4)。
- **不变**: 流结束后同一份 markdown 的解析结果不变(表格行列数、单元格文本、对齐)。
- **可见行为变化(预期,需在 commit message 说明)**:
  1. 流式过程中未写完的尾行会以"表格内逐格填充"的方式出现(而不是先掉到表格外的正文段落);
  2. 末列省略尾竖线的行,现在归入表格(此前若列数不匹配会被当作正文)。
- **不做**: 首竖线可省(GFM 也允许)。它同样不在两个实现里,但会扩大爆炸半径,本次不碰,另开条目。

## 5. Change Necessity

- **不能只改文档/配置**: 缺陷在渲染链路的判定逻辑里。
- **不能只做消费者侧兜底**(让 `util.rs` 容忍"Table + 尾行"): 两套判定仍然并存,任一侧规则变化即复发;且治不了 `| 1 |` 的尾行翻转。属消费者补丁,按补丁形状判据必须上溯到 owner。
- **不做流式态特判**(把 `PartiallyReceived` 传进解析层): 能治两类翻转,但让解析层知道 UI 的"流式中"概念,契约变更最大,且根因(两套判定)保留。
- **最小边界**: 解析器"末列终止符"1 处 + 收集器"行结束条件"1 处 + 判据测试(已落地)。

## 6. TDD Route

- **mode**: `off`(项目/Aegis 当前配置)。
- **decision**: `skipped`(无 strict 授权)。
- **test posture**: 本次验证已先落地 8 帧判据测试作为回归判据(RED),实施阶段不强制 RED/GREEN 循环,但判据测试必须先绿。
- **authority**: 用户本轮指令"先验证,然后写一份开发计划"。

## 7. 任务分解

> 任务顺序即依赖顺序。Task 1 与 Task 2 互相独立可单独回退,但**只做 Task 2 修不掉"整表塌回"**,所以按 1 → 2 落地。

### Task 0 — 判据测试(已完成,本次验证产出)

- **文件**: `app/src/ai/agent/util_tests.rs`(新增 `streaming_table_frames_never_fall_back_or_flip`)
- **内容**: 8 帧中间态矩阵,断言每帧都必须产出**恰好一个** `Table` section。
- **实施前基线**: **RED**(5/8 帧失败,原始输出见附录 A)。这是预期的 RED 门,不是回归。
- **实施后**: **GREEN**(8/8),见附录 B。
- **边界**: 矩阵起点是"表头 + 完整分隔行"——更早的分隔行未写完时无法判定这是表格,那一次 text→table 是**固有**过渡,不在判据范围内(不允许为了它去猜测性渲染)。

### Task 1 — 解析器接受"末列省略尾竖线"

- **文件**: `crates/markdown_parser/src/markdown_parser.rs`(`parse_table_row` 578-590,`parse_table_cell` 594-628;`parse_table_separator` 631-675 同类处理)
- **最小改动**: 让**末列**允许以 行尾/EOF 结束,而不是必须撞到 `|`。注意不要吞掉换行 —— `parse_table_row` 收尾仍需 `alt((line_ending, eof))`;必要时把"终止符"判定从 cell 层提到 row 层。
- **同类处理**: 分隔行末列(`parse_separator_cell`)同样允许省略尾竖线。它本身不造成闪烁(收集器在分隔行列数补齐前不会收),但两处不一致属同一 bug 类,一并修掉,否则留下第二个"两套判定"。
- **效果**: 单独完成即可消掉"整表塌回"(冲突矩阵第 2 行)。
- **验证**:
  - 新增单元用例: `| A | B |\n| --- | --- |\n| 1 | 2` 必须解析为 **1 行** `Table(rows=1)`(现状是 `Table(rows=0)` + `Line("| 1 | 2\n")`);落点 `crates/markdown_parser/src/markdown_parser_test.rs`。
  - `cargo nextest run -p markdown-parser`
- **兼容性**: 仅影响"末列无尾竖线"的输入 —— 此前被当作段落,现在归入表格(GFM 正确)。

### Task 2 — 收集器行结束条件改为"含 `|`"

- **文件**: `crates/ai/src/gfm_table.rs:98-107`
- **最小改动**: 删除 `has_wrong_column_count` 这个 break 条件,改为"**该行不含 `|` 就结束表格**";保留 `is_blank` 与 `should_stop`(代码块围栏)。
- **效果**: 单独完成**不能**修掉塌回,但消掉尾行翻转(冲突矩阵第 3 行)。
- **验证**:
  - 新增用例: 短行(如 `| 3 |` 对 2 列表头)被收入表格;纯文本行(如 `After`)不收入;落点 `crates/ai/src/gfm_table_tests.rs`。
  - `cargo nextest run -p ai`
- **风险**: 表格尾部"含 `|` 的正文行"会被吸入(如 `a | b`)。当前规则下"列数恰好相同"同样会被吸入,不是新增类别;用"含 `|`"显式限定并加测试钉住。

### Task 3 — 判据转 GREEN

- **命令**: `cargo nextest run -p warp -E 'test(streaming_table_frames_never_fall_back_or_flip)'`
- **期望**: 8/8 帧均为单一 `Table` section;若仍有帧失败,回到 Task 1/2 定位该帧而不是放宽判据。

### Task 4 — 共享下游回归

- **命令**:
  - `cargo nextest run -p markdown-parser`
  - `cargo nextest run -p warp_editor -E 'test(table)'`
  - `cargo nextest run -p warp -E 'test(ai::agent::util::tests)'`
  - Notebook 表格用例(实现时按现有过滤名确认)
  - `cargo check`(PR 门)
- **期望**: 全绿;若有红,先判断是"断言了旧的尾竖线必需行为"还是真回归。

### Task 5 — 用户可见验收(由用户执行,一次)

- 在 Zap 智能体页让助手流式输出一个 3 列表格,确认不再闪。
- 同时确认: 流结束后表格行列数、单元格文本、对齐 与改动前一致。

### Task 6 — 独立 follow-up(不在本次范围;**已评估,决定推迟**)

**现象**(代码级已核实): 流式期间表格 UI 状态句柄每帧新建 —— 唯一赋值点是
`app/src/ai/blocklist/block.rs:2242`(在 `handle_complete_output` 内,只有回复**结束**时才跑),
而渲染侧 `app/src/ai/blocklist/block/view_impl/common.rs:1250-1254` 每帧 `.unwrap_or_default()`,
于是每次重排都拿到全新的 `TableStateHandle` / `ClippedScrollStateHandle`。与本次闪烁无关。

**纠正(2026-10-14 复核,推翻本文档最初的"每帧全量测量"说法)**:

- `docs/warpui-transition-animation.md:280` 确认:重绘期间**每帧都会重新 layout + paint 整棵树**;
  我最初据此推断"每秒 60 次重测"——**推断错了**。
- 实测 `app/src/ai` 下 `repaint_after` / `notify_after` **数量为 0**:流式纯文本回复**没有重绘心跳**。
- 因此重测实际发生在**每次重排事件**时(新增量、滚动、鼠标移动、resize、主题变更),
  而不是每秒 60 次。收益比最初描述小一档。

**收益**:

| 收益 | 确定度 | 说明 |
|---|---|---|
| 流式期间宽表格横向滚动位置不再归零 | 确定(代码级) | 每帧新建 `ClippedScrollStateHandle` |
| 流式期间重排不必重测全部单元格 | 中等,有前提 | 需"流式 + 大表格 + 高频重排"三者叠加才可感知 |
| 代码一致性 | 确定 | `block.rs:352-356` 声明此类句柄"必须跨帧持久",表格在流式期不满足 |

性能量级(**未实测,仅估算**): 4×6 表每次重排约 48 次文本 layout(无约束测量 + 约束布局各一遍),
20×6 约 240 次;按单次短文本 layout 十几微秒量级估,约 1–5ms/次重排,滚动/拖拽时才会叠加成可感知掉帧。

**成本**:

1. **必须连带做缓存失效,否则是净回归**: `invalidate_intrinsic_width*` 在**生产代码里 0 个调用点**
   (只有 `crates/warpui_core/src/elements/table/mod.rs` 的定义与测试)。句柄一旦持久化,intrinsic
   列宽会永远停在第一帧的值 → 流式内容变长后列宽不跟着长,表格宽带/换行都会错。
   所以本任务不是"两行提前创建句柄",而是"提前创建 **+** 内容变更时失效"。
2. **与并行工作线撞同一文件**: `block.rs` 正被另一条线反复修改,冲突概率高。
3. **无判据测试**: 现有判据只覆盖"不翻转",覆盖不了"列宽随内容增长"。

**决定: 推迟(优先级低)**。唯一确定的用户可见症状是"流式那几秒里宽表滚不动",至今无人反馈。
**满足任一条件再做**: ① 有人反馈流式期宽表滚动跳回/滚不动;② 要做单元格级交互(hover/选择/tooltip),
句柄必须跨帧;③ 出现"大表格流式期间卡顿"的实际采样;④ 并行线稳定后整体重做表格渲染时顺带。

**真要做时的正确做法**(防止只加句柄就上):

1. 在**流式更新路径**按 section 数**增量补齐**句柄(保留已有、只 append),而不是 `handle_complete_output`
   里整体替换;
2. 内容变化后调 `invalidate_all_intrinsic_widths()`(或按内容签名判断)—— **硬前提**;
3. 判据三条: ① 列宽随流式内容增长;② 内容未变的重排之间不重测(可用列宽不变或 `render_fn`
   调用计数断言);③ 流式期横向滚动位置保持。

## 8. 验证矩阵(判据)

| # | 帧(header = `\| A \| B \|\n\| --- \| --- \|\n`) | 现状 | 期望 |
|---|---|---|---|
| 1 | `header` | Table(0 rows) | Table |
| 2 | `header\|` | Table + PlainText | Table |
| 3 | `header\| 1` | Table + PlainText | Table |
| 4 | `header\| 1 \|` | Table + PlainText | Table |
| 5 | `header\| 1 \| 2` | **PlainText(整表塌回)** | Table |
| 6 | `header\| 1 \| 2 \|` | Table(1 row) | Table |
| 7 | `header\| 1 \| 2 \|\n` | Table(1 row) | Table |
| 8 | `header\| 1 \| 2 \|\n\| 3 \|` | Table(1 row) + PlainText | Table |

## 9. 风险与回退

| 风险 | 处置 |
|---|---|
| 共享 parser 改动打翻编辑器/Notebook 表格 | Task 4 先跑;若变红,判断是否为同一 GFM 语义问题,否则**保留 Task 1**、把 Task 2 降级为"只对末行放宽"的保守版 |
| 收集器放宽导致正文被吸入表格 | 用"含 `|`"显式限定 + 加用例;必要时加"表格已在尾部连续收纳 N 行"之类的收紧条件(仅在确有误吸案例时) |
| 判据测试被"改判据"绕过 | 判据是 section 结构,不接受改成"允许 PlainText" |
| 回退粒度 | Task 1 / Task 2 独立,可分别 revert;单 Task 1 即可消掉最主要的整表塌回 |

## 10. Retirement / 清理

- 无旧路径需要删除,不新增 fallback / adapter / 兼容分支。
- `parse_agent_output_table` 的 `.exactly_one()` **保留**: Task 1 之后它重新变得成立,作为"收集器与解析器一致"的契约断言,是防止本 bug 复发的护栏。
- 验证期用过的临时探针(已删)不留痕;唯一新增测试产物是 Task 0 的判据测试。

## 附录 A: 修复前基线证据(原始输出)

下列输出采集于**修复前**,用于固定 bug 现象;修复后同一命令全绿(见附录 B)。

```
流式表格的中间态不应改变 section 结构,但以下帧发生了回落或翻转:
帧 `row-lone-pipe`    输入 "| A | B |\n| --- | --- |\n|"       渲染成 ["Table(cols=2, rows=0)", "PlainText(\"|\")"]
帧 `row-cell1-start`  输入 "| A | B |\n| --- | --- |\n| 1"     渲染成 ["Table(cols=2, rows=0)", "PlainText(\"| 1\")"]
帧 `row-cell1-closed` 输入 "| A | B |\n| --- | --- |\n| 1 |"   渲染成 ["Table(cols=2, rows=0)", "PlainText(\"| 1 |\")"]
帧 `row-cell2-start`  输入 "| A | B |\n| --- | --- |\n| 1 | 2" 渲染成 ["PlainText(\"| A | B |\\n| --- | --- |\\n| 1 | 2\")"]
帧 `row2-cell1-closed` 输入 "…| 1 | 2 |\n| 3 |"               渲染成 ["Table(cols=2, rows=1)", "PlainText(\"| 3 |\")"]
```

解析器级证据(临时探针,已删):

```
n=1 ["Table(rows=1)"]                          <- "| A | B |\n| --- | --- |\n| 1 | 2 |"
n=2 ["Table(rows=0)", "Line(\"| 1 | 2\\n\")"]   <- "| A | B |\n| --- | --- |\n| 1 | 2"   ← 无尾竖线
n=1 ["Table(rows=1)"]                          <- "| A | B |\n| --- | --- |\n| 1 |"
n=2 ["Table(rows=0)", "Line(\"| 1\\n\")"]       <- "| A | B |\n| --- | --- |\n| 1"
n=1 ["Table(rows=1)"]                          <- "| A | B |\n| --- | --- |\n|"
n=1 ["Table(rows=0)"]                          <- "| A | B |\n| --- | --- |"
```

复现命令:

```bash
cargo nextest run -p warp -E 'test(streaming_table_frames_never_fall_back_or_flip)'
```

## 附录 B: 实施记录(2026-10-14)

改动文件:

| 文件 | 改动 |
|---|---|
| `crates/markdown_parser/src/markdown_parser.rs` | 新增 `at_markdown_line_end` / `take_until_markdown_line_end`;`parse_table_row` 末列允许省略尾竖线;`parse_table_separator` 同规则;抽出 `separator_alignment`;`parse_separator_cell` 改为**行内**取内容(`take_till`),不再跨行匹配 `|` |
| `crates/ai/src/gfm_table.rs` | 行结束条件从"列数必须等于表头"改为"不含竖线才结束" |
| `crates/markdown_parser/src/markdown_parser_test.rs` | +5 条用例(无尾竖线的行/EOF 无换行/表头与分隔行均省/短行列仍属表格/不以竖线开头的行仍结束表格) |
| `crates/ai/src/gfm_table_tests.rs` | +3 条用例(短行保留/流式尾行保留/无竖线行结束表格) |
| `app/src/ai/agent/util_tests.rs` | +1 条判据测试(8 帧矩阵) |

实施中发现并修掉的额外缺陷(由新分支暴露): `parse_separator_cell` 原来用 `take_until("|")` 取内容,会**跨过行尾**去匹配下一行的 `|`,导致 `| --- | ---\n| 1 | 2` 这类"分隔行省略尾竖线"的输入被误判;现在限定在同一行内。

验证证据:

```
cargo nextest run -p markdown_parser                            139/139 passed
cargo nextest run -p ai -E 'test(gfm_table)'                      7/7   passed
cargo nextest run -p warp -E 'test(ai::agent::util::tests)'      15/15  passed(含判据 8 帧全绿)
cargo nextest run -p warp_editor -E 'test(table)'                54/54  passed
cargo nextest run -p warp -E 'test(markdown_table) + test(notebooks::editor::)'   79/79 passed
cargo check                                                      clean
rustfmt --check(.rustfmt.toml)                                   5 个改动文件全合规
```

共享面收口: `parse_markdown_with_gfm_tables` 的生产调用点只有两处 —— `app/src/ai/agent/util.rs:186`(本次路径)与 `crates/editor/src/content/buffer.rs:840`(编辑器,受 `FeatureFlag::MarkdownTables` 门控),后者已有 54 条表格用例覆盖。

任务状态: Task 0-5 全部完成(Task 5 用户已在 debug 实例确认流式表格不再闪);Task 6 已评估并**推迟**,依据见第 7 节。

提交: `cc850ddcb`(修复 + 判据测试 + 本文件)。
