# Terminal Control And Confirmation Card Layout Design

## Goal

修复 Agent 工具确认卡中 JSON 入参编辑器被横向压缩的问题，并为 AI 提供一个与 Agent 对话取消解耦的显式终端 `Ctrl+C` 中断能力。

## Scope

本次改动包含两个边界清晰但需要共同交付的行为：

1. 工具确认卡中的 JSON block、内部 frame 和实际 `Input` 必须使用可用消息列宽度，长内容通过滚动展示，不能把控件压缩成窄条。
2. AI 可以通过独立的高风险终端控制工具向可见终端发送 ETX（`Ctrl+C`，字节 `0x03`），而 Agent 的取消按钮继续只取消 turn，不操作终端。

本次不改变后台进程所有权，不把 Agent 取消映射为 signal，不增加任意控制字符写入能力，也不重构通用 Input 组件。

## Layout Design

### Root Cause

现有 GPUI 回归测试只对 `agent-tool-json-block` 外层做宽度断言。外层虽然设置了 `.w_full().min_w_0()`，但内部 frame 和 `Input` 没有独立的布局断言；在真实侧栏的嵌套 flex 环境中，内部控件仍可能按内容收缩。

### Selected Approach

保留现有 JSON code editor，在 `tool_card_json_block` 内显式建立三层宽度 contract：

1. JSON block 外层：`.w_full().min_w_0()`。
2. 代码 frame：全宽、最小宽度为零、固定高度、裁剪溢出。
3. `Input`：`.flex_1().min_w_0().h_full()`，由 flex 分配剩余宽度，不仅依赖百分比宽度推导。

代码 frame 与实际 Input 增加独立 debug selector。GPUI 测试分别读取三层 bounds，断言 frame 和 Input 都使用消息列的大部分可用宽度，并且不超过外层边界。

不采用固定 `min-width`，因为它会在更窄的侧栏中溢出；不替换为普通文本，因为需要保留 JSON 高亮、选择和滚动能力。

## Terminal Control Design

### Public Contract

新增独立工具 `terminal.control`，Function Calling 规范化名称为 `terminal_control`。第一版仅支持：

```json
{
  "target": "terminal-resource-id",
  "action": "interrupt"
}
```

`target` 和 `action` 必填，`action` 只接受 `interrupt`。该工具属于开放世界写操作，风险等级为 High，不支持并行调用。Agent Auto 模式直接执行，Manual 模式展示审批卡。

成功结果为结构化数据：

```json
{
  "target": "terminal-resource-id",
  "action": "interrupt",
  "sent": true,
  "readiness_before": "command_running"
}
```

### Lifecycle Separation

终端控制操作不使用 Agent turn cancellation token 表达 `Ctrl+C`，也不复用 `terminal.exec` 的 safe-replace preflight。

```text
Agent cancel
  -> TurnCancelled
  -> detach tool waiter when needed
  -> never writes terminal bytes

terminal.control(action=interrupt)
  -> high-risk approval
  -> terminal supervisor validates readiness
  -> writes ETX 0x03
  -> returns bounded structured result
```

如果已有 `terminal.exec` observer 正在观察前台命令，发送 ETX 后 observer 继续由真实 OSC `CommandFinished` 或新 prompt epoch 收口；control 工具不伪造 exit code，也不等待 PTY EOF。

### Supervisor State Policy

终端 actor 必须原子检查 supervisor 状态并决定是否写入：

| Readiness | `interrupt` 行为 |
| --- | --- |
| `CommandRunning` | 允许发送 ETX |
| `SubmissionPending` | 允许发送 ETX，目标仍是刚提交的前台输入/命令 |
| `AwaitingPrompt` | 返回 `terminal_not_running`，零写入 |
| `Ready` | 返回 `terminal_not_running`，零写入；清理半行输入仍由下一次 `terminal.exec` safe-replace 完成 |
| `Initializing` / `PromptRendering` / `ClearingInput` | 返回 `terminal_busy`，零写入 |
| `Unknown` | 返回 `readiness_unknown`，零写入 |
| `Disconnected` | 返回 `terminal_disconnected`，零写入 |

