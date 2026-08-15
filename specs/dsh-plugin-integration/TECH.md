# 计划:dsh 插件集成(第二阶段,依赖 webui 集成完成)

## 目标与原则

webui 集成落地后,dsh 在 Zap 的 WebView 中运行。本计划:把 Zap 的能力以**插件**方式
逐个注入 dsh,让 dsh 跟随 Zap 的世界(目录、项目、终端),而非独立孤岛。

**原则(每功能 = 三件套)**:
- Zap 侧一个服务(能力提供者)
- dsh 侧 `zap-bridge` 插件内的一个注册(工具/上下文/事件)
- 一条通道(启动时注入的 `ZAP_BRIDGE_ADDRESS`)

dsh 一切皆插件:每个集成功能 = `cordis.yml` 一行配置 + bridge 插件一段注册代码,独立增量、
可单独启用/回滚。**不做**的事件映射/自研 UI 不在本计划内。

## 前置依赖

- webui 集成完成(验收 6 条全过):dsh runtime 自动管理、DSH_HOME 隔离、动态端口
- `ZAP_BRIDGE_ADDRESS` 已注入(webui 计划中已预留,本计划落地为真实服务)

## 阶段 0:桥通道(前置,最小可用)

**目标**:Zap ⇄ dsh 有一条可信的双向通道,可承载后续所有插件。

**实现**:
- Zap 侧:复用 `http_server`(或 `ipc`)在 127.0.0.1 随机端口起 bridge 服务:
  - `POST /rpc` JSON-RPC 2.0(Zap 能力调用:list_files / read_file / workspace_dir / terminal_context)
  - 启动 dsh 时注入 `ZAP_BRIDGE_ADDRESS=http://127.0.0.1:<port>`
- dsh 侧:`zap-bridge` 插件(TS,独立小包,`cordis.yml` 一行挂载):
  - 启动握手:`bridge/hello`(报告 dsh 版本、workspace、能力清单)→ Zap 记录连接状态
  - 断线重连:Zap 崩溃重启 → dsh 侧周期性探测;dsh 重启 → 重新握手
- 协议:JSON-RPC 2.0 over HTTP;错误码规范;所有请求带 timeout

**验收**:
1. dsh 面板打开 → bridge 握手成功(Zap 侧可见连接状态)
2. Zap 侧调用 bridge 服务一个探活方法,往返 <100ms
3. kill dsh → 重启 → 自动重新握手,无需重启 Zap

**工作量**:中(Zap 服务 + TS 插件骨架 + 握手协议)

## 阶段 1:workspace 同步(最小闭环,验证桥)

**目标**:dsh 的 workspace 跟随 Zap 当前项目/活动终端目录。

**实现**:
- 简单版(先做):启动时把 Zap 当前项目目录写入 dsh 启动配置(cordis.yml 生成时注入),
  面板打开即指向正确目录
- 进阶版:Zap 监听活动终端 cwd 变化(terminal 现有事件)→ bridge 推送 → dsh 更新 workspace
  (dsh 侧需确认 workspace 运行时更新 API;若无,降级为"下次会话生效"并记录)

**验收**:
1. 面板打开,dsh 会话中 `pwd`/文件工具显示 Zap 当前项目目录
2. Zap 切换项目后新开会话,目录跟随(进阶:运行中目录实时跟随)
3. 目录含空格/中文等特殊字符,无转义错误

**工作量**:小-中(主要风险在 dsh workspace API 调研)

## 阶段 2:项目文件注入(Zap 文件树能力进 dsh)

**目标**:dsh agent 可用 `zap_*` 工具访问 Zap 的文件能力(与 Zap UI 文件树一致)。

**实现**:
- Zap 侧服务:`zap_list_files`(repo_metadata 文件树,含 .gitignore 语义)、
  `zap_read_file`、`zap_search`(代理 warp_ripgrep)、`zap_open_in_editor`
- dsh 侧:`zap-bridge` 注册同名工具(`ctx.tools.register`),执行时经桥调用 Zap
- 与 dsh 自带 fs 工具关系:并存,`zap_*` 前缀区分;文档说明适用场景
  (一致性优先用 zap_*,性能/沙箱场景用 dsh 原生)

**验收**:
1. 会话中调用 `zap_list_files`,结果与 Zap 文件树一致(含 gitignore 过滤)
2. `zap_search` 返回结果可点击跳转(UI 动作可后置)
3. 工具 schema 完整(参数校验、错误信息友好)

**工作量**:中(dsh 工具注册 API 调研 + Zap 服务暴露)

## 阶段 3:终端上下文注入

**目标**:dsh agent 能看到 Zap 当前终端上下文(最近命令/输出)。

**实现**:
- Zap 侧:terminal 现有 API 提取当前活动终端最近命令与输出(需调研可用事件/存储)
- dsh 侧:作为会话上下文注入(agent.inject 或 context 插件方式,按 dsh API 选择)
- 隐私开关:设置项控制是否注入、注入深度(命令数)

**验收**:
1. 会话中 agent 能引用当前终端最近命令(回答"我刚跑的什么命令"类问题)
2. 关闭隐私开关后,上下文不再注入
3. 终端无活动内容时,注入为空,不报错

**工作量**:中-高(terminal 数据提取 + 注入机制调研;隐私策略需产品决策)

## 阶段 4(可选):webview JS 注入 UI 动作

**目标**:dsh UI 内出现 Zap 动作(如"在 Zap 中打开此文件")。

**实现**:webview 注入 JS + IPC(复用双光标修复的 JS→IPC 链路),在 dsh UI 挂动作。
**前置评估**:DOM 结构依赖、dsh 版本漂移风险;若 dsh UI 无稳定 hook 则放弃,
改由 `zap_*` 工具返回结果承载动作(如 zap_open_in_editor 结果带按钮)。

**验收**(仅当评估通过):按钮出现、点击后 Zap 打开文件、焦点正确。

**工作量**:中(评估先行)

## 总验收(阶段 0-3 完成时)

1. 桥通道稳定:dsh 重启自动重连,Zap 重启后新会话正常
2. workspace 跟随 Zap 项目目录
3. `zap_*` 工具可用且与 Zap UI 一致
4. 终端上下文注入可控(隐私开关)
5. 全部功能可单独禁用(cordis.yml 配置行),回滚无残留

## 风险与对策

| 风险 | 对策 |
|---|---|
| dsh 插件 API(工具注册/workspace/context)不熟悉 | 每阶段先做 API 调研 spike,输出结论再实现 |
| dsh preview 版本漂移 | 桥协议独立于 dsh 内部 API;dsh 升级时 bridge 插件单独验证 |
| workspace 运行时更新无 API | 降级:会话级生效;记录并报告 |
| 终端上下文提取复杂/敏感 | 阶段 3 前置调研 + 隐私开关;可裁剪范围 |
| 桥成为单点 | 每功能独立注册,桥挂起时 zap_* 工具返回明确错误,不阻塞 dsh 原生能力 |

## 实施顺序

阶段 0(桥)→ 阶段 1(workspace,验证闭环)→ 阶段 2(文件工具)→ 阶段 3(终端上下文)
→ 阶段 4(可选评估)。每阶段独立可交付、可回滚。
