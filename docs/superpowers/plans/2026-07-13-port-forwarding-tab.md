# Port Forwarding Management Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Port Forwarding 双击崩溃，并提供与 tunnel 生命周期绑定的 A 型路由总览管理 Tab 和可重复 Docker 端到端验证。

**Architecture:** `PortForwardingRuntime` 唯一持有 tunnel guard；`PortForwardingTab` 通过 `Tokio::spawn_result` 启停 tunnel并维护可见状态；`HomePage` 只负责按 connection ID 查找、创建和激活唯一 Tab。关闭通过 `TabContent::try_close` 异步确认并在停止成功后允许移除。

**Tech Stack:** Rust、GPUI、gpui-component、Tokio、russh、Docker Compose、nginx。

## Global Constraints

- 所有 SSH socket/timer Future 必须运行在应用 Tokio runtime，禁止直接从 GPUI foreground/background executor 轮询。
- 取消关闭时 Tab 不关闭且 tunnel 不停止。
- 同一 connection ID 最多一个 Tab 和一个 tunnel。
- 首版不展示没有可靠来源的流量或活动连接统计。
- 函数不超过 50 行、文件不超过 300 行、嵌套不超过 3 层、位置参数不超过 3 个。
- 所有行为改动先看到定向测试按预期失败，再写最小实现。

---

### Task 1: Runtime 停止契约

**Files:**
- Modify: `crates/port_forwarding/src/runtime.rs`
- Modify: `crates/port_forwarding/src/runtime_tests.rs`
- Modify: `crates/port_forwarding/tests/docker_e2e.rs`

**Interfaces:**
- Consumes: `LocalPortForwardTunnel::close`、`DynamicSocksTunnel::close`。
- Produces: `PortForwardingRuntime::stop(&mut self, connection_id: i64) -> Result<bool>`。

- [ ] 在 `runtime_tests.rs` 增加不存在 ID 的幂等 stop contract，运行 `rtk cargo test -p port_forwarding stop`，确认因缺少 API 编译失败。
- [ ] 在 `runtime.rs` 添加 `stop`，分别处理 local、dynamic 和不存在的 ID；只有 close 成功才完成移除。
- [ ] 扩展 Docker E2E：记录 Local/Dynamic 实际地址，调用 stop 后断言 `is_running` 为 false，并断言原监听地址无法再次连接。
- [ ] 运行 `rtk cargo test -p port_forwarding`，确认普通测试通过且 Docker 测试仍默认 ignored。

### Task 2: Port Forwarding Tab 状态与关闭决策

**Files:**
- Create: `crates/port_forwarding_view/src/tab.rs`
- Create: `crates/port_forwarding_view/src/tab_state.rs`
- Modify: `crates/port_forwarding_view/src/lib.rs`
- Modify: `crates/port_forwarding_view/Cargo.toml`
- Modify: `crates/port_forwarding_view/locales/port_forwarding_view.yml`

**Interfaces:**
- Consumes: `PortForwardingRuntime`、`StoredConnection`、`Tokio::spawn_result`、`TabContent::try_close`。
- Produces: `PortForwardingTab::new(config, window, cx)`、`PortForwardingTabConfig`、`PortForwardingTabState`、`PortForwardingEvent`。

- [ ] 先在 `tab_state.rs` 写纯状态测试：Starting→Running、失败→重试、取消关闭保持 Running、确认关闭进入 Stopping、停止失败保留 Tab、停止成功允许关闭。
- [ ] 运行 `rtk cargo test -p port_forwarding_view tab_state`，确认状态类型尚不存在而失败。
- [ ] 写最小状态机和事件记录，重复运行测试直到通过。
- [ ] 在 `tab.rs` 实现 Focusable/EventEmitter/TabContent 骨架，使用稳定标题和 Port Forwarding 图标。
- [ ] 实现 `start`：先切换 Starting，再用 `Tokio::spawn_result` 锁 runtime 并调用 Local/Dynamic start，结果回到 GPUI 更新状态与 `ActiveConnections`。
- [ ] 实现 `try_close`：Failed 无 tunnel 直接 true；其余状态打开确认框，取消返回 false，确认后通过 Tokio stop，成功返回 true，失败返回 false。
- [ ] 用结构 contract 测试断言生产路径包含 `Tokio::spawn_result` 且不在 `cx.spawn` block 内直接调用 `runtime.start_*`/`runtime.stop`。

