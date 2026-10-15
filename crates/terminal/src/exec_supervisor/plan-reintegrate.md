# Terminal Control `reintegrate` 注入 · 实现计划

## 目标

`sudo -i` / `su` 等嵌套 shell 丢失 shell integration 后，Agent 通过 `terminal.control action=reintegrate` 向 PTY 注入集成脚本，恢复 OSC 133 事件流；`ExecSupervisor::on_input_start` 无条件置 `Ready`，readiness 随之自愈，后续 `terminal.exec` 恢复可用。

## 边界

- 只做显式 `reintegrate` 动作，不做自动注入、不动 readiness 状态机转移
- 不复用 `RuntimeShellIntegration` phase 机（它只从登录态起跳）；actor 直接写注入命令
- local 终端本次不动，只做 SSH 链路（后续照搬）
- 风险等级保持 High（显式审批）

## 风险

- 注入要求提示符空闲；前台 TUI 运行时 eval 会喂给它 → 保持显式动作 + 描述警告
- 注入命令会在终端回显一行 → 接受，不为它扩状态机
- `on_input_start` 无条件翻转是既有行为，新依赖它做自愈，需测试钉住

## 验证方式

- 协议层：`parse_control_request` 接受 `reintegrate`；schema enum 含 `reintegrate`
- actor：reintegrate → 写 `injection_command()` 且不改 readiness；Disconnected 拒绝
- 状态机：`SubmissionPending` 下收到 `InputStart` → `Ready`（钉住自愈前提）
- `cargo test -p terminal -p terminal_view -p public_mcp`、`clippy --all-targets`、`cargo check -p main`

## 步骤

- [x] 1. 摸清 `TerminalControlAction` 与 SSH/local control 链路处理点
  - 枚举：`terminal/src/types.rs`（core）+ `public_mcp/src/terminal_control.rs`（协议）
  - actor：`ssh_backend.rs` ~L1028（`shell_integration` 在作用域，`injection_command()` 可直接用）、`pty_backend.rs` ~L623（local 无集成实例，复用自由函数）
  - `readiness_before` 对 Reintegrate 无意义 → 改 `Option`（~8 处构造点）
  - Windows local（PowerShell/cmd）不支持 POSIX eval 注入 → 新增 `TerminalControlError::Unsupported`，`#[cfg]` 分支
  - 自愈前提：`on_input_start` 无条件置 Ready，注入后 OSC 恢复即自愈；`ready_timeout_ms>0` 的 exec 可在 reintegrate 前先挂起，恢复后自动提交
- [x] 2. Red：补 Reintegrate 失败测试（10 个，涵盖门控/自愈链/actor contract）
- [x] 3. Green：`TerminalControlAction::Reintegrate`（core + 协议层）+ SSH actor 写 `runtime_shell_integration_command()` + local actor Unsupported
- [x] 4. Busy 错误富化：`TerminalExecError::BusyContext{readiness}` 带恢复指引，替换两个 Busy 出口
- [x] 5. Verify：terminal 525 / public_mcp+terminal_view 782 全绿，clippy 新增代码零告警，`cargo check -p main` 通过
