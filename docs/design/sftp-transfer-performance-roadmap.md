# SFTP 传输性能优化路线图

> 文档状态：Baseline Implemented, Roadmap Pending
>
> 目标读者：维护 `sftp`、`sftp_transfer`、`sftp_view` 与 SSH transport 的开发者
>
> 编写日期：2026-11-13
>
> 当前代码基线：目录下载有界并发已落地；后续按本文的任务顺序推进

---

## 1. 结论

Navop 的 SFTP 已经完成协议层和单文件流水线优化，当前主要缺口不是
`russh-sftp` 本身，而是**目录传输的资源预算、可配置程度和可观测性**。

后续优化应采用以下顺序：

1. 先建立可重复的吞吐、延迟、内存、句柄和请求数基线；
2. 根据服务器 `limits@openssh.com` 和工作负载动态分配并发；
3. 将 buffer、pipeline 深度和目录并发从常量改为连接级可调参数；
4. 用真实链路数据判断是否需要 SFTP session pool；
5. 最后由 profiling 决定是否做全局 UI 进度合并；
6. 不整体 fork `russh-sftp`，除非出现无法在上游 API 之外解决的阻塞。

优化的核心模型仍然是：

```text
吞吐 ≈ 在途字节数 / RTT
```

因此在 RTT 较高的链路上，单纯增加线程数没有意义。必须同时控制：

```text
外层任务并发
  -> 目录文件并发
    -> 单文件读/写窗口
      -> SFTP 请求数量
        -> SSH transport window
```

---

## 2. 当前代码基线

### 2.1 已落地能力

| 能力 | 当前实现 | 代码锚点 |
| --- | --- | --- |
| 固定 SSH transport 窗口 | 独占连接 16 MiB，共享连接收窄 | `crates/sftp/src/russh_impl.rs` 的 `build_sftp_russh_config` |
| 单文件大文件下载 | 60 KiB 分片、最多 64 个在途请求、连续顺序落盘 | `pipelined_read_into_writer` |
| 单文件上传 | 256 KiB 写包、最多 64 个 ACK 在途 | `sftp_session_config` + `with_remote_replace` |
| 目录上传并发 | 小文件 6 路，大文件独占预算 | `crates/sftp/src/upload_batch.rs` |
| 目录下载并发 | 小文件 6 路，大文件独占预算 | `download_dir_with_progress` |
| 目录下载 raw session | 每个目录任务复用一个 raw session；失败时回退高层 `File` | `get_or_create_raw_session` |
| 小文件读窗口分摊 | 总读窗口按目录并发数分配 | `download_dir_with_progress` |
| 进度单调 | 原子累计 + `ProgressHighWater` | `ProgressHighWater` |
| 传输任务并发 | 同连接默认 3 条任务车道 | `crates/sftp_transfer/src/scheduler.rs` |
| 进度 UI 节流 | 每任务约 50ms 合并 | `crates/sftp_transfer/src/progress.rs` |
| 短读处理 | 同一个分片内续读补齐 | `read_chunk_fully` |
| 并发回归测试 | fake SFTP server 验证并发、字节一致、进度单调 | `russh_impl_directory_concurrency_tests.rs` |

### 2.2 当前默认预算

当前目录传输的默认策略不是“无限并发”，而是：

```text
小文件并发：6
大文件并发：1
大文件阈值：512 KiB
独占连接总读窗口：约 64
共享终端连接总读窗口：约 8
```

目录任务内部只有一个 raw session，避免在现有 3 条连接级任务车道之上再乘出
不可控的 SFTP channel 数量。

### 2.3 当前刻意不做的事情

- 不引入 session pool；
- 不把 NyaTerm 的 16 个小文件并发直接照搬；
- 不改变每连接 3 条任务车道；
- 不重写 `russh-sftp`；
- 不在没有真实基准的情况下承诺固定的倍数提升。

### 2.4 当前验证状态

2026-11-13 当前基线已通过：

```bash
cargo test -p sftp --lib
# 106 passed

cargo test -p sftp_transfer --lib
# 49 passed

cargo check -p sftp_view -p remote_file_editor
# passed

cargo clippy -p sftp --all-targets --no-deps \
  -- -D warnings -A clippy::needless-as-bytes
# passed
```