### Task 3: A 型路由总览 UI

**Files:**
- Create: `crates/port_forwarding_view/src/tab_render.rs`
- Modify: `crates/port_forwarding_view/src/tab.rs`
- Modify: `crates/port_forwarding_view/locales/port_forwarding_view.yml`

**Interfaces:**
- Consumes: `PortForwardingTabState`、转发配置、实际 bind address、事件列表。
- Produces: A 型三段式链路、状态 badge、信息卡、事件区域、停止/重试操作。

- [ ] 写结构测试，断言 Running 页面模型包含 local、SSH、target 三个节点和 stop action；Dynamic 模型包含 SOCKS、SSH、dynamic target。
- [ ] 运行定向测试并确认因 render model 缺失而失败。
- [ ] 实现小于 300 行的 `tab_render.rs`：顶部状态、路由链路、信息卡、事件列表；复用现有主题色、Icon、Button 和滚动容器。
- [ ] 为 Starting/Running/Stopping/Failed 分别映射颜色、文案和可用操作。
- [ ] 运行 `rtk cargo fmt --check` 和 `rtk cargo test -p port_forwarding_view`。

### Task 4: HomePage 唯一 Tab 打开路径

**Files:**
- Modify: `main/src/home/home_strategy.rs`
- Modify: `main/src/home_tab.rs`
- Add or modify focused tests near the HomePage tab-opening contracts.

**Interfaces:**
- Consumes: `PortForwardingTabConfig` 和共享 `Arc<tokio::sync::Mutex<PortForwardingRuntime>>`。
- Produces: `open_port_forwarding_tab(connection, mode, window, cx)`；稳定 ID `port-forwarding-{connection_id}`。

- [ ] 写失败 contract：Port Forwarding strategy 必须调用 Tab 打开入口；旧 `HomePage::open_port_forwarding` 不得包含 runtime start。
- [ ] 运行 `rtk cargo test -p main port_forwarding`，确认旧路径使测试失败。
- [ ] 把参数构建与 SSH 引用校验保留在 HomePage，创建 `PortForwardingTab` 并加入主 TabContainer。
- [ ] 使用现有 TabContainer 查找/激活能力确保重复双击复用已有 Tab。
- [ ] 删除旧的 detached `cx.spawn` runtime 启动代码。
- [ ] 运行 `rtk cargo test -p main port_forwarding` 和 `rtk cargo check -p main`。

### Task 5: 可重复 Docker fixture 与完成验证

**Files:**
- Create: `crates/port_forwarding/tests/docker/docker-compose.yml`
- Create: `crates/port_forwarding/tests/docker/sshd_config`
- Create: `crates/port_forwarding/tests/docker/README.md`
- Modify: `crates/port_forwarding/tests/docker_e2e.rs`
- Modify: project `AGENTS.md` only if the final investigation yields a reusable rule not already documented.

**Interfaces:**
- Consumes: Docker Compose、ignored `docker_e2e` test。
- Produces: 一条可复制的启动/执行/清理流程。

- [ ] Compose 启动 nginx 和 OpenSSH；sshd 配置明确 `AllowTcpForwarding yes`、密码认证和固定测试用户。
- [ ] README 给出 `docker compose up -d --wait`、环境变量、测试命令和 `docker compose down`。
- [ ] 运行 Compose，并执行 `ONETCLI_DOCKER_E2E=1 ... cargo test -p port_forwarding --test docker_e2e -- --ignored --nocapture`，确认 Local/Dynamic/start/stop/port release 全部通过。
- [ ] 运行 `rtk cargo test -p port_forwarding -p port_forwarding_view`、`rtk cargo test -p main port_forwarding`、`rtk cargo check -p main`。
- [ ] 运行 `rtk cargo clippy -p port_forwarding -p port_forwarding_view -p main --all-targets -- -D warnings`。
- [ ] 执行请求代码审查与 completion verification，检查 diff、文件大小、函数长度和所有验收项。

