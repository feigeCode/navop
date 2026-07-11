# Terminal Control And Confirmation Card Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复工具确认卡 JSON 编辑器横向收缩，并新增经审批的 `terminal_control(action=interrupt)` 能力，让 AI 可以显式向运行中的可见终端发送 `Ctrl+C`，同时保持 Agent 取消与终端控制解耦。

**Architecture:** UI 在 JSON block、frame、Input 三层建立明确的 flex 宽度 contract。终端控制从 `ExecSupervisor` 的 readiness 状态出发，经 SSH actor 原子校验并写入 ETX，再通过 terminal core handle、Public MCP registry/runtime tool 和 Agent adapter 暴露；Agent cancel 不接入这条链路。

**Tech Stack:** Rust 2024、GPUI/gpui-component、Tokio、SSH PTY actor、OSC 133 terminal supervisor、tool_runtime、Public MCP、agent_runtime。

---

## File Map

- Modify `crates/ai_chat_view/src/agent_cards.rs`: 为 JSON frame/Input 增加明确 flex 约束和 debug selector。
- Modify `crates/ai_chat_view/src/agent_view.rs`: 扩展侧栏 GPUI 回归测试，覆盖 block/frame/Input 三层宽度。
- Modify `crates/terminal/src/types.rs`: 定义 terminal control request/result/error/handle contract。
- Modify `crates/terminal/src/lib.rs`: 导出 terminal control 公共类型。
- Modify `crates/terminal/src/exec_supervisor/model.rs`: 增加公开可映射的 control readiness 与错误。
- Modify `crates/terminal/src/exec_supervisor/mod.rs`: 根据 shell readiness 判定是否允许显式 interrupt。
- Modify `crates/terminal/src/exec_supervisor/tests.rs`: 覆盖允许/拒绝状态和 observer 保留语义。
- Modify `crates/terminal/src/ssh_backend.rs`: 增加 actor control 消息与 `TerminalControlHandle`。
- Modify `crates/terminal/src/terminal.rs`: 暴露 external terminal control handle。
- Create `crates/public_mcp/src/terminal_control.rs`: 定义序列化 request/result/action/readiness。
- Create `crates/public_mcp/src/tools/terminal_control.rs`: 定义 `terminal.control` runtime tool/schema。
- Modify `crates/public_mcp/src/lib.rs`: 导出 terminal control 模块。
- Modify `crates/public_mcp/src/tools.rs`: 导出 terminal control registry builder。
- Modify `crates/public_mcp/src/tools/registry.rs`: 默认 terminal registry 合并 control tool。
- Modify `crates/public_mcp/src/registry.rs`: 注册、解析和调用 terminal control session handle。
- Modify `crates/public_mcp/tests/registry.rs`: 验证 capability 和 target lookup。
- Create `crates/public_mcp/tests/terminal_control.rs`: 验证 schema、风险和结构化调用结果。
- Modify `crates/terminal_view/src/public_mcp.rs`: 桥接 terminal core control handle 并注册到 Public MCP。
- Modify `crates/tool_runtime/src/resource.rs`: 增加 `ResourceCapability::TerminalControl`。
- Modify `crates/tool_runtime/tests/resource_pool.rs`: 验证 control capability target resolution。
- Modify `main/src/public_mcp_runtime/tool_registry.rs`: 在 terminal exposure 下加入 control registry，并扩展测试 fake。
- Modify `main/src/public_mcp_runtime/resource_pool.rs`: 验证 registry capability 能进入 Agent 资源池。
- Modify `crates/agent_runtime/src/tasks/agent_prompt.rs`: 指导模型区分 exec、control 和 Agent cancel。
- Modify `crates/agent_runtime/tests/integration.rs`: 验证 `terminal_control` prompt 规则。
- Modify `docs/agent-tools-current-state.md`: 记录工具 contract 和生命周期边界。
- Modify `AGENTS.md`: 沉淀显式终端控制不能复用 Agent cancel 的经验。

