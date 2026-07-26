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

### 2026-07-26：新增两个独立产品目标

状态：**已补充设计目标，尚未实现。**

在既有安全、数据完整性、连接所有权和 bounded ingress 工作之外，后续增加两个
彼此独立、也不与当前 P0/P1 修复混合交付的产品目标：

1. **终端会话录制**：记录带时间信息的 output、resize、marker 和必要会话元数据，
   在终端 pane 底部的稳定 footer/status bar 提供开始、暂停/继续、停止控制及状态
   指示，并支持只读回放；输入录制必须显式 opt-in，默认不持久化密码、token、私钥
   或其他认证材料，并定义格式版本、资源上限、异常退出和损坏文件恢复边界。
2. **重连后恢复历史操作日志**：连接中断和应用重启后恢复经过脱敏的命令/操作记录、
   状态、错误、输出摘要及列表展示状态；重连只恢复日志和展示上下文，**绝不自动
   重放历史命令或操作**。中断时尚未完成的操作必须显示为 `interrupted`、
   `unknown` 或 `needs_review`，显式重试前重新展示命令、参数和风险。

详细范围、状态模型、安全边界和验收标准分别见 `6.7` 与 `6.8`。两个目标当前均为
规划项，不能因为本文已有 contract 就标记为 `[已接入]`。

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

### 2026-07-26：切片 9，Serial bounded parser ingress integration

状态：**实现完成并通过 Serial ingress 定向、terminal 全量和格式检查，作为独立的
Serial 接入提交。**

本切片把切片 7/7.5 的 bounded queue 和 reservation-aware consumer 接到 Serial
真实数据面，范围和边界如下：

- 独立代码提交为 `ab7ff553 feat(terminal): bound serial parser ingress`；
- Serial reader 使用固定 4 KiB source buffer，只负责串口 I/O 和有界入队；
- 新增名为 `serial-parser` 的标准 OS thread，串行持有
  `Processor<StdSyncHandler>`，并在 parser 线程中同步更新 `Term`；
- Serial ingress 默认预算为 `64 KiB / 16 chunks / 1 control`（byte/chunk/control
  三条预算分别计算）；
- parser 通过 `recv_reserved()` 接收数据，`TerminalIngressDataGuard` 的 byte
  reservation 一直保持到 `Processor::advance()` 返回，避免 dequeue 后又把 payload
  转发到无界阶段；
- reader 和 parser 都是标准 OS thread；`futures::executor::block_on` 只用于驱动
  不依赖 Tokio runtime 的 Tokio sync primitive future，不把 GPUI 线程或 parser
  线程绑定到某个 Tokio runtime；
- reader 检测到自然 EOF/断开时只发送 `SourceClosed` control。parser 先 graceful
  drain 已接受 payload，完成 drain 后才发送 disconnect callback；用户 shutdown
  则 cancel、abort ingress 并丢弃尚未消费 backlog，两种语义不混淆；
- `RUNNING -> DRAINED/ABORTED` completion state 防止 abort 后再发送延迟的自然断开
  通知；control lane 独立于 data budget，`SourceClosed` 不会因 data lane 已满而
  永久等待；
- Serial `Term`、parser、event loop 和 reconnect 复用同一个 `GpuiEventProxy`；
  `reset_terminal_surface()` 也复用该 proxy/metrics，不在重连时静默创建第二套
  wakeup 边界；
- 本切片没有引入 Text/Hex/Mixed 模式或 encoding/schema/UI model；Local 尚未迁移；
  Serial write command channel 仍是既有 unbounded channel，属于明确保留的后续风险；
- 因此当前只完成 Serial 这条真实 ingress，Terminal P1 和 Local/SSH/Serial 全部
  端到端 bounded 仍未完成，不能据此宣称整个 P1 已完成。

完整 Red/Green、回归和未覆盖范围见 `12.9`。切片 8 中“Serial 尚未接入”的描述
保留为当时的历史事实，不回写或删除。

### 2026-07-26：Local PTY ingress decision gate

状态：**已完成运行时边界核对，暂不机械增加第二层 parser queue。**

当前 Local PTY 主数据链不是 SSH/Serial 那种“reader 把 payload 送入独立
unbounded parser channel”的结构。`alacritty_terminal` 的 `EventLoop` 在同一个
线程中执行 PTY read 和 `Processor::advance()`；无法取得 `Term` lock 时，待读数据
受上游固定 read buffer 约束，取得 lock 后同步消费，再继续下一次读取。GPUI channel
只承载已有的 payload-free、coalesced wakeup edge。

因此，在没有 RSS/throughput 证据前为 Local 主 parser 路径再套一层通用 queue，
会形成双重背压，并可能要求复制或重写 Alacritty event loop，违反最新架构手册
“保留现有 Terminal 主体、根据真实边界接入预算”的原则。

Local 仍有一个需要单独处理的 payload 边界：

```text
OscTrackingReader
  -> LocalPtyCommand::TerminalChunk { data: data.to_vec(), events }
  -> exec supervisor command channel
```

该复制在 output capture 或 OSC event 存在时发生。后续 Local 切片应先为
`TerminalChunk`/exec capture relay 定义 byte、chunk、control 与顺序预算，或用真实
flood/RSS baseline 证明现有边界；不能直接照搬 SSH/Serial parser worker。

下一实现切片转为最新架构手册排序中的应用级 SSH registry 前置工作：先在 `ssh`
crate 建立脱敏、不可序列化、可单独测试的 `ConnectionKey` domain contract，再分步
实现 slot、lease、应用级 owner 和 consumer 迁移。这个 decision gate 不代表 Local
端到端验收已经完成。

### 2026-07-26：切片 10，SSH `ConnectionKey` 纯 domain contract

状态：**实现完成并通过 SSH crate 与主要下游编译验证，已独立提交。**

代码提交：

```text
833cee6b feat(ssh): define secure connection keys
```

本切片只建立应用级 registry 的安全 identity 前置 contract：

- `ConnectionKey` 复用现有 `HostKeyIdentity` 的 endpoint/route normalization；
- target、jump、proxy、host-key policy/trust namespace、timeout、keepalive、
  keyboard-interactive context 和 X11 forwarding 都参与相等性；
- username 保持 transport 实际字符串，不做可能导致误共享的宽松 normalization；
- password、passphrase、private-key content、proxy password 和 MFA response 不进入
  key；调用方必须提供非敏感 credential slot/version 形成的 opaque
  `CredentialRevision`；
- jump、authenticated proxy 和 keyboard-interactive config 与 revision shape 不一致
  时 fail closed；
- key 字段私有，不实现持久化序列化；`Debug` 和错误只输出脱敏元数据；
- 为完整 trust namespace 增加 OpenSSH `known_hosts` path 的只读 getter。

本切片没有创建 registry、slot 或 global owner，没有拨号，也没有迁移 Terminal、
SFTP、forwarding 或 server-copy consumer。验证和已知 workspace 范围外格式/lint
阻塞见 `12.10`。

### 2026-07-26：切片 11，SSH registry single-flight slot contract

状态：**实现完成并通过 SSH crate 与主要下游编译验证，已独立提交。**

代码提交：

```text
bae3832e feat(ssh): add single-flight session registry
```

本切片在 `ConnectionKey` 上建立应用级 registry 的第一个生命周期原语：

- 同一个 key 只有一个 `Creating` 或 `Ready` slot；
- 第一个 acquire 启动 detached manager creation，后续 acquire 使用各自的
  `oneshot` waiter 加入同一 flight；
- registry map 使用只保护短临界区的同步 mutex，factory/create 不在锁内执行，也不
  跨 `.await` 持锁，因此一个 key 的慢创建不会阻塞其他 key；
- 首个 acquire caller 被取消不会取消共享创建，仍存活的 waiter 可以取得同一
  manager；
- factory 失败会通知同一 flight 的所有 waiter，失败 slot 被移除，后续 acquire
  可以建立新 generation；
- generation 除了单调计数，还带不可复用的 `Arc` identity；retire 后旧 flight
  即使晚完成，也不能覆盖新 slot；
- detached creation 被取消或 panic 时，RAII cleanup 会同步移除仍属于自己的 slot
  并唤醒 waiter，避免永久留下 `Creating`；
- 公开 `retire()` 只改变 registry visibility；它不会断开返回的 manager，也不把
  “没有 consumer”错误表达为 transport disconnected。

本切片仍然没有实现 lease/refcount、idle reaper、GPUI Global/application service、
health/shutdown policy，也没有迁移 Terminal、SFTP、remote file editor、
forwarding/SOCKS 或 server-copy consumer。完整测试、编译和既有 Clippy 阻塞见
`12.11`。下一切片只建立 generation-bound lease/release contract，不提前混入 timer
或 consumer 迁移。

### 2026-07-26：切片 12，SSH generation-bound session lease contract

状态：**实现完成并通过 SSH crate 与主要下游编译验证，已独立提交。**

代码提交：

```text
d98a0be6 feat(ssh): add generation-bound session leases
```

本切片把 registry 的公开 acquire 结果从裸 `Arc<SshSessionManager>` 收紧为
`SshSessionLease`，并建立与 slot generation 绑定的 consumer accounting：

- `Ready` slot 保留 creation token、manager、`lease_count` 和 `idle_since`；
- manager publish 时不预先为 waiter 增加计数；waiter 只收到 `Published` 信号，
  随后重新进入 map，在锁内确认当前 generation 并完成 counted checkout，因此取消
  的 acquire 不会留下 phantom lease；
- 同 generation 的 lease clone 在同一 registry 临界区内精确增加计数；
- lease release/drop 先做本地幂等保护，再只减少 token 同时匹配 generation 与
  identity 的当前 slot；retire/replacement 后的 stale drop 不会修改新 generation；
