# Port Forwarding 管理 Tab 设计

## 背景

Port Forwarding 连接当前在首页双击后直接从 GPUI foreground executor 轮询依赖 Tokio socket、timer 和 reactor 的 SSH Future。该执行边界会导致桌面进程 panic/退出，而 Tokio 后台线程仍可能短暂存活。当前实现也没有可见的运行页面，用户无法确认链路状态或安全停止转发。

Docker 实测已证明底层 `PortForwardingRuntime` 在正确 Tokio runtime 中可以完成 Local 与 Dynamic SOCKS 的真实 HTTP roundtrip；原有 ignored E2E 缺少可重复的 SSH forwarding 配置，不能视为已验证。

## 目标

- 双击 Port Forwarding 连接时创建并激活一个管理 Tab，不再直接从 GPUI executor 启动 SSH Future。
- 同一 connection ID 最多存在一个管理 Tab 和一个 tunnel。
- Tab 使用 A「路由总览型」视觉，清楚展示本机监听、SSH 节点和远端目标。
- 关闭运行中的 Tab 时询问是否停止转发；取消后 Tab 与转发都保持。
- 确认关闭时先正常停止 tunnel，成功后再关闭 Tab 并清除活动状态。
- 提供可重复运行的 Docker SSH + HTTP 目标环境，验证启动、传输和停止后的端口释放。

## 非目标

- 首版不统计字节流量、活动 TCP 连接数或吞吐图表。
- 不允许关闭 Tab 后让 tunnel 隐藏在后台继续运行。
- 不新增通用 UI 控件或重构现有 TabContainer。
- 不持久化运行中的 Tab；应用重启后由用户重新启动转发。

## 用户体验

### 打开

第一次双击连接时立即创建 Tab，状态为 `Starting`。Tab 建立完成后切换到 `Running`；失败则保留 Tab 并切换到 `Failed`。再次双击同一 connection ID 只激活现有 Tab，不重复启动。

### 页面布局

页面顶部显示 Port Forwarding 图标、连接名称、转发类型、状态胶囊和主操作按钮。中央使用三段式链路卡片：

```text
本机监听                  SSH 隧道                  远端目标
127.0.0.1:9000    →    43.136.137.108    →    127.0.0.1:29000
```

Dynamic SOCKS 模式显示“本机 SOCKS5 监听 → SSH 隧道 → 动态目标”。链路下方显示 SSH 用户/地址、实际 bind address、启动时间/运行时长和最近状态。页面底部显示当前 Tab 生命周期内的真实事件：开始启动、启动成功/失败、请求停止、停止成功/失败和重试。

### 状态机

```text
Starting ──成功──> Running ──确认停止──> Stopping ──成功──> Closed
    │                 │                       │
    └──失败──> Failed └──停止失败─────────────┴──> Failed
                  │
                  └──重试──> Starting
```

### 关闭

`Starting`、`Running` 或 `Stopping` 状态关闭 Tab 时进入关闭保护。运行中的确认框展示完整链路和中断影响：

- `取消`：`try_close` 返回 `false`，Tab 不关闭，tunnel 不停止。
- `停止转发并关闭`：通过应用 Tokio runtime 执行停止；成功后返回 `true`。
- 停止失败：显示错误，返回 `false`，Tab 保留。
- `Failed` 且 runtime 中没有 tunnel 时可直接关闭。

## 架构

### Runtime

`PortForwardingRuntime` 继续作为 tunnel guard 的唯一所有者，并新增：

```rust
pub async fn stop(&mut self, connection_id: i64) -> anyhow::Result<bool>;
```

找到 Local 或 Dynamic tunnel 时执行其 async `close()`，成功后移除并返回 `true`；没有 tunnel 时返回 `false`。失败时必须保留或恢复可重试的一致状态，不能提前把活动状态清除。

### Tab

`port_forwarding_view` 新增 `PortForwardingTab`，实现 `Render`、`Focusable`、`EventEmitter<TabContentEvent>` 和 `TabContent`。Tab 持有 connection/SSH 配置、connection ID、共享 runtime、状态、事件列表、实际监听地址和启动时间。

所有 SSH/Tokio-bound 工作通过：

```rust
one_core::gpui_tokio::Tokio::spawn_result(cx, future)
```

运行。GPUI task 只等待结果、更新实体、维护 `ActiveConnections` 和通知界面。

### HomePage

`HomePage` 不再直接调用 runtime start。它负责构建 Tab、以稳定 tab ID 打开/激活、并向 Tab 注入共享 runtime。稳定 ID 由 connection ID 派生，例如 `port-forwarding-{id}`。

## Docker 验证

仓库提供 Compose fixture：一个允许 TCP forwarding 的 OpenSSH 服务和一个 nginx HTTP 目标，共享隔离网络。E2E 动态获取目标容器 IP，避免 chroot SSH 用户无法解析 Docker DNS 名称的问题。

验证必须覆盖：Local HTTP roundtrip、Dynamic SOCKS HTTP roundtrip、重复启动拒绝、stop 后 `is_running=false`、监听端口释放，以及 UI 启动路径不再直接在 GPUI executor 上轮询 SSH Future。

## 验收标准

- 双击不导致应用退出或遗留挂起的转发任务。
- 出现视觉完整的 A 型管理 Tab。
- 重复双击只激活已有 Tab。
- 取消关闭后 Tab 和 tunnel 均保留。
- 确认关闭后 tunnel 先停止，随后 Tab 关闭、监听端口释放、绿色状态清除。
- Docker Local 与 Dynamic 真实网络 roundtrip 通过。

