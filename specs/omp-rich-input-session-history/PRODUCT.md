# PRODUCT.md — Rich Input 历史菜单读取 omp 会话用户消息

## Problem

CLI agent(omp 等)在终端里运行时,rich input(Ctrl-G 打开)按 ↑ 打开的是 inline history 菜单,内容为 **shell 命令历史**——对正在跟 omp 对话的用户毫无用处。用户想要的是:**当前 omp 会话中自己发送过的消息**(prompt),像普通聊天工具的历史回看一样。同时,输入 `/` 时 slash 命令菜单经常被其它菜单(如 ↑ 历史菜单)挡住无法打开。

## Goals

- rich input 按 ↑ 时,显示**当前 omp 会话的用户消息历史**(发送给 omp 的 prompt 文本)。
- 有 omp 消息时,**只显示 omp 消息**,shell 命令历史与会话条目全部让位。
- **新会话**(尚未发送任何消息)按 ↑ 显示**空菜单**,不得显示其它会话的旧消息。
- 输入 `/` 时 **slash 命令菜单优先级最高**:其它菜单打开时自动关闭并打开 slash。
- Zap **不持久化**任何内容:按需读取 omp 侧磁盘记录,关闭即弃。

## Non-goals

- 持久化 omp 历史到 Zap 侧(数据库/配置)。
- 读取 omp 会话的 assistant 回复内容(仅用户消息)。
- 支持非 omp 的 CLI agent(Claude Code、Codex 等无此文件布局,自然回退命令历史)。
- 修改 omp 侧的会话记录格式。

## User experience

### 场景:历史会话(有消息)

1. 用户终端运行 omp,选择/恢复了历史会话。
2. Ctrl-G 打开 rich input,按 ↑。
3. 弹出 inline history 菜单,**只有该 omp 会话的用户消息**(带 AI prompt 图标),按时间排序(旧在上、新在下,贴近输入框)。
4. 输入文字可前缀过滤;Enter 回填并提交;Escape 关闭恢复原 buffer。

### 场景:新会话(无消息)

1. 用户新开 omp 会话,Ctrl-G 打开 rich input,按 ↑。
2. 菜单为空(显示 no-results 占位)——**不显示其它会话的旧消息,也不回退命令历史**。

### 场景:非 omp agent / 无 omp 会话

- Claude Code、Codex 等 agent,或无法定位 omp 会话文件时:按 ↑ 行为与改动前一致(命令历史 + 会话条目)。

### 场景:输入 /(slash 命令)

1. 任意菜单(如 ↑ 历史菜单)打开时,用户直接输入 `/`。
2. 其它菜单自动关闭,slash 命令菜单立即打开(omp 模式显示 omp 命令列表)。
3. 若 slash 菜单因长运行命令守卫无法打开(非 CLI agent 输入场景),保持旧行为:禁用 slash、不动当前菜单。

### 边界

- omp 会话文件损坏/不可读时:视为"无法定位",回退命令历史(不因损坏文件空白整个菜单)。
- 多终端同 cwd 时:按"最近切换过会话的终端"定位(映射文件 mtime 最新),极端并发下可能显示其它终端的会话消息(可接受的降级)。
