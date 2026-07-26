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

### 2026-07-25：切片 3，Performance Metrics 纯 Contract

状态：**实现完成并通过 terminal crate 回归验证，作为第三个独立优化提交。**

本切片只建立无 payload、原子统计的 `TerminalPerformanceMetrics` contract：

- parser/ingress 字节、chunk 数量和生命周期最大 chunk；
- ingress 当前 backlog 和生命周期峰值；
- user input 与 terminal response 字节数；
- `Term` lock wait/hold 样本、总时长和最大时长；
- wakeup request、queued、coalesced 计数；
- render 样本、总时长、最大时长和最近可见状态；
- SSH connect、reconnect 和 invalidation 计数；
- snapshot delta、吞吐率、平均值和 activity 分类。

指标 API 只接受字节数、`Duration`、布尔值和枚举，不保存 terminal payload、
命令内容或认证信息。所有累计 counter 和 duration 都使用饱和运算，避免长期运行后
整数回绕；跨字段 snapshot 是由独立 atomic load 组成的 best-effort observability
数据，不承诺事务一致性，也不得用于驱动 correctness-sensitive terminal 行为。
`ingress_pending_bytes_max` 表达 metrics 实例的生命周期峰值，不是 snapshot window
内的峰值。

本切片明确**尚未接入** parser、backend、SSH、GPUI render 或 wakeup 的真实埋点。
当前 `dev` 已经存在 `GpuiEventProxy::wakeup_pending` 去重和 terminal event loop 的
8ms 聚合，不能机械覆盖为历史分支实现；wakeup 行为及其 request/queued/coalesced
埋点将在后续独立切片中审查，重点验证不会丢失最终 repaint。

独立审查确认：本次没有改变现有 wakeup、render、parser、SSH lifecycle、exec
supervisor 或 ingress queue 行为；atomic 使用适合统计数据的低成本顺序，snapshot
的不一致边界已经在模块 contract 中明确；duration 转换、累计值和 delta 均采用
saturating 语义。

验证记录见本文末尾的“实施验证记录”。

### 2026-07-25：切片 4，接入现有 Wakeup 去重指标

状态：**实现完成并通过 terminal crate 回归验证，作为第四个独立优化提交。**

本切片没有重新实现 wakeup coalescing，而是在当前 `dev` 已有行为上接入切片 3
的三项指标：

- 每次 Alacritty `Wakeup` 请求记录 `wakeup_requests`；
- 首次通过 pending gate 且成功进入 terminal event channel 时记录
  `wakeup_queued`；
- pending gate 已经为 true、当前请求被现有去重逻辑吸收时记录
  `wakeup_coalesced`。

`GpuiEventProxy::new` 继续保持原调用方式，并为每个 proxy 创建独立 metrics；
新增 `with_metrics` 用于显式共享/测试注入，`performance_metrics` 返回同一个
`Arc<TerminalPerformanceMetrics>`。指标不包含事件 payload，也不增加日志。

本切片刻意不修改：

- `wakeup_pending.swap(true)` 的去重 gate；
- terminal event loop 的 8ms 聚合周期；
- tick 中先转发非 Wakeup、最后转发 Wakeup 的顺序；
- 转发 Wakeup 前 reset pending gate 的时机；
- SSH/Serial 直接发送的 Wakeup；
- parser、lock 和 render 的真实埋点。

最终 repaint 不丢失的关键边界仍由当前实现保证：event loop 转发已聚合 Wakeup
前先 reset gate，因此 reset 后到达的新 Wakeup 会进入下一批，而不会被上一批永久
吞掉。现有 reset 后重新入队测试继续通过，新增 metrics 测试也覆盖 reset 后请求、
queued 和 coalesced 计数继续增长。

独立审查确认：指标更新位于既有分支判定旁边，queued 只在 channel send 成功后
增加；Title、Bell、Exit 等非 Wakeup 事件仍走原路径，未受 metrics 或 pending gate
影响。

验证记录见本文末尾的“实施验证记录”。