`clippy` 放行 `needless_as_bytes` 是因为仓库已有
`crates/sftp/src/server_copy_auth.rs` 中与该改动无关的历史告警；新增代码没有告警。

尚未完成：

- 真实 OpenSSH / 跳板机吞吐与取消测试；
- macOS、Windows 真机传输测试；
- `navop_app` 全量构建验证。当前环境的 `quickjs-jit-sys` / `aws-lc-sys`
  C 构建缺少目标 C11 头文件，阻塞发生在第三方 C 构建阶段，不是本次 SFTP
  改动的 Rust 编译错误。

---

## 3. 后续优化优先级

### P0：建立基线、计数器和回归门槛

**目标：先知道慢在哪里，再调参数。**

当前只有调试日志，没有稳定的性能计数。建议增加一个只在 debug/trace 或测试开关下
启用的 `SftpTransferMetrics`：

```rust
struct SftpTransferMetrics {
    files_total: u64,
    files_completed: u64,
    bytes_total: u64,
    bytes_transferred: u64,
    read_requests: u64,
    write_requests: u64,
    peak_open_handles: u64,
    peak_inflight_requests: u64,
    scan_elapsed: Duration,
    transfer_elapsed: Duration,
    commit_elapsed: Duration,
    server_limits: Option<SftpServerLimits>,
}
```

必须有这些观测点：

- 目录扫描耗时；
- 首个文件开始耗时；
- 每文件 open/read/write/flush/close/rename 耗时；
- 峰值在途请求数；
- 峰值打开句柄数；
- 重试次数；
- 取消后从请求到任务释放的耗时；
- 进度 callback 次数和最终 UI 批次次数。

建议输出一条结构化 tracing 日志，例如：

```text
target = "sftp.transfer.metrics"
operation = "directory_download"
files = 10000
bytes = 4294967296
workers = 6
read_window = 10
scan_ms = 1800
transfer_ms = 12000
commit_ms = 320
peak_open_handles = 8
peak_inflight_requests = 60
```

**验收标准**

- 同一数据集重复运行至少 3 次，指标波动可解释；
- 测试能断言并发上限和峰值句柄数；
- 没有 metrics 开关时性能不被日志影响；
- 日志中不出现用户文件内容、密码、key 或 path 之外的敏感数据。

---

### P1：按服务器限制和工作负载分配目录预算

**目标：避免固定 6 路在部分服务器上过载，在另一些服务器上又太保守。**

当前固定值：

```text
小文件并发 = 6
大文件 = 独占预算
```

建议引入内部配置结构：

```rust
struct DirectoryTransferBudget {
    small_file_concurrency: usize,
    large_file_concurrency: usize,
    per_file_read_window: usize,
    max_inflight_requests: usize,
    max_open_handles: usize,
}
```

计算规则：

```text
server_open_limit = limits.max_open_handles - HANDLE_RESERVE
small_file_concurrency =
    min(configured_small, server_open_limit, outer_capacity)
large_file_concurrency =
    min(configured_large, remaining_budget)
per_file_read_window =
    max(1, total_read_window / small_file_concurrency)
```

建议：

- `HANDLE_RESERVE = 8`；
- 没有 `limits@openssh.com` 时使用保守默认值；
- `max_open_handles` 小于 8 时至少保留 1 个 worker；
- 目录读取和写入都使用同一套预算模型；
- 服务器返回 `max_read_len` / `max_write_len` 时覆盖本地 packet 设置；
- compatibility / 受限设备模式仍然允许显式降为 1 路。

**验收标准**

- fake server 返回不同 `max_open_handles` 时，打开句柄峰值不超过限制；
- `max_read_len` / `max_write_len` 较小时不会发送超限请求；
- 大文件不会被小文件持续饿死；
- 大文件独占期间，新的小文件不插入其并发槽位；
- 取消后没有句柄泄漏。

---

### P2：把 wire 参数从常量改为连接级配置

**目标：让高 RTT / 高带宽链路可以增加在途窗口，让低性能设备可以主动收缩。**

当前关键常量位于 `russh_impl.rs`：

```text
BUFFER_SIZE = 256 KiB
PIPELINE_CHUNK_SIZE = 60 KiB
MAX_INFLIGHT_REQUESTS = 64
```

