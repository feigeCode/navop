# Terminal Optimization Branch Review

> 日期：2026-07-25  
> 目标分支：`dev`  
> 审查分支：
>
> - `feat/terminal-optimization-focused`
> - `feat/terminal-remote-modernization`

## 0. 实施跟踪

为避免污染日常 `dev` 工作区，本文档及后续实现已经迁移到独立 worktree：

```text
worktree: /Users/hufei/RustroverProjects/navop-workspace/navop-terminal-optimization
branch:   feat/terminal-optimization-rework
baseline: cf16096f
```

创建时，本地 `dev` 为 `cf16096f`，完整包含当时的 `origin/dev`
`774a383c`，并额外包含 5 个本地已提交变更。因此本轮将本地 `dev` HEAD
作为“最新 dev”基线。

### 2026-07-25：切片 1，`terminal.exec` capture 内存有界化

状态：**实现完成并通过定向验证，作为独立分支的首个优化提交。**

本切片只处理内部 capture 内存上限，不在同一步扩展公共
`TerminalExecOutput` / `TerminalExecProgress` contract：

- 新增 `BoundedCaptureBuffer`，上限为 1 MiB；
- 超限时保留最新输出 tail；
- 单个 chunk 超过上限时只保留该 chunk 的最后 1 MiB；
- `ActiveExec` 已从无界 `Vec<u8>` 切换到有界 buffer；
- 保持 cancel/detach、OSC 133、prompt epoch、timeout、no-wait 和
  background/nohup 现有语义；
- `truncated`、`captured_bytes`、`discarded_bytes` 等调用方可见 contract
  留作下一个独立切片，避免首步扩大 Public MCP 和 UI 改动范围。

验证记录见本文末尾的“实施验证记录”。

### 2026-07-25：切片 2，暴露 `terminal.exec` 输出截断元数据

状态：**实现完成并通过跨 crate 验证，作为第二个独立优化提交。**

本切片在切片 1 的 1 MiB capture 上限基础上，将截断状态从 terminal core
一路暴露到 Public MCP structured result：

```rust
truncated: bool
captured_bytes: usize
discarded_bytes: u64
```

字段语义：

- `captured_bytes` 是当前内部 capture buffer 保留的原始 terminal 字节数；
- `discarded_bytes` 是由于 1 MiB 上限累计淘汰的原始字节数；
- `truncated` 等价于 `discarded_bytes > 0`；
- submitted-only 结果没有 output capture，三个字段分别为 `false`、`0`、`0`；
- timeout、observer progress 和最终完成结果使用同一组 capture 元数据；
- Public MCP 新字段使用 `#[serde(default)]`，旧版缺少这些字段的 JSON 仍可反序列化。

本切片明确不扩展 command store 的历史记录 schema；历史命令是否持久化截断
元数据留到 command store contract 单独评估，避免把直接执行结果的兼容扩展与
持久化迁移混在同一提交。

独立审查确认：普通 overflow 和单 chunk 超限都会精确累计丢弃字节并使用
saturating 计数；observer、timeout、terminal view bridge 和 Public MCP JSON 映射一致；
没有修改 cancel/detach、OSC 133、no-wait、terminal control、SSH lifecycle、encoding、
recording 或 ingress queue 行为。

验证记录见本文末尾的“实施验证记录”。

## 1. 背景与目标

本文审查两个历史终端优化分支，目标是识别其中值得在当前 `dev` 分支重新实现的优化点，并明确：

- 哪些能力值得保留；
- 哪些实现可作为代码或测试参考；
- 哪些提交不应直接合并或 cherry-pick；
- 当前实现存在哪些依赖、架构和回归风险；
- 如何按低风险、可验证的顺序拆分后续实现。

初始分支审查为只读分析；后续实现已按上面的实施跟踪迁移到独立 worktree，
不会覆盖原 `dev` 工作区中的未提交内容。

## 2. 执行摘要

两个分支都不建议直接 merge，也不建议整体 cherry-pick 到当前 `dev`。

### 2.1 `feat/terminal-remote-modernization`

该分支适合作为设计资料和优化点来源，但不适合作为代码迁移源：

- 基线明显早于当前 `dev`；
- 改动范围过大；
- 混入 updater 签名、durable atomic file、release workflow 等大量非终端工作；
- 与当前 `dev` 直接比较会产生大量无关差异；
- 多个后续能力被组织为大切片，难以独立验证。

### 2.2 `feat/terminal-optimization-focused`

该分支比 modernization 更接近可参考的代码源，但目前仍不是可直接合入的成品：

