# plan2go: 同一 SSH 连接记录下远程文件协议可选 SFTP / FTP（任务 ftp-support-172）

> 本文件是实现方案（plan-only），交给实现 agent 执行。执行方式：`plan2go=/Users/hufei/RustroverProjects/navop-workspace/navop-ftp-support-172/plan2go-ftp-remote-file-protocol.md`
>
> 核心结论：**同一条连接记录可以让用户可选 SFTP 或 FTP，但底层连接必须分开，trait 必须抽出协议无关层。**

## 0. 执行环境与硬约束

- 只在原 worktree 工作：`/Users/hufei/RustroverProjects/navop-workspace/navop-ftp-support-172`（分支 `impl/ftp-support-172`）
- 不新建 worktree（此前误建的空 worktree 已确认不存在，`git worktree list` 只有主仓与本 worktree）
- 不回滚、不动 worktree 内已有的约 176 个未提交改动（含格式化污染文件）
- 不执行 workspace 级 `cargo fmt`；只格式化实际修改的文件：
  ```bash
  rustfmt --edition 2024 path/to/changed_file.rs
  ```
- 审查命令：
  ```bash
  git status --short
  git diff --stat
  git diff --check
  ```
- **禁止新增 `ConnectionType::Ftp`**，全程保持 `ConnectionType::SshSftp`

## 0.1 当前进度基线（已核实，2026-09-12）

worktree 内已有部分脚手架，属于保留不回滚的既有改动：

| 项 | 状态 |
|---|---|
| `crates/ftp/` | 已存在：`Cargo.toml` + `src/lib.rs`（约 279 行，早期 FTP client 草稿）；尚无 `listing.rs` / `transfer.rs` |
| workspace 成员 | `Cargo.toml` 已加入 `"crates/ftp"`，workspace dep `async_ftp = "6.0.0"` 已声明 |
| `RemoteFileProtocol` / `remote_file` 字段 | **未实现**（`crates/core/src/storage/models.rs` 中无此符号） |
| `RemoteFileClient` trait | **未实现**（`crates/sftp/src/lib.rs` 仍只有 `SftpClient`） |

已核实的代码锚点（行号为当前工作区状态，可能有 ± 少量偏移）：

- `pub trait SftpClient` — `crates/sftp/src/lib.rs:116`
- `pub enum SftpUploadConnection` — `crates/sftp_transfer/src/model.rs:102`（多处 `connection_source` 字段引用）
- `pub struct SshParams` — `crates/core/src/storage/models.rs:463`
- `StoredConnection::new_ssh` — `crates/core/src/storage/models.rs:2331`
- `StoredConnection::to_ssh_params` — `crates/core/src/storage/models.rs:2474`
- `CredentialResolver::resolve_connection` / `resolve_ssh` — `crates/core/src/storage/credential_vault/resolver.rs:16 / 45`
- `active_tab: usize` — `crates/terminal_view/src/ssh_form_window.rs:185`

## 0.2 目标

同一条 `ConnectionType::SshSftp` 连接记录，远程文件协议可选：

```text
SFTP
FTP / FTPS
```

SSH terminal 仍然使用 SSH。FTP 不复用 SSH socket，只复用连接记录、连接名称、工作区、同步关系和凭据引用。

---

## 一、数据模型

不要新增 `ConnectionType::Ftp`，保留 `ConnectionType::SshSftp`。原因：

- 用户看到一条连接记录，不产生重复连接
- 不新增顶层连接类型筛选项
- 不破坏已有 SSH/SFTP 连接分类
- 旧数据继续默认使用 SFTP

### 新增协议枚举

位置：`crates/core/src/storage/models.rs`

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteFileProtocol {
    #[default]
    Sftp,
    Ftp,
}

