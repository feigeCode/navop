# 跳板机连接体系优化（ProxyJump 统一与堡垒机直连）

> 文档状态：Proposed Design
>
> 目标读者：维护 `crates/ssh` / `crates/sftp` / `crates/terminal` / `crates/ssh_form` /
> `crates/connection_tunnel` 的开发 Agent
>
> 编写日期：2026-11-19

---

## 1. 问题

用户以 JumpServer（Koko）作跳板机连接资产时，得到的是一句无法行动的报错
`Failed to open channel (AdministrativelyProhibited)`。顺着这条报错把整个跳板机链路勘察一遍后，
发现它是**体系性问题**的一个切面，共七项：

1. **错误黑盒**：跳板拒绝 `direct-tcpip` 通道时，russh 在
   `client/encrypted.rs:558-585` 已解出服务端的 `description`（如 JumpServer 的
   `port forwarding is disabled`），但只把原因码枚举下发，description 被丢弃。
   Navop 下游只能靠字符串匹配兜底（`crates/terminal/src/ssh_backend.rs:153-230`），
   没有任何一条提示「堡垒机策略禁用端口转发」这一最常见成因。
2. **SFTP 跳板是一套重复且退化的实现**：`crates/sftp/src/russh_impl.rs:2357-2440`
   重复了 `crates/ssh/src/ssh.rs:2623-2723` 约 80 行跳板连接逻辑；且认证用的是
   `authenticate_with_strategy`（`crates/ssh/src/ssh.rs:1460-1478`，内部转发为
   `KeyboardInteractiveTarget::TargetServer` + responder 恒 None）→
   **SFTP 走跳板时无 MFA/OTP 能力**，与终端路径行为不一致。
3. **五处独立的 `JumpServerConnectConfig` 映射**：
   `crates/sftp_transfer/src/ssh_config.rs:66`、`crates/terminal/src/terminal.rs:740`、
   `crates/runtime_sftp/src/sftp_tools.rs:330`、`crates/port_forwarding/src/runtime.rs:250`、
   `crates/ssh_form/src/ssh_form_window.rs:1919`。
   键盘交互 responder 只有 ssh_form（测试连接）和 terminal 两条路传入，其余全是 None。
4. **跨子系统零会话复用**：终端每个连接独立建 `SshSessionManager`
   （`crates/terminal/src/terminal.rs:3242`，仅同连接多 tab 共享）；SFTP 工具、
   端口转发（`crates/ssh/src/ssh.rs:2481`）、SOCKS、DB 隧道每次调用都新建完整
   TCP+认证链（含跳板 MFA 重答）。而仓库里**已建好未接线**的应用级共享层：
   `crates/ssh/src/session_registry.rs`（`SshSessionService`，ConnectionKey→单飞 manager、
   租约、60s idle 回收，由 `crates/shell_domain/src/lifecycle.rs:12` 挂进 GPUI global）
   与 `crates/terminal/src/ssh_session_identity.rs`（`PersistedSshSessionIdentity::derive`）
   全库无任何 acquire 调用者；`crates/ssh/src/connection_key.rs` 的 `JumpKey`
   本就是为跨连接共享跳板传输设计的。
5. **堡垒机（直连型）形态不支持**：JumpServer/Koko 默认拒绝 direct-tcpip，
   只支持「用户名指定资产」的直连格式（`user#asset#account@堡垒机:2222`，
   终端走 proxyAssetCommand、SFTP 走 DirectConnectSftpMode；2026-11-19 已在
   10.2.5.220:2222 实测验证）。Navop 的跳板机只有 ProxyJump 语义，且 UI 没有模式区分，
   用户只能靠手填 `user#asset#account` 用户名绕过。
6. **引用 SSH 连接的隧道丢失跳板**：`crates/core/src/storage/models.rs`
   `apply_referenced_ssh_tunnel` 三份重复实现（:1056/:1155/:1280，Redis/MongoDB/Db）
   只回填 host/port/username/auth/timeout，jump_server 与 proxy 全部丢弃；
   `crates/redis-runtime/src/builtin.rs:469`、mongodb builtin 更是 `jump_server: None` 硬编码。