### 2026-07-25：切片 5，Terminal 共享并暴露 Performance Metrics

状态：**实现完成并通过 terminal crate 回归验证，作为第五个独立优化提交。**

本切片只建立 `Terminal` 对 metrics 的稳定所有权和只读观察入口，不增加新的
parser、render、lock、SSH 或 ingress 埋点：

- `Terminal::create_term` 为每个终端实例创建唯一的
  `Arc<TerminalPerformanceMetrics>`；
- 同一个 `Arc` 同时交给 `GpuiEventProxy` 并保存在 `Terminal`；
- local、local-disconnected、SSH 和 Serial 四条构造路径统一持有该实例；
- `performance_metrics`、`performance_snapshot` 和 `performance_window` 提供读取入口；
- surface reset 在没有缓存 `event_proxy` 时重建 proxy，仍复用原 metrics，避免重连或
  reset 后指标无声归零。

snapshot 仍是切片 3 定义的 best-effort observability 数据，不承诺跨字段事务一致性，
也没有被接入任何 correctness-sensitive 决策。公开 `Arc` 只暴露 payload-free counter
contract，不包含命令、终端输出、认证信息或其他敏感 payload。

本切片刻意不修改：

- 现有 wakeup pending gate 和 8ms event loop；
- parser、render、term lock、SSH lifecycle 或 ingress queue；
- Local/SSH/Serial backend 的执行、重连与 shutdown 行为；
- terminal view、Public MCP 或设置 UI。

独立审查确认：四条生产构造路径均从 `create_term` 接收同一个 metrics；local 和
Serial 的 `event_proxy: None` reset fallback 不再调用会创建独立 metrics 的
`GpuiEventProxy::new`；已有 `GpuiEventProxy::new` API 仍保持兼容，未影响 crate 内其他
测试和调用方。

验证记录见本文末尾的“实施验证记录”。

### 2026-07-26：切片 6，接入真实 Backend/Parser Observability 与 Throughput Baseline

状态：**实现完成并通过定向 contract、terminal 回归和 ignored baseline 验证。**

本切片把切片 3–5 的 payload-free metrics 接到当前 backend 的真实数据路径，范围
保持在可观测性和基准测试，不改变终端数据面调度语义：

- Local PTY 的每次非空 OS read 记录 parser chunk；Local direct write 和
  `input_handle` 各记录一次 user input；
- `GpuiEventProxy::write_back` 统一记录所有 terminal response bytes，即使当前没有
  write-back sink；Local supervisor 和 SSH `pty_write_rx` 不重复统计；
- SSH 的 `Data`/`ExtendedData` 统一经过 parser chunk 与 `Term` lock wait/hold
  helper；Serial 每次 `read n > 0` 同样记录 parser chunk 和 lock 样本；
- SSH task 仅在非显式 shutdown 退出时记录 invalidation；成功连接按 generation
  记录 connects，只有 generation 大于 1 才记录 reconnect；
- Local、SSH、Serial 的 direct write 和 metrics-aware input handle 均覆盖去重测试；
- 新增 `crates/terminal/tests/throughput_baseline.rs`，保留 parser、metrics overhead、
  并发 parser、background activity 和真实 Local PTY 高输出关闭五项 ignored baseline。
  Local PTY baseline 的启动等待和关闭检查均有 deadline，默认测试不会启动无限输出
  子进程。

本切片刻意没有混入：

- bounded ingress queue 或 SSH ingress 架构改造；
- render policy、UI overlay、encoding、recording；
- SSH connection registry、storage/schema 修改；
- Serial/SSH Wakeup coalescing 行为改造。Serial 与 SSH 仍直接发送既有 `Wakeup`，
  不机械伪造 coalesced 指标，也没有改变 event loop 的 8ms/reset 顺序。

实现先于本轮新增测试，故不伪造 TDD Red 证据；本轮记录为定向 contract 与回归
验证，完整命令和结果见 `12.6`。

### 2026-07-26：切片 7，Bounded ingress queue 纯 Contract

