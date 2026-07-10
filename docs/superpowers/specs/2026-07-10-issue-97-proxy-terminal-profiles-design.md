# Issue #97 连接代理与本地终端 Profile 设计

## 目标

完整实现 GitHub Issue #97：数据库连接与 RDP/VNC 连接支持每连接代理；本地终端支持系统默认、PowerShell、CMD、WSL、Git Bash 以及自定义程序和参数。

## 范围

- 网络数据库连接支持 SOCKS5 与 HTTP CONNECT 代理，可配置用户名和密码。
- RDP 与 VNC 连接支持同样的代理配置。
- 数据库同时启用 SSH 隧道和代理时，代理用于连接 SSH 服务器，再由 SSH 隧道访问数据库目标。
- SQLite、DuckDB 等文件型数据库不显示代理入口，也不创建网络隧道。
- 本地终端默认 profile 是全局设置；新建、复制以及从文件管理器打开的本地终端都使用该设置。
- 内置终端 profile：系统默认、PowerShell、CMD、WSL、Git Bash、自定义。
- 自定义 profile 支持可执行程序路径和 shell 风格参数字符串。

## 方案选择

采用应用内本地 TCP 转发边界。代理转发器在 `127.0.0.1:0` 建立临时监听端口，通过 SOCKS5 或 HTTP CONNECT 连接真实目标，并双向转发字节。数据库驱动和远程桌面 provider 只看到本地 TCP 目标。

该方案避免为 MySQL、PostgreSQL、MSSQL、Oracle、ClickHouse 和外部驱动分别实现不同 connector，也不需要升级远程桌面 helper 协议。现有 SSH 连接代理结构和认证语义继续复用。

## 数据模型

- `one_core::storage::ProxyConfig` 继续作为连接级代理的唯一持久化模型。
- `DbConnectionConfig` 新增可选 `proxy` 字段；旧配置通过 `#[serde(default)]` 无损迁移。
- `RemoteDesktopParams` 新增可选 `proxy` 字段；旧配置同样兼容。
- `AppSettings` 新增结构化本地终端 profile 设置，包含 profile 类型、自定义程序和自定义参数。
- 代理密码沿用现有递归敏感字段加密机制；日志和 `Debug` 输出不得泄露密码。

## 运行时

### 代理隧道

`connection_tunnel` 提供通用 `ProxyTunnel` 与 `TunnelGuard`：

- SOCKS5 使用现有 `ssh::connect_via_proxy` 建连能力。
- HTTP CONNECT 完整读取响应头并只接受 2xx 成功状态。
- 独立后台 Tokio runtime 持有监听器和连接转发任务，使数据库异步运行时与远程桌面同步后台线程都能复用。
- guard 被释放时停止接受新连接；已有连接随 session 关闭。

### 数据库

- 无 SSH 隧道：代理隧道直接连接数据库目标。
- 有 SSH 隧道：代理映射为 `SshConnectConfig.proxy`，SSH 本地端口转发保持现有行为。
- 所有调用 `db::ssh_tunnel::resolve_connection_target` 的内置和外部数据库驱动自动获得代理能力。

### 远程桌面

`RemoteDesktopConnectionOptions` 携带可选代理。在 helper 启动前建立代理隧道，并将 helper 的 destination 替换为本地临时地址。重连复用同一代理隧道；隧道建立失败通过现有 `ConnectionFailure` 路径呈现。

### 本地终端

`LocalConfig` 增加参数列表，并提供从 `AppSettings` 构造配置的统一入口：

- System：保持当前平台默认探测。
- PowerShell：优先 `pwsh.exe`，回退 Windows PowerShell。
- CMD：优先 `COMSPEC`，回退 `cmd.exe`。
- WSL：使用 `wsl.exe`。
- Git Bash：优先 PATH 中的 `bash.exe`，再检查标准 Git for Windows 安装路径，并附加 `--login -i`。
- Custom：校验程序非空，按 shell 风格解析参数，不通过 shell 拼接执行。

## UI

- 数据库连接表单增加“代理”页签，字段为启用、类型、主机、端口、用户名、密码。
- RDP/VNC 表单在基础字段后增加同样的可折叠代理区域。
- 常规设置增加“本地终端”分组：默认终端下拉框；选择自定义时显示程序和参数输入。
- 代理启用时校验 host、port，以及“有密码必须有用户名”。
- UI 复用现有 `Checkbox`、`Radio`、`Input`、`Select`、`SettingField` 组件与多语言结构。

## 测试与验收

- 代理隧道：SOCKS5/HTTP 配置映射、目标选择、guard 生命周期和错误路径单元测试。
- 数据库：序列化兼容、表单构建/校验、直连代理与 SSH+代理路由测试。
- 远程桌面：参数 round-trip、表单构建、options 映射、helper destination 改写测试。
- 终端：profile 序列化、程序/参数解析、Windows 内置 profile 解析和所有本地终端入口使用统一配置的结构测试。
- 运行相关 crate 的测试、`cargo fmt --check`、`cargo check` 和 `cargo clippy -- -D warnings`；无法在当前 macOS 主机真实启动 WSL/Git Bash 时，以纯解析测试和 Windows 条件编译检查作为自动化证据。