- 三个提交仍然同时包含性能、队列、编码、录制和 SSH 生命周期改造；
- 与当前 `dev` 的终端、SSH 近期修复存在大面积重叠；
- Cargo workspace 依赖声明不闭合；
- terminal recording 又依赖未包含在该分支中的 atomic-file 实现；
- 当前无法通过最基本的 Cargo manifest 加载。

### 2.3 总体策略

推荐从两个分支中提取设计、contract 和测试思路，在当前 `dev` 上按以下顺序重新实现：

1. 性能指标与 GPUI wakeup 合并；
2. `terminal.exec` 输出 capture 有界化；
3. 独立的 bounded ingress queue contract；
4. SSH ingress 接入；
5. Serial ingress 接入；
6. 根据实际指标决定是否调整 local PTY 数据面；
7. SSH connection registry 单独立项；
8. 连接级编码单独立项；
9. 终端录制单独立项；
10. 基于实测结果再决定 render policy。

## 3. 分支结构

### 3.1 `feat/terminal-optimization-focused`

相对当前 `dev`，该分支有三个独有提交：

```text
38cb20f5 migrate terminal optimization core slice
3f1c9863 add terminal shared transport and encoding storage contracts
8625da02 integrate terminal shared ssh transport lifecycle
```

相对共同基线，改动规模约为：

```text
47 files changed
7293 insertions
847 deletions
```

主要影响：

- `crates/terminal`
- `crates/ssh`
- `crates/core/src/storage`

该分支可以理解为：将 modernization 分支后面的几个大切片迁移到一个更新的仓库基线上。但它没有被进一步拆成独立、依赖闭合、可分别验证的功能提交。

### 3.2 `feat/terminal-remote-modernization`

相对当前 `dev`，该分支有九个独有提交：

```text
b5e5f4e7 feat(terminal): establish observability baseline
3243c42c feat(storage): add durable atomic file writes
c26111a6 feat(update): verify signed release manifests
7ffd2861 docs(terminal): design bounded ingress contract
1c16382d docs(terminal): lock phase 3 queue semantics
df42a7a6 feat(terminal): add bounded ingress queue
44ad5da3 migrate terminal optimization core slice
78767744 add terminal shared transport and encoding storage contracts
a3b17447 integrate terminal shared ssh transport lifecycle
```

总体改动规模约为：

```text
120 files changed
15144 insertions
1904 deletions
```

除终端和 SSH 外，该分支还包含：

- durable atomic file；
- updater signed manifest；
- release 和 R2 workflow；
- core settings、crypto、key storage；
- terminal performance overlay；
- 大量 plans、specs 和 invariants 文档。

因此，不应把该分支视为一个单一的终端优化功能分支。

## 4. 与当前 `dev` 的重叠和回归风险

focused 分支建立后，当前 `dev` 的终端和 SSH 已继续演进，包括但不限于：

- Windows shell readiness 修复；
- OSC 133 exec supervisor 调整；
- `terminal.control` 前台中断语义；
- Agent cancel 与可见终端进程所有权分离；
- terminal search match 高亮；
- terminal light theme 文字渲染；
- X11 forwarding；
- command bar、resize、layout 和 theme 调整。

与 focused 分支直接重叠的关键文件包括：

```text
crates/core/src/storage/models.rs
crates/core/src/storage/repository.rs
crates/ssh/src/lib.rs
crates/ssh/src/session_manager.rs
crates/ssh/src/ssh.rs
crates/terminal/Cargo.toml
crates/terminal/src/exec_supervisor/operation.rs
crates/terminal/src/exec_supervisor/tests.rs
crates/terminal/src/lib.rs
crates/terminal/src/pty_backend.rs
crates/terminal/src/ssh_backend.rs
crates/terminal/src/terminal.rs
```

即使 cherry-pick 时没有文本冲突，也可能发生语义回退，尤其是：

- 可见终端命令完成边界；
- Agent cancel 和 terminal interrupt 的分离；
- Windows shell prompt readiness；
- `wait_for_output=false` 的 busy 状态；
- detached waiter 的生命周期；
- X11 forwarding；
- 当前 terminal theme 和 render 行为。

因此，应以当前 `dev` 的行为为基准，将分支代码作为参考重新实现，而不是反向覆盖当前实现。

## 5. 验证结果与依赖问题

为避免影响当前工作区，focused 分支被导出到临时目录：

```text
/tmp/navop-terminal-focused-review
```

实际尝试运行：