状态：**实现完成并通过定向、terminal 全量、跨 crate 编译和 throughput baseline
验证。**

本切片只新增 GPUI-independent 的通用 ingress primitive，不接入任何 backend：

- `TerminalIngressBudget` 分别约束非零的 data byte、data chunk 和 control 三项预算；
- data sender 先取得完整 byte reservation，再等待 data channel 的 chunk slot；
- control 使用独立的有界 lane，不占用 data byte permits，receiver 对已就绪 control
  保持优先；
- pending bytes 精确包含已完整取得 byte reservation、即使仍在等待 chunk slot 的
  data，不包含 `acquire_many` 尚未完整成功的 waiter；
- 最后一个 sender drop 是 graceful close，receiver 会自然 drain 已接受的 control
  和 data；
- sender/receiver `abort()` 及 receiver drop 是 abortive shutdown，会唤醒 byte、
  chunk、control waiter，并丢弃 backlog；
- abort 使用 out-of-band latch，不占 control capacity；data permit 在 `recv` 返回
  payload 前释放；
- sender 和 receiver 都暴露当前/生命周期峰值 pending bytes，所有 payload/error
  `Debug` 均脱敏；
- cancelled send 通过 `ByteReservation` RAII 精确释放 reservation，不重复释放。

实现中特别不能使用
`max_pending_bytes - Semaphore::available_permits()` 推导 contract pending。Tokio
`Semaphore::acquire_many` 的 waiter 在尚未取得完整请求前，可能已经部分占用可用
permits；这种中间状态不属于本 contract 的 pending bytes。当前实现以完整
`ByteReservation` 建立/释放时的原子计数为权威，并在 permit 再次可用之前先减少
pending，因此不会把部分 waiter 误计入当前值或峰值。

本切片没有新增 Tokio-owned worker task、GPUI entity、parser、transport 或
`TerminalPerformanceMetrics` wiring，也没有修改 SSH、Serial、Local PTY、encoding、
recording、registry、storage/schema 或 render policy。下一切片是 SSH ingress
decision gate / integration，不在本切片提前扩大范围。

完整 Red/Green 与回归命令见 `12.7`。

### 2026-07-26：切片 7.5，保留 ingress reservation 至消费边界

状态：**实现完成并通过 queue contract 验证，作为独立的 reservation 小切片。**

本切片不接入任何 backend，只补齐切片 7 的端到端预算语义：

- 新增 `recv_reserved()`，返回带 `ByteReservation` 的
  `TerminalIngressDataGuard`；
- guard 的 reservation 在 `as_slice()`/`len()` 使用期间保持有效，只有
  `drop` 或显式 `into_vec()` 才释放；
- 旧 `recv()` 通过 `into_vec()` 保持“交付前释放”的兼容语义；
- guard 和 reserved item 的 `Debug` 只暴露字节数，不输出 terminal payload；
- receiver abort/drop 可以丢弃队列 backlog，但不会重复释放仍由 parser 持有的
  guard；消费方释放 guard 后 pending bytes 才归零。

这样，后续 parser worker 可以把 reservation 覆盖到真正的
`Processor::advance()` 消费边界，而不是仅覆盖 channel dequeue 边界。本切片仍未
接入 SSH、Serial、Local、GPUI wakeup 或其他 unbounded stage。

完整 Red/Green 与回归命令见 `12.7.5`。

### 2026-07-26：切片 8，SSH bounded parser ingress integration

状态：**实现完成并通过 SSH ingress 定向、terminal 全量和跨 crate 回归验证，
作为独立的 SSH 接入提交。**

在本切片开始前，先执行 `git fetch origin dev`，并将最新 `origin/dev`
（`5abc1f32`）以独立 merge commit `23da4ed7` 合入当前
`feat/terminal-optimization-rework`。切片 7 的 queue contract 已由
`4e5d3fb8` 独立提交，切片 7.5 的 reservation guard 已由 `d2a92d1e`
独立提交；本切片只在其上接入 SSH，不覆盖已有 stash 或其他 backend 的 WIP。