7. **模型能力上限**：`JumpServerConfig`（`crates/core/src/storage/models.rs:812-819`）
   是单跳模型：无链式多跳、无每跳独立 proxy/超时/MFA；`SshConnectConfig` 里
   proxy 与 MFA responder 是全局的（跳板和目标两段共用）。

### 现状地图

| 连接路径 | 跳板实现 | 跳板 MFA | 会话复用 |
| --- | --- | --- | --- |
| 终端（`ssh.rs::connect` 跳板分支） | ssh.rs:2623-2723 | ✅ TerminalMfaResponder | 单连接内（SessionPool） |
| 表单「测试连接」 | 同上 | ✅ JumpServerMfaResponder | 无 |
| SFTP 客户端（`russh_impl.rs`） | :2357-2440 重复实现 | ❌ | 无 |
| SFTP 工具（`runtime_sftp`） | jump_config :330 | ❌ | 无 |
| 端口转发（`port_forwarding`） | runtime.rs:250 | ❌ | 无 |
| 本地转发 / SOCKS（ssh.rs:2481、dynamic_socks） | 每次 connect 重建 | ❌ | 无 |
| DB / Redis / Mongo 隧道 | 引用 SSH 连接但丢 jump | — | 无 |

## 2. 目标与非目标

**目标**

- G1 错误可读可行动：direct-tcpip 被拒时透出服务端 description，并给出
  「该堡垒机禁用端口转发，建议改用堡垒机直连模式」的专门提示。
- G2 SFTP 走跳板支持 MFA，与终端路径行为一致。
- G3 跳板连接构造与实现收敛到唯一入口，五处映射、两套实现合一。
- G4 堡垒机（直连型）成为一等公民连接模式：资产/账号字段化，终端/SFTP/
  exec 全部可用，不再依赖手拼 `user#asset#account`。
- G5 引用 SSH 连接的隧道继承完整上游语义（jump/proxy/MFA）。
- G6 跳板传输跨连接复用（接线现有 SshSessionService / ConnectionKey 体系）。

**非目标**

- 本期不做链式多跳（数据模型预留字段即可）。
- 不改 russh 公共 API 语义；fork 只做增量（新增携带信息，不改已有签名含义）。

## 3. 设计

### 3.1 错误链修复（F1）：把 description 送达 UI

两步走，先零 fork 改动，后 fork 收尾：

- **方案 B（Navop 侧，先行）**：russh 的 Handler trait 已有钩子
  `channel_open_failure(channel, reason, description, language, session)`
  （fork `client/mod.rs:2903`，默认 no-op）。给 Navop 的 client Handler 增加
  `last_open_failures: Arc<Mutex<Vec<(ChannelId, ChannelOpenFailure, String)>>>`，
  在钩子里记录；`channel_open_direct_tcpip` 返回
  `Err(Error::ChannelOpenFailure(reason))` 后（ssh.rs:2680、russh_impl.rs:2405、
  ssh.rs:2422 三个调用点）按 channel id 查记录，把 description 拼进 anyhow context。
- **方案 C（fork 增量，后续）**：`Error::ChannelOpenFailure` 增加
  `description: Option<String>`（或新增变体），`wait_channel_confirmation`
  从 handler 缓存补齐。落地后退回 Navop 的 slot 方案。
- **分类提示**：`crates/terminal/src/ssh_backend.rs` 的
  `add_connect_error_context` / `is_channel_open_failure` 增加
  AdministrativelyProhibited + 跳板场景分支：提示「跳板机/堡垒机拒绝了端口转发通道
  （{description}）。JumpServer 等堡垒机默认禁用 direct-tcpip，请在连接设置的
  跳板机页改用『堡垒机直连』模式，或联系管理员」。ssh_form 的
  `format_connection_error`（:527）同步受益。

### 3.2 统一跳板传输（U1/U2）：一个入口、一套实现

从 `RusshClient::connect` 里抽出可复用的跳板传输层：