```bash
rtk cargo test -p terminal --lib
rtk cargo test -p ssh --lib
```

两个命令均在 Cargo workspace manifest 加载阶段失败，退出码为 `101`。

错误为：

```text
error inheriting `encoding_rs` from workspace root manifest's
`workspace.dependencies.encoding_rs`

Caused by:
`dependency.encoding_rs` was not found in `workspace.dependencies`
```

focused 分支的 `crates/terminal/Cargo.toml` 增加了：

```toml
encoding_rs = { workspace = true }
```

但根 `Cargo.toml` 没有增加对应的 workspace dependency。虽然 `Cargo.lock` 已存在 `encoding_rs`，但 lockfile 不能代替 workspace dependency 声明。

### 5.1 Recording 的 atomic-file 依赖也未闭合

focused 分支的：

```text
crates/terminal/src/recording/asciicast.rs
```

调用：

```rust
one_core::atomic_file::durable_write(...)
```

但是 focused 分支没有包含：

```text
crates/core/src/atomic_file.rs
crates/core/src/atomic_file_error.rs
crates/core/src/atomic_file_windows.rs
```

也没有完整包含 `crates/core/src/lib.rs` 对这些模块的导出。

这些实现存在于 modernization 的独立提交：

```text
3243c42c feat(storage): add durable atomic file writes
```

因此，即使补上 `encoding_rs` workspace dependency，focused 分支仍很可能因缺少 `one_core::atomic_file` 而继续编译失败。

### 5.2 验证结论

目前不能声称：

- 任一分支测试通过；
- focused 分支可以直接合入；
- focused 的三个提交依赖闭合；
- recording 功能可以独立编译；
- modernization 的后续大切片完成了最终 completion verification。

## 6. 优化点评估

## 6.1 P0：终端性能可观测性

参考文件：

```text
crates/terminal/src/performance_metrics.rs
crates/terminal/tests/throughput_baseline.rs
```

主要指标包括：

- parser chunk 大小；
- ingress pending bytes；
- ingress peak bytes；
- `Term` 锁等待时间；
- `Term` 锁持有时间；
- wakeup request 次数；
- queued wakeup 次数；
- coalesced wakeup 次数；
- render duration；
- SSH connect、reconnect 和 invalidation 次数。

### 价值

在重构 terminal data plane 前，首先需要知道瓶颈在哪里：

- SSH 网络读取是否快于 parser；
- `Term` 锁竞争是否严重；
- GPUI wakeup 是否过于频繁；
- renderer 是否是主要瓶颈；
- exec capture 是否无界增长；
- backend channel backlog 是否过大。

该分支中的 metrics 主要使用原子计数，并未将终端输出内容写入指标，安全方向合理。

### 推荐实现范围

第一版只做：

1. `TerminalPerformanceMetrics` 纯 contract；
2. parser/backend 埋点；
3. `GpuiEventProxy::queue_wakeup()` wakeup 合并；
4. ignored throughput baseline 测试或独立 benchmark 脚本。

暂不做：

- performance UI overlay；
- render policy；
- backend 架构重写；
- encoding；
- recording。

### 验收要求

- 指标不得包含终端 payload、命令内容或认证信息；
- 默认路径不能引入高成本日志；
- wakeup 合并不能丢失最终 repaint；
- baseline 能在本机重复执行；
- 可对比优化前后的 pending bytes、wakeup 数和锁时间。

## 6.2 P0：GPUI wakeup 合并

大量终端 output chunk 可能触发密集 GPUI wakeup。分支中的 `queue_wakeup()` 方向是将多个尚未处理的 wakeup 请求合并成一次已排队通知。

### 预期收益

- 减少 UI foreground task 数量；
- 避免每个小 chunk 都触发一次 repaint；
- 降低高吞吐场景下的调度和锁竞争；
- 为后续 throughput mode 提供数据基础。

### 风险

- 合并标志的 reset 时机错误可能丢失最后一次 repaint；
- event proxy 生命周期结束时不能留下永久 pending 状态；
- output、resize、cursor blink、selection 等不同 repaint 来源需要统一考虑。

因此应先补状态 contract，再接真实 event proxy。

## 6.3 P0：`terminal.exec` capture buffer 有界化

当前 `dev` 的 exec supervisor 仍使用：

```rust
raw: Vec<u8>
```

等待输出的 `terminal.exec` 在大量输出下可能持续扩张内存。

focused 分支增加：

```text
crates/terminal/src/exec_supervisor/capture_buffer.rs
crates/terminal/src/exec_supervisor/capture_buffer_tests.rs
```