本切片新增 `SshParserIngress` 和 SSH actor scheduling gate：

- SSH data 使用 byte/chunk/control 分离的 bounded queue（默认
  `512 KiB / 16 chunks / 8 controls`），由独立 Tokio worker 串行持有
  `Processor<StdSyncHandler>` 并同步更新 `Term`；
- worker 使用 `recv_reserved()`，`TerminalIngressDataGuard` 一直持有到
  `Processor::advance()` 返回并完成同步消费，之后才释放 byte reservation；
- actor 同时最多保留一个尚未入队的 SSH source chunk；pending 时暂停后续
  transport read，但 command 和 terminal response 仍可优先处理；
- EOF、Close 和 `None` 走 graceful parser drain；shutdown、send/queue
  error 和其他异常走 abortive discard；pending future 在等待 worker
  完成前显式 drop，避免 sender clone 让 graceful finish 永久等待；
- parser worker 复用现有 `GpuiEventProxy::queue_wakeup()` 及 wakeup
  coalescing，不再创建 SSH 专用的 unbounded notify relay；
- 空 SSH data event 被忽略；单个 source chunk 必须不超过
  `SSH_PENDING_BYTES`，超大 chunk 由 queue 明确拒绝，actor 不隐式复制或
  拆分，以保持 transport 的“一次最多一个 source chunk”内存契约；
- ingress error 只记录字节数和预算的脱敏 warning，不记录 payload。

当前接入范围明确只有 SSH。Serial、Local 尚未接入 bounded parser queue；
command 和 terminal-response 路径仍沿用既有 unbounded channel，因此不能把
本切片描述为整个 Terminal 或 GPUI 数据面的端到端 bounded 完成，也不能宣称
整个 P1 已完成。OxideTerm 只作为 clean-room 的行为和架构参考，没有复制其
源码、测试文本、注释、错误字符串或独特常量组合。

完整 Red/Green、reservation 边界证据和回归命令见 `12.8`。

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

### 12.3 切片 3：Performance Metrics 纯 Contract

TDD Red 证据：

```text
cargo test -p terminal performance_metrics --lib
编译失败，退出码 101：
error[E0583]: file not found for module `performance_metrics`
```

Green、contract 与回归验证：

```text
cargo test -p terminal performance_metrics --lib
5 passed，168 filtered out

cargo test -p terminal --lib
173 passed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check \
  crates/terminal/src/lib.rs \
  crates/terminal/src/performance_metrics.rs \
  crates/terminal/src/performance_metrics_tests.rs
通过

git diff --check
通过
```

额外回归保护覆盖：

- parser、backlog、input、wakeup 和 SSH counter/max 聚合；
- lock/render duration 的总值、最大值和 activity 状态；
- duration total/max 在极值输入下饱和而不回绕；
- snapshot window 使用 saturating delta，且零 elapsed 时 rate 为零；
- 多线程并发 atomic 更新不会丢失计数。

补充验证：

```text
cargo clippy -p terminal --lib -- -D warnings
未通过，退出码 101；唯一错误位于本切片未修改的
crates/x11_forwarding/src/detect.rs:238：
clippy::unnecessary_sort_by
```

因此本切片自身的定向测试、terminal 全量 lib 测试、编译、格式和 whitespace
检查均通过；全依赖 Clippy 门禁被既有的 `x11_forwarding` lint 阻塞，本提交不顺手
修改该无关 crate。

### 12.4 切片 4：接入现有 Wakeup 去重指标

TDD Red 证据：

```text
cargo test -p terminal \
  event_proxy_records_wakeup_request_queue_and_coalescing --lib
编译失败，退出码 101：
error[E0599]: no associated function or constant named `with_metrics`
found for struct `GpuiEventProxy`
```

Green、contract 与回归验证：