- 最后一个 lease 释放只写入 idle candidate 时间，不移除或断开 manager；idle slot
  再次 acquire 会复用同一 manager，并清除当前 idle candidate；
- `Drop` 不执行异步 I/O，disconnect、timeout 和 reaper 仍由后续 registry lifecycle
  切片统一实现；
- lease 使用共享的脱敏 `ConnectionKey` identity，避免每个 lease 复制大型 key；
  `Debug` 只包含 credential-free label、generation 和 active 状态。

本切片仍未增加 idle timeout/reaper、transport health/shutdown、GPUI
application-level service 或任何生产 consumer 迁移。完整状态机、测试、下游验证
与下一切片边界见 `12.12`。

### 2026-07-26：切片 13，SSH registry-owned cancellable idle reaper

状态：**实现完成并通过 SSH crate 与主要下游编译验证，已独立提交。**

代码提交：

```text
25fbf085 feat(ssh): reap idle session generations
```

本切片在 generation-bound lease 之上增加 registry 自有的空闲回收生命周期：

- 默认 idle timeout 为 60 秒，同时提供固定于 registry 生命周期的显式配置 API；
  zero 或无法用 Tokio `Instant` 表示的 timeout 在构造阶段即被拒绝，避免 lease
  `Drop` 到最后一刻才因 deadline 溢出 panic；
- 每次进入 idle 都生成带不可复用 identity 的 `IdleCandidate`；reacquire 会清除旧
  candidate，再次 release 会得到新的 identity 和完整 deadline；
- 每个 registry 只在首次 async acquire 时启动一个 reaper task，使用 `Notify`
  响应 acquire/release/publish/retire，并始终只等待当前最早 deadline；
- lease release/drop 只在同步临界区更新计数和 candidate，再发送 `Notify`；不会
  `tokio::spawn()`、await、disconnect 或执行阻塞 I/O，且已验证可从普通非 Tokio
  线程安全 drop；
- 到期后必须在 registry lock 内重新确认 key、creation token、idle candidate
  identity、deadline 和 `lease_count == 0`，随后先从 map 移除 exact generation，
  再在锁外异步 disconnect；
- `JoinSet` 只追踪真正执行中的 disconnect job，不为每次 idle/reacquire 创建一个
  长期 sleeping task；慢 disconnect 不阻塞 registry lock 或其他 deadline；
- retire/replacement、reacquire、disconnect failure 和 registry drop 均有确定性
  contract 测试；失败日志只使用脱敏 connection label，失败 slot 不会复活。

当前 `SessionRegistryCore::Drop` 只同步请求 reaper 取消并使其收敛，不在 `Drop`
中执行 async I/O；这不等价于应用级 graceful shutdown。application owner、health/
reconnect、snapshot/observer 和生产 consumer 迁移仍是后续独立切片。完整状态机、
20 项 registry 测试、74 项 SSH 全量回归和既有 Clippy 阻塞见 `12.13`。

### 2026-07-27：切片 14，SSH application session service

状态：**实现完成并通过 SSH crate 与主要下游验证，已独立提交。**

代码提交：

```text
a29a0f04 feat(ssh): add application session service
```

本切片在 `ssh` crate 内建立与 GPUI 无关的应用级生命周期 service：

- `SshSessionService` 是 shared SSH registry 的可 clone facade；clone 共享同一个
  lifecycle、registry、shutdown driver 和最终 report；
- `acquire()` 继续只返回 generation-bound `SshSessionLease`，没有为了方便应用接线
  重新公开裸 `Arc<SshSessionManager>`；
- service lifecycle 明确为 `Running -> ShuttingDown -> Stopped`；shutdown
  线性化后 admission 永久关闭，创建任务的迟到结果不能重新发布 generation；
- shutdown 先同步关闭所有已发布 manager 的 reconnect gate，再在一个固定的总
  deadline 内取消 registry-owned work、等待 transport cleanup 并生成稳定 report；
- `snapshot()` 与 `subscribe()` 只暴露 lifecycle、slot、lease 和 task 计数，不暴露
  `ConnectionKey`、config、host、用户名、密码、私钥、passphrase、MFA 内容或
  credential revision；
- 并发或重复 `shutdown()` 调用共享一次 teardown 和同一个 sticky report；首个
  waiter 被取消不会取消 service-owned shutdown driver；
- manager connector panic/cancellation、in-flight creation cancellation、stuck
  disconnect 和 disconnect failure 均有恢复或有界收敛测试；
- `Drop` 只同步关闭 admission/reconnect gate 并请求取消，不启动、不阻塞等待任何
  async I/O；正常应用退出必须显式 await `shutdown()`。

本切片的默认总 shutdown deadline 是 5 秒。SSH 全量回归为 90 项通过，并完成
`terminal`、`sftp`、`sftp_view`、`remote_file_editor` 和 `port_forwarding` 下游
编译验证。该提交边界内**尚未**创建 GPUI Global，也没有迁移任何生产 consumer；
完整 contract 与命令见 `12.14`。

### 2026-07-27：切片 15，GPUI application Global owner and explicit quit shutdown

状态：**实现完成并通过 main 定向、模块回归和编译验证，已独立提交。**

代码提交：

```text
5b11e09c feat(app): own and shut down shared ssh sessions
```

本切片把切片 14 的 GPUI-independent service 接到唯一应用 owner 与正常退出路径：

- `GlobalSshSessionService` 在应用初始化时恰好安装一次；初始化顺序为
  `one_core::init(cx) -> init_ssh_session_service(cx) -> ai_chat_view::init(cx)`，
  确保先安装 Tokio runtime，再创建 shared SSH service；
- 正常退出统一走 `shutdown_ssh_sessions_and_quit()`：在 Tokio runtime 上等待
  service 的有界 shutdown，记录不含凭据的 report，随后才调用 `cx.quit()`；
- 用户确认退出保持 `close_all_tabs -> service.shutdown -> cx.quit` 顺序；tab
  关闭失败时不会错误关闭 shared SSH service；
- 没有 active window、缺少 application entity 和 update installer 要求退出的路径
  也复用同一个 helper，不再各自绕过 shared transport teardown；
- GPUI `on_app_quit` observer 只保留为平台驱动退出的幂等 fallback。当前 GPUI
  revision 对所有 quit observer 总共只等待 200ms，短于 service 默认 5 秒 deadline，
  因此 observer 不能替代正常路径中的显式 await；
- shutdown 日志只包含 reason、deadline/完成状态和 manager/task 计数，不记录连接
  identity、host、用户名或认证材料。

定向测试证明应用 Global 的多个 service clone 共享同一生命周期，并静态守护确认
退出与 updater 路径都先调用 shared shutdown helper；`onetcli_app` 模块 28 项回归和
`cargo check -p main` 均通过。该切片仍然只建立 owner 和退出边界，Terminal、SFTP、
Remote File Editor、forwarding/SOCKS 与 server-copy 仍未迁移到 service/lease；
完整验证见 `12.15`。

### 当前后续实施边界

完成切片 15 不表示整个 terminal optimization 目标完成。仍需按独立小提交继续：

1. 定义 SSH transport health、invalidation 和 reconnect policy；
2. 依次迁移 Terminal、SFTP、Remote File Editor、forwarding/SOCKS 与 server-copy，
   并收口生产路径直接 `SshSessionManager::new()` 或绕过 lease 的 manager clone；
3. 依据已有测量结果为 Local PTY capture/OSC relay 建立 bounded budget，并将 Serial
   write command channel 有界化；
4. 完成 Local/SSH/Serial 的 flood、slow consumer、abort、reconnect、hash 和 control
   latency 压力验收；
5. 实现真实 recorder 状态机、版本化 durable recording format 与崩溃恢复，然后才
   在**每个 terminal pane 底部** footer/status bar 接入开始、暂停/继续、停止按钮；
   控件不能覆盖 terminal viewport，也不能在 recorder 尚不存在时先放空壳 UI；
6. 实现与活动 backend 强隔离的只读 playback；
7. 实现 versioned reconnect operation journal、crash-safe checkpoint、历史展示和
   用户显式 retry；任何重连或恢复路径都**绝不自动重放**历史命令、输入、文件操作
   或控制序列。

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
9. 终端会话录制单独立项；
10. 重连后的历史操作日志恢复单独立项；
11. 基于实测结果再决定 render policy。

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

运行时核对进一步确认：Local 的 PTY read 与 `Processor::advance()` 位于同一个
Alacritty event-loop 线程，上游固定 read buffer 是 parser 前的现有上界，GPUI
channel 只发送不带 payload 的 coalesced wakeup。因此 local PTY 是否接入仍应由
metrics 和 baseline 决定，而不是因为 SSH 和 Serial 已接入就自动照搬。

当前应优先审查 Local 特有的 `LocalPtyCommand::TerminalChunk` capture/OSC relay；
这里会复制 `Vec<u8>` 到既有 unbounded command channel，才是已经确认的额外 payload
边界。任何改动都必须同时保留 output、OSC event、command/control 的顺序，以及
自然 EOF graceful drain 与显式 shutdown abort 的差异。

## 6.5 P1：SSH connection registry

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

1. 实现 `ConnectionKey`；
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

首个纯 domain 切片只建立 key，不拨号、不创建 global、不迁移 consumer：

- 复用 `HostKeyIdentity` 的 target/route normalization；
- 隔离 username、auth 类型及调用方提供的 opaque credential revision；
- 隔离 jump/proxy route 和各自的 auth identity；
- 隔离 host-key policy、应用 trust store 与 OpenSSH `known_hosts` namespace；
- 隔离 X11、timeout、keepalive 和 keyboard-interactive auth context；
- `Debug`、错误和测试不得包含 password、passphrase、private-key content 或 proxy
  password；
- key 字段保持私有，不实现持久化序列化。credential revision 必须来自非敏感的
  credential slot/version；调用方在秘密或 responder context 变化时必须递增 revision，
  不能把明文 secret 或普通未加盐摘要当 identity。

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