只允许中断明确的前台运行状态，避免 AI 在空闲 prompt 上发送无意义 ETX，或在 readiness 不可信时盲写控制字符。

### Terminal Bridge

在 terminal core 中新增受 supervisor 管理的 control handle/request/result，而不是暴露任意 `TerminalInputHandle` 给 Agent。SSH actor 增加控制消息分支，由 actor 内的 supervisor 完成状态检查和 ETX 写入，从而保证判断与写入之间没有竞态窗口。

Public MCP registry 为 terminal control 使用独立 session handle/capability，并由 `terminal_view` 注册到当前 SSH terminal。工具层负责 schema 解析、目标解析、权限/风险声明和结构化结果映射。

### Agent Guidance

Agent system prompt 增加以下规则：

- 用户明确要求停止、打断或发送 `Ctrl+C` 到当前可见终端任务时，使用 `terminal_control`。
- `terminal_exec` 用于在安全 prompt 上输入并执行命令，不能用字符串 `"\\u0003"` 模拟控制操作。
- 点击 Agent 的取消按钮只取消对话，不代表终端任务已中断。
- 只有工具观测明确返回 `sent=true` 时，才能声称已经向终端发送 `Ctrl+C`。

## Error Contract

终端控制工具使用稳定的机器可识别错误文本：

```text
terminal_not_running
terminal_busy
readiness_unknown
terminal_disconnected
control_unavailable
```

所有失败路径必须零写入。工具调用取消如果发生在写入前则返回取消；ETX 一旦由 actor 写入，结果不能被回滚，迟到的 Agent turn 取消只影响对话等待。

## Testing Strategy

该改动涉及 UI 布局、公共工具 contract、终端 actor 和状态机，采用 TDD。

### UI Regression Tests

- 先扩展现有 sidebar tool-confirm GPUI 测试，使其读取 JSON block、内部 frame 和 Input 的 bounds。
- 在生产修改前确认 frame 或 Input 的宽度断言失败。
- 修复后断言三层均使用可用消息列宽度，并保持在外层边界内。

### Terminal Supervisor Tests

- `CommandRunning` 与 `SubmissionPending` 返回单个 ETX write effect。
- `Ready` / `AwaitingPrompt` 返回 `terminal_not_running` 且零写入。
- busy、unknown、disconnected 状态均零写入并返回对应错误。
- control 不移除或伪造已有 exec observer；真实 command-finished 仍能完成 observer。

### Bridge And Tool Tests

- terminal control handle 把 request 交给 SSH actor 并返回结构化结果。
- Public MCP schema 只接受 `action=interrupt`，并将工具声明为 High risk。
- registry 能按 terminal target 解析 control handle，未知目标和未注册能力返回稳定错误。
- Agent adapter 暴露规范化的 `terminal_control`；Auto 直接执行，Manual 高风险调用触发审批。
- system prompt 指导模型区分 `terminal_exec`、`terminal_control` 和 Agent cancel。

### Verification

至少运行：

```text
rtk cargo test -p terminal terminal_control
rtk cargo test -p terminal_view terminal_control
rtk cargo test -p public_mcp terminal_control
rtk cargo test -p agent_runtime terminal_control
rtk cargo test -p ai_chat_view sidebar_mode_tool_confirm_json
rtk cargo test -p ai_chat_view
rtk cargo check -p terminal -p terminal_view -p public_mcp -p agent_runtime -p ai_chat_view -p main
rtk git diff --check
```

## Acceptance Criteria

- 工具确认卡的 JSON frame 和实际 Input 不再收缩成左侧窄条。
- 长 JSON 保持代码高亮并通过滚动浏览，窄侧栏不溢出卡片。
- AI 可以通过 `terminal_control(action=interrupt)` 向明确运行中的可见终端发送一次 ETX。
- Agent 取消继续立即结束对话，且不会发送 ETX、signal 或关闭 terminal。
- 非运行、未知、断开和 automation 冲突状态全部 fail closed、零写入。
- control 结果不依赖 EOF，不伪造命令完成或 exit code。
- 既有 `terminal.exec` safe-replace、后台/nohup、observer detach 和 turn-id 隔离语义保持不变。