```text
cargo test -p terminal \
  event_proxy_records_wakeup_request_queue_and_coalescing --lib
1 passed，173 filtered out

cargo test -p terminal wakeup --lib
4 passed，171 filtered out

cargo test -p terminal --lib
175 passed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check \
  crates/terminal/src/pty_backend.rs \
  crates/terminal/src/performance_metrics.rs \
  crates/terminal/src/performance_metrics_tests.rs \
  crates/terminal/src/lib.rs
通过

git diff --check
通过
```

额外回归保护覆盖：

- 三个连续 Wakeup 只入队一个，分别累计 3 requests、1 queued、2 coalesced；
- reset pending gate 后的新 Wakeup 能再次入队并累计 queued；
- event channel 已关闭时只累计 request，不误报 queued；
- 非 Wakeup 事件不被 pending gate 吞掉；
- 当前 8ms event loop 的 reset-before-forward 顺序保持未修改。

补充验证：

```text
cargo clippy -p terminal --lib -- -D warnings
未通过，退出码 101；唯一错误仍位于本切片未修改的
crates/x11_forwarding/src/detect.rs:238：
clippy::unnecessary_sort_by
```

因此本切片的 wakeup 定向测试、terminal 全量 lib 测试、编译、格式和 whitespace
检查均通过；Clippy 仍被同一个 workspace 既有 lint 阻塞。

### 12.5 切片 5：Terminal 共享并暴露 Performance Metrics

TDD Red 证据：

```text
cargo test -p terminal \
  create_term_shares_performance_metrics_with_event_proxy --lib
编译失败，退出码 101：
error[E0308]: mismatched types
expected a tuple with 3 elements, found one with 4 elements
```

该 Red 明确证明当前 `create_term` 尚未返回可由 `Terminal` 和 `GpuiEventProxy` 共享的
metrics 实例。补齐测试作用域 import 后重新执行，失败原因只剩上述 contract 缺失。

Green、contract 与回归验证：

```text
cargo test -p terminal \
  create_term_shares_performance_metrics_with_event_proxy --lib
1 passed，175 filtered out

cargo test -p terminal \
  reset_terminal_surface_clears_buffer_and_stale_connection_metadata --lib
1 passed，175 filtered out

cargo test -p terminal --lib
176 passed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

cargo check -p main
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check \
  crates/terminal/src/terminal.rs \
  crates/terminal/src/pty_backend.rs \
  crates/terminal/src/performance_metrics.rs \
  crates/terminal/src/performance_metrics_tests.rs \
  crates/terminal/src/lib.rs
通过

git diff --check
通过
```

额外回归保护覆盖：

- `create_term` 返回的 metrics 与 proxy 内部 metrics 为同一个 `Arc`；
- proxy 的 Wakeup request/queued 更新可从该共享实例读取；
- surface reset 在 `event_proxy: None` fallback 路径复用原 metrics；
- reset 前后 `Terminal::performance_metrics` 返回同一个实例；
- `performance_snapshot` 和 `performance_window` 可正确观察已有 counter contract；
- local、local-disconnected、SSH、Serial 构造路径均通过编译和 terminal 全量测试。

补充验证：

```text
cargo clippy -p terminal --lib -- -D warnings
未通过，退出码 101；唯一错误仍位于本切片未修改的
crates/x11_forwarding/src/detect.rs:238：
clippy::unnecessary_sort_by
```

因此本切片的两项定向 contract、terminal 全量 lib 测试、编译、格式和 whitespace
检查均通过；Clippy 仍被同一个 workspace 既有 lint 阻塞，本切片不修改无关 crate。

### 12.6 切片 6：真实 Backend/Parser Observability 与 Throughput Baseline

本轮没有把实现倒写成 TDD Red；新增测试是在实现完成后补齐的定向 contract 与
回归保护，重点确认埋点位置和“不重复计数”边界：

```text
cargo test -p terminal --lib
185 passed，0 failed

cargo test -p terminal --test throughput_baseline
5 ignored，0 failed

cargo test -p terminal --test throughput_baseline -- --ignored
5 passed，0 failed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

cargo check -p main
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check \
  crates/terminal/src/types.rs \
  crates/terminal/src/pty_backend.rs \
  crates/terminal/src/ssh_backend.rs \
  crates/terminal/src/serial_backend.rs \
  crates/terminal/src/terminal.rs \
  crates/terminal/tests/throughput_baseline.rs
通过

git diff --check
通过
```