## 6.7 P3：Terminal session recording / Asciicast v2

状态：**规划中，尚未实现。**

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

### 目标范围

首版录制能力应覆盖：

- output、resize、marker 和必要的会话生命周期事件；
- 单调递增的相对时间，以及录制开始时的 wall-clock 时间；
- 不含认证材料的会话元数据，例如 `recording_id`、逻辑 `session_id`、
  backend 类型、初始终端尺寸、应用版本和录制格式版本；
- 用户可见的开始、暂停、继续和停止操作；
- 录制控制按钮固定放在每个 terminal pane 底部的 footer/status bar，不覆盖 terminal
  内容；按钮应能清楚显示 `recording`、`paused`、`stopping`、`failed` 等状态，
  并为键盘和辅助功能提供等价操作；
- 录制完成后的只读回放、暂停、seek、倍速和搜索；
- 异常退出后的 `.partial`/未完成录制识别，以及尽可能恢复到最后一个完整事件。

首版不以“逐字节复现底层 transport”为目标。必须先明确记录的是进入 terminal parser
前的 raw bytes、解码后的 terminal bytes，还是 terminal 已接受的语义事件；不能让
SSH、Serial 和 Local 在没有版本标记的情况下产生含义不同但格式相同的文件。

### 状态与持久化 contract

建议显式状态机：

```text
Idle
  -> Recording
  -> Paused
  -> Recording
  -> Stopping
  -> Stopped

Recording / Paused / Stopping
  -> Failed
```

核心约束：

- `stop` 必须幂等；并发 stop、pane close、应用退出不能重复发布同一录制；
- `pause` 后不能继续写入 output 或 input，只允许写入必要的状态边界事件；
- 使用临时文件增量写入，完整 header 和事件落盘成功后才发布最终文件；
- 文件必须带格式版本；未知版本应拒绝播放，而不是猜测字段含义；
- 需要定义单事件大小、总文件大小、持续时长、事件数量、内存缓冲和 flush 间隔上限；
- 达到任一硬上限时应安全停止并展示原因，不能继续无界增长；
- 崩溃或磁盘写入失败后不得把损坏文件伪装为完整录制；
- reader 应能识别截断尾部、非法时间戳和超大事件，隔离或跳过损坏尾部时必须向用户
  明示“部分恢复”，不能静默当作完整内容。

Asciicast v2 可作为互操作格式，但 Navop 自有 metadata 或扩展事件必须遵循版本化、
向后兼容和未知字段处理规则。durable atomic file 能力应先独立稳定，再用于最终发布。

### 安全与隐私边界

- `capture_input` 默认必须为 `false`，输入录制只能由用户针对当前录制显式开启；
- 密码提示、token、私钥、认证 challenge 或其他敏感输入期间，应自动暂停输入捕获，
  或要求用户使用明确的“暂停录制”控制；不能仅依赖普通文本正则声称已完全识别秘密；
- Agent 或自动化入口开启录制、特别是开启输入捕获时，必须经过独立 disclosure 和
  grant，不能继承一个过宽的通用终端权限；
- header、marker、错误信息和日志同样不得写入密码、token、私钥正文、完整认证命令行
  或 secret environment；
- 如果未来提供静态加密，密钥不得与录制文件无保护地放在同一位置；未实现可靠密钥
  管理前，不应把“文件可加密”写成已完成；
- 录制文件导出、分享或上传前必须再次提示其中可能包含终端输出和显式 opt-in 的输入。

### 回放隔离

回放 surface 必须与活动终端有清晰、持续可见的区别，并满足：

- playback 事件只驱动离线 terminal state，不写入当前 PTY、SSH channel 或 Serial；
- 文件中的 input 事件只能作为历史展示，默认不执行，也不能通过播放自动重新发送；
- marker、OSC、链接和潜在控制序列不能绕过当前安全策略触发本地命令、文件读取或网络
  操作；
- 关闭原始连接、删除 profile 或凭据过期后，历史录制仍不得获得连接能力；
- seek、倍速和搜索只影响回放视图，不改变任何活动 session。

### 完成定义

只有同时满足以下条件，才能把本目标从“规划中”改为“已接入”：

1. Local、SSH、Serial 各自的事件语义和不支持范围有明确 contract；
2. 默认不记录 input，敏感输入路径和 Agent disclosure 有自动化测试；
3. start/pause/resume/stop、并发关闭、磁盘写失败和崩溃恢复有确定性测试；
4. 文件大小、时长、事件数和单事件上限均有生产代码约束和超限测试；
5. 完整文件、截断尾部、非法 header、未知版本和超大事件均有 parser 测试；
6. playback 无法向活动 backend 发送 input 或重新执行历史动作；
7. TerminalView 底部 footer/status bar 有明确录制状态、暂停原因、失败状态和导出
   提示；开始录制、开启 input capture、停止和导出均有明确的用户确认或 disclosure；
8. 回放入口与活动终端 footer 控件有清晰视觉区分，回放不会显示成可发送输入的活动
   会话；
9. 高吞吐输出下的复制、锁竞争、flush 和内存峰值经过基准验证。

### 不建议首轮实现的原因

- 不是性能优化的必要前置；
- focused 分支缺少 atomic-file 依赖；
- 涉及敏感数据、安全授权、持久化和播放 UI；
- 没有确认完整成熟的 TerminalView 录制控制 UI；
- recording tap 使用同步 `Mutex`，需要评估高吞吐锁成本；
- output event 对 data 做复制，可能增加分配；
- 需要明确录制 raw bytes 还是 decoded terminal bytes。

建议 durable atomic file 在 `dev` 独立稳定后，再单独实现 recording。

## 6.8 P3：Reconnect history / operation log recovery

状态：**规划中，尚未实现。**

该目标用于在 Terminal 连接中断、成功重连或应用异常退出后，恢复用户已经看到的
历史操作记录和日志展示上下文。它不是远端 shell history 的替代品，也不是 command
replay queue。

### 目标范围

历史记录至少应包含：

- 稳定的 `operation_id` 和逻辑 `session_id`；
- 区分每次实际连接的 `connection_generation`；
- 命令或操作类型、开始/结束时间和最近状态；
- 经过脱敏的命令/参数摘要；
- 成功、失败、取消、中断和不确定结果；
- 经过大小限制的输出摘要及 `truncated`、`captured_bytes`、
  `discarded_bytes` 等已有 capture 元数据；
- 经过脱敏的错误类别和用户可理解的失败说明；
- 日志列表的筛选、展开项、滚动锚点等必要展示状态。

“恢复展示状态”不等于把旧输出重新注入活动 PTY。历史日志必须以只读记录展示，并与
重连后新 generation 产生的实时 terminal 内容明确区分。

### 状态模型

建议统一记录以下终态和恢复态：

```text
queued
running
succeeded
failed
cancelled
interrupted
unknown
needs_review
```

断线或崩溃时：

- 尚未开始的本地 queued 操作可以标记 `cancelled` 或保留为待用户处理，但不能自动
  提交给新连接；
- 已经发送、但没有收到可靠完成证据的操作必须标记 `interrupted` 或 `unknown`；
- 可能产生不可逆副作用、无法判断远端结果的操作应标记 `needs_review`；
- 不得因为重连成功就把旧 generation 的 `running` 操作改成 `succeeded`；
- 新连接必须增加 `connection_generation`，同时保留同一个逻辑 session 的历史链路。

### 持久化与恢复边界

- 日志 schema 必须版本化，并定义旧版本迁移与未知版本拒绝策略；
- 写入应采用有界 append journal 或等价的可恢复结构，定期生成原子 checkpoint；
- 明确定义每 session 条目数、总字节数、保存时长、单条输出摘要和错误文本上限；
- 淘汰策略必须可观察，不能无提示地丢失用户认为仍被保留的历史；
- 应用启动或重连时只读取到最后一个通过长度、版本和完整性校验的记录；
- 截断尾部、校验失败或非法超大记录必须被隔离，并向用户显示“日志部分恢复”；
- 日志存储失败不能阻塞 terminal 数据面或让连接无限等待，但失败状态必须可见；
- profile 删除、账号切换和 workspace 隔离时，要定义日志归属与清理策略，不能把一个
  用户或连接的记录串到另一个 session。

### 重连与显式重试 contract

重连成功后只允许：

1. 恢复历史日志；
2. 恢复日志列表的展示上下文；
3. 建立新的 `connection_generation`；
4. 让用户查看哪些操作被中断、结果未知或需要复核。

明确禁止：

- 自动重新发送历史命令、粘贴内容、控制序列或文件操作；
- 因为命令文本相同就推断操作是幂等的；
- 把历史 output 当作新连接返回的实时 output；
- 自动把 `unknown`/`needs_review` 归类为成功；
- 在没有用户确认的情况下恢复 side-effecting workflow。

用户点击“重试”时，必须重新展示将要执行的命令或结构化参数、目标连接、工作目录、
可能的副作用和脱敏后的风险提示，并经过显式确认。重试创建新的 `operation_id`，
通过 `retry_of` 指向原记录；不得覆盖或改写原操作的最终状态。

### 安全与隐私边界

- 默认不得持久化密码、token、私钥正文、认证 challenge、secret environment 或完整
  凭据命令行；
- redaction 必须发生在进入持久化层之前，不能先明文落盘再异步清理；
- `Debug`、错误、搜索索引、checkpoint 和备份文件都遵循同一脱敏规则；
- output 只保存有界摘要；允许用户针对敏感 session 关闭持久化或清除历史；
- 查看、导出和重试日志必须服从当前 workspace、连接和自动化授权边界；
- 记录中不得保存可直接重新取得认证能力的 opaque handle 或可复用临时 secret。

### 完成定义

只有同时满足以下条件，才能把本目标从“规划中”改为“已接入”：