建议增加连接级或高级设置：

```rust
struct SftpWireTuning {
    read_chunk_bytes: usize,
    write_chunk_bytes: usize,
    read_pipeline_depth: usize,
    write_pipeline_depth: usize,
    max_packet_bytes: usize,
}
```

默认值保持当前行为，不要求用户先理解参数：

| 参数 | 默认值 | 说明 |
| --- | ---: | --- |
| `read_chunk_bytes` | 60 KiB | 需要服务器和通道实测后再尝试 128/256 KiB |
| `write_chunk_bytes` | 256 KiB | 当前与写包上限对齐 |
| `read_pipeline_depth` | 64（独占）/ 8（共享） | 共享连接优先保护终端输入 |
| `write_pipeline_depth` | 64（独占）/ 2（共享） | 共享连接优先降低按键延迟 |
| `max_packet_bytes` | 256 KiB | 由服务器 limits 自动收缩 |

实现原则：

- 自动值优先；
- 高级设置只在明确需要时暴露；
- 配置改变只影响新连接或新传输；
- 旧设置文件缺字段时使用默认值；
- `read_chunk_bytes`、`write_chunk_bytes` 必须有上限，避免用户输入导致内存爆炸。

**验收标准**

- 64 KiB 和 256 KiB chunk 在 fake server 上请求数、字节数和并发数正确；
- 共享终端连接不会因为大窗口导致输入延迟明显恶化；
- `max_packet_len` 与服务器 limits 冲突时自动取较小值；
- 高 RTT 测试中，提高 pipeline 深度确实提高吞吐而不是只提高请求数。

---

### P3：用真实基准决定是否引入 session pool

**暂时不要把 session pool 当作默认优化。**

当前结构是：

```text
3 条连接级任务车道
  -> 每个目录任务 1 个 raw session
    -> 目录内最多 6 个小文件并发
      -> 单文件有独立读窗口
```

只有当基准明确证明“单 raw session 的请求调度是瓶颈”时，才考虑：

```text
每个目录任务最多 2 个 session
每个 session 最多 8 个 worker
总 worker 数不超过 16
总打开句柄数不超过 server_limit - reserve
```

引入前必须记录：

- 单 session 的峰值在途请求；
- 单 session 的 RTT / 响应分布；
- 服务器 `MaxSessions`；
- `max_open_handles`；
- 目录任务并发数；
- 终端输入延迟。

若没有这些证据，session pool 只会增加连接数、句柄数和取消清理复杂度。

**验收标准**

- 有 session pool 相比单 session 的 A/B 数据；
- 单文件大文件吞吐不下降；
- 小文件吞吐提升足以抵消额外 channel 和内存；
- 服务器连接数不超限；
- 取消、错误、断开后没有残留 session。

---

### P4：按 profiling 结果做全局 UI 进度合并

当前已经有有效的保护：

- 每任务约 50ms 合并；
- 只保留最新进度；
- `wakeup` channel 容量为 1；
- UI 不直接承担网络 I/O。

只有出现以下证据时才做全局合并：

- 多任务并发时 UI 帧时间明显上升；
- 进度事件批次高于每帧预算；
- 传输面板成为 CPU 热点；
- 进度更新导致浏览器列表或整个 shell 重绘。

推荐最终结构：

```text
worker progress
  -> executor 级 latest-progress map
  -> 16ms 有界 batch
  -> 一次 GPUI update
  -> 只 notify transfer panel
  -> 非进度事件立即 flush
```

关键约束：

- batch deadline 从第一条事件开始，不随后续事件延长；
- 同一个 job 只保留最后一条进度；
- 完成、失败、取消、目录列表等控制事件立即结束 batch；
- 后台任务进度不重绘当前活动窗口；
- 进度批次不得重新排序整个目录列表。

**验收标准**

- 单任务进度频率不高于当前 50ms 节流；
- 多任务下 GPUI update 次数由传输任务数量解耦；
- 完成事件不在 batch 中被延迟；
- 目录列表不因进度批次重算；
- CPU、帧时间和内存有改动前后对比。

---

### P5：取消、错误和兼容性加固

并发化以后，错误路径比串行更容易留下半状态。必须逐项覆盖：