新增/覆盖的 contract 包括：

- Local `OscTrackingReader` 每次非空 read 的 parser bytes/chunk；
- terminal response 在有、无 write-back sink 时均只由 `write_back` 计数；
- SSH/Serial parser 与 `Term` lock wait/hold 各自产生一个样本；
- Local/SSH/Serial direct write 与 input handle 的 user bytes 不重复；
- SSH reconnect generation 的首次连接与后续重连边界；
- ignored baseline 的默认安全跳过、background activity、并发 parser 和真实 Local
  PTY 高输出有界关闭。

ignored baseline 首次并行运行暴露了真实 PTY 启动时序：固定 100ms 等待可能在首次
read 前就发送 shutdown，导致观测到 0 bytes。随后将 baseline 改为带 2 秒 deadline
的“等待首个 ingress 后再关闭”，并重新运行 `-- --ignored`，五项全部通过；这不是
生产路径语义变更。

本轮仍明确未改变 bounded ingress、SSH ingress、render、encoding、recording、
connection registry、storage/schema 以及 Serial/SSH Wakeup coalescing；后者继续
使用既有 direct `Wakeup` 发送路径。

补充验证：

```text
cargo clippy -p terminal --lib -- -D warnings
未通过，退出码 101；唯一错误仍位于本切片未修改的
crates/x11_forwarding/src/detect.rs:238：
clippy::unnecessary_sort_by
```

因此本切片的 terminal 全量 lib 测试、ignored throughput baseline、编译、格式和
whitespace 检查均通过；Clippy 仍被同一个 workspace 既有 lint 阻塞，本切片不修改
无关 crate。

### 12.7 切片 7：Bounded ingress queue 纯 Contract

第一轮先加入测试与模块声明，尚未提供实现，得到真实的编译 Red：

```text
cargo test -p terminal ingress_queue --lib
未通过，退出码 101：
error[E0583]: file not found for module `ingress_queue`
```

补入参考历史实现的首版后又得到一项真实行为 Red：

```text
cargo test -p terminal ingress_queue --lib
12 passed，1 failed

ingress_queue_tests::byte_budget_backpressures_until_data_is_received
assertion failed：
left: 4
right: 3
```

该失败证明不能通过 semaphore 的 available permits 反推 contract pending：预算为
4，已有完整的 3-byte reservation 时，另一个 `acquire_many(2)` waiter 会部分占用
剩余 1 permit，使 available permits 暂时变成 0；但该 waiter 尚未完整取得 2 个
permits，contract pending 必须仍为 3，而不是 4。修正为完整 `ByteReservation`
建立/销毁时的 RAII 原子 accounting 后，定向测试转绿：

```text
cargo test -p terminal ingress_queue --lib
13 passed，0 failed

cargo test -p terminal --lib
198 passed，0 failed
```

全量、跨 crate 和 baseline 验证：

```text
cargo test -p terminal
unit tests：198 passed，0 failed
throughput_baseline：5 ignored，0 failed
doc tests：0 failed

cargo test -p terminal --test throughput_baseline
5 ignored，0 failed

cargo test -p terminal --test throughput_baseline -- --ignored
5 passed，0 failed

cargo check -p terminal
0 errors；仅有 workspace 既有 future-incompatibility warning

cargo check -p main
0 errors；仅有 workspace 既有 future-incompatibility warning

rustfmt --check --edition 2021 \
  crates/terminal/src/lib.rs \
  crates/terminal/src/ingress_queue.rs \
  crates/terminal/src/ingress_queue/types.rs \
  crates/terminal/src/ingress_queue_tests.rs
通过

git diff --check
通过
```

新增的 13 项 contract 测试覆盖：