默认限制：

```rust
pub(super) const TERMINAL_EXEC_CAPTURE_LIMIT_BYTES: usize = 1024 * 1024;
```

实现保留最后一段输出：

- 新数据未超过上限时，淘汰头部溢出的字节；
- 单个 chunk 大于上限时，只保留 chunk 最后的 `limit` 字节；
- 内存最多保留 1 MiB。

### 推荐增强

除了限制内存，还应让调用方知道输出被截断。建议结果中至少包含：

```rust
truncated: bool
```

也可以进一步包含：

```rust
captured_bytes: usize
discarded_bytes: u64
```

否则 Agent 可能把最后 1 MiB 误认为完整输出。

### 必须保留的当前语义

实现时不得覆盖 focused 分支中的旧 `operation.rs`，必须保留当前 `dev` 的：

- OSC `CommandFinished` 和新 prompt epoch 完成边界；
- 不依赖 EOF；
- Agent cancel 只 detach waiter，不自动发送 Ctrl+C；
- `terminal.control(action=interrupt)` 独立处理显式中断；
- `wait_for_output=false` 提交后立即进入 busy/submission-pending；
- detached waiter 不再缓存无人消费的输出；
- Windows shell readiness；
- `terminal.read` live PTY/scrollback tail。

### 验收要求

- 大输出时 capture 内存有明确上限；
- 输出截断状态对调用方可见；
- cancel/detach 后不继续积累 output；
- UTF-8 边界截断由现有 sanitize 层安全处理；
- timeout、cancel、no-wait、background/nohup 场景不回退。

## 6.4 P1：有界 terminal ingress queue

参考文件：

```text
crates/terminal/src/ingress_queue.rs
crates/terminal/src/ingress_queue_tests.rs
crates/terminal/src/ssh_ingress.rs
crates/terminal/src/serial_ingress.rs
crates/terminal/src/local_supervisor_queue.rs
```

这是两个分支中最重要的数据面优化。

### 核心设计

队列同时限制：

- pending data bytes；
- pending data chunks；
- pending control messages。

control 和 data 使用独立预算，从而保证：

- 大量输出不能无限堆积；
- control 消息不会被 data backlog 挤掉；
- abort 能唤醒正在等待 permit 的 producer；
- receiver drop 能唤醒 producer；
- shutdown 时可以主动丢弃 backlog；
- pending 和 peak bytes 可观测；
- payload 的 `Debug` 不泄露终端内容。

focused 中 SSH 默认预算：

```rust
SSH_PENDING_BYTES = 512 * 1024;
SSH_PENDING_CHUNKS = 16;
SSH_PENDING_CONTROLS = 8;
```

主要使用：

```rust
Semaphore
mpsc::channel
CancellationToken
```

### SSH actor 调度方向

`next_ssh_actor_input()` 使用 biased select，优先级大致为：

1. command；
2. terminal response；
3. pending ingress 完成；
4. 只有不存在 pending ingress 时，才继续读取 SSH channel。

这样可以在 parser queue 饱和时暂停继续读取大块远端输出，同时继续处理用户命令和 terminal response，避免数据背压完全冻结控制面。

### 推荐拆分

#### 阶段一：纯 queue contract

只增加：

```text
ingress_queue.rs
ingress_queue_tests.rs
```

测试至少覆盖：

- 按真实字节数计费；
- byte limit 背压；
- chunk limit 背压；
- control limit 背压；
- control 和 data 独立预算；
- control 绕过 data backlog；
- abort 唤醒等待中的 producer；
- receiver drop 唤醒 producer；
- permit 不重复释放；
- pending/peak 统计；
- payload `Debug` 脱敏。

该阶段不接入任何 backend。

#### 阶段二：SSH ingress

SSH 最容易出现：

- 连续高吞吐输出；
- 网络读取速度高于 parser 消费速度；
- unbounded channel 内存增长；
- 远程日志、构建和大文件输出造成 backlog。

必须回归：

- OSC 133 shell readiness；
- init commands；
- `terminal.exec`；
- `terminal.control`；
- resize；
- disconnect；
- command/terminal-response 优先级；
- X11 forwarding；
- shutdown 时 parser queue 的 abort/drain。

#### 阶段三：Serial ingress

Serial 存在 Text、Hex、Mixed 等模式，不能把所有数据都走文本 decoder。

必须保证：