1. 正常断线、连接失败、重连成功、应用崩溃和强制关闭都有状态转换测试；
2. 所有未确认完成的操作恢复为 `interrupted`、`unknown` 或 `needs_review`，不会伪装
   成成功；
3. 重连自动化测试证明没有历史 input/command 被写入新 backend；
4. 显式重试生成新 ID、保留 `retry_of`，并要求重新确认目标、参数和风险；
5. schema 版本、容量/保留上限、淘汰提示、截断 journal 和损坏 checkpoint 均有测试；
6. 密码、token、私钥和 secret environment 不出现在主日志、索引、错误或备份中；
7. 历史 output 与实时 output 在 UI 和数据模型中可区分；
8. 日志写入失败不会阻塞 terminal ingress，同时能向用户报告持久化降级。

该能力可以复用已有 `terminal.exec` capture 元数据，但不能把当前 command store 是否
保存过某些字段当作已经完成恢复 contract。存储 schema、重连 generation、UI 和安全
审查应作为独立 PR 拆分。

## 6.9 P3：Render policy

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

- 脱敏 `ConnectionKey` identity；
- opaque credential revision 与 keyboard-interactive context identity；
- host-key policy/trust namespace 与 X11 等安全边界；

随后再做：

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
- Agent disclosure；
- 输入默认关闭、敏感输入暂停/脱敏；
- 资源上限、异常退出和损坏文件部分恢复；
- playback 与活动 backend 的强隔离。

详细 contract 和完成定义见 `6.7`。录制应作为独立功能提交，不能与 ingress、
connection registry 或历史日志恢复混为一个大 PR。

### Reconnect history / operation log recovery

单独处理：

- versioned operation journal 与原子 checkpoint；
- logical session、connection generation 和 operation ID；
- `interrupted` / `unknown` / `needs_review` 恢复语义；
- 有界 output 摘要、容量和保留策略；
- 日志展示状态恢复；
- 敏感字段进入持久化前脱敏；
- 显式重试、`retry_of` 和二次风险确认；
- 截断 journal、损坏 checkpoint 和持久化失败降级。

详细 contract 和完成定义见 `6.8`。重连只恢复日志和展示上下文，自动化验收必须证明
旧命令、输入或文件操作不会被自动重放。

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

### 12.9 切片 9：Serial bounded parser ingress integration

#### 12.9.1 实现范围与边界

切片 9 在已有 queue contract（`4e5d3fb8`）和 reservation guard
（`d2a92d1e`）之上接入 Serial 真实数据面，代码以独立提交
`ab7ff553 feat(terminal): bound serial parser ingress` 交付。数据链路为：

```text
serial-read OS thread
    -> BoundedTerminalSender
    -> serial-parser OS thread
    -> recv_reserved()
    -> Processor::advance()
    -> shared GpuiEventProxy::queue_wakeup()
```

具体 contract：

- reader 固定使用 4 KiB source buffer；在当前 chunk 入队完成前不再执行下一次
  串口 read；
- data lane 预算为 64 KiB 和 16 chunks，control lane 预算为 1；source buffer、
  pending bytes、pending chunks 和 pending controls 的上界彼此独立；
- parser worker 是标准 OS thread，串行持有 `Processor<StdSyncHandler>` 并同步
  更新 `Term`。`futures::executor::block_on` 只驱动 runtime-independent 的
  Tokio sync primitive future，不要求 worker 运行在 Tokio runtime 中；
- worker 使用 `recv_reserved()`。`TerminalIngressDataGuard` 在
  `Processor::advance()` 返回后才 drop，因此 reservation 覆盖真正的 parser
  consumption boundary，而不只是 channel dequeue boundary；
- `Serial` 的 `Term`、parser、event loop 和 reconnect 复用同一个
  `GpuiEventProxy`；surface reset 复用原 proxy/metrics，保留现有 wakeup
  coalescing；
- 自然断开先发送 `SourceClosed` control，parser graceful drain 已接受 payload
  后才触发 disconnect callback；用户 shutdown 走 cancel + abortive discard，
  不等待或回放未消费 backlog。completion state 防止 abort 后出现延迟的自然断开
  通知；
- 本切片没有新增 Text/Hex/Mixed 或 encoding/schema/UI model，Local ingress 尚未
  迁移；Serial write command channel 仍是 unbounded，作为后续独立风险记录；
- 这只是 Serial 真实入口接线，不代表 Local/SSH/Serial 全部端到端 bounded，也不
  代表 Terminal P1 已完成。

#### 12.9.2 TDD Red：reservation 必须覆盖 parser 消费

先以旧的 `receiver.recv()` 语义运行 reservation contract，得到真实失败：

```bash
cargo test -p terminal \
  serial_ingress_tests::parser_worker_holds_byte_reservation_through_term_lock_and_parser_consumption \
  --lib -- --exact --nocapture
```

失败信息：

```text
assertion `left == right` failed:
a dequeued Serial payload must retain its reservation until parsing finishes
left: 0
right: 3
```

失败原因是 dequeue 后 byte reservation 已被释放，parser 尚未取得 `Term` lock
或执行 `Processor::advance()` 时，producer 已经可以继续制造 payload，违反架构
审查手册要求的“reservation 覆盖真正消费边界”。

#### 12.9.3 Green：定向与 crate 回归

改为 `recv_reserved()` 并把 guard 保持到同步 parser 消费完成后，以下验证全部
通过：

```text
cargo test -p terminal serial_ingress_tests --lib -- --nocapture
6 passed; 0 failed

cargo test -p terminal serial_backend::tests --lib -- --nocapture
5 passed; 0 failed

cargo check -p terminal --lib
通过

cargo test -p terminal --lib
214 passed; 0 failed

cargo test -p terminal
214 passed; 0 failed
5 ignored（既有 throughput_baseline 性能测试）
doc-tests：0 passed; 0 failed
```

Serial backend 的虚拟串口读写用例在 macOS 上按既有逻辑遇到
`Not a typewriter` 时跳过；这不改变真实串口连接路径的编译和定向 ingress
contract 结果。

#### 12.9.4 格式与 whitespace

```bash
rustfmt --edition 2021 --check \
  crates/terminal/src/lib.rs \
  crates/terminal/src/serial_backend.rs \
  crates/terminal/src/serial_ingress.rs \
  crates/terminal/src/serial_ingress_tests.rs \
  crates/terminal/src/terminal.rs
通过

git diff --check
通过
```

#### 12.9.5 Contract 覆盖

新增的 6 项 Serial ingress 测试覆盖：

- reader 到 parser 的字节顺序保持不变，parser drain 后只触发一次 coalesced
  wakeup 边；
- 自然断开在所有已接受 payload 消费完成后才发送 disconnect callback；
- parser 在 `Term` lock 和 `Processor::advance()` 阻塞期间保持 byte reservation，
  blocked sender 只有在消费结束后才能继续；
- 满 data budget 时 reader 最多保留一个额外 source chunk，`abort()` 能唤醒
  阻塞 reader；
- `SourceClosed` control 可绕过满 data budget，关闭后拒绝新的 payload；
- 已取消的 reader 不执行串口 read，abortive shutdown 不发送延迟的自然断开
  callback。

这些测试同时核对了 reader/parser 的 shutdown 生命周期、control/data lane 的
优先级和 `GpuiEventProxy` 的复用边界，没有把 payload 写入日志或错误文本。

#### 12.9.6 自然断开与 shutdown 修复说明

旧实现由 reader 直接触发 `on_disconnect`。串口断开瞬间，Terminal disconnect
handler 可能先清空 backend 并 drop parser，使已经入队但尚未解析的 backlog 被
abort 丢弃。切片 9 将通知改为 `SourceClosed` control，由 parser 在 graceful
drain 完成后发 callback；只有显式 shutdown、abort 或 queue/send error 才走
abortive discard。`RUNNING -> DRAINED` 与 `RUNNING -> ABORTED` 的 completion
state 保证两条路径不会交叉，也不会在 abort 后补发过期 disconnect。

#### 12.9.7 Clippy 范围外阻塞

```bash
cargo clippy -p terminal --lib -- -D warnings
```

严格检查仍在本切片未修改的
`crates/x11_forwarding/src/detect.rs:238` 处因
`clippy::unnecessary_sort_by` 失败。补充的
`cargo clippy -p terminal --lib --no-deps -- -D warnings` 还报告
`ingress_queue/types.rs`、`pty_backend.rs`、`ssh_backend.rs` 和
`terminal.rs` 中的既有 lint；逐项与本提交对照后均不属于 Serial ingress
接入，因此本切片不混入无关修复。

#### 12.9.8 未覆盖范围与后续工作

本切片仍未实现：

- Local PTY 的最终端到端验收，以及三条真实路径的统一 flood/slow-consumer/
  reconnect 压力验收；运行时 decision gate 已确认主 parser 前存在固定 read buffer，
  后续先处理 `LocalPtyCommand::TerminalChunk` capture/OSC relay，而不是机械复制
  SSH/Serial queue；
- Serial write command channel 的 bounded 化；
- Text/Hex/Mixed、encoding model、schema 或 UI；
- SSH host-key policy、应用级 session registry、SFTP 完整性与 transfer
  registry；
- Terminal P1 的整体完成定义。

后续应在不改变本切片 reservation 和 wakeup 语义的前提下为 Local 已确认的 payload
relay 建立预算，并补齐跨 Local/SSH/Serial 的 byte hash、关闭、重连、控制事件延迟和
per-pane budget 验收，再根据结果判断 ingress P1 是否达到手册中的完成标准。按最新
架构手册顺序，下一个独立实现切片先建立应用级 SSH registry 所需的
`ConnectionKey` 纯 contract。

### 12.10 切片 10：SSH `ConnectionKey` 纯 domain contract

#### 12.10.1 实现边界

新增：