impl RemoteFileProtocol {
    pub fn is_sftp(self) -> bool { matches!(self, Self::Sftp) }
    pub fn is_ftp(self) -> bool { matches!(self, Self::Ftp) }
}
```

### FTP 参数

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FtpParams {
    #[serde(default = "default_ftp_host")]
    pub host: String,
    #[serde(default = "default_ftp_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_username: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_password: Option<bool>,
    #[serde(default = "default_true")]
    pub passive_mode: bool,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout: Option<u64>,
}

fn default_ftp_host() -> String { "127.0.0.1".to_string() }
fn default_ftp_port() -> u16 { 21 }
```

### 远程文件配置 envelope

不要直接给 `SshParams` 增加非默认字段——仓库有大量 `SshParams { ... }` 字面量，会造成大范围编译改动；且这类字段属于文件传输配置，不属于 SSH 连接本身。

增加独立 envelope：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteFileParams {
    #[serde(default)]
    pub protocol: RemoteFileProtocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp: Option<FtpParams>,
}
```

在 `SshParams` 增加唯一一个带 `#[serde(default)]` 的新字段：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub remote_file: Option<RemoteFileParams>
```

连接 JSON 顶层形态：

```json
{
  "host": "ssh.example.com",
  "port": 22,
  "username": "deploy",
  "auth_method": "...",
  "remote_file": {
    "protocol": "Ftp",
    "ftp": {
      "host": "ftp.example.com",
      "port": 21,
      "username": "deploy",
      "password": "...",
      "passive_mode": true,
      "use_tls": false
    }
  }
}
```

语义约束：

- `remote_file` 缺失 = 旧配置，协议默认 `Sftp`
- `remote_file.protocol == Ftp` 时必须有 `remote_file.ftp`
- FTP 配置缺失时返回明确错误，**不静默回退到 SFTP**

```rust
impl SshParams {
    pub fn remote_file_protocol(&self) -> RemoteFileProtocol {
        self.remote_file.as_ref().map(|p| p.protocol).unwrap_or_default()
    }
    pub fn ftp_params(&self) -> Option<&FtpParams> {
        self.remote_file.as_ref()?.ftp.as_ref()
    }
}
```

## 二、StoredConnection API

位置：`crates/core/src/storage/models.rs`

保持 `StoredConnection::new_ssh(...)`（models.rs:2331）与 `to_ssh_params(...)`（models.rs:2474）不变。

不要保留 `new_ftp` / `to_ftp_params`（不再有独立 `ConnectionType::Ftp`）。新建或编辑 FTP 模式时仍调用 `new_ssh`，`connection_type` 继续写 `ConnectionType::SshSftp`。

新增：

```rust
impl StoredConnection {
    pub fn remote_file_protocol(&self) -> Result<RemoteFileProtocol, serde_json::Error> {
        Ok(self.to_ssh_params()?.remote_file_protocol())
    }