- Text 模式可走 encoding decoder；
- Hex/Mixed 模式绕过文本 decoder；
- 原始字节不会被错误转码；
- serial close 唤醒所有 waiter；
- burst data 不无限占用内存。

#### 阶段四：评估 local PTY

local terminal 已受 Alacritty event loop、PTY reader 和 GPUI wakeup 调度约束。如果机械增加一层 queue，可能形成双重背压。

因此 local PTY 是否接入应由 metrics 和 baseline 决定，而不是因为 SSH 和 Serial 已接入就自动照搬。

## 6.5 P2：SSH connection registry

参考文件：

```text
crates/ssh/src/connection_identity.rs
crates/ssh/src/connection_registry.rs
crates/ssh/src/connection_registry_tests.rs
```

### 目标

- 相同 SSH identity 复用同一 `SshSessionManager`；
- terminal、SFTP、forwarding 等 consumer 通过 lease 引用连接；
- 一个 consumer 关闭时不误断其他 consumer；
- 所有 consumer 释放后进入 idle timeout；
- generation 防止 stale result 污染新 entry；
- 提供 snapshot 和 lifecycle observer。

### Identity 当前覆盖内容

- normalized host；
- port；
- username；
- auth type；
- credential revision；
- timeout；
- keepalive interval；
- keepalive max；
- jump server 的 host、port、username、auth；
- proxy 的 type、host、port、username、credential revision。

其 `Debug` 不直接打印密码和私钥内容，使用 label 和 opaque revision，方向合理。

### 当前迁移不完整

实际调用主要出现在：

- terminal；
- dynamic SOCKS；
- SSH forwarding。

没有确认以下 consumer 已完整迁移：

- SFTP；
- remote file editor；
- 所有 local/remote/dynamic forwarding 入口；
- 其他直接创建 `SshSessionManager` 的入口。

因此当前不能称为全局 SSH transport 已统一。

### 风险一：每次 idle 创建 OS 线程

当前实现使用：

```rust
std::thread::spawn(move || {
    std::thread::sleep(idle_timeout);
    ...
})
```

每次 consumer 数量归零都会创建一个 sleeping OS thread。频繁 acquire/release 时会产生：

- 大量等待线程；
- 已失效 epoch 的线程仍存活到 timeout；
- runtime handle 和 shutdown 生命周期复杂；
- 测试时间难以确定性驱动。

建议改成：

- registry-owned Tokio reaper；
- `DelayQueue`；
- 或每个 entry 一个可取消 Tokio timer。

新 acquire 应取消旧 timer，不应每次 idle 创建独立 OS 线程。

### 风险二：Idle acquire 被标为 Disconnected

当前 `mark_acquired()` 在 state 为 `Idle` 时执行：

```rust
lifecycle.state = SshConnectionState::Disconnected;
```

但 idle entry 内部的 manager 可能仍持有活连接，只是暂时没有 consumer。将其标为 disconnected 可能导致：

- snapshot 短暂表达错误；
- UI/observer 误判连接状态；
- 不必要的 connect；
- registry 状态与 manager 实际 transport 状态不一致。

应明确区分：

- transport lifecycle；
- consumer/lease lifecycle。

### 推荐实施方式

将 registry 作为独立架构项目：

1. 实现 `ConnectionIdentity`；
2. 实现 fake manager contract；
3. 覆盖 lease、generation、idle cancel 和 stale result；
4. 使用统一可取消 timer；
5. 迁移 terminal；
6. 迁移 SFTP；
7. 迁移 forwarding；
8. 清理直接创建 manager 的旧入口。

核心验收：

> 关闭 terminal 不影响共享连接的 SFTP；关闭 SFTP 不影响 forwarding；只有最后一个 lease 释放且 idle timeout 到期后，才断开 transport。

迁移时必须保留当前 `dev` 的 X11 forwarding。

## 6.6 P2：连接级终端编码

参考文件：

```text
crates/terminal/src/encoding.rs
crates/terminal/src/encoding_tests.rs
crates/core/src/storage/models.rs
```

支持：

- UTF-8；
- GBK；
- GB18030；
- Big5；
- Shift_JIS；
- EUC-JP；
- EUC-KR；
- Windows-1252。

### 合理设计

- 编码为 per connection/session，而不是全局设置；
- output decoder 保留跨 chunk 状态；
- input/paste 反向编码；
- 切换编码时重置 decoder；
- mismatch detector 只提示候选，不自动切换；
- Serial Hex/Mixed 绕过文本 decoder。

### 当前问题