```rust
// crates/ssh（终端/SFTP/端口转发共用）
pub struct JumpConnectOptions<'a> {
    pub jump: &'a JumpServerConnectConfig,
    pub proxy: Option<&'a ProxyConfig>,
    pub mfa: Option<&'a dyn KeyboardInteractiveResponder>, // 作用于跳板段
    pub timeouts: ConnectTimeouts,
}

/// 一条已认证的跳板传输（含路由身份，供将来的跨连接复用）
pub struct JumpTransport { /* session handle + JumpRoute */ }

impl JumpTransport {
    /// 对目标 host:port 开 direct-tcpip 通道（每目标一条）
    pub async fn open_channel_to(&self, host: &str, port: u16) -> Result<Channel<Msg>>;
}

pub async fn open_jump_transport(opts: JumpConnectOptions<'_>) -> Result<JumpTransport>;
```

- `ssh.rs::connect` 跳板分支改为 `open_jump_transport` + `open_channel_to` +
  在通道上 `connect_stream` 建目标 SSH；
- `crates/sftp/src/russh_impl.rs:2357-2440` 删除重复实现，改调同一入口；
- MFA：认证统一走 `authenticate_with_strategy_for_target(...,
  KeyboardInteractiveTarget::JumpServer, opts.mfa)`；`ssh.rs:1460` 的无 target
  变体收敛（改签名带 target 或直接删掉，编译期逼出所有调用点）；
- 五处 `JumpServerConnectConfig` 映射收敛为一个 builder（放 `crates/ssh`，
  输入 core 的 `JumpServerConfig` + 可选 responder/proxy），SFTP 工具、
  端口转发、终端、表单测试全部改走它——MFA 从「散落的 None」变成显式参数。

### 3.3 堡垒机直连模式（B1）：解决 JumpServer 形态

数据模型（core）：

```rust
pub enum JumpMode {
    /// 现有语义：跳板上开 direct-tcpip 通道连目标（默认，向后兼容）
    ProxyJump,
    /// 直连型堡垒机：目标会话就是堡垒机会话，用户名按模板渲染
    BastionDirect,
}

pub struct JumpServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    pub credential_reference: Option<CredentialReference>,
    pub mode: JumpMode,                 // 新增，缺省 ProxyJump（旧数据无感）
    pub bastion: Option<BastionTarget>, // BastionDirect 时生效
}

pub struct BastionTarget {
    pub asset: String,        // 资产 IP / 名称 / UUID
    pub account: String,      // 资产账号（如 root）
    pub user_format: BastionUserFormat, // 预设：JumpServer 的 user#account#asset 等
}
```

连接流（BastionDirect）：

- **不开 direct-tcpip**；目标会话 = 堡垒机 SSH 会话本身；
- username 按 `user_format` 渲染（JumpServer：`{user}#{account}#{asset}`）；
- host key 沿用 `jump_host_key_identity`（HostKeyRoute::Jump）；
- 终端（PTY）、exec 通道、SFTP 子系统直接跑在堡垒机会话上——
  Koko 对直连格式会代理到资产（exec 走 proxyAssetCommand，SFTP 走
  DirectConnectSftpMode，2026-11-19 已实测），三层全部可用；
- `RusshClient` 需要区分「会话即堡垒机」与「会话在通道上」两种形态
  （`_jump_session` 在 BastionDirect 下为 None，断线重连、保活逻辑走同一会话）。

UI（ssh_form 跳板机 tab）：

- 模式单选：「SSH 跳板（ProxyJump）」/「堡垒机直连（JumpServer 等）」；
- BastionDirect 下显示资产/账号/用户名格式字段，隐藏「目标主机经跳板转发」相关说明；
- 存量行为不变：未选模式 = ProxyJump，`disabled_jump_server` 保留逻辑照旧。

检测辅助（可选，P2）：连跳板成功但 direct-tcpip 被拒、且 banner 含
`SSH-2.0-JumpServer`（或 description 含 `port forwarding is disabled`）时，
在错误提示里给出「一键切换为堡垒机直连模式」。

### 3.4 会话复用接线（R1）：让已建好的共享层生效