```text
crates/ssh/src/connection_key.rs
```

并只对现有 host-key/public export 做两个窄改动：

- `HostKeyVerifier::openssh_known_hosts_path()` 暴露只读 trust namespace；
- `host_key::normalize_host()` 提升为 crate 内共享，避免 registry key 复制另一套
  host normalization。

`ConnectionKey` 的 `Eq + Hash` 覆盖：

- normalized target endpoint 和 jump/proxy route；
- target/jump/proxy username 及 auth type；
- target/jump/proxy opaque credential revision；
- keyboard-interactive responder context revision；
- host-key policy、app trust store、OpenSSH `known_hosts` path；
- timeout、keepalive interval/max 和 X11 forwarding。

credential revision 是调用方提供的非敏感 slot/version，不从明文 secret 或普通
未加盐 hash 派生。secret 或 responder context 变化而 revision 不变属于调用方违反
contract；后续 registry consumer 接线必须从配置存储的稳定 identity/revision 提供
该值。

#### 12.10.2 Contract 测试

新增测试覆盖：

- 等价 config/key 相等，host 大小写、首尾空格和尾点按现有 host-key contract
  normalization；
- target、jump、proxy username 不做宽松 normalization，避免不同认证主体误共享；
- auth 类型、credential revision、jump/proxy route、trust policy/namespace、
  timeout 或 X11 任一变化都生成不同 key；
- authenticated proxy、jump 和 keyboard-interactive revision 缺失或多余时 fail
  closed；
- password、passphrase、private-key content、proxy password、certificate path
  不出现在 `Debug` 或错误；
- lifecycle label 使用 normalized endpoint 和 auth 类型，不包含 credential。

#### 12.10.3 验证

```text
cargo test -p ssh --lib
54 passed; 0 failed

cargo check -p ssh
通过

cargo check -p terminal -p sftp -p sftp_view -p remote_file_editor \
  -p port_forwarding
通过；只有 extension-runtime 的 5 个既有 unused/dead-code warning 和 workspace
future-incompat 提示

git diff --check
通过
```

严格 clippy：

```text
cargo clippy -p ssh --lib --no-deps -- -D warnings
```

仍在本切片未修改的 `HostKeyVerifier::verify()` 返回大型
`HostKeyRejection` 处因 `clippy::result_large_err` 失败；本切片没有为通过检查而
混入错误枚举装箱或公共 API 变更。

workspace 全量 `cargo fmt --all -- --check` 仍会报告以下三个从最新 `dev` 合并而来、
且不属于本切片的既有格式差异：

```text
crates/markdown-editor/examples/visual_snapshot.rs
crates/markdown-editor/src/editor/render/table_toolbar.rs
crates/markdown-editor/tests/editor.rs
```

本切片涉及的三个 SSH 文件已用定向 `rustfmt --edition 2021` 格式化，未修改上述
Markdown Editor 文件。

#### 12.10.4 后续

下一独立切片建立 `ConnectionKey -> slot` 的 registry-owned single-flight
contract，先用 fake connector/manager 验证：

- 相同 key 的并发 acquire 只创建一个 slot/manager；
- 不同 key 绝不共享；
- dial/connect 期间不持有全局 registry lock；
- stale generation 结果不能覆盖新 slot；
- 此阶段仍不迁移生产 consumer，lease、idle reaper 和应用级 owner 分开提交。

### 12.11 切片 11：SSH registry single-flight slot contract

#### 12.11.1 实现范围与并发边界

新增：

```text
crates/ssh/src/session_registry.rs
```

并从 `crates/ssh/src/lib.rs` 公开：

```rust
SshSessionRegistry
```

registry 内部状态为：

```text
ConnectionKey
  -> Creating { creation token, independent oneshot waiters }
  -> Ready(Arc<SshSessionManager>)
```

实现刻意把 map 所有权和 manager transport 生命周期分开：

- map mutex 只覆盖 slot 查找、插入、替换和移除；
- manager factory 始终在锁外的 detached Tokio task 中运行；
- 创建成功只有在 key 当前 slot 的 generation 与不可复用 identity 都匹配时才能
  publish；
- 创建失败先原子移除当前 flight，再在锁外通知全部 waiter；
- `retire()` 会移除 `Ready` 或 `Creating` slot；对 `Creating` waiter 发送
  `Superseded`，waiter 随后重新进入 map 并加入或创建当前 generation；
- 被 retire 的旧 factory result 只会被 drop，不能重新插回 map；
- 首个 acquire future drop 不拥有 detached creation 的生死；
- creation task 在完成 publish 前取消或 panic 时，`CreationCleanup::drop()` 只清理
  token 仍匹配的旧 slot，不会误删 replacement generation；
- waiter 通知均在 state lock 外执行，不在临界区中 poll 或唤醒下游任务；
- registry 不对 `ConnectionKey` 或 `SshConnectConfig` 调用 `Debug`/`Display`，也不
  记录它们；它只把 factory 已生成的错误文本共享给 waiter，后续真实 factory 仍必须
  保证自己的错误不包含认证材料。

公开 API 当前只有：

```rust
SshSessionRegistry::new()
SshSessionRegistry::acquire(...)
SshSessionRegistry::retire(...)
```

其中 `retire()` 返回当前已发布的 `Arc<SshSessionManager>`，将最终 disconnect 决策
明确留给后续 lifecycle owner。当前不能把它解释为“最后一个 consumer 已释放”，也
不能据此宣称共享 transport 生命周期已经完成。

#### 12.11.2 Contract 测试

定向测试：

```text
cargo test -p ssh session_registry --lib
6 passed; 0 failed
```

六项测试覆盖：

- 同 key 的 12 个并发 acquire 只调用一次 factory，并取得同一个 `Arc`；
- 不同 key 创建并返回不同 manager；
- 一个 key 的 blocked creation 不持有 registry lock，另一个 key 可独立完成；
- 两个并发 waiter 共享同一次 factory failure，失败后下一次 acquire 建立新 flight；
- retire 后新 generation 先完成、旧 generation 后完成时，旧结果不能覆盖新 manager，
  原 waiter 会转入 replacement generation；
- 首个 acquire caller 取消后 detached creation 继续运行，后续 waiter 和 cached
  acquire 仍取得同一个 manager。

SSH crate 全量回归：

```text
cargo test -p ssh --lib
60 passed; 0 failed
```

#### 12.11.3 编译、格式与下游验证

```text
cargo check -p ssh
通过

cargo check -p terminal -p sftp -p sftp_view -p remote_file_editor \
  -p port_forwarding
通过
```

下游检查只报告 `extension-runtime` 中 5 个既有 unused/dead-code warning，以及
workspace 的 future-incompatibility 提示；没有 registry 或 SSH 新错误。

```text
rustfmt --edition 2021 \
  crates/ssh/src/session_registry.rs \
  crates/ssh/src/lib.rs
通过

git diff --check
通过
```

严格 Clippy：

```text
cargo clippy -p ssh --lib --no-deps -- -D warnings
```

仍然只在本切片未修改的 `crates/ssh/src/host_key.rs:399`
`HostKeyVerifier::verify()` 处因 `clippy::result_large_err` 失败。registry 新代码
没有新增 Clippy 报告；本切片没有为了通过门禁而装箱 `HostKeyRejection` 或改变既有
公共错误 API。

#### 12.11.4 未覆盖范围与下一切片

本切片明确未实现：

- lease/refcount 和 double-release 防护；
- last-release 到 idle 的状态转换；
- reacquire 取消 idle retirement；
- registry-owned cancellable reaper；
- transport health、disconnect 和 application shutdown；
- snapshot/lifecycle observer；
- GPUI application-level owner；
- Terminal、SFTP、remote file editor、forwarding/SOCKS 和 server-copy 迁移。

下一独立切片应只实现 generation-bound lease contract，并用 fake manager 验证：

1. acquire 返回 lease，clone/drop 精确增加和减少当前 generation 的 consumer 数；
2. 最后一个 lease 释放只进入 idle candidate，不立即断开 manager；
3. replacement generation 的 stale lease drop 不能减少新 slot 的 consumer 数；
4. 重复 release 或复杂 drop 顺序不能 underflow；
5. reacquire idle slot 复用同一个 manager，并取消该 generation 的 idle candidate；
6. lease 的 `Debug`、snapshot 和错误不暴露 `SshConnectConfig` 或认证信息。

idle timer、application owner 和生产 consumer 迁移继续保留为后续独立提交，避免把
slot、lease、timer、disconnect 和调用方改造重新合成一个不可验证的大切片。

### 12.12 切片 12：SSH generation-bound session lease contract

#### 12.12.1 公开 API 与 slot 状态

`crates/ssh/src/lib.rs` 新增公开：

```rust
SshSessionLease
```

`SshSessionRegistry::acquire()` 现在返回：

```rust
Result<SshSessionLease>
```

仓库检索确认此前新增的 registry API 尚无生产调用方，因此本切片直接收紧返回类型，
没有保留一个可绕过 accounting 的裸 manager acquire 入口。lease 提供：

```rust
SshSessionLease::manager()
SshSessionLease::release()
Clone
Deref<Target = SshSessionManager>
Debug
```

调用方必须在从 manager 取得的 client/channel 整个使用期内持有 lease。直接 clone
兼容期内仍可 clone 的 `SshSessionManager` 不等价于 clone lease，也不会增加 registry
计数；后续生产 consumer 迁移必须把 lease 本身纳入 owner，而不能只保存 manager。

registry 的 ready 状态从：

```text
Ready(Arc<Manager>)
```

演进为：

```text
Ready {
  shared credential-free ConnectionKey,
  creation token { generation, non-reusable identity },
  Arc<Manager>,
  lease_count,
  idle_since,
}
```