- 缺根 workspace 的 `encoding_rs` dependency；
- storage model 与当前 `dev` 可能冲突；
- SSH、Serial、local PTY 同时修改，范围过大；
- 没有确认完整的 TerminalView 设置 UI；
- 运行中切换编码的 pending bytes 语义不清晰；
- local shell 不应被远程编码功能无必要地复杂化。

### 推荐首版范围

- SSH connection profile 编码字段；
- Serial profile 编码字段；
- SSH output incremental decode；
- SSH input/paste encode；
- Serial Text 模式编码；
- local terminal 保持 UTF-8。

后续再考虑：

- 运行中切换；
- mismatch UI；
- tab 临时 override；
- 重连生效规则；
- 旧配置 migration。

编码功能应作为独立功能开发，不与性能优化合并。

## 6.7 P3：Terminal recording / Asciicast v2

参考文件：

```text
crates/terminal/src/recording/asciicast.rs
crates/terminal/src/recording/disclosure.rs
crates/terminal/src/recording/model.rs
crates/terminal/src/recording/playback.rs
crates/terminal/src/recording/recorder.rs
crates/terminal/src/recording/runtime.rs
```

### 当前设计

- 默认 `capture_input = false`；
- 支持 output、resize 和 marker；
- input 只有显式 opt-in 才记录；
- 支持 pause、resume 和 stop；
- 有时长、事件数量和文件大小限制；
- Agent disclosure 需要显式 grant；
- 支持 Asciicast v2 roundtrip；
- playback 支持 seek、speed 和 search。

backend 中已经存在 input/output 接线：

- local PTY；
- SSH；
- Serial；
- resize。

`record_input()` 在 `capture_input == false` 时直接返回，不保存用户输入。

### 不建议首轮实现的原因

- 不是性能优化的必要前置；
- focused 分支缺少 atomic-file 依赖；
- 涉及敏感数据、安全授权、持久化和播放 UI；
- 没有确认完整成熟的 TerminalView 录制控制 UI；
- recording tap 使用同步 `Mutex`，需要评估高吞吐锁成本；
- output event 对 data 做复制，可能增加分配；
- 需要明确录制 raw bytes 还是 decoded terminal bytes。

建议 durable atomic file 在 `dev` 独立稳定后，再单独实现 recording。

## 6.8 P3：Render policy

参考文件：

```text
crates/terminal/src/render_policy.rs
crates/terminal/src/render_policy_tests.rs
```

定义了：

- Quality；
- LowPower；
- Compatibility；
- Auto；
- Interactive；
- Normal；
- Throughput；
- Idle。

并提供：

- drain bytes；
- event budget；
- repaint interval；
- image budget。

但 `resolve_terminal_render_policy()` 基本只在自身模块和测试中使用，没有确认当前 runtime 或 TerminalView 已真正消费它。

因此它当前属于有测试的设计 contract，不是成熟接入功能。其预算常量也缺少当前 `dev` 的实测依据。

推荐先完成 metrics、baseline、wakeup coalescing 和 SSH ingress，再基于数据决定是否需要：

- activity class；
- throughput mode；
- low-power repaint interval；
- compatibility profile；
- image budget。

## 7. 推荐 PR 拆分

## 7.1 PR 1：Terminal observability baseline

范围：

```text
performance metrics
parser chunk metrics
Term lock metrics
wakeup request/queued/coalesced metrics
SSH lifecycle metrics
ignored throughput baseline
```

不包含：

- UI overlay；
- backend 重构；
- encoding；
- recording；
- render policy。

验收：

- 指标不包含 payload；
- 默认不开启高成本日志；
- wakeup 合并不丢 repaint；
- baseline 可重复执行；
- 当前 terminal/SSH 定向测试通过。

## 7.2 PR 2：Bounded terminal.exec capture

范围：

```text
BoundedCaptureBuffer
exec supervisor integration
truncated result contract
detached waiter capture cleanup
```

建议初始上限为 1 MiB，但调用方必须能识别截断。

验收：

- 大输出内存有界；
- 返回最后 N 字节；
- 截断状态可见；
- detached/cancelled waiter 不再积累；
- OSC 133、Windows readiness、timeout、cancel 和 no-wait 不回退。

## 7.3 PR 3：Bounded ingress primitive

只增加纯 queue 和测试，不接 backend。

验收：

- bytes/chunks/control 三类预算；
- abort/drop 唤醒；
- control 不被 data backlog 阻断；
- payload Debug 脱敏；
- pending/peak 统计准确。

## 7.4 PR 4：SSH ingress integration

建议处理顺序：

