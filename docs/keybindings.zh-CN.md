# 快捷键参考

## CLI Agent 焦点切换

`Cmd-Up` / `Cmd-Down` 的行为根据当前场景变化：

| 场景 | `Cmd-Up` | `Cmd-Down` |
|---|---|---|
| 无 agent 运行，终端有 block | SelectBlockAbove（选中上一个 block） | SelectBlockBelow（选中下一个 block） |
| Rich Input 打开，焦点在输入框 | 聚焦终端 TUI | — |
| Rich Input 打开，焦点在终端 TUI | — | 聚焦 Rich Input 输入框 |

### 技术实现

- 焦点切换绑定使用 `with_key_binding`，仅在有 `Terminal` 和 `CLI_AGENT_RICH_INPUT_OPEN` 上下文时触发
- SelectBlockAbove/SelectBlockBelow 绑定使用 `with_custom_action`，上下文加入 `!CLI_AGENT_RICH_INPUT_OPEN` 确保不在 agent 打开时抢占快捷键
- `performKeyEquivalent:` 保留原始条件 `(keystrokeIsAssigned && !triggersCustomAction)`：仅有 `with_custom_action` 的快捷键走菜单路径，仅有 `with_key_binding` 的走 Rust dispatch
- WarpHostView 的 `moveToBeginningOfDocument:` / `moveToEndOfDocument:` 重载为 no-op，防止 AppKit NSTextView 本地消费 `Cmd-Up`/`Cmd-Down`

## 终端通用

| 快捷键 | 功能 |
|---|---|
| `Cmd-D` | 分屏 |
| `Cmd-W` | 关闭标签/分屏 |
| `Cmd-T` | 新建标签 |
| `Ctrl-G` | 打开 CLI Agent（需要对应的 flag 开启） |
| `Cmd-Shift-K` | 清屏 |
| `Cmd-/` | 注释选中行 |

（更多 Warp 原有快捷键保留不变，本文件仅记录 Zap 新增或变更的快捷键。）