共享的 `Arc<ConnectionKey>` 只在 slot publish 时建立一次，各 lease 只 clone 该
identity，不复制包含 endpoint/route/trust namespace 的大型 key。这也使
`AcquirePhase` 保持小尺寸；严格 Clippy 不再报告本切片的 `large_enum_variant`。

#### 12.12.2 Checkout、Clone、Release 与取消语义

manager creation 成功后，registry 先发布一个：

```text
lease_count = 0
idle_since = Some(now)
```

的 ready slot，再在 state lock 外向 waiter 发送不携带 manager 的 `Published`
通知。每个仍存活的 waiter 被唤醒后重新进入 acquire loop，并在持锁时完成：

```text
确认当前 Ready generation
  -> checked_add(lease_count)
  -> clear idle_since
  -> clone manager/key/token into lease
```

这条线性化边界避免了“waiter 先拿到 manager、随后才异步登记 consumer”的竞态：

- waiter 在 publish 前取消：dead oneshot 不产生计数；
- waiter 在 publish 后、checkout 前取消：同样不产生计数；
- checkout 完成后 caller task 取消：返回值被 drop，lease 同步释放计数；
- publish 与 retire/replacement 竞态：waiter 重新查 map，只能 checkout 当前
  generation，不会拿到 stale manager。

同 generation 的 `SshSessionLease::clone()` 在 registry mutex 内增加计数并清除 idle
candidate。若原 generation 已被 retire 或替换，clone 仍可通过自身 `Arc` 保持旧
manager 存活，但被标记为 uncounted；它不会把旧 manager 的使用错误登记到新 slot。

release/drop 具备以下边界：

- 每个 lease 先把本地 `counted` 置为 false，重复 release/drop 为 no-op；
- 只有 key 和不可复用 creation token 都匹配当前 `Ready` slot 时才允许减计数；
- 已移除 slot 或 token 不匹配时 stale release 为 no-op；
- `lease_count == 0` 时拒绝继续递减，不发生 underflow；
- 从 1 降到 0 只设置 `idle_since = Some(now)`；
- release 不 remove manager、不调用 disconnect、不持锁跨 `.await`；
- poisoned mutex 继续沿用 registry 的 recover 语义，`Drop` 不因 poison 主动 panic。

`SshSessionLease::Debug` 不要求 manager 实现 `Debug`，只输出：

```text
credential-free connection label
slot generation
active/counted 状态
```

password、private-key content、passphrase、certificate path、完整
`SshConnectConfig` 和 token identity 都不会进入输出。

#### 12.12.3 Contract 测试

定向测试：

```text
cargo test -p ssh session_registry --lib
10 passed; 0 failed
```

覆盖：

1. 同 key 的 12 个并发 acquire 仍只创建一个 manager，12 个 checkout 精确记为
   12 个 lease，全部 drop 后变为 0/idle；
2. 不同 key 不共享 manager；
3. blocked creation 不持有 registry lock；
4. 同一 flight 的 failure 被全部 waiter 共享，后续 acquire 可重试；
5. stale creation result 不能覆盖 replacement generation；
6. 首个 waiter 取消不会取消 detached shared creation；
7. lease acquire、clone、部分 drop、最后 drop 和显式 release 的计数及 idle 转换
   精确；
8. idle slot reacquire 复用原 manager、清除 idle candidate 且不再次调用 factory；
9. retire 后旧 lease 与 stale clone 的 drop 不减少 replacement generation 计数；
10. 所有 waiter 在 manager publish 前取消时，ready slot 保持 `lease_count = 0`，
    后续 acquire 可复用 manager，不存在 phantom lease；
11. 公开 `SshSessionLease::clone()` 返回 counted lease，而不是因 `Deref` 误调用
    manager clone；
12. lease `Debug` 不泄露 private-key content、passphrase、certificate path 或
    config。

SSH crate 全量回归：

```text
cargo test -p ssh --lib
64 passed; 0 failed
```

#### 12.12.4 编译、格式、Clippy 与下游验证

```text
cargo check -p ssh
通过

cargo check -p terminal -p sftp -p sftp_view -p remote_file_editor \
  -p port_forwarding
通过

rustfmt --edition 2021 \
  crates/ssh/src/session_registry.rs \
  crates/ssh/src/lib.rs
通过

git diff --check
通过
```

下游检查仍只报告 `extension-runtime` 的 5 个既有 unused/dead-code warning，以及
workspace future-incompatibility 提示；没有 lease 或 SSH 新错误。

严格 Clippy：

```text
cargo clippy -p ssh --lib --no-deps -- -D warnings
```

最终只在本切片未修改的：

```text
crates/ssh/src/host_key.rs:399
HostKeyVerifier::verify()
clippy::result_large_err
```

失败。最终 lease 通过共享 `Arc<ConnectionKey>` 保持小尺寸，严格检查没有本切片
新增的 `large_enum_variant` 或其他报告。本切片仍不顺手装箱
`HostKeyRejection`，也不改变该既有公共错误 API。

#### 12.12.5 未覆盖范围与下一切片

本切片明确未实现：

- idle timeout 配置、cancellable reaper 和到期 disconnect；
- idle candidate 的 observer/snapshot；
- transport health、reconnect 和 application shutdown；
- GPUI Global/application-level owner；
- Terminal、SFTP、remote file editor、forwarding/SOCKS 和 server-copy 对 lease 的
  持有与移交；
- 生产路径中直接 `SshSessionManager::new()`/manager clone 的收口；
- “最后一个 lease + timeout”与同时 reacquire/retire/shutdown 的完整竞态处理。

下一独立切片只增加 registry-owned idle reaper，至少验证：

1. last release 为当前 generation 建立可失效的 idle candidate；
2. timeout 到期时必须在锁内重新确认 key、creation token、idle marker 和
   `lease_count == 0`；
3. remove/retire manager 后才在锁外执行异步 disconnect；
4. timeout 前 reacquire 会使旧 idle work 失效，不能断开重新活跃的 manager；
5. retire/replacement 后旧 timer 不能移除或断开新 generation；
6. repeated idle/reacquire 不累积无限 sleeping thread/task；
7. registry drop/application shutdown 可以取消并收敛 reaper task；
8. disconnect 失败保留真实状态和可诊断结果，不在 `Drop` 中阻塞或吞掉。

application-level owner、health policy 和生产 consumer 迁移继续拆成后续独立提交，
避免把 lease、timer、disconnect、GPUI lifecycle 与调用方改造重新合并成一个难以
证明的大切片。

### 12.13 切片 13：SSH registry-owned cancellable idle reaper

#### 12.13.1 Timeout、idle candidate 与单一 reaper

`SshSessionRegistry` 新增公开默认值和显式构造入口：

```rust
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn with_idle_timeout(idle_timeout: Duration) -> Self;
```

timeout 在 registry 构造时固定，不会静默修改已经绑定到 slot 的 deadline。构造阶段
拒绝：

- `Duration::ZERO`；
- `Instant::now().checked_add(idle_timeout) == None` 的不可表示值。

这使极端 timeout 不会延迟到最后一个 lease 的 `Drop` 路径中才通过裸
`Instant + Duration` 触发溢出。创建 idle candidate 时仍使用 `checked_add()`，
并明确依赖构造期已经验证的 contract。

原来的：

```text
idle_since: Option<Instant>
```

替换为：

```text
IdleCandidate {
  deadline: tokio::time::Instant,
  identity: Arc<()>,
}
```

identity 不序列化、不输出到 `Debug` 或日志，也不会复用。每次 generation 的最后
一个 lease 从 1 降到 0 时都创建新的 candidate；timeout 前 reacquire 在同一
registry lock 内取走 candidate；以后再次释放会创建全新的 identity 和完整 timeout。
因此旧的 wakeup、已缓存 deadline 或 stale work 只能触发重新检查，不能命中新一轮
idle。

每个 registry 在首次 async `acquire()` 时通过 `AtomicBool::compare_exchange`
只启动一个 reaper。reaper 的调度模型为：

```text
scan Ready slots under registry lock
  -> select exact earliest IdleCandidate
  -> release lock
  -> select {
       Notify from acquire/release/publish/retire
       sleep_until(earliest deadline)
       completion of a real disconnect job
     }
  -> recalculate
```

它不会为每次 last-release 创建 sleeping OS thread，也不会为每次 idle/reacquire
保留一个长期 sleeping Tokio task。`JoinSet` 只容纳 timeout 已真实到期、slot 已经
移除后的 disconnect job；一个慢 disconnect 不会阻止 reaper 继续处理其他 key 的
deadline。

#### 12.13.2 Release、到期核验与 disconnect 边界

lease release/drop 路径保持同步、短临界区：

```text
mark this lease locally uncounted
  -> lock registry
  -> verify exact key + creation token
  -> checked decrement
  -> if last lease, publish a fresh IdleCandidate
  -> unlock registry
  -> Notify::notify_one()
```

该路径明确不执行：

- `tokio::spawn()`；
- `.await`；
- transport disconnect；
- filesystem/network I/O；
- `std::thread::sleep()` 或其他阻塞等待。

普通 `std::thread` 上 drop lease 的测试验证了 `tokio::time::Instant::now()` 与
`Notify` 在该路径不依赖“当前线程正在进入 Tokio runtime”，不会 panic，也不会额外
启动 reaper。

deadline 到期并不直接信任此前缓存的 candidate。`take_expired_manager()` 必须在
同一个 registry mutex 临界区内重新确认：

1. map 中仍是相同 `ConnectionKey`；
2. creation generation 与不可复用 token identity 都相同；
3. `lease_count == 0`；
4. 当前 slot 的 idle identity 与 scheduled candidate 完全相同；
5. 当前 candidate deadline 已到。

只有全部满足，才在锁内 remove 该 exact slot 并取得 manager。真正的
`disconnect_for_registry().await` 始终在锁外的 `JoinSet` job 中执行。由此保证：