    pub fn remote_file_params(&self) -> Result<RemoteFileParams, serde_json::Error> {
        Ok(self.to_ssh_params()?.remote_file.unwrap_or_default())
    }
}
```

## 三、凭据解析

修改：

- `crates/core/src/storage/credential_vault/resolver.rs`
- `crates/core/src/storage/credential_vault/reference_scanner.rs`

`resolve_connection()`（resolver.rs:16）对 `ConnectionType::SshSftp` 的流程：

1. 解析 SSH 主凭据
2. 读取 `SshParams.remote_file`
3. 协议为 `Sftp` → 完成
4. 协议为 `Ftp` → 解析 `FtpParams.credential_reference`
5. 将解析后的 FTP 用户名密码写入运行时副本
6. 不把明文密码重新持久化

建议入口（或在现有 `resolve_ssh`（resolver.rs:45）内完成）：

```rust
pub fn resolve_ssh_with_remote_file(&self, params: SshParams) -> Result<SshParams>
```

约束：

- 持久化参数保留 credential reference
- 运行时参数可含明文密码，但禁止写回数据库
- FTP 不参与 SSH jump server 凭据路径
- FTP 不生成 SSH host key 记录
- FTP 配置必须进入 credential scan 与 credential delete protection（见第十二节）

## 四、Trait 拆分

当前问题：`crates/sftp/src/lib.rs:116` 的

```rust
pub trait SftpClient: Send + Sync {
    async fn connect(ssh_config: SshConnectConfig) -> Result<Self>;
    ...
}
```

FTP 无法实现这个 `connect`。拆分：

```rust
#[async_trait]
pub trait RemoteFileClient: Send + Sync {
    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>>;
    async fn stat(&mut self, path: &str) -> Result<Option<PathMetadata>>;
    async fn download_with_progress(&mut self, remote_path: &str, local_path: &str,
        cancelled: Arc<AtomicBool>, progress: ProgressCallback) -> Result<()>;
    async fn upload_with_progress(&mut self, local_path: &str, remote_path: &str,
        cancelled: Arc<AtomicBool>, progress: ProgressCallback) -> Result<()>;
    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()>;
    async fn delete_recursive(&mut self, path: &str,
        cancelled: Arc<AtomicBool>, progress: ProgressCallback) -> Result<()>;
    async fn mkdir(&mut self, path: &str) -> Result<()>;
    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()>;
    async fn chmod(&mut self, path: &str, mode: u32) -> Result<()>;
    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>>;
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()>;
    async fn list_dir_recursive(&mut self, path: &str, cancelled: Arc<AtomicBool>)
        -> Result<Vec<FileEntry>>;
    async fn download_dir_with_progress(&mut self, remote_path: &str, local_path: &str,
        cancelled: Arc<AtomicBool>, progress: ProgressCallback) -> Result<()>;
    async fn upload_dir_with_progress(&mut self, local_path: &str, remote_path: &str,
        conflict_policy: DirectoryConflictPolicy, cancelled: Arc<AtomicBool>,
        progress: ProgressCallback) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
    async fn realpath(&mut self, path: &str) -> Result<String>;
}

#[async_trait]
pub trait SftpClient: RemoteFileClient {
    async fn connect(ssh_config: SshConnectConfig) -> Result<Self>
    where
        Self: Sized;
}
```

改动要点：

- `impl SftpClient for RusshSftpClient` 改为 `impl RemoteFileClient for RusshSftpClient`，并保留 `impl SftpClient for RusshSftpClient`
- 现有泛型函数若只需要文件操作，约束从 `C: SftpClient + ?Sized` 改为 `C: RemoteFileClient + ?Sized`
- SSH 专属能力继续使用 `RusshSftpClient`：host key、SSH server copy、SSH terminal、SSH jump server、SSH session manager

## 五、FTP crate

目录（`crates/ftp/` 已存在，按此补全）：

```text
crates/ftp/
├── Cargo.toml          # 已存在，async_ftp = { workspace = true }
└── src/
    ├── lib.rs          # 已有约 279 行草稿，整理为 FtpConnectConfig / FtpClient
    ├── listing.rs      # LIST 解析
    └── transfer.rs     # 上传/下载/递归
```

依赖：`async_ftp = "6.0.0"`（workspace 已声明）。crate 名就是 `async_ftp`，不要写成 `async-ftp`。

```rust
pub struct FtpConnectConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub passive_mode: bool,
    pub use_tls: bool,
    pub connect_timeout: Option<u64>,
}