### Task 1: Reproduce And Fix The Confirmation Card Width

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `crates/ai_chat_view/src/agent_cards.rs`

- [ ] **Step 1: Strengthen the GPUI regression test**

给 `tool_card_json_block` 的内部 frame 和 `Input` 分别预留 selector 名称 `agent-tool-json-frame`、`agent-tool-json-input`。先在 `sidebar_mode_tool_confirm_json_block_uses_available_column` 中读取这两个 bounds，并断言：

```rust
let block = cx.debug_bounds("agent-tool-json-block").expect("json block");
let frame = cx.debug_bounds("agent-tool-json-frame").expect("json frame");
let input = cx.debug_bounds("agent-tool-json-input").expect("json input");

assert!(frame.size.width > column.size.width * 0.75);
assert!(input.size.width > column.size.width * 0.75);
assert!(frame.right() <= block.right());
assert!(input.right() <= frame.right());
```

为使测试能在生产 selector 尚未添加时编译并形成正确 RED，先只把 selector 临时加到当前 frame/Input，不改变其布局属性；运行测试后要求宽度断言失败，而不是 selector 缺失。

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```text
rtk cargo test -p ai_chat_view sidebar_mode_tool_confirm_json_block_uses_available_column -- --nocapture
```

Expected: FAIL，错误信息显示 frame 或 Input 宽度明显小于消息列宽度。

- [ ] **Step 3: Apply the minimal flex fix**

把 JSON frame 改为明确的横向 flex 边界，并让 Input 使用剩余宽度：

```rust
h_flex()
    .debug_selector(|| "agent-tool-json-frame".to_string())
    .w_full()
    .min_w_0()
    .h(height)
    .overflow_hidden()
    .child(
        Input::new(&input)
            .debug_selector(|| "agent-tool-json-input".to_string())
            .flex_1()
            .min_w_0()
            .h_full()
            // existing appearance/disabled/text styles
    )
```

不要增加固定像素最小宽度，不修改通用 Input 组件。

- [ ] **Step 4: Verify GREEN and surrounding UI tests**

Run:

```text
rtk cargo test -p ai_chat_view sidebar_mode_tool_confirm_json_block_uses_available_column -- --nocapture
rtk cargo test -p ai_chat_view sidebar_mode_tool_confirm -- --nocapture
```

Expected: PASS。

- [ ] **Step 5: Commit the isolated UI fix**

```text
rtk git add crates/ai_chat_view/src/agent_cards.rs crates/ai_chat_view/src/agent_view.rs
rtk git commit -m "fix(ai-chat): keep tool confirmation json full width"
```

### Task 2: Add Terminal Supervisor Control Contract

**Files:**
- Modify: `crates/terminal/src/types.rs`
- Modify: `crates/terminal/src/lib.rs`
- Modify: `crates/terminal/src/exec_supervisor/model.rs`
- Modify: `crates/terminal/src/exec_supervisor/mod.rs`
- Modify: `crates/terminal/src/exec_supervisor/tests.rs`

- [ ] **Step 1: Write failing supervisor tests**

新增测试，构造对应 OSC/readiness 状态后调用 `interrupt_foreground()`：

```rust
assert_eq!(
    Ok(TerminalControlReadiness::CommandRunning),
    supervisor.interrupt_foreground()
);
```

并覆盖：

```rust
SubmissionPending -> Ok(SubmissionPending)
Ready/AwaitingPrompt -> Err(NotRunning)
Initializing/PromptRendering/ClearingInput -> Err(Busy)
Unknown -> Err(ReadinessUnknown)
Disconnected -> Err(Disconnected)
```

在 active exec observer 场景调用 interrupt 后，再发送 `CommandFinished`，断言原 observer 仍产生真实 `ExecEffect::Complete`。

- [ ] **Step 2: Run supervisor tests and verify RED**

```text
rtk cargo test -p terminal terminal_control -- --nocapture
```

Expected: compile/test failure because control contract does not exist。

- [ ] **Step 3: Define minimal public terminal control types**