- 首错停止接纳新文件；
- 已启动文件执行完自己的清理；
- 本地临时文件不残留；
- raw session 失败后不继续复用；
- 高层 `File` 回退路径显式 `shutdown`；
- 远端句柄在成功、失败、超时、取消后都释放；
- 进度不会因 in-flight 文件失败而回退；
- 单个文件失败不会被误报为整个目录成功；
- compatibility 模式强制 1 个 worker 和 1 个 session。

建议把以下行为写成测试契约：

```text
first_error_stops_new_jobs
cancel_stops_new_jobs_and_drains_inflight
raw_failure_falls_back_or_evicts_session
high_level_fallback_closes_remote_handle
progress_high_water_never_regresses
temp_files_are_absent_after_error
```

---

## 4. 推荐实施顺序

| 阶段 | 内容 | 依赖 | 预计 |
| --- | --- | --- | ---: |
| M0 | metrics、fake server、基准脚本和基线记录 | 无 | 0.5–1 天 |
| M1 | 目录传输统一 budget、服务器 limits 限制 | M0 | 2–4 天 |
| M2 | read/write chunk、pipeline 深度参数化 | M0/M1 | 2–4 天 |
| M3 | session pool A/B 实验；无收益则不引入 | M1/M2 | 2–4 天 |
| M4 | 全局 UI 进度 batch（有 profiling 证据才做） | M0 | 1–2 天 |
| M5 | 真实服务器、Windows/macOS/Linux 与取消回归 | M1–M4 | 2–3 天 |

M1 + M2 是下一阶段的主线；M3 和 M4 都是条件性任务，不应提前写死实现。

---

## 5. 基准矩阵

所有性能结论必须带以下上下文：

### 5.1 数据集

| 数据集 | 文件数 | 单文件大小 | 主要验证 |
| --- | ---: | ---: | --- |
| 大量小文件 | 10,000 | 4 KiB | 控制往返、目录调度、UI 进度 |
| 中文件 | 1,000 | 64 KiB | pipeline 与目录并发的平衡 |
| 大文件 | 10 | 100 MiB | 单文件窗口、吞吐、内存 |
| 混合目录 | 1,000 小 + 10 大 | 4 KiB / 100 MiB | 大文件独占是否造成小文件饥饿 |
| 空文件/小目录 | 100 | 0–1 KiB | 句柄、进度、边界条件 |

### 5.2 网络条件

至少覆盖：

- 本机 loopback；
- 局域网，RTT < 5ms；
- 30ms RTT；
- 100ms RTT；
- 1% 丢包或可控短时断链（如果测试环境支持）。

### 5.3 服务器条件

- 标准 OpenSSH；
- `limits@openssh.com` 可用和不可用两种情况；
- 低 `max_open_handles` 配置；
- 低 `max_read_len` / `max_write_len` 配置；
- 禁止多个 SFTP session 或兼容模式。

### 5.4 指标

每次运行记录：

- 总字节数；
- 总耗时；
- 首字节时间；
- 平均和 p95 吞吐；
- 峰值 RSS；
- 峰值打开句柄；
- 峰值在途请求；
- 请求总数；
- 失败数、重试数；
- 取消到资源释放的耗时；
- UI 批次数量和最大帧耗时。

---

## 6. 验收门槛

由于当前没有真实公网链路的稳定基线，不以固定倍数作为第一版门槛。建议采用以下
相对门槛：

- 大量小文件：在 30ms RTT 条件下，M1 相比当前基线吞吐提升至少 30%；
- 单个大文件：M1/M2 不得比当前基线退化超过 5%；
- 混合目录：大文件独占期间，小文件不能在错误状态下被误报成功；
- 峰值句柄：不超过 `server_limit - reserve`；
- 峰值内存：不因目录文件数线性无界增长；
- 取消：停止接纳新文件，且已启动文件完成清理；
- 进度：任何时刻不回退，最终值等于预期总字节数；
- 错误：任意单文件失败都不产生成功终态；
- 平台：Linux 必跑；macOS/Windows 至少完成编译和 fake server 测试；
- 真实服务器：至少完成一个局域网 OpenSSH 和一个高 RTT/代理链路的手工测试。

如果基准没有达到门槛，应保留当前默认参数，不通过“增加更多线程”掩盖问题。

---

## 7. 非目标