pub struct FtpClient { stream: FtpStream }
```

- 禁止 `impl SftpClient for FtpClient`
- 正确：`impl RemoteFileClient for FtpClient`

### 首版必须支持

binary transfer、passive mode、login、list、stat、read file、write file、upload file、download file、mkdir、rename、delete file、recursive delete、recursive upload、recursive download、disconnect、realpath、cancellation。

### 首版限制

- 主动模式：未实现就不在 UI 显示
- `chmod`：返回明确错误，UI 隐藏该按钮
- SSH jump server / SSH host-key：不适用于 FTP
- server-to-server copy：FTP 首版禁用
- FTP 目录操作不能挂 UI 后再返回 `not implemented`

### 文件安全

下载：写同目录临时文件 → 完成并 flush → rename 到目标 → 失败删除临时文件。
上传：上传到远程临时路径 → 成功后远程 rename → 失败清理临时文件 → 不直接截断已有文件。

### LIST 解析（listing.rs）

不要假设所有服务器格式相同，至少支持：

```text
-rw-r--r-- 1 user group 123 Jan 01 12:00 file.txt
drwxr-xr-x 1 user group 0 Jan 01 12:00 directory
```

解析失败策略：跳过无法解析行、记 debug 日志、不让整个目录列表失败。
文件名中的空格必须保留。

`FileEntry` 的 `permissions/uid/gid/user/group`：若改成 `Option` 影响面过大，则保留旧字段，但 FTP 填 `0`/`None`，UI 不展示伪造权限。

## 六、传输队列

位置：

- `crates/sftp_transfer/src/model.rs`（`SftpUploadConnection` 在 :102）
- `crates/sftp_transfer/src/provider.rs`

将 `SftpUploadConnection` 改为：

```rust
pub enum RemoteFileConnection {
    SshSessionManager(Arc<SshSessionManager>),
    SshConfig(SshConnectConfig),
    Ftp(FtpConnectConfig),
}
```

若为兼容暂时保留 `SftpUploadConnection` 名称，也必须新增 FTP 变体并逐步改名。

provider 路由：

```rust
match source {
    RemoteFileConnection::SshSessionManager(manager) => {
        let client = RusshSftpClient::connect_with_client(manager.client().await?).await?;
    }
    RemoteFileConnection::SshConfig(config) => {
        let client = RusshSftpClient::connect(config).await?;
    }
    RemoteFileConnection::Ftp(config) => {
        let client = FtpClient::connect(config).await?;
    }
}
```

provider 内部统一接收 `&mut dyn RemoteFileClient`。任务 key 建议从 `sftp-*` 改为：

```text
remote-file-upload
remote-file-download
remote-file-delete
```

## 七、SFTP View

重点文件：

- `crates/sftp_view/src/endpoint.rs`
- `crates/sftp_view/src/left_remote.rs`
- `crates/sftp_view/src/left_remote_state.rs`
- `crates/sftp_view/src/context_menu_handler.rs`
- `crates/sftp_view/src/file_clipboard.rs`
- `crates/sftp_view/src/lib.rs`

这些位置大量硬编码 `Arc<Mutex<RusshSftpClient>>`，改为：

```rust
Arc<Mutex<Box<dyn RemoteFileClient>>>
```

或项目内别名：

```rust
pub type SharedRemoteFileClient = Arc<tokio::sync::Mutex<Box<dyn RemoteFileClient>>>;
```

统一连接创建函数：

```rust
pub async fn connect_remote_file_client(
    connection: &StoredConnection,
) -> Result<Box<dyn RemoteFileClient>>
```

逻辑：

```rust
let ssh = connection.to_ssh_params()?;
match ssh.remote_file_protocol() {
    RemoteFileProtocol::Sftp => Ok(Box::new(
        RusshSftpClient::connect(build_ssh_config(&ssh)?).await?)),
    RemoteFileProtocol::Ftp => {
        let ftp = ssh.ftp_params().ok_or_else(/* 明确错误，不回退 SFTP */)?;
        Ok(Box::new(FtpClient::connect(ftp_config_from_params(ftp)).await?))
    }
}
```

`endpoint.rs`：

- 仍只过滤 `ConnectionType::SshSftp`，不新增 FTP 类型过滤
- `connection_title()` 按协议读取 host：FTP 读 FTP host，SFTP 读 SSH host

FTP 模式禁用：Open SSH terminal、Open host-key prompt、server-to-server copy、SSH-specific remote commands、chmod 按钮。

## 八、Remote File Editor

重点文件：

- `crates/remote_file_editor/src/external_editor.rs`
- `crates/remote_file_editor/src/open_routing.rs`
- `crates/remote_file_editor/src/editor_window.rs`
- `crates/remote_file_editor/src/external_edit_controller.rs`

所有 `Arc<Mutex<RusshSftpClient>>` 改为协议无关 trait object。编辑器只需要 `read_file` / `write_file` / `realpath` / `stat`，不得依赖 SSH host key、SSH session、SSH terminal、SFTP server copy。

## 九、Remote Image Preview

`crates/remote_image_preview/src/lib.rs`：只依赖 `RemoteFileClient::read_file`，FTP 和 SFTP 共用。

## 十、SSH Form UI

重点：`crates/terminal_view/src/ssh_form_window.rs`（`active_tab: usize` 在 :185）。

在“基本信息”或“SFTP”区顶部增加远程文件协议选择：

```text
远程文件协议
[SFTP] [FTP / FTPS]
```

选择 FTP 后展示：FTP Host、FTP Port、FTP Username、FTP Password、Credential Reference、Passive Mode、FTPS。
选择 SFTP 后展示：SFTP default directory、独立 SFTP account、SSH 相关文件选项。

同一表单保存仍调用 `StoredConnection::new_ssh(...)`，不要创建 `new_ftp`。

新增字段：

```text
remote_file_protocol_select
ftp_host_input
ftp_port_input
ftp_username_input
ftp_password_input
ftp_passive_mode
ftp_use_tls
ftp_credential_picker
```

保存时：

```rust
let remote_file = match selected_protocol {
    RemoteFileProtocol::Sftp => RemoteFileParams {
        protocol: RemoteFileProtocol::Sftp,
        ftp: None,
    },
    RemoteFileProtocol::Ftp => RemoteFileParams {
        protocol: RemoteFileProtocol::Ftp,
        ftp: Some(FtpParams { /* ... */ }),
    },
};
```

校验规则：

- SSH host/port/username：SSH terminal 仍需要
- FTP host/port/username：FTP 文件操作需要
- FTP password 可为空，仅当 credential reference 或 prompt 开启时允许
- FTP port 默认 21
- SFTP 默认保持旧行为

## 十一、连接打开规则

- `main/src/home/home_strategy.rs`：`ConnectionType::SshSftp` 仍使用 SSH open strategy；双击连接继续打开 SSH terminal；FTP 只是文件协议，不改变连接主类型
- `main/src/home/home_tabs.rs`：`open_sftp_view` 可保留旧名或改为 `open_remote_file_view`；内部创建 view 时由 view 根据协议创建 client
- 右键菜单文案：`打开远程文件`；不让 FTP 连接出现独立顶层菜单

## 十二、过滤、同步、分享

禁止新增 `ConnectionType::Ftp`。以下路径保持只识别 `ConnectionType::SshSftp`，需要文件协议语义处调用 `connection.remote_file_protocol()`：

```text
main/src/connection_visuals.rs
main/src/home_tab/connection_filter.rs
main/src/home_tab/connection_info.rs
main/src/home_tab/connection_details.rs
main/src/persistent_connection_sidebar/connection_copy.rs
main/src/persistent_connection_sidebar/connection_share.rs
main/src/home/connection_import_draft_conversion.rs
main/src/personal_sync_runtime.rs
crates/core/src/storage/credential_vault/resolver.rs
crates/core/src/storage/credential_vault/reference_scanner.rs
```

FTP 配置必须进入：cloud sync params、copy、share、import/export、credential scan、credential delete protection。

## 十三、国际化

**注意（已修正）**：本仓库 locale 是单文件内联三语格式，不是 en.yml/zh-CN.yml/zh-HK.yml 三个文件。实际文件：

```text
crates/terminal_view/locales/terminal_view.yml
```

格式为每个 key 下 `en:` / `zh-CN:` / `zh-HK:` 三个值（文件顶部 `_version: 2`）。按此格式新增（示例，落在 terminal_view.yml 对应分组内）：

```yaml
  remote_file_protocol:
    en: Remote file protocol
    zh-CN: 远程文件协议
    zh-HK: 遠端檔案協定
  ftp:
    en: FTP / FTPS
    zh-CN: FTP / FTPS
    zh-HK: FTP / FTPS
  ftp_host:
    en: FTP host
    zh-CN: FTP 主机
    zh-HK: FTP 主機
  ftp_port:
    en: FTP port
    zh-CN: FTP 端口
    zh-HK: FTP 埠
  ftp_username:
    en: FTP username
    zh-CN: FTP 用户名
    zh-HK: FTP 使用者名稱
  ftp_password:
    en: FTP password
    zh-CN: FTP 密码
    zh-HK: FTP 密碼
  ftp_passive_mode:
    en: Passive mode
    zh-CN: 被动模式
    zh-HK: 被動模式
  ftp_tls:
    en: Explicit TLS
    zh-CN: 显式 TLS
    zh-HK: 顯式 TLS
  ftp_not_supported:
    en: This FTP operation is not supported
    zh-CN: FTP 不支持此操作
    zh-HK: FTP 不支援此操作