- reacquire 在 deadline 前清除 candidate 后，旧 work 不能断开 active manager；
- retire 后旧 candidate 不能命中 replacement generation；
- slot remove 后同 key acquire 可以立即创建 replacement，不等待旧 transport 的
  慢 disconnect；
- disconnect failure 不会把已经过期并移除的 slot 重新插回 registry；
- warning/debug 日志使用 `ConnectionKey::label()`，不输出 config、密码、私钥、
  passphrase、certificate path 或 token identity。

显式 `retire()` 的语义没有被 idle reaper 偷换：它仍然只是 visibility operation，
对 Ready slot 返回 manager，不自动 disconnect；existing lease 可以继续保持旧
generation 存活，而 stale lease drop 不能触碰 replacement。idle expiration 是与
explicit retire 分离的另一条 lifecycle 路径。

#### 12.13.3 Reaper 取消和当前 shutdown 边界

`SessionRegistryCore::Drop` 只执行：

```text
reaper_shutdown.store(true)
Notify::notify_one()
```

reaper 收到请求后：

1. 停止选择新的 deadline；
2. `abort_all()` 当前 registry-owned disconnect jobs；
3. drain `JoinSet` 的取消结果；
4. 退出并更新内部收敛状态。

reaper task 只持有 `Arc<RegistryShared>`，不持有
`Arc<SessionRegistryCore>`，因此不会反过来阻止 core 的最后一个 owner 进入
`Drop`。取消中的 `JoinError::is_cancelled()` 不会产生误导性的 warning。

这个行为只解决 registry 自有 task 的取消与资源收敛，**不是完整应用 shutdown
协议**。`Drop` 不能 await，也不会尝试在同步析构路径中逐个优雅 disconnect。后续
application-level owner 仍需提供显式 async graceful shutdown、拒绝新 acquire、
处理 live lease、设置总 deadline，并把最终状态暴露给 observer/UI。

#### 12.13.4 Contract 测试

定向测试：

```text
cargo test -p ssh session_registry --lib
20 passed; 0 failed
```

在切片 12 的 10 项 slot/lease 测试基础上，新增 10 项覆盖：

1. timeout 前 generation 保持可见且不 disconnect，到期后才 remove/disconnect；
2. reacquire 使旧 idle work 失效，再次 release 从新时间点取得完整 timeout；
3. 多个不同 key 的 deadline 会持续重新选择真正最早项；
4. retire/replacement 后旧 generation 的 scheduled work 不能触碰 replacement；
5. 慢 disconnect 在 registry lock 外运行，同 key replacement acquire 可立即推进；
6. disconnect failure 可诊断、不会恢复已移除 slot，后续 acquire 建立新 generation；
7. 20 次 idle/reacquire churn 仍只启动一个 registry reaper；
8. registry drop 会取消并收敛 reaper，且 `Drop` 本身不执行 disconnect I/O；
9. lease 可移交给普通 `std::thread` drop，slot 正确变为 0/idle，reaper 数不增加；
10. `Duration::MAX` 这类不可表示 timeout 在 registry 构造阶段被拒绝。

SSH crate 全量回归：

```text
cargo test -p ssh --lib
74 passed; 0 failed
```

#### 12.13.5 编译、格式、Clippy 与下游验证

```text
cargo check -p ssh
通过

cargo check -p terminal -p sftp -p sftp_view -p remote_file_editor \
  -p port_forwarding
通过

rustfmt --edition 2021 \
  crates/ssh/src/session_registry.rs \
  crates/ssh/src/lib.rs
通过

git diff --check
通过
```

下游检查只报告 `extension-runtime` 的 5 个既有 unused/dead-code warning，以及
workspace future-incompatibility 提示；没有 idle reaper 或 SSH 新错误。

严格 Clippy：

```text
cargo clippy -p ssh --lib --no-deps -- -D warnings
```

仍然只在本切片未修改的：

```text
crates/ssh/src/host_key.rs:399
HostKeyVerifier::verify()
clippy::result_large_err
```

失败。为区分既有阻塞和本切片新增问题，补充运行：

```text
cargo clippy -p ssh --lib --no-deps -- \
  -D warnings -A clippy::result_large_err
通过
```

因此本切片没有新增 Clippy 报告；没有为了通过门禁而顺手装箱
`HostKeyRejection` 或改变既有公共错误 API。

#### 12.13.6 未覆盖范围与下一切片

本切片明确未实现：

- GPUI/application-level SSH registry owner/service；
- 拒绝新 acquire、等待/取消 live use、逐 transport disconnect 和总 deadline
  组成的显式 graceful application shutdown；
- transport health、失效检测和 reconnect policy；
- registry lifecycle snapshot/observer 与 metrics；
- Terminal consumer 对 `SshSessionLease` 的持有、channel generation 和重连移交；
- SFTP、remote file editor、forwarding/SOCKS 与 server-copy consumer 迁移；
- 生产路径中直接 `SshSessionManager::new()` 和绕过 lease 的 manager clone 收口。

下一独立切片应建立 application-level registry owner/service，但仍不一次迁移所有
consumer。至少需要先证明：

1. 应用只创建一个 registry owner，view/pane 不决定共享 transport 生死；
2. service 的公开 acquire 继续返回 generation-bound lease，不重新暴露裸 manager；
3. application shutdown 有显式状态、幂等入口和总 deadline，不把异步关闭塞进
   `Drop`；
4. registry lifecycle 可以通过不包含 secret 的 snapshot/observer 观察；
5. owner/service 与后续 Terminal、SFTP、forwarding 迁移之间有清晰兼容边界。

### 12.14 切片 14：SSH application session service

代码提交：

```text
a29a0f04 feat(ssh): add application session service
```

#### 12.14.1 GPUI-independent service contract

`ssh` crate 新增公开 application facade：

```rust
#[derive(Clone)]
pub struct SshSessionService { /* shared core */ }

impl SshSessionService {
    pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
    pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

    pub async fn acquire(
        &self,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<SshSessionLease>;

    pub fn snapshot(&self) -> SshSessionServiceSnapshot;
    pub fn subscribe(&self) -> watch::Receiver<SshSessionServiceSnapshot>;
    pub async fn shutdown(&self) -> SshSessionShutdownReport;
}
```

service clone 只增加 shared core 的引用，不创建新的 registry 或独立 shutdown
lifecycle。`acquire()` 继续返回切片 12 定义的 generation-bound
`SshSessionLease`；应用接线不能借 service 重新获得裸 manager 所有权。

显式 lifecycle 为：

```text
Running
  -> ShuttingDown
  -> Stopped
```

只有 `Running` 接受 acquire 和 manager publication。一旦任意 caller 线性化
shutdown：

1. 在第一次 `.await` 之前同步把 lifecycle 改为 `ShuttingDown`；
2. 永久关闭 registry admission；
3. 取消所有 creating slot 并唤醒 waiter；
4. 从 registry 取走所有已发布 generation；
5. 对每个 manager 同步关闭 reconnect gate；
6. 迟到的 connector/client 结果只能清理，不能重新 publish 或 checkout；
7. 在同一个总 deadline 内执行 manager cleanup 和 registry task convergence；
8. 发布一个 sticky report，再进入 `Stopped`。

因此 shutdown 不是“清空当前 map 后仍允许下次 acquire 重建”的 reconnectable
disconnect，也不是依靠最后一个 view/lease `Drop` 猜测应用是否退出。

#### 12.14.2 Single total deadline 与稳定 shutdown report

默认 total deadline 为 5 秒；显式 `with_timeouts()` 会在构造阶段拒绝 zero 和无法由
Tokio `Instant` 表示的 deadline。第一次 shutdown caller 建立唯一 deadline，driver
调度延迟也计入该预算，不会为每个 manager 或每个 cleanup phase 重置 timeout。

公开 report 只包含：

```rust
pub struct SshSessionShutdownReport {
    pub timed_out: bool,
    pub managers_requested: usize,
    pub managers_completed: usize,
    pub manager_failures: usize,
    pub managers_remaining: usize,
    pub registry_tasks_remaining: usize,
}
```

并发、重复和迟到的 caller 都读取同一个 report。service 自己持有 detached shutdown
driver；某一个等待 future 被取消不会取消底层 teardown，也不会让下一 caller 启动
第二次 cleanup。deadline 到期后，剩余 manager cleanup task 被 abort 并 drain，
report 真实标记 timeout、remaining 和 failure，而不是无限等待或伪报成功。

#### 12.14.3 Manager reconnect shutdown gate

为支持 registry/service shutdown，`SshSessionManager` 增加与普通 invalidation
不同的永久 shutdown gate：

- 普通 invalidation 只使当前 cached client 失效，后续 checkout 可以 reconnect；
- shutdown 先同步拒绝新 checkout 和 reconnect，再取消正在进行的 single-flight
  connector；
- connector 在 shutdown 后才返回 client 时，该 client 会被断开而不能缓存或交给
  caller；
- cached-client ping、integration write 或其他跨 generation 的迟到结果不能重新
  激活 manager；
- manager shutdown 幂等，完成后后续 client 请求稳定失败。

connector task panic 不再永久卡住 single-flight 状态；等待者会收到失败并可在正常
lifecycle 中重试。shutdown 与 connector cancellation 竞态也会使 waiter 收敛，而
不是遗留永不完成的 acquire。

#### 12.14.4 Credential-free snapshot/observer

`SshSessionServiceSnapshot` 只暴露：

- monotonic-within-service revision；
- lifecycle state；
- slot / creating / ready 数量；
- 当前 generation 的 active lease 数量；
- idle generation 数量；
- registry-owned task 数量。

它明确不包含：

- `ConnectionKey`；
- `SshConnectConfig`；
- host 或 username；
- password、private key、passphrase、MFA response；
- credential revision；
- connector error 中可能携带的认证内容。