- 不把 Navop 改造成 NyaTerm 的目录调度实现；
- 不在没有基准证据时引入 16 个目录 worker；
- 不改变 FTP 的协议语义；
- 不让 FTP 继承 SFTP 的多文件并发假设；
- 不把大文件和小文件放进同一个无界任务池；
- 不把进度日志作为性能证明；
- 不为了追求吞吐牺牲取消、文件完整性和错误可观测性；
- 不在主线程执行 SFTP I/O、目录扫描或排序。

---

## 8. 风险与回滚

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 目录并发过高 | 服务器句柄耗尽、请求排队 | limits 限制、服务器实测、动态 budget |
| 单 session 请求过密 | 终端输入延迟、响应抖动 | 共享连接低窗口、每文件窗口分摊 |
| raw session 创建失败 | 回退高层读取后请求数升高 | 保留回退、记录失败原因、必要时重建 |
| 进度并发更新 | 进度回退或 UI 抖动 | `ProgressHighWater`、全局 batch |
| 大文件独占预算 | 混合目录小文件暂时等待 | 记录等待时间，必要时改为双车道 |
| 参数化配置错误 | 超包、超内存、服务器拒绝 | clamp、server limits、启动时校验 |
| 取消清理不完整 | 临时文件/句柄残留 | 错误路径测试、drop 清理、超时 watchdog |

回滚原则：

- M1/M2 参数先内部默认，确认收益后再暴露 UI；
- 出现吞吐或稳定性回归时，回退默认参数而不是移除错误清理；
- session pool 必须有独立开关和 A/B 证据，不能和主路径一次性绑定。

---

## 9. 需要新增或扩展的测试

### 单元测试

- `DirectoryTransferBudget` 的 clamp、服务器 limits 和并发分摊；
- `ProgressHighWater` 在乱序并发回调下的单调性；
- 大文件独占、小文件排队、首错停止接纳；
- 取消后不再启动排队任务；
- raw session 失败后的高层回退；
- 句柄峰值统计。

### fake SFTP server 测试

- 多文件同时打开读取；
- 单文件短读续读；
- `max_open_handles` 限制；
- `max_read_len` / `max_write_len` 限制；
- 取消时 in-flight 文件完成清理；
- 失败时临时文件不残留。

### 真实服务器测试

- OpenSSH 默认配置；
- `limits@openssh.com` 可用/不可用；
- 跳板机或代理导致的高 RTT；
- 目录混合大小文件；
- SFTP session 受限的服务器；
- 取消、断线、重连和重试。

---

## 10. 关键文件与职责

| 文件 | 职责 |
| --- | --- |
| `crates/sftp/src/russh_impl.rs` | 单文件流水线、目录调度、raw session、传输进度 |
| `crates/sftp/src/upload_batch.rs` | bounded scheduler、并发预算、进度高水位 |
| `crates/sftp_transfer/src/scheduler.rs` | 连接级传输任务车道 |
| `crates/sftp_transfer/src/progress.rs` | GPUI 进度合并入口 |
| `crates/sftp_transfer/src/executor.rs` | 传输任务生命周期、取消、完成状态 |
| `crates/sftp_view/src/lib.rs` | SFTP 视图、传输状态镜像、刷新策略 |
| `crates/terminal_view/src/sidebar/file_manager_panel.rs` | 侧边栏文件管理器与共享连接路径 |
| `crates/sftp/src/russh_impl_directory_concurrency_tests.rs` | 目录并发 fake server 回归测试 |

---

## 11. 下一步行动清单

- [ ] 增加 `SftpTransferMetrics` 与结构化日志；
- [ ] 建立 5 组基准数据和 3 种 RTT 的脚本；
- [ ] 把固定目录并发替换为 `DirectoryTransferBudget`；
- [ ] 接入服务器 `max_open_handles` / `max_read_len` / `max_write_len`；
- [ ] 给 read/write chunk 和 pipeline 深度建立内部配置；
- [ ] 根据 A/B 数据决定是否做 session pool；
- [ ] 根据 profiling 决定是否做 16ms 全局 UI 合并；
- [ ] 补齐取消、错误、句柄释放和临时文件清理测试；
- [ ] 完成 Linux + 局域网 + 高 RTT 实测并回填本文档结果。