```text
raw SSH bytes
→ incremental encoding decode
→ decoded terminal bytes
→ OSC extraction / exec supervisor
→ bounded parser ingress
→ Alacritty Processor
→ coalesced GPUI wakeup
```

如果此时尚未实现 encoding，则保持 UTF-8/现有 decoder contract，不应为了接 ingress 同时扩大编码功能范围。

验收：

- backlog 有界；
- command/control 不被饿死；
- disconnect 和 shutdown 不挂起；
- parser task 正常退出；
- init command 和 shell readiness 正常；
- `terminal.exec` 和 `terminal.control` 正常；
- X11 forwarding 不回退。

## 7.5 PR 5：Serial ingress

范围：

- Serial ingress queue；
- Text/Hex/Mixed 模式 contract；
- close/abort；
- burst backlog；
- 相关 metrics。

local PTY 是否接入应根据 PR 1 的指标决定。

## 7.6 PR 6：SSH connection registry domain

第一阶段只做：

- identity；
- credential revision；
- lease；
- fake manager；
- generation；
- registry-owned idle timer；
- snapshot；
- lifecycle observer。

后续分别迁移：

- terminal；
- SFTP；
- forwarding/SOCKS；
- 其他直接 manager 入口。

## 7.7 后续独立功能

### Encoding

单独处理：

- profile schema；
- migration；
- SSH/Serial backend；
- 设置 UI；
- mismatch 提示。

### Recording

先完成并验证 durable atomic file，再处理：

- recorder；
- Asciicast；
- playback；
- TerminalView UI；
- Agent disclosure。

### Render policy

基于实际指标落地，不直接移植未接入的固定常量。

## 8. 提交迁移建议

### 8.1 可优先阅读和提取 contract 的提交

modernization：

```text
b5e5f4e7 feat(terminal): establish observability baseline
df42a7a6 feat(terminal): add bounded ingress queue
```

适合提取：

- metrics contract；
- baseline 维度；
- ingress queue contract；
- queue tests。

但仍建议按当前 `dev` 重新实现。

### 8.2 focused 中适合参考的文件

```text
crates/terminal/src/exec_supervisor/capture_buffer.rs
crates/terminal/src/exec_supervisor/capture_buffer_tests.rs
crates/terminal/src/ingress_queue.rs
crates/terminal/src/ingress_queue_tests.rs
crates/terminal/src/ssh_ingress.rs
crates/terminal/src/ssh_ingress_tests.rs
crates/terminal/src/performance_metrics.rs
```

### 8.3 不建议直接 cherry-pick 的提交

```text
38cb20f5 migrate terminal optimization core slice
3f1c9863 add terminal shared transport and encoding storage contracts
8625da02 integrate terminal shared ssh transport lifecycle
```

原因：

- 提交过大；
- 同时改变多个独立行为面；
- 与当前 `dev` 的关键终端修复重叠；
- 依赖不闭合；
- 难以判断回归来源；
- registry、encoding 和 recording 应分别立项。

也不建议直接 cherry-pick modernization 的三个对应大切片，因为其基线更旧。

## 9. 建议首先落地的三个优化

如果目标是尽快将高价值优化带回 `dev`，第一批建议：

### 9.1 `terminal.exec` capture 有界化

范围最小，可以直接降低大输出和 detached waiter 的内存风险。

### 9.2 Metrics 与 wakeup coalescing

先建立性能证据，再决定后续数据面和渲染策略。

### 9.3 Bounded ingress queue 纯 contract

先稳定通用队列和测试，再接 SSH，不在第一步同时修改 backend。

这三个切片不需要立即触碰：

- storage migration；
- SSH 全局生命周期；
- encoding UI；
- recording security；
- durable file；
- render policy。

## 10. 优先级汇总

| 优化点 | 价值 | 当前成熟度 | 实施风险 | 建议 |
|---|---:|---:|---:|---|
| 性能 metrics | 高 | 中高 | 低 | 第一批重做 |
| Wakeup coalescing | 高 | 中高 | 中 | 第一批重做 |
| Exec capture 有界化 | 高 | 高 | 低 | 最先实现 |
| Bounded ingress primitive | 很高 | 高 | 中 | 纯 contract 后接 backend |
| SSH ingress | 很高 | 中高 | 中高 | Queue 稳定后实现 |
| Serial ingress | 中高 | 中 | 中 | SSH 后实现 |
| Local PTY queue | 不确定 | 中 | 中高 | 根据 metrics 决定 |
| SSH connection registry | 高 | 中 | 高 | 单独架构项目 |
| 连接级编码 | 中高 | 中 | 中高 | 单独功能 |
| Terminal recording | 中 | 中 | 高 | 延后、独立功能 |
| Render policy | 潜在中高 | 低 | 中 | 有实测后再实现 |
| 整体 merge modernization | 低 | 低 | 极高 | 不实施 |
| 整体 cherry-pick focused | 低 | 低 | 极高 | 不实施 |