- 三项 budget 的零值拒绝、byte budget 的 `u32::MAX` 上界和只读 accessor；
- empty/oversized data 立即失败，不等待队列容量；
- byte 与 chunk 两层 data backpressure，以及完全独立的 control backpressure；
- 已就绪 control 绕过受阻 data，并在 receiver 中优先于 data；
- pending/peak 的精确 accounting、delivery 前 release 和部分
  `acquire_many` waiter 不计入 pending；
- abort 唤醒所有类型 waiter、拒绝后续 send 并丢弃 backlog；
- receiver abort/drop 的即时 abortive shutdown；
- 最后一个 sender drop 后自然 drain 已接受项；
- cancelled chunk-slot waiter 释放完整 reservation，后续 full-budget send 仍准确
  backpressure，证明没有 double release；
- data、control、item 和 send error 的 `Debug` 不泄露 payload。

`crates/terminal/src/ingress_queue.rs` 为 298 行，保持在历史计划对 primitive 主文件
`< 300 lines` 的范围内。最终点验确认 `send_data` 的顺序是完整 byte reservation
先于 chunk slot，control 不获取 data permit，receiver 的 select 顺序为
abort/control/data，旧 `recv()` 兼容路径在返回 payload 前显式释放 reservation，
receiver drop 会 abort、close 并 drain；模块中没有 backend 或 metrics 引用。

补充静态检查：

```text
cargo clippy -p terminal --lib -- -D warnings
未通过，退出码 101；唯一错误位于本切片未修改的
crates/x11_forwarding/src/detect.rs:238：
clippy::unnecessary_sort_by
```

为继续点验 terminal 自身又运行了：

```text
cargo clippy -p terminal --lib --no-deps -- -D warnings
未通过，退出码 101；报告 8 项位于 pty_backend.rs、ssh_backend.rs 和 terminal.rs
的既有 lint；逐项与 HEAD 对照后确认对应表达式均非本切片新增，
且没有 ingress_queue.rs / ingress_queue/types.rs 错误。
```

因此本切片的 contract、terminal 回归、baseline、跨 crate 编译、格式和 whitespace
检查均通过；严格 Clippy 门禁仍被范围外既有 lint 阻塞，本切片不顺手修改无关代码。
下一步保持为 SSH ingress decision gate / integration。

### 12.7.5 切片 7.5：保留 ingress reservation 至消费边界

TDD Red 证据：

```text
cargo test -p terminal ingress_queue --lib
未通过，退出码 101：
ReservedTerminalIngressItem 未导出，且 BoundedTerminalReceiver 没有
recv_reserved() 方法
```

Green 与回归验证：

```text
cargo test -p terminal ingress_queue --lib
16 passed，0 failed

cargo check -p terminal
未通过，退出码 101；错误位于本切片未修改的 SSH WIP：
crates/terminal/src/ssh_ingress.rs:96 的 SshParserIngress::sender() 调用
```

queue 小切片自身的格式与 whitespace 检查通过：

```text
rustfmt --edition 2021 --check \
  crates/terminal/src/ingress_queue.rs \
  crates/terminal/src/ingress_queue/types.rs \
  crates/terminal/src/ingress_queue_tests.rs
通过

git diff --check
通过
```

新增 contract 覆盖：

- reserved receive 返回时 pending bytes 仍包含 parser 尚未消费的 payload；
- guard drop 后 sender 才能越过 byte backpressure；
- `into_vec()` 返回完整字节并在返回前释放 reservation；
- abort 与仍存活 guard 的生命周期不会 double release；
- guard `Debug` 不泄露 payload。

本切片没有修改 SSH actor 接线；`cargo check -p terminal` 的阻塞项留给后续
SSH ingress 切片修复。

### 12.8 切片 8：SSH bounded parser ingress integration

实现范围与边界：

- `origin/dev` 已在 `23da4ed7` 合入；queue contract
  `4e5d3fb8` 与 reservation guard `d2a92d1e` 保持为先前的独立提交；
