# Issue #97 Proxy and Local Terminal Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为数据库和远程桌面连接增加 SOCKS5/HTTP 代理，并为本地终端增加 WSL、Git Bash 与自定义 shell profile。

**Architecture:** 在 `connection_tunnel` 中建立通用本地 TCP 代理转发器，数据库驱动和远程桌面 helper 都连接本地临时端口。连接配置复用 `ProxyConfig`；终端 profile 存在 `AppSettings`，由 `LocalConfig` 统一解析为程序与参数。

**Tech Stack:** Rust、Tokio、GPUI、serde、tokio-socks、HTTP CONNECT、alacritty_terminal PTY

---

### Task 1: 通用代理隧道

**Files:**
- Modify: `crates/ssh/src/ssh.rs`
- Modify: `crates/ssh/src/lib.rs`
- Modify: `crates/connection_tunnel/src/lib.rs`
- Create: `crates/connection_tunnel/src/proxy.rs`
- Modify: `crates/connection_tunnel/Cargo.toml`

- [x] **Step 1: 写代理配置映射与目标转发失败测试**

在 `connection_tunnel` 测试中先引用尚不存在的 `ProxyTunnelConfig`、`start_proxy_tunnel` 和 `TunnelGuard`，覆盖 SOCKS5/HTTP、认证字段与本地监听地址。

- [x] **Step 2: 运行测试确认 RED**

Run: `rtk cargo test -p connection_tunnel`

Expected: 因代理隧道 API 尚不存在而编译失败。

- [x] **Step 3: 暴露并复用 SSH 代理建连函数**

```rust
pub async fn connect_via_proxy(
    proxy: &ProxyConnectConfig,
    target_host: &str,
    target_port: u16,
) -> anyhow::Result<tokio::net::TcpStream>;
```

- [x] **Step 4: 实现独立 runtime 的本地代理转发器**

```rust
pub struct ProxyTunnelConfig {
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub enum TunnelGuard {
    Ssh(LocalPortForwardTunnel),
    Proxy(ProxyTunnel),
}
```

- [x] **Step 5: 运行测试确认 GREEN**

Run: `rtk cargo test -p connection_tunnel -p ssh`

Expected: 新增与既有测试全部通过。

### Task 2: 数据库连接代理

**Files:**
- Modify: `crates/core/src/storage/models.rs`
- Modify: `crates/db/src/ssh_tunnel.rs`
- Modify: `crates/db/src/{mysql,postgresql,mssql,oracle,clickhouse}/connection.rs`
- Modify: `crates/db/src/ipc/{connection,plugin}.rs`
- Modify: `crates/db_view/src/common/db_connection_form.rs`
- Modify: `crates/db_view/locales/db_view.yml`

- [ ] **Step 1: 写模型、路由与表单校验失败测试**

```rust
assert_eq!(Some(ProxyType::Socks5), config.proxy.map(|proxy| proxy.proxy_type));
assert_eq!(Some("proxy_host"), missing_proxy_required_field(true, "", 1080, "", ""));
```

- [ ] **Step 2: 运行定向测试确认 RED**

Run: `rtk cargo test -p one-core storage::models -p db ssh_tunnel -p db_view db_connection_form`

Expected: `DbConnectionConfig::proxy` 和表单代理 API 尚不存在。