`subscribe()` 使用 sticky `watch` snapshot。observer 可以看到
`Running -> ShuttingDown -> Stopped`，但 snapshot 是可观测性接口，不用于反向驱动
correctness-sensitive registry 决策。

#### 12.14.5 Drop 与显式应用退出边界

`SessionServiceCore::Drop` 不启动 async work，也不等待 network I/O。它只同步：

```text
begin_shutdown()
  -> close admission
  -> close published manager reconnect gates
finish_shutdown()
```

这只是异常 owner 丢失时的安全 gate，不承诺 transport 已优雅断开。正常应用退出
必须由 application owner 显式 await `SshSessionService::shutdown()`；该 owner 在本
切片尚未接入，留给切片 15。

#### 12.14.6 验证

SSH crate 全量回归：

```text
cargo test -p ssh --lib
90 passed; 0 failed
```

新增 service/manager contract 覆盖：

1. public service acquire、snapshot、observer 和 shutdown；
2. snapshot 精确追踪 lease 且不含连接数据；
3. shutdown 后拒绝 acquire 并关闭 retained generation；
4. 并发和重复 shutdown 共享 report；
5. 首个 shutdown waiter 被取消后 driver 仍完成；
6. in-flight creation、waiter 和 registry task 在 shutdown 时收敛；
7. stuck disconnect 由总 deadline 截断并报告 remaining；
8. disconnect failure 可诊断且 generation 不恢复；
9. observer 可见 `Running`、`ShuttingDown` 和 `Stopped`；
10. service `Drop` 只触发同步 gate，不发起 async I/O；
11. manager shutdown 永久禁止 reconnect 且保持幂等；
12. connector panic/cancellation 和 cached ping 迟到结果不会遗留 single-flight 或
    在 shutdown 后发布 client。

编译、格式和下游验证：

```text
cargo check -p ssh
通过

cargo check -p terminal -p sftp -p sftp_view \
  -p remote_file_editor -p port_forwarding
通过

cargo fmt -p ssh -- --check
通过

git diff --check
通过
```

严格 Clippy：

```text
cargo clippy -p ssh --lib --no-deps -- -D warnings
```

仍只被本切片未修改的：

```text
crates/ssh/src/host_key.rs
HostKeyVerifier::verify()
clippy::result_large_err
```

阻塞。隔离该既有 lint 后：

```text
cargo clippy -p ssh --lib --no-deps -- \
  -D warnings -A clippy::result_large_err
通过
```

本实现先于本轮补写的跟踪文档，因此不虚构 TDD Red 记录。

#### 12.14.7 切片边界

该提交明确尚未实现：

- GPUI Global/application owner；
- 正常退出路径显式 await service shutdown；
- transport health/invalidation/reconnect 的完整应用 policy；
- Terminal、SFTP、Remote File Editor、forwarding/SOCKS 和 server-copy consumer
  迁移；
- 生产路径直接 `SshSessionManager::new()` 或裸 manager clone 的收口；
- recorder、pane 底部录制按钮、playback；
- reconnect operation journal 和历史展示。

### 12.15 切片 15：GPUI application owner and explicit shutdown

代码提交：

```text
5b11e09c feat(app): own and shut down shared ssh sessions
```

#### 12.15.1 唯一应用 owner 与初始化顺序

`main/src/onetcli_app.rs` 新增窄 GPUI wrapper：

```rust
#[derive(Clone)]
pub(crate) struct GlobalSshSessionService {
    service: SshSessionService,
}

impl gpui::Global for GlobalSshSessionService {}
```

应用初始化顺序为：

```text
one_core::init(cx)
  -> init_ssh_session_service(cx)
  -> ai_chat_view::init(cx)
```

`one_core::init` 先安装应用 Tokio global；随后才创建唯一
`SshSessionService::new()` 并放入 GPUI Global。初始化函数断言同类型 Global 尚不
存在，避免无声覆盖一个仍持有 transport 的旧 owner。

Global 只向调用方提供 service clone。clone 共享切片 14 的 registry/lifecycle，
不会让某个 view、pane 或后台任务成为 shared transport 的最终 owner。

#### 12.15.2 正常退出显式 await 顺序

新增统一 helper：

```rust
pub(crate) fn shutdown_ssh_sessions_and_quit(
    cx: &mut App,
    reason: &'static str,
)
```

其生产顺序为：

```text
clone application-owned service
  -> Tokio::spawn(service.shutdown())
  -> await bounded shutdown task
  -> log credential-free report
  -> cx.quit()
```

以下路径已经接入：

- 没有 active window 的应用退出；
- 缺少 `GlobalOnetCliApp` entity 的应用退出；
- 用户确认退出且 `close_all_tabs` 成功；
- update installer 返回 `UpdateInstallAction::Quit`。

用户确认退出的顺序保持为：

```text
await close_all_tabs
  -> if can_quit:
       await shared SSH shutdown
       cx.quit
     else:
       reset quit state
```

因此尚未关闭成功或拒绝关闭的 tab 不会被 shared transport teardown 抢先打断。
updater 也不再直接 `cx.quit()` 绕过应用 owner。

正常生产路径中的 `cx.quit()` 已收口到该 helper。Global 缺失表示启动不变量已经
损坏，此时 helper 记录 error 后使用 bounded emergency fallback 退出，避免应用永久
卡死；这不是正常 teardown contract。

#### 12.15.3 `on_app_quit` 只是幂等 fallback

应用初始化还注册 `cx.on_app_quit(...)`，但它只调用同一个幂等 service shutdown，
用于平台驱动或未经过正常 helper 的退出。

当前 workspace 锁定的 GPUI revision：

```text
23bb2fc135a69492847c3aa68444a7d14cc282f6
```

其 `App::shutdown()` 对所有 quit observer 的总等待常量为：

```text
SHUTDOWN_TIMEOUT = 200ms
```

该预算明显短于 SSH service 默认 5 秒总 deadline。因此 `on_app_quit` 不可能承担
“必须等待 transport cleanup”的主所有权；主路径必须在调用平台 `quit` 之前显式
await。fallback 的作用只是再次关闭 admission/reconnect gate，并在 GPUI 给出的短
预算内尽可能加入相同 teardown。

#### 12.15.4 日志安全边界

应用 shutdown 日志只记录：

- 非敏感静态 reason；
- `timed_out`；
- manager requested/completed/failure/remaining 计数；
- registry task remaining；
- Tokio `JoinError`。

它不记录 `ConnectionKey`、`SshConnectConfig`、host、username、password、private
key、passphrase、credential revision 或 MFA 内容。service report 本身也不持有这些
数据，因此调用方无法因为格式化整个 report 而意外泄密。

#### 12.15.5 定向与回归验证

退出顺序守护：

```text
cargo test -p main \
  confirmed_and_update_quit_paths_await_shared_ssh_shutdown \
  -- --nocapture

1 passed; 0 failed
```

该测试确认用户确认退出和 update installer 都调用 shared shutdown helper，并守护
生产 `cx.quit()` 不重新散落到这些路径。

唯一 Global owner 与共享 lifecycle：

```text
cargo test -p main \
  ssh_session_service_has_one_application_global_owner \
  -- --nocapture

1 passed; 0 failed
```

测试从同一个 GPUI Global 取得两个 service clone，shutdown 第一个后，第二个的
snapshot 必须是 `SshSessionServiceState::Stopped`，证明它们不是两个独立 registry。

模块回归和编译：

```text
cargo test -p main onetcli_app::tests -- --nocapture
28 passed; 0 failed

cargo check -p main
通过

cargo fmt -p main -- --check
通过

git diff --check
通过
```

严格 Clippy：

```text
cargo clippy -p main --bin navop --no-deps -- -D warnings
```

被本切片未修改代码中的 7 个既有 lint 阻塞：

```text
main/src/home_tab/modern_home.rs
  clippy::too_many_arguments

main/src/new_connection/connection_window.rs
  2 x clippy::iter_overeager_cloned

main/src/personal_sync_status.rs
  clippy::derivable_impls

main/src/public_mcp_runtime/status.rs
  clippy::derivable_impls

main/src/settings/tool_exposure_settings.rs
  2 x clippy::needless_lifetimes
```

隔离这些既有 lint 后：

```text
cargo clippy -p main --bin navop --no-deps -- \
  -D warnings \
  -A clippy::too_many_arguments \
  -A clippy::iter_overeager_cloned \
  -A clippy::derivable_impls \
  -A clippy::needless_lifetimes
通过
```

测试链接阶段出现既有 macOS linker warning：

```text
__eh_frame section too large
```

不影响测试结果，也不是本切片引入的 Rust 编译错误。本实现先于本轮补写的跟踪文档，
因此不虚构 TDD Red 记录。

#### 12.15.6 未覆盖范围与后续顺序

切片 15 只建立 application owner 和显式退出 linearization point，仍未迁移生产
consumer。后续必须保持独立小提交：

1. SSH transport health、invalidation 和 reconnect policy；
2. Terminal consumer 持有 application service / generation-bound lease；
3. SFTP 和 Remote File Editor 迁移；
4. forwarding/SOCKS 和 server-copy 迁移；
5. 收口直接 manager construction 与绕过 lease 的 clone；
6. Local PTY capture/OSC relay bounded budget；
7. Serial write command channel bounded 化；
8. Local/SSH/Serial 真实 backend 压力与数据完整性验收；
9. recorder 状态机、versioned durable format、资源上限和 `.partial` 恢复；
10. 只有 recorder 可用后，才在每个 terminal pane **底部 footer/status bar** 接入
    真实开始、暂停/继续、停止按钮，且不得覆盖 viewport；
11. readonly playback 与活动 backend 强隔离；
12. versioned reconnect operation journal、crash-safe checkpoint、历史展示与用户
    显式 retry；
13. 自动化证明 reconnect、restore 和 retry UI 初始化都不会自动重放任何历史命令、
    输入、文件操作或控制序列。