## 11. 最终建议

不要将两个分支中的任何一个整体合入 `dev`。

推荐以当前 `dev` 的终端行为为权威基线，按以下路线推进：

```text
Bounded exec capture
→ Performance metrics and wakeup coalescing
→ Bounded ingress queue contract
→ SSH ingress
→ Serial ingress
→ Evaluate local PTY with measurements
→ Separate SSH registry project
→ Separate encoding feature
→ Separate recording feature
→ Render policy based on measurements
```

其中最重要的约束是：

- 不削弱当前 OSC 133 exec supervisor；
- 不把 Agent cancel 映射成 terminal Ctrl+C；
- 不依赖 EOF 判断可见终端命令完成；
- 不让 terminal control 被数据面背压饿死；
- 不将敏感 payload 写入日志、指标或默认录制；
- 不在一个大 PR 中同时重构 SSH lifecycle 和 terminal data plane；
- 不用旧分支代码覆盖当前 Windows readiness、X11 forwarding 和其他近期修复。

## 12. 实施验证记录

### 12.1 切片 1：`terminal.exec` capture 内存有界化

TDD Red 证据：

```text
cargo test -p terminal capture_buffer --lib
2 failed, 1 passed
实际无界长度分别为 5（期望 4）和 9（期望 4）

cargo test -p terminal active_exec_capture_keeps_only_the_newest_bounded_tail --lib
1 failed
实际 ActiveExec capture 长度为 1048580（期望 1048576）
```

Green 与回归验证：

```text
cargo test -p terminal capture_buffer --lib
3 passed

cargo test -p terminal active_exec_capture_keeps_only_the_newest_bounded_tail --lib
1 passed

cargo test -p terminal exec_supervisor --lib
30 passed

cargo test -p terminal --lib
165 passed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check <本次修改的 5 个 Rust 文件>
通过

git diff --check
通过
```

全仓 `cargo fmt --check` 未通过，但报告的剩余差异位于未修改的
`crates/markdown-editor` 文件；本次涉及的 terminal 文件已单独通过
`rustfmt --check`。

### 12.2 切片 2：暴露 `terminal.exec` 输出截断元数据

TDD Red 证据：

```text
cargo test -p terminal capture_buffer --lib
编译失败，共 9 个错误：captured_bytes、discarded_bytes、truncated 方法不存在

cargo test -p terminal completed_output_reports_capture_truncation_metadata --lib
编译失败，共 3 个错误：TerminalExecOutput 缺少 truncated、captured_bytes、discarded_bytes

cargo test -p public_mcp --test terminal_exec \
  terminal_exec_inserts_command_into_terminal_and_returns_observed_output
1 failed：structured_content["truncated"] 实际为 Null，期望 false
```

Green、contract 与回归验证：

```text
cargo test -p terminal capture_buffer --lib
3 passed

cargo test -p terminal completed_output_reports_capture_truncation_metadata --lib
1 passed

cargo test -p terminal observer_progress_reports_capture_truncation_metadata --lib
1 passed

cargo test -p public_mcp --test terminal_exec \
  terminal_exec_inserts_command_into_terminal_and_returns_observed_output
1 passed

cargo test -p terminal_view \
  terminal_exec_handle_maps_backend_output_to_public_mcp_result --lib
1 passed

cargo test -p terminal --lib
168 passed

cargo test -p public_mcp
132 passed

cargo test -p terminal_view --lib
290 passed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

cargo check -p public_mcp
通过

cargo check -p terminal_view
0 errors；仅有 workspace 既有 future-incompatibility warning

cargo check -p main
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check <本次修改的 13 个 Rust 文件>
通过

git diff --check
通过
```

额外回归保护覆盖：

- 普通 overflow 和单个超大 chunk 的累计 `discarded_bytes`；
- submitted-only 的零 capture 元数据；
- timeout 返回的截断元数据；
- observer 最终 progress 的截断元数据；
- terminal core 到 terminal view/Public MCP 的完整字段映射；
- Public MCP 旧版 JSON 缺少新增字段时的 `serde(default)` 兼容行为。