- `SshParserIngress` 为每条 SSH 连接创建 byte/chunk/control bounded queue
  （默认 `512 KiB / 16 / 8`）和独立 parser worker；
- worker 串行持有 `Processor<StdSyncHandler>`，通过
  `recv_reserved()` 接收 payload，并把 reservation 保留到
  `Processor::advance()` 完成；
- SSH actor 通过 `next_ssh_actor_input()` 先处理 actor command、terminal
  response 和 pending ingress send，只有没有 pending source chunk 时才读取
  transport；queue 满时最多额外持有一个 source chunk；
- command/terminal response 继续走既有 unbounded channel，属于本切片明确的
  未覆盖范围；不能据此宣称整个 Terminal 数据面已经端到端 bounded；
- parser worker 的 repaint 使用既有 `GpuiEventProxy` wakeup gate，保持
  “最多一个 pending edge”的 coalescing 语义；
- EOF/Close/`None` 在 drop pending future 后 graceful drain；shutdown、
  queue/send error 和其他异常 abort 并丢弃 backlog；
- 空 `Data`/`ExtendedData` 不会制造误断线；超过
  `SSH_PENDING_BYTES` 的 source chunk 采用“明确拒绝 + transport chunk
  上界契约”，不在 actor 中隐式复制或拆分；`Display`/`Debug` warning
  只包含长度和预算，不泄露 payload；
- 当前只有 SSH 接入；Serial、Local 和其他 GPUI producer 尚未迁移。

TDD/回归证据：

```text
旧 worker 使用 receiver.recv() 的 reservation boundary 临时回归：
assertion `left == right` failed
the dequeued payload must remain reserved while Processor::advance is blocked
left: 0
right: 3

恢复 recv_reserved() 后：
cargo test -p terminal \
  ssh_ingress_tests::parser_worker_holds_byte_reservation_through_term_lock_and_parser_consumption \
  --lib -- --exact
1 passed，0 failed（重复运行 5 次）

cargo test -p terminal ssh_ingress --lib -- --nocapture
8 passed，0 failed

cargo check -p terminal --lib
通过

cargo test -p terminal --lib
208 passed，0 failed

cargo test -p terminal
208 passed，0 failed；throughput_baseline 的 5 个测试按设计 ignored

rustfmt --edition 2021 --check \
  crates/terminal/src/lib.rs \
  crates/terminal/src/pty_backend.rs \
  crates/terminal/src/ssh_backend.rs \
  crates/terminal/src/ssh_ingress.rs \
  crates/terminal/src/ssh_ingress_tests.rs \
  crates/terminal/src/terminal.rs
通过

git diff --check
通过
```

新增 SSH ingress contract 测试覆盖：

- command 和 terminal response 在 data queue/backpressure 下仍可被 actor
  处理，transport read 被暂停；
- sustained transport 只保留一个 pending source chunk，pending/peak bytes
  不超过预算；
- oversized source chunk 立即拒绝、不入队、不触发 transport read，错误
  debug/display 不包含 payload；
- parser worker 保持输入顺序、graceful drain，并复用 wakeup coalescing；
- parser worker 在 `Term` lock 和 `Processor::advance()` 阻塞期间保持 byte
  reservation，释放 lock 后 blocked sender 才能继续；
- abort 会丢弃尚未消费 backlog，不产生 parser metrics 或 wakeup，也不会
  double release 仍由 consumer 持有的 guard。

补充说明：

- 现有 workspace 的严格 Clippy 仍被范围外既有问题
  `crates/x11_forwarding/src/detect.rs:238`（以及 terminal 自身未修改的
  lint）阻塞；本切片没有顺手改无关代码；
- 本切片没有实现 SSH host-key policy、应用级 session registry、Serial/Local
  ingress、unbounded command/response 重构或完整端到端压力验收；
- 下一步应分别为 Local、Serial 接入同一 reservation-aware contract，并在
  三条真实路径上补齐 flood、slow consumer、abort、reconnect 和 control
  latency 验收，之后才能评估 P1 完成度。