在 `types.rs` 增加：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalControlAction { Interrupt }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalControlReadiness { SubmissionPending, CommandRunning }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalControlRequest { pub action: TerminalControlAction }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalControlOutput {
    pub action: TerminalControlAction,
    pub sent: bool,
    pub readiness_before: TerminalControlReadiness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalControlError {
    NotRunning,
    Busy,
    ReadinessUnknown,
    Disconnected,
    Cancelled,
}
```

实现稳定 `Display` 文本，并增加异步 `TerminalControlHandle`，签名与 exec handle 一致地接收 `CancellationToken`。

- [ ] **Step 4: Implement supervisor state mapping**

在 `ExecSupervisor` 中实现纯状态判定：

```rust
pub(crate) fn interrupt_foreground(
    &self,
) -> Result<TerminalControlReadiness, TerminalControlError>
```

该方法不移除 `active`、不修改 observer、不产生 exec completion，只返回 actor 是否可以提交 ETX。

- [ ] **Step 5: Verify GREEN**

```text
rtk cargo test -p terminal terminal_control -- --nocapture
rtk cargo test -p terminal exec_supervisor -- --nocapture
```

Expected: PASS。

### Task 3: Route Interrupt Through The SSH Actor

**Files:**
- Modify: `crates/terminal/src/ssh_backend.rs`
- Modify: `crates/terminal/src/terminal.rs`
- Modify: `crates/terminal/src/types.rs`

- [ ] **Step 1: Write failing actor/handle tests**

新增 `terminal_control_handle_*` 测试：

- handle 发送 `InterruptForeground` actor 消息；
- actor result 映射回 `TerminalControlOutput`；
- 预取消 token 返回 `Cancelled`；
- `TerminalBackend::control_handle` 可由 SSH backend 暴露。

- [ ] **Step 2: Verify RED**

```text
rtk cargo test -p terminal terminal_control_handle -- --nocapture
```

Expected: FAIL because SSH control message/handle is absent。

- [ ] **Step 3: Implement actor message and handle**

增加 actor 消息：

```rust
SshCommand::InterruptForeground {
    cancellation: CancellationToken,
    result: oneshot::Sender<Result<TerminalControlOutput, TerminalControlError>>,
}
```

actor 分支先检查 cancellation，再调用 `exec_supervisor.interrupt_foreground()`；成功时直接向同一 SSH channel 写入 `[0x03]`，写成功后返回 `sent=true`。发送失败返回 `Disconnected`。

`TerminalBackend` 增加默认返回 `None` 的 `control_handle()`；SSH backend 返回 handle，`Terminal::external_control_handle()` 向 terminal_view 暴露。

- [ ] **Step 4: Verify GREEN and SSH exec regressions**

```text
rtk cargo test -p terminal terminal_control_handle -- --nocapture
rtk cargo test -p terminal ssh_backend -- --nocapture
rtk cargo test -p terminal terminal_exec -- --nocapture
```

Expected: PASS，既有 exec tests 不回归。

- [ ] **Step 5: Commit terminal core**

```text
rtk git add crates/terminal/src
rtk git commit -m "feat(terminal): add supervised foreground interrupt"
```

### Task 4: Expose Terminal Control Through Public MCP

**Files:**
- Create: `crates/public_mcp/src/terminal_control.rs`
- Create: `crates/public_mcp/src/tools/terminal_control.rs`
- Create: `crates/public_mcp/tests/terminal_control.rs`
- Modify: `crates/public_mcp/src/lib.rs`
- Modify: `crates/public_mcp/src/tools.rs`
- Modify: `crates/public_mcp/src/tools/registry.rs`
- Modify: `crates/public_mcp/src/registry.rs`
- Modify: `crates/public_mcp/tests/registry.rs`
- Modify: `crates/tool_runtime/src/resource.rs`
- Modify: `crates/tool_runtime/tests/resource_pool.rs`

- [ ] **Step 1: Write failing schema and registry tests**

测试要求：

```rust
assert_eq!("terminal.control", descriptor.id);
assert_eq!(json!(["target", "action"]), descriptor.input_schema["required"]);
assert_eq!(json!(["interrupt"]), descriptor.input_schema["properties"]["action"]["enum"]);
assert_eq!(RiskLevel::High, descriptor.annotations.risk);
```

fake control session 记录 request 并返回 `sent=true`；调用 runtime registry 后断言结构化结果和目标。registry session info 必须包含 `TerminalControl` capability。

- [ ] **Step 2: Verify RED**

```text
rtk cargo test -p public_mcp terminal_control -- --nocapture
rtk cargo test -p tool_runtime terminal_control -- --nocapture
```

Expected: compile/test failure because modules/capability are absent。

- [ ] **Step 3: Implement serializable public contract**

创建：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlAction { Interrupt }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlReadiness { SubmissionPending, CommandRunning }

pub struct TerminalControlRequest { pub target: String, pub action: TerminalControlAction }
pub struct TerminalControlResult {
    pub target: String,
    pub action: TerminalControlAction,
    pub sent: bool,
    pub readiness_before: TerminalControlReadiness,
}
```

- [ ] **Step 4: Add registry handle and capability**

在 Public MCP registry 增加独立 `TerminalControlSessionHandle`、map、register/unregister/call/lookup/id-set。`session_capabilities` 在 handle 存在时增加：

```rust
ResourceCapability::TerminalControl
```

`terminal.control` 的 target spec 要求 `ResourceKind::Terminal` + `TerminalControl`。

- [ ] **Step 5: Implement runtime tool**

descriptor 使用：

```rust
id: "terminal.control"
open_world: true
supports_parallel: false
risk: RiskLevel::High
required: ["target", "action"]
action enum: ["interrupt"]
```

parser 拒绝未知 action，调用 registry 后返回 `ToolResult::structured`。

- [ ] **Step 6: Merge control into terminal tool registry**

`PublicMcpToolRegistry::terminal` 合并 remote ops、terminal exec、terminal control 三个 runtime registry，并导出 `terminal_control_tool_registry`。

- [ ] **Step 7: Verify GREEN**

```text
rtk cargo test -p public_mcp --test terminal_control -- --nocapture
rtk cargo test -p public_mcp --test registry -- --nocapture
rtk cargo test -p tool_runtime terminal_control -- --nocapture
```

Expected: PASS。

### Task 5: Register The Live Terminal Control Bridge

**Files:**
- Modify: `crates/terminal_view/src/public_mcp.rs`

- [ ] **Step 1: Write failing terminal_view bridge tests**

fake `TerminalControlHandle` 记录 core request，`ThreadSafeTerminalControlHandle` 应映射 Public MCP `interrupt` 到 core `Interrupt`，并映射 `readiness_before`。

- [ ] **Step 2: Verify RED**

```text
rtk cargo test -p terminal_view terminal_control -- --nocapture
```

- [ ] **Step 3: Implement registration lifecycle**

`TerminalPublicMcpRegistration` 增加 control slot 和 registered flag；`refresh_parts` 同步 `terminal.external_control_handle()`；register/unregister 与 exec 独立。桥接层只做类型映射和 cancellation 转发。

- [ ] **Step 4: Verify GREEN**

```text
rtk cargo test -p terminal_view terminal_control -- --nocapture
rtk cargo test -p terminal_view public_mcp -- --nocapture
```

- [ ] **Step 5: Commit bridge and Public MCP tool**

```text
rtk git add crates/public_mcp crates/tool_runtime crates/terminal_view
rtk git commit -m "feat(agent): expose terminal interrupt control"
```

### Task 6: Wire Main Registry, Resource Pool, And Agent Guidance

**Files:**
- Modify: `main/src/public_mcp_runtime/tool_registry.rs`
- Modify: `main/src/public_mcp_runtime/resource_pool.rs`
- Modify: `crates/agent_runtime/src/tasks/agent_prompt.rs`
- Modify: `crates/agent_runtime/tests/integration.rs`

- [ ] **Step 1: Write failing main/Agent tests**

扩展 main registry test：当 `toolsets.terminal_exec=true` 时同时包含 `terminal.exec` 和 `terminal.control`；关闭时两者都不包含。资源池保留 `TerminalControl` capability。

扩展 Agent prompt test，工具列表含 `terminal.control` 时断言 system prompt 包含：

```text
terminal_control
Ctrl+C
取消对话不会中断终端任务
```

- [ ] **Step 2: Verify RED**

```text
rtk cargo test -p main terminal_control -- --nocapture
rtk cargo test -p agent_runtime terminal_control -- --nocapture
```

- [ ] **Step 3: Wire registries and resources**

main 的 terminal exec exposure 开关同时 push exec 与 control registry；不新增设置项，避免把同一可见终端自动化能力拆成两个用户难以理解的开关。fake terminal 同时实现 exec/control session handle。

- [ ] **Step 4: Add explicit model guidance**

`append_terminal_tool_selection_rules` 同时查找：

```rust
terminal_exec: ["terminal_exec", "terminal.exec"]
terminal_control: ["terminal_control", "terminal.control"]
```

提示模型只有明确 control observation `sent=true` 后才能声称已经中断，不得把 literal `\u0003` 作为 command，也不得把 Agent cancel 当作 Ctrl+C。

- [ ] **Step 5: Verify GREEN**

```text
rtk cargo test -p main terminal_control -- --nocapture
rtk cargo test -p agent_runtime terminal_control -- --nocapture
rtk cargo test -p agent_runtime system_prompt_guides_visible_terminal_requests -- --nocapture
```

Expected: PASS。

### Task 7: Documentation, Review, And Completion Verification

**Files:**
- Modify: `docs/agent-tools-current-state.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Update operational documentation**

记录 `terminal.control` 的 schema、审批要求、允许状态、错误 contract，并重申：

```text
Agent cancel != terminal Ctrl+C
terminal.exec safe-replace != terminal.control interrupt
```

- [ ] **Step 2: Add project experience**

在 `AGENTS.md` 既有终端生命周期经验后追加：显式中断必须走 supervisor 原子状态检查，不能暴露任意输入字节，也不能复用 Agent cancellation token。

- [ ] **Step 3: Format all changed Rust files**

```text
rtk cargo fmt --all -- --check
```

若 check 失败，只格式化本次改动文件，再重新运行 check。

- [ ] **Step 4: Run focused and full verification**

```text
rtk cargo test -p terminal terminal_control -- --nocapture
rtk cargo test -p terminal_view terminal_control -- --nocapture
rtk cargo test -p public_mcp --test terminal_control -- --nocapture
rtk cargo test -p public_mcp --test registry -- --nocapture
rtk cargo test -p agent_runtime terminal_control -- --nocapture
rtk cargo test -p ai_chat_view sidebar_mode_tool_confirm_json -- --nocapture
rtk cargo test -p ai_chat_view
rtk cargo check -p terminal -p terminal_view -p public_mcp -p agent_runtime -p ai_chat_view -p main
rtk git diff --check
```

- [ ] **Step 5: Review the final diff**

确认：

- Agent cancel 路径没有新增 terminal write；
- control 失败路径零写入；
- supervisor observer 没有被 control 提前完成；
- terminal tool exposure 开关同时控制 exec/control；
- UI 测试测到实际 Input，而不是只测外层 wrapper；
- 没有纳入用户或并发任务的无关改动。

- [ ] **Step 6: Commit final wiring/docs**

```text
rtk git add AGENTS.md docs/agent-tools-current-state.md main/src/public_mcp_runtime crates/agent_runtime
rtk git commit -m "feat(agent): guide explicit terminal interrupts"
```

- [ ] **Step 7: Report evidence and remaining repository-wide lint debt**

最终报告每条验证命令的真实结果；若仓库既有 `clippy -D warnings` 问题仍存在，列出首个无关错误，不把它误报为本次通过。