```

若 sftp_view 等其他 crate 也有自己的 `locales/*.yml`，其中新增的 UI 文案同样按此内联三语格式补齐。

## 十四、测试要求

模型（`one-core`）：

```rust
#[test] fn old_ssh_params_default_to_sftp() {}
#[test] fn ftp_remote_file_params_roundtrip() {}
#[test] fn ftp_protocol_requires_ftp_params() {}
```

凭据解析（resolver）：

```rust
#[test] fn ftp_credential_reference_resolves_username_and_password() {}
#[test] fn old_ssh_json_defaults_to_sftp() {}
#[test] fn ftp_remote_file_params_roundtrip() {}
```

FTP LIST parser（`crates/ftp`）：

```rust
#[test] fn parses_unix_file_listing() {}
#[test] fn parses_unix_directory_listing() {}
#[test] fn skips_invalid_listing_lines() {}
#[test] fn preserves_spaces_in_file_names() {}
```

传输（`sftp_transfer`）：

```rust
#[test] fn transfer_provider_routes_ftp_source_to_ftp_client() {}
```

UI：

```rust
#[test] fn ssh_form_saves_ftp_protocol_inside_same_connection_type() {}
```

验证命令：

```bash
cargo fmt --check   # 注意：仅对本次修改文件运行 rustfmt，见第 0 节硬约束
cargo check -p one-core
cargo check -p ftp
cargo check -p sftp_transfer
cargo check -p sftp_view
cargo check -p remote_file_editor
cargo check -p main
cargo test -p one-core --lib
cargo test -p ftp
cargo test -p sftp_transfer --lib
```

## 十五、推荐实施顺序

1. 数据模型：`RemoteFileProtocol` / `FtpParams` / `RemoteFileParams` / `SshParams.remote_file` + 单测（第一、二节）
2. Trait 拆分：`RemoteFileClient` + `RusshSftpClient` 改 impl（第四节），跑 `cargo check -p sftp -p sftp_view -p remote_file_editor`
3. 凭据解析：resolver / reference_scanner + 单测（第三节）
4. FTP crate 补全：listing.rs / transfer.rs + parser 单测（第五节）
5. 传输队列：`RemoteFileConnection` 路由（第六节）
6. UI：sftp_view / remote_file_editor / remote_image_preview 去 `RusshSftpClient` 硬编码（第七、八、九节）
7. SSH Form UI + 打开规则 + i18n（第十、十一、十三节）
8. 过滤/同步/分享全链路核查（第十二节）
9. 全量验证命令 + `git diff --check` 收尾（第十四节）

每步完成后运行对应 `cargo check`，发现影响面超出预期立即停下上报，不擅自扩大改动。