- 终端建连：`crates/terminal/src/terminal.rs:3242` 从
  `Arc::new(SshSessionManager::new(...))` 改为经
  `GlobalSshSessionService` 以 `PersistedSshSessionIdentity::derive(connection, config)`
  acquire（租约即生命周期，`transition_to` 的 ReuseLease/ReplaceLease 分支生效）；
- 跳板传输复用：`JumpTransport` 按 `ConnectionKey::JumpKey` 缓存进
  `SshSessionRegistry`——同一跳板 host+凭据修订 下，多个目标连接共享一条
  跳板传输（每目标仍各开 direct-tcpip 通道），跳板 MFA 只答一次；
- 端口转发/DB 隧道按需接入（先观察终端接线稳定性）。

### 3.5 隧道继承（T1）：引用 SSH 连接不再丢语义

- `crates/connection_tunnel` 的 `SshTunnelConfig` 增加 `jump`/`proxy` 字段
  （或改为直接持有 `connection_id`，运行时经 `resolve_runtime_connection` 取全量）；
- `apply_referenced_ssh_tunnel` 三份重复实现合并为一个共享函数，回填 jump/proxy；
- redis/mongodb builtin 去掉 `jump_server: None` 硬编码，跟随引用连接。

## 4. 分阶段实施

| 阶段 | 内容 | 主要改动点 | 规模/风险 |
| --- | --- | --- | --- |
| **P0 快赢** | F1 错误透传 + 跳板拒绝专用提示；F2 SFTP 跳板 MFA（改用 `_for_target` + responder 传通） | russh Handler 钩子实现、ssh.rs 三个 direct-tcpip 调用点、ssh_backend.rs 分类、russh_impl.rs 认证行 | 小；行为只增不改，风险低 |
| **P1 堡垒机直连** | B1 模型 + 连接流 + 表单模式 | models.rs、ssh.rs（会话形态分支）、ssh_form（tab + i18n 三语）、迁移（mode 缺省） | 中；新路径，老路径不动 |
| **P2 统一收敛** | U1/U2 `JumpTransport` 唯一入口、五处映射合一、删 SFTP 重复实现 | ssh.rs、russh_impl.rs、五个构造点 | 中；纯重构，靠测试兜底 |
| **P3 复用接线** | R1 registry acquire + 跳板传输共享；T1 隧道继承 | terminal.rs、session_registry/identity、connection_tunnel、models.rs | 中大；生命周期交互需小心 |
| **P4 长尾** | fork 方案 C（Error 携带 description）、堡垒机检测一键切换、链式多跳预留、ssh_exec 空结果 UX | russh fork、模型预留 | 按需 |

依赖关系：P0 独立可做；P1 依赖 P0 的错误提示（切换建议文案）；P2 与 P1 可并行；
P3 依赖 P2（`JumpTransport` 稳定后才能按 JumpKey 缓存）。

## 5. 验证

- 每阶段：`cargo test -p ssh -p sftp -p terminal -p ssh_form` +
  `cargo clippy -- --deny warnings`（按 AGENTS.md，TDD：先补 Red 测试再实现）；
- F1：单测覆盖 Handler slot 记录与错误上下文拼接（AdministrativelyProhibited +
  description 透传）；利用 `ssh_backend.rs:2555` 附近 MockClient 测试模式补
  分类提示断言；
- F2：单测断言 SFTP 跳板认证收到 `KeyboardInteractiveTarget::JumpServer` 与
  非 None responder；
- B1：以 10.2.5.220:2222 实测（用户名 `hufei#root#{asset-uuid}`，JumpServer
  账号密码），验证终端、SFTP 浏览/传输、`ssh_exec` 三层；
- R1：多终端连接同一跳板下多个资产，观察跳板认证次数与 idle 回收后重连。

## 6. 开放问题

- BastionDirect 的用户名格式预设集（JumpServer `user#account#asset` 之外，
  Teleport `user@asset`、齐治等）先只做 JumpServer，格式字段留扩展位；
- 跳板传输共享时，JumpServer 会话并发 direct-tcpip 上限（MaxSessions）与
  通道排队策略需要在 P3 实测定参。