- [ ] **Step 3: 增加持久化字段与数据库路由**

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub proxy: Option<ProxyConfig>,
```

无 SSH 时建立代理隧道；有 SSH 时把代理映射到 `SshConnectConfig.proxy`。

- [ ] **Step 4: 增加数据库代理页签和校验**

添加 `proxy_enabled`、`proxy_type`、`proxy_host`、`proxy_port`、`proxy_username`、`proxy_password` 字段，并在 `build_connection` 中生成 `ProxyConfig`。

- [ ] **Step 5: 更新所有配置构造并验证 GREEN**

Run: `rtk cargo test -p one-core -p db -p db_view`

Expected: 全部通过。

### Task 3: RDP/VNC 代理

**Files:**
- Modify: `crates/core/src/storage/models.rs`
- Modify: `crates/remote_desktop/src/config.rs`
- Modify: `crates/remote_desktop/src/backends/rdp.rs`
- Modify: `crates/remote_desktop/Cargo.toml`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form.rs`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form/{inputs,view}.rs`
- Modify: `crates/remote_desktop_view/locales/remote_desktop_view.yml`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `main/src/home_tab.rs`

- [ ] **Step 1: 写参数 round-trip、options 映射和 destination 改写失败测试**

```rust
assert_eq!(Some("proxy.example"), params.proxy.as_ref().map(|proxy| proxy.host.as_str()));
assert_eq!("127.0.0.1:40000", helper_destination_with_proxy(...));
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `rtk cargo test -p remote_desktop -p remote_desktop_view -p main remote_desktop`

Expected: 代理字段和映射函数不存在。

- [ ] **Step 3: 实现模型、表单和 backend 隧道生命周期**

`RemoteDesktopConnectionOptions` 携带 `Option<ProxyTunnelConfig>`；backend 启动线程时创建一次代理隧道，重连期间复用，并把 helper destination 指向本地地址。

- [ ] **Step 4: 运行测试确认 GREEN**

Run: `rtk cargo test -p remote_desktop -p remote_desktop_view -p main remote_desktop`

Expected: 全部通过。

### Task 4: 本地终端 profile

**Files:**
- Modify: `crates/core/src/settings.rs`
- Modify: `crates/terminal/src/types.rs`
- Modify: `crates/terminal/src/terminal.rs`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`

- [ ] **Step 1: 写 profile 序列化与程序/参数解析失败测试**

```rust
assert_eq!("wsl.exe", LocalConfig::from_settings(&wsl_settings, None)?.shell.unwrap());
assert_eq!(vec!["--login", "-i"], git_bash.args);
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `rtk cargo test -p one-core settings -p terminal local_shell -p main home_tabs`

Expected: profile 类型、参数字段和统一构造器不存在。

- [ ] **Step 3: 实现设置模型和安全参数解析**

```rust
pub enum LocalTerminalProfileKind {
    System,
    PowerShell,
    Cmd,
    Wsl,
    GitBash,
    Custom,
}
```

自定义参数使用确定性 shell-words 解析器转换为 `Vec<String>`，不经 `cmd /C` 或 `sh -c`。

- [ ] **Step 4: 将所有本地终端入口切换到统一配置**

`HomePage::add_terminal_tab`、SFTP 文件管理器打开本地终端、复制本地终端都保留当前工作目录，并从 `AppSettings` 读取同一 profile。

- [ ] **Step 5: 增加设置 UI 并确认 GREEN**

Run: `rtk cargo test -p one-core -p terminal -p terminal_view -p main`

Expected: 全部通过。

### Task 5: 文档、审查与完成验证

**Files:**
- Modify: `README.md`
- Modify: `README_CN.md`
- Modify: `AGENTS.md`（仅当本次发现可复用的稳定经验）

- [ ] **Step 1: 更新能力说明**

记录连接级代理支持范围和 Windows 本地终端 profile。

- [ ] **Step 2: 格式化与定向验证**

Run: `rtk cargo fmt --all -- --check`

Run: `rtk cargo test -p connection_tunnel -p ssh -p one-core -p db -p db_view -p remote_desktop -p remote_desktop_view -p terminal -p terminal_view -p main`

- [ ] **Step 3: 编译与 lint**

Run: `rtk cargo check -p main`

Run: `rtk cargo clippy -p main --all-targets -- -D warnings`

- [ ] **Step 4: 代码审查和需求逐项审计**

核对 Issue #97 两项原始需求、序列化兼容、密码脱敏、所有入口一致性和测试证据；修复所有高/中优先级问题。
