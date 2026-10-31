# Navop 三产品拆分设计（navop / navopShell / navopData）

状态：设计稿（待评审）  
日期：2026-10-28  
范围：编译期、包级彻底拆分；本文只定架构与迁移顺序，不含实现代码。

## 1. 结论

采用 **“共享应用壳 + 领域包 + 三个薄 bin 包”**：

- `crates/app_kit`：窗口、主页、侧栏、连接管理、设置容器、账号/同步、AI、扩展市场、更新、托盘等**领域无关**能力。
- `crates/domain_api`：领域与外壳之间唯一的契约层（注册表、Opener、Form、MCP Provider、文件处理、设置页）。
- `crates/data_domain`：数据库 / Redis / MongoDB / SQL 编辑器 / ER 图 / DB 驱动扩展。
- `crates/shell_domain`：SSH / SFTP / FTP / 本地终端 / 端口转发 / RDP / VNC / known_hosts / 会话日志。
- `apps/navop`：全量，链接 data + shell。
- `apps/navop-shell`：只链接 shell。
- `apps/navop-data`：只链接 data。

领域选择靠 **Cargo 依赖**，不靠 `cfg(feature = "database")`；产品包各自决定链接哪个 `*_domain`。这样 `navopShell` 的依赖图里根本没有 `db_view`，不是“编译了但运行时隐藏”。

只加 feature 的旧方案收益很小：Shell 只减 3.7%，Data 减 17%；把 `extension-runtime`、`navop_runtime` 的域硬依赖拆掉后，Shell 可减 31%，Data 减 17%。本设计就是把“拆掉硬依赖”作为前置条件，而不是可选项。

## 2. 目标与非目标

### 目标

1. 三个应用能独立构建、独立安装、独立更新、独立启动。
2. `navopShell` 不初始化 DB 全局状态、不注册 DB 工具、不加载 DB 驱动资产；`navopData` 不初始化终端/SFTP/RDP 视图。
3. 三个应用共享同一套连接模型、凭据加密、云同步、设置 schema、AI 与扩展底座。
4. 全量 `navop` 行为、数据目录、bundle id、更新源保持兼容，老用户无感升级。
5. 领域新增功能只改对应领域包，不再向 `main` 增加 `match connection_type` 分支。

### 非目标

- 不在本轮把 `one-core` 拆成多个 crate。它是共享地基，拆它收益低、风险高。
- 不在本轮把 AI / Notes / Team / 工具箱做成“可卸载领域”。它们跨领域，仍由 `app_kit` 链接；产品差异只控制入口可见性。
- 不做三份 fork，也不做“一个二进制靠授权文件切换产品”。
- 不在本轮改连接表结构；`ConnectionType` 继续是超集，按产品过滤。

## 3. 现状评估摘要

### 3.1 体量与闭包

workspace 共 54 个 crate、约 807,931 行 `.rs`（含测试）。关键域：

| crate | 行数 | 说明 |
| --- | ---: | --- |
| `db` | 105,180 | 数据库后端、驱动 IPC、连接池 |
| `db_view` | 88,546 | SQL 编辑器、数据库 Tab、连接表单 |
| `terminal_view` | 58,921 | SSH/本地终端/SFTP 视图 |
| `terminal` | 37,906 | 终端模拟、PTY、WSL |
| `redis_view` | 19,212 | Redis 视图 |
| `extension-runtime` | 19,167 | 扩展宿主，硬依赖 DB |
| `sftp_view` | 17,136 | SFTP 视图 |
| `ssh` | 11,985 | SSH 传输 |
| `mongodb_view` | 8,417 | MongoDB 视图 |
| `main` | 72,005 | 应用外壳，755 个测试 |

只把 `main` 的直接依赖设为 optional 时：

| 产品 | crate 数 | 行数 | 占比 |
| --- | ---: | ---: | ---: |
| 全量 | 54 | 807,931 | 100% |
| Shell（只加 feature） | 51 | 778,248 | 96.3% |
| Data（只加 feature） | 45 | 670,359 | 83.0% |
| Shell（解耦 extension/runtime 后） | 45 | 559,306 | 69.2% |
| Data（解耦后） | 45 | 670,359 | 83.0% |

只加 feature 时 Shell 实际只少 `mongodb-runtime`、`mongodb_view`、`redis_view` 三个 crate，因为：

- `crates/extension-runtime/Cargo.toml:11-13` 非 optional 依赖 `db`、`db-value`、`db_view`；
- `crates/terminal_view` → `workspace_explorer` → `remote_file_editor` → `extension-runtime` → `db/db_view`；
- `crates/navop_runtime/Cargo.toml:12/15/17/18` 非 optional 依赖 `db`、`redis-runtime`、`sftp`、`ssh`。

### 3.2 外壳里的域分叉

- `main/src` 对域 crate 的直接引用 83 处、43 个文件；直接触及域代码的 36 个文件共 23,291 行，约占 `main` 32%。
- `ConnectionType::` 共 369 处、30 个文件；真正遍历全部类型的入口只有 `main/src/connection_type_menu.rs:36 builtin_filter_types` 与 `:51 filter_targets_for`，其余多是按类型渲染/分支。
- `main/src/home/home_tabs.rs` 里 20 多个 `open_*` 函数各自 `use` 具体域类型；`add_item_to_tab_with_mode`（`:1535`）是 DB 专用，直接构造 `DatabaseTabView`。
- `main/src/navop_app.rs:947-1004` 的 `init` 无条件调用 `init_ssh_session_service`、`terminal_view::init`、`redis_view::init`、`mongodb_view::init`、`db::init_cache`、`db_view::init_ask_ai_notifier`，三个产品无法共用。
- `main/src/home_tab.rs:174 HomePage` 同时持有 `IpcDriverRegistry`（DB）、`wsl_distributions`（终端）、`port_forwarding_runtime`（Shell）、`auth_service`/`cloud_sync_service`（共享），职责混杂。
- `main/src` 有 253 处 `include_str!` 源码断言测试（如 `home_tabs.rs` 断言 `TabItem::new("notes", ...)`、`home_tab/tests/rendering.rs` 断言字段字符串）。移动模块会直接打断这些测试，必须同步改成行为测试。

### 3.3 可复用的接缝

现状并非一片散沙，已有四个可用接缝：

1. **Tab 模型**：`crates/core/src/tab_container.rs:569 TabContentView` 已是对象安全的 `Arc<dyn TabContentView>`；`TabItem` 内部就是 `Arc<dyn TabContentView>`（`:773`），只是缺少从 trait object 构造的公开 API。
2. **侧栏贡献**：`SidebarContribution`（`crates/core/src/sidebar_contribution.rs`）已由 DB/Redis/Mongo/终端 Tab 实现，天然领域无关。
3. **MCP Provider**：`public_mcp::tools::PublicMcpToolProvider` + `PublicMcpToolRegistry::new(Vec<Arc<dyn ...>>)` 已支持按 provider 组合；Redis/Mongo/终端工具已经是 provider。
4. **打开策略**：`main/src/home/home_strategy.rs:14 ConnectionOpenStrategy` 已是“按连接类型选策略”，可以替换为注册表。

## 4. 核心设计决策

| # | 决策 | 理由 |
| --- | --- | --- |
| D1 | 领域选择用 crate 依赖，不用 feature | feature 是包级且会被 workspace 求并集，无法保证 Shell 不编译 DB |
| D2 | 新增 `app_kit` 作为唯一应用壳 | 三个 bin 共享窗口/主页/侧栏/设置/账号，避免复制 |
| D3 | 新增 `domain_api` 作为唯一契约层 | 外壳不 `use db_view`/`terminal_view`，领域也不互相依赖 |
| D4 | 领域按产品纵向切，不按“前端/后端”横切 | 一个领域自带上连接类型、表单、Tab、设置、MCP、扩展贡献 |
| D5 | 全量 app 保留现有身份；两个新产品用新身份 | 老用户数据/bundle/更新源不能断 |
| D6 | 三个产品默认各自 data dir / keychain | 避免 schema 版本倒挂、SQLite 并发写、凭据边界变宽 |
| D7 | 全量 app 默认独占既有文件关联与 MCP 发现 | 避免两个产品争同一 `.db`/`.md` 关联和同一个 `public-mcp.json` |
| D8 | 先拆 `extension-runtime`，再拆 `navop_runtime`，最后搬 `main` | 否则“独立应用”仍会经扩展/工具层拉入 DB/SSH |
| D9 | `one-core` 本轮不拆，只做“去领域泄漏” | 它是共享地基；拆它会把风险放大到账号/同步/存储 |
| D10 | 每个阶段都必须保持全量 app 可运行、测试通过 | 这是渐进重构，不是重写 |

## 5. 目标架构

### 5.1 包关系

```text
apps/navop ───────────────┐
apps/navop-shell ──┐      │
apps/navop-data ─┐ │      │
                 │ │      │
                 ▼ ▼      ▼
              crates/app_kit
                 │  ├── crates/one-core
                 │  ├── crates/one_ui
                 │  ├── crates/public_mcp
                 │  ├── crates/extension-core
                 │  ├── crates/ai_chat_view / notes / team
                 │  └── crates/domain_api
                 │            ▲
      ┌──────────┴────────┐   │
      ▼                   ▼   │
crates/shell_domain   crates/data_domain
      │                   │
      ├── ssh/sftp        ├── db/db_view
      ├── terminal(_view) ├── redis_view / mongodb_view
      ├── sftp_view       ├── er_flow / ferrum-flow
      ├── remote_desktop  ├── runtime_database / runtime_redis
      ├── runtime_sftp    └── extension-db
      └── extension-rdp
```

### 5.2 新包

| 包 | 职责 | 明确不依赖 |
| --- | --- | --- |
| `crates/domain_api` | `Domain`/`DomainRegistry`/`ConnectionOpener`/`ConnectionFormProvider`/`TabOpener`/`TabIntent`/`McpProvider`/`FileOpenHandler`/`SettingSection`/`HomeAction`/`ConnectionKindDescriptor` | 任何域 crate |
| `crates/app_kit` | 启动、窗口、`NavopApp`、主页、侧栏、连接仓库、设置容器、账号/同步、AI、扩展市场、更新、托盘、单实例、文件打开 | `db*`、`terminal*`、`ssh`、`sftp*`、`redis*`、`mongodb*`、`remote*`、`workspace_explorer` |
| `crates/data_domain` | DB/Redis/Mongo/SQL/ER/驱动扩展 | `terminal*`、`sftp*`、`remote*`、`workspace_explorer` |
| `crates/shell_domain` | SSH/SFTP/FTP/终端/端口转发/RDP/VNC/known_hosts/会话日志 | `db*`、`redis*`、`mongodb*`、`er_flow` |
| `crates/extension-core` | 扩展 manifest、市场、下载、通用宿主、通用 action、连接导入协议 | `db`、`db_view`、`db-value`、`remote_desktop` |
| `crates/extension-db` | DB 驱动安装、DB 扩展网关、DB 贡献/菜单 | 终端/SFTP |
| `crates/extension-rdp` | RDP provider 安装、远桌面贡献 | DB |
| `crates/ssh_form` | SSH 连接表单（目标只依赖 `ssh`/`connection-form`/`gpui`；短期可依赖 `terminal` 后端） | `terminal_view`、`db*` |
| `crates/runtime_core` | 连接/工作区工具、通用校验 | 域 crate |
| `crates/runtime_database` | `database_tools`、metadata、schema | 终端/SFTP |
| `crates/runtime_redis` | Redis 命令工具 | DB/终端 |
| `crates/runtime_sftp` | SFTP 工具 | DB/终端 |

包名可以用 `navop-` 前缀，目录名按现有习惯不加；上表以 crate 名为准。

### 5.3 三产品矩阵

| 项 | navop | navopShell | navopData |
| --- | --- | --- | --- |
| Cargo 包 | `navop` | `navop-shell` | `navop-data` |
| 可执行文件 | `navop` | `navop-shell` | `navop-data` |
| 显示名 | Navop | Navop Shell | Navop Data |
| app_id | `navop` | `navop-shell` | `navop-data` |
| macOS bundle id | `com.onetcli.app`（保持） | `com.navop.shell` | `com.navop.data` |
| 安装版 data dir | `navop` | `navop-shell` | `navop-data` |
| 开发版 data dir | `.navop-dev` | `.navop-shell-dev` | `.navop-data-dev` |
| config dir | `navop` | `navop-shell` | `navop-data` |
| cache dir | `navop` | `navop-shell` | `navop-data` |
| keychain service | `com.navop.desktop` | `com.navop.desktop.shell` | `com.navop.desktop.data` |
| portable marker | `navop.portable` | `navop-shell.portable` | `navop-data.portable` |
| MCP discovery | `navop` / 现有 `public-mcp.json` | v1 关闭 | v1 关闭 |
| 更新 channel | `navop` | `navop-shell` | `navop-data` |
| Windows UpgradeCode | 现有 `2EAB58E6-…` | 新 GUID | 新 GUID |
| Linux desktop | `navop` | `navop-shell` | `navop-data` |

> 全量 app 的 bundle id 目前是 `com.onetcli.app`（`resources/macos/Info.plist`），不是 `com.navop.desktop`。本轮**不改**它，否则 macOS 会当成新应用、丢掉钥匙串和更新身份。新产品才用新 id。

## 6. 领域 API

### 6.1 最小契约

```rust
// crates/domain_api/src/lib.rs

pub trait Domain: Send + Sync + 'static {
    fn id(&self) -> DomainId;
    /// 只做声明式注册：不碰 GPUI 全局状态，不创建 window。
    fn register(&self, registry: &mut DomainRegistry);
    /// 在公共 init 之后调用：设置全局状态、注册 Tab builder、MCP provider 等。
    fn init(&self, cx: &mut App);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainId {
    Data,
    Shell,
}

pub trait TabHost {
    fn tab_container(&self) -> Entity<TabContainer>;
    fn activate(&mut self, item: TabItem, window: &mut Window, cx: &mut App);
}

/// 领域能看到的“主页”最小能力；app_kit::HomePage 实现它。
pub trait HomeContext: TabHost {
    fn connections(&self) -> &[StoredConnection];
    fn open_tab(
        &mut self,
        intent: TabIntent,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
    fn notify(&mut self, message: SharedString, cx: &mut App);
}

pub trait ConnectionOpener: Send + Sync {
    fn supports(&self, connection: &StoredConnection) -> bool;
    fn open(
        &self,
        connection: StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    );
}

pub trait ConnectionFormProvider: Send + Sync {
    fn kinds(&self) -> Vec<ConnectionKindDescriptor>;
    fn open_new(
        &self,
        kind_id: &str,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    );
}

pub trait TabOpener: Send + Sync {
    fn key(&self) -> &'static str;
    fn open(
        &self,
        intent: TabIntent,
        host: &mut dyn TabHost,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabItem>;
}

pub trait McpProvider: Send + Sync {
    fn tools(&self, cx: &App) -> Vec<Arc<dyn PublicMcpToolProvider>>;
}

pub trait FileOpenHandler: Send + Sync {
    fn extensions(&self) -> &'static [&'static str];
    fn open(&self, path: &Path, window: &mut Window, cx: &mut App) -> bool;
}

pub trait SettingSection: Send + Sync {
    fn id(&self) -> &'static str;
    fn order(&self) -> i32;
    fn title(&self, cx: &App) -> SharedString;
    fn build(&self, window: &mut Window, cx: &App) -> SettingPage;
}

pub trait ConnectionDetailProvider: Send + Sync {
    fn supports(&self, connection: &StoredConnection) -> bool;
    fn details(&self, connection: &StoredConnection, cx: &App) -> Vec<DetailRow>;
}

pub trait ConnectionActionProvider: Send + Sync {
    fn actions(
        &self,
        connection: &StoredConnection,
        cx: &App,
    ) -> Vec<ConnectionAction>;
}

pub trait HomeAction: Send + Sync {
    fn id(&self) -> &'static str;
    fn order(&self) -> i32;
    fn render(
        &self,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement;
}

/// app_kit 定义、data_domain 适配 IpcDriverRegistry、shell 用 NoExternalDrivers。
pub trait ExternalDriverCatalog: Send + Sync {
    fn find(&self, driver_id: &str) -> Option<ExternalDriverDescriptor>;
    fn list(&self) -> Vec<ExternalDriverDescriptor>;
    fn refresh(&self, cx: &mut App);
}
```

`app_kit::HomePage` 实现 `HomeContext` 与 `TabHost`；领域 opener 只接收 `&mut dyn HomeContext`。`Context<HomePage>` 不出现在 `domain_api` 的签名里，避免 trait object 与 GPUI 具体实体耦合。

### 6.2 注册表

```rust
#[derive(Default)]
pub struct DomainRegistry {
    openers: Vec<Arc<dyn ConnectionOpener>>,
    forms: Vec<Arc<dyn ConnectionFormProvider>>,
    kinds: Vec<ConnectionKindDescriptor>,
    tab_openers: Vec<Arc<dyn TabOpener>>,
    mcp: Vec<Arc<dyn McpProvider>>,
    file_handlers: Vec<Arc<dyn FileOpenHandler>>,
    settings: Vec<Arc<dyn SettingSection>>,
    home_actions: Vec<Arc<dyn HomeAction>>,
    detail_providers: Vec<Arc<dyn ConnectionDetailProvider>>,
    actions: Vec<Arc<dyn ConnectionActionProvider>>,
}

impl DomainRegistry {
    pub fn register(&mut self, domain: &dyn Domain) { domain.register(self); }
    pub fn opener_for(&self, c: &StoredConnection) -> Option<&Arc<dyn ConnectionOpener>> {
        self.openers.iter().find(|o| o.supports(c))
    }
    pub fn enabled_types(&self) -> Vec<ConnectionType> {
        self.kinds.iter().map(|k| k.connection_type).collect()
    }
}
```

`app_kit::init` 的顺序固定为：

1. `gpui_component::init`、主题。
2. 构造 `DomainRegistry`，逐个 `domain.register()`（只做声明式注册，不碰 GPUI 全局状态），`cx.set_global(registry)`。
3. `setting_tab::init_settings` 的容器部分、`one_core::init`、`one_ui::init`。
4. 共享服务：auth、license、AI chat、MCP approval、账号/同步、凭据库。
5. 逐个 `domain.init(cx)`。DB 域在这里做 `db::init_cache` + `GlobalDbState::with_connection_repository` + `db_view` 初始化；Shell 域在这里做 `init_ssh_session_service`、`terminal_view::init`、`remote_desktop_view::init`。
6. `app_kit` 自己的 Tab/Home/MCP runtime/快捷键/action handler 初始化。
7. 打开主窗口。

关键点：**第 5 步是领域唯一的 GPUI 初始化入口**；`app_kit` 不再出现 `db_view::init` / `terminal_view::init` 字样。

### 6.3 连接类型描述

`ConnectionType` 仍是超集，但 UI 不再直接 `match` 它：

```rust
pub struct ConnectionKindDescriptor {
    pub id: &'static str,
    pub connection_type: ConnectionType,
    pub label: fn(&App) -> SharedString,
    pub icon: fn(&App) -> Icon,
    pub group: ConnectionKindGroup, // Database / Terminal / Remote / Extension
    pub order: i32,
}
```

`main/src/connection_type_menu.rs` 的 `builtin_filter_types`/`filter_targets_for` 改为读注册表；侧栏筛选栏、主页筛选、`new_connection` 分类都读同一份 `kinds`。全量产品注册所有 kind；Shell 只注册 SSH/SFTP/FTP/Serial/Telnet/PortForwarding/RDP/VNC；Data 只注册 DB/Redis/Mongo。

## 7. 三个关键解耦

### 7.1 Tab：用 `TabItem::from_dyn` + `TabIntent`

`TabItem::new<T: TabContent>` 要求具体类型，领域 crate 无法在 `app_kit` 里构造。新增：

```rust
impl TabItem {
    pub fn from_dyn(
        id: impl Into<String>,
        from: impl Into<String>,
        content: Arc<dyn TabContentView>,
    ) -> Self {
        Self {
            id: SharedString::from(id.into()),
            from: SharedString::from(from.into()),
            metadata: HashMap::new(),
            content,
        }
    }
}
```

领域 opener 自己构造 `Arc<dyn TabContentView>`：

```rust
// data_domain
let tab = DatabaseTabView::new_with_active_conn(...);
let item = TabItem::from_dyn(tab_id, "home", Arc::new(tab));
home.tab_container().update(cx, |c, cx| c.add_and_activate_tab_with_focus(item, window, cx));
```

通用 Tab 用 `TabIntent`：

```rust
pub enum TabIntent {
    Settings,
    AiWorkbench,
    Notes,
    Team,
    Toolbox,
    JsonFormatter,
    SessionLogs,
    CredentialVault,
    KnownHosts,
    Extensions,
    Database { connection_id: String, mode: DatabaseTabMode },
    SqlEditor { connection_id: String, sql: String },
    Terminal { profile: TerminalProfileDescriptor },
    Sftp { connection_id: String, path: Option<String> },
    RemoteDesktop { connection_id: String, protocol: RemoteProtocol },
    Extension { extension_id: String, contribution_id: String },
}

// domain_api 自己的“产品级”枚举，不引用 db_view/remote_desktop 的类型。
pub enum DatabaseTabMode { Single, Workspace }
pub enum RemoteProtocol { Rdp, Vnc }
pub struct TerminalProfileDescriptor {
    pub id: String,
    pub label: String,
    pub kind: TerminalProfileKind, // System / PowerShell / Wsl / Custom ...
}
pub enum TerminalProfileKind {
    System, PowerShell, Cmd, Wsl, GitBash, Zsh, Bash, Fish, Nushell, Custom,
}
```

`ConnectionKindDescriptor`、`ConnectionAction`、`DetailRow`、`ExternalDriverDescriptor` 都是 `domain_api` 的纯数据描述，字段只用 `String`/`Icon`/`ConnectionType` 等共享类型，不出现 `DatabaseType`、`SshSession`、`IpcDriverRegistry` 等域类型。

`app_kit` 提供 `TabHost` 实现；`TabOpener` 按 key 注册。`TabContentRegistry` 仍用于恢复：领域 `init` 时 `register_fn("Database", ...)`，`app_kit` 根据 dump 的 key 重建。

同时修掉两个现有反模式：

- `home_tabs.rs:1650 duplicate_active_tab` 不再 `downcast::<TerminalWorkspace>()`，改用 `TabContentView::duplicate`。
- `add_item_to_tab_with_mode` 不再由 `app_kit` 构造 `DatabaseTabView`，改由 data 域的 `DatabaseTabOpener` 处理。

### 7.2 Home：从 `HomePage` 搬走域状态

`HomePage` 只保留视图状态和共享服务（连接仓库、工作区、auth、cloud sync、team 权限）。域相关字段按表迁移：

| 现状 | 目标 |
| --- | --- |
| `external_driver_registry: IpcDriverRegistry` | `Arc<dyn ExternalDriverCatalog>`（`domain_api` 定义 trait，data 域提供 `IpcDriverCatalog` 适配器；Shell 用 `NoExternalDrivers`） |
| `wsl_distributions: Option<Arc<Vec<terminal::WslDistribution>>>` | `shell_domain::LocalTerminalService` 全局；主页按钮由 shell 的 `HomeAction` 渲染 |
| `port_forwarding_runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>` | `shell_domain::PortForwardingService` 全局 |
| `show_connection_form(db_type)` / `show_ssh_form` / `show_redis_form` / `show_ftp_form` / Mongo / RDP 表单 | 各域 `ConnectionFormProvider::open_new` |
| `open_ssh_terminal` / `open_sftp_view` / `open_redis_tab` / `open_mongodb_tab` / `open_remote_desktop` | 各域 `ConnectionOpener` + `TabOpener` |
| `add_item_to_tab_with_mode`（DB） | data 域 `DatabaseTabOpener` |
| `add_session_logs_tab` / `add_known_hosts_tab` | shell 域 `HomeAction` / `ConnectionOpener` |
| `add_team_tab` / `add_notes_tab` / `add_extensions_tab` | 仍由 `app_kit` 提供（共享功能） |

`HomeAction`（定义见 §6.1）允许领域返回自定义按钮/菜单，签名使用 `&mut dyn HomeContext`，不引用 `HomePage` 类型：

```rust
fn render(
    &self,
    home: &mut dyn HomeContext,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement;
```

Shell 的“本地终端”按钮、Data 的“新建查询”按钮都用它，`app_kit` 只负责布局。

### 7.3 启动：从一段 `init` 变成 Product 组合

```rust
// crates/app_kit/src/product.rs
pub trait Product: 'static {
    type Assets: AssetSource + 'static;
    fn identity(&self) -> AppIdentity;
    fn features(&self) -> ProductFeatures;
    fn assets(&self) -> Self::Assets;
    fn domains(&self) -> Vec<Arc<dyn Domain>>;
}

pub fn run<P: Product>(product: P) -> anyhow::Result<()>;

pub struct ProductFeatures {
    pub ai: bool,
    pub notes: bool,
    pub team: bool,
    pub cloud_sync: bool,
    pub credential_vault: bool,
    pub toolbox: bool,
    pub public_mcp: bool,
}

impl ProductFeatures {
    pub const FULL: Self = Self {
        ai: true, notes: true, team: true, cloud_sync: true,
        credential_vault: true, toolbox: true, public_mcp: true,
    };
    pub const SHELL: Self = Self {
        ai: true, notes: true, team: true, cloud_sync: true,
        credential_vault: true, toolbox: true, public_mcp: false,
    };
    pub const DATA: Self = Self {
        ai: true, notes: true, team: true, cloud_sync: true,
        credential_vault: true, toolbox: true, public_mcp: false,
    };
}
```

`ProductFeatures` 只控制**入口可见性与共享模块的 init**，不承担领域裁剪；数据库/终端是否链接由 `domains()` 决定。

三个产品的入口都只有十几行：

```rust
// apps/navop-shell/src/main.rs
fn main() -> anyhow::Result<()> {
    app_kit::run(NavopShellProduct)
}

struct NavopShellProduct;

impl Product for NavopShellProduct {
    type Assets = ShellAssets;
    fn identity(&self) -> AppIdentity { AppIdentity::NAVOP_SHELL }
    fn features(&self) -> ProductFeatures { ProductFeatures::SHELL }
    fn assets(&self) -> Self::Assets { ShellAssets::new() }
    fn domains(&self) -> Vec<Arc<dyn Domain>> {
        vec![Arc::new(shell_domain::ShellDomain::new())]
    }
}
```

`AppIdentity` 把现在散落的常量收口：

```rust
pub struct AppIdentity {
    pub app_id: &'static str,
    pub display_name: &'static str,
    pub bundle_id: &'static str,
    pub binary_name: &'static str,
    pub data_dir_name: &'static str,
    pub legacy_data_dir_names: &'static [&'static str],
    pub keychain_service: &'static str,
    pub portable_marker: &'static str,
    pub mcp_app_name: &'static str,
    pub mcp_legacy_names: &'static [&'static str],
    pub update_channel: &'static str,
    pub file_associations: &'static [FileAssociation],
}

pub struct FileAssociation {
    pub extension: &'static str,
    pub role: &'static str,
}
```

`one_core::app_paths` / `app_dirs` / `key_storage` / `public_mcp::discovery` / `update::config` 都改成从 `AppIdentity` 取值，而不是各自写死 `"navop"`。

## 8. crate 拆分细节

### 8.1 先拆 `extension-runtime`

现状（`crates/extension-runtime/Cargo.toml:11-13`）非 optional 依赖 `db`、`db-value`、`db_view`，DB 相关引用 112 处（其中 `db`/`db_view` 的导入点十余处）。目标：

- `extension-core`：`extension/mod.rs`、`manifest`、`registration`、`catalog`、`extension_view_host`、`extension_downloader`、`marketplace`、`transfer`、`connection_import_provider`、`contributes/*`、通用 action handler。**不依赖** `db*`、`remote_desktop`。
- `extension-db`：`database_driver_install.rs`、`extension_db_gateway.rs`、`database_driver_provider.rs`、DB 菜单/上下文（`DbTreeExtensionActionContext`、`DbTreeExtensionMenuItem`、`DbTreeExtensionMenuRegistry`）。
- `extension-rdp`：`remote_desktop_provider_install.rs` 与远桌面贡献。

泛化点：

- `DbTreeExtensionMenuRegistry` 改为 `extension-core` 的通用 `ExtensionMenuRegistry`，菜单项只带 `ExtensionMenuItem { extension_id, contribution_id, label, icon }`；DB 树在 data 域把通用菜单项适配成 DB 菜单。
- `GlobalDbState` 不再出现在 `extension-core` 的 action 参数里；action handler 改收 `ExtensionActionContext { connection_id, resource_id, payload }`，由 data 域解析成 DB 操作。
- `IpcDriverRegistry::refresh_global_registry` 从 `extension-core` 移到 `extension-db`；通用宿主不感知驱动。
- `remote_desktop` 的 provider 安装移到 `extension-rdp`。

完成标准：`cargo tree -p extension-core -e normal | grep -E 'db_view|db-value|remote_desktop'` 为空；`cargo tree -p remote_file_editor` 不再出现 `db_view`。

### 8.2 再拆 `navop_runtime`

现状 `crates/navop_runtime/Cargo.toml:12/15/17/18` 依赖 `db`、`redis-runtime`、`sftp`、`ssh`；`lib.rs:15 tool_registry()` 把 connections + database_tools + redis_tools 合并，sftp_tools 由 main 单独加。

目标：

| 新 crate | 内容 | 依赖 |
| --- | --- | --- |
| `runtime_core` | `connections.rs`、`connections/management.rs`、`schema.rs`、`validation.rs`、`workspaces.rs`、`build.rs` | `one-core`、`public_mcp` |
| `runtime_database` | `database_tools.rs`、`database_tools/metadata.rs`、`database_tools/schema.rs` | `runtime_core`、`db` |
| `runtime_redis` | `redis_tools.rs`、`command_io.rs` | `runtime_core`、`redis-runtime` |
| `runtime_sftp` | `sftp_tools.rs` | `runtime_core`、`sftp`、`ssh` |

每个 crate 暴露一个 `PublicMcpToolProvider` 实现；`app_kit` 的 MCP runtime 从 `DomainRegistry::mcp` 收集 provider，再合并**当前产品允许的**内置 provider（Shell 才有 terminal/sftp 内置工具；Data 只有 database/redis/mongo）。不要无条件 `public_mcp::terminal()`，否则 Data 会暴露终端工具。`ToolExposureToolsetSettings` 的超集字段保留，UI 只展示已注册的 toolset。

### 8.3 `main` → `app_kit`

按模块搬迁，先用 `pub use` 做过渡，最后删壳：

| `main/src` | 行数 | 去向 |
| --- | ---: | --- |
| `main.rs` | 1,189 | `apps/*/main.rs` + `app_kit::bootstrap` |
| `navop_app.rs` | 2,974 | `app_kit::app` + `app_kit::init` |
| `app_init.rs` | 515 | `app_kit::bootstrap`（快捷键按产品配置） |
| `home_tab.rs` + `home_tab/*` | 9,884 | `app_kit::home`（通用）+ 域表单/opener |
| `home/*` | 9,891 | `app_kit::home`（卡片/列表/筛选/快速打开） |
| `persistent_connection_sidebar/*` | 7,710 | `app_kit::sidebar` |
| `new_connection/*` | 1,757 | `app_kit::new_connection`（向导）+ 域表单 |
| `settings/*` + `setting_tab.rs` | 10,710 | `app_kit::settings` + 域 `SettingSection` |
| `public_mcp_runtime/*` | 3,855 | `app_kit::mcp` + 域 provider |
| `update/*` | 4,325 | `app_kit::update` |
| `credential_vault/*` | 1,846 | `app_kit::credential_vault` |
| `auth.rs` / `license.rs` / `master_key_flow.rs` | ~2,400 | `app_kit::account` |
| `personal_sync_runtime.rs` / `team_tab.rs` | 3,621 | `app_kit::account` / `app_kit::team` |
| `session_logs/*` | 1,620 | `shell_domain` |
| `file_open.rs` / `file_association.rs` | 1,007 | `app_kit` 通用 + 域 handler |
| `system_tray.rs` / `windows_single_instance.rs` | 1,636 | `app_kit` |
| `dev_extension_registry.rs` | 900 | `extension-core` |
| `connection_visuals.rs` / `connection_info.rs` / `connection_actions.rs` / `connection_share.rs` | ~1,500 | `app_kit` 描述符 + 域 provider |

`app_kit` 的 Cargo 依赖必须显式白名单：`one-core`、`one_ui`、`public_mcp`、`extension-core`、`extension_view`、`agent_runtime`、`ai_chat_view`、`tool_runtime`、`connection_form`、`connection-import-protocol`、`gpui`、`gpui-component`。不得出现 `db`、`db_view`、`terminal`、`terminal_view`、`ssh`、`sftp_view`、`redis_view`、`mongodb_view`、`remote_desktop_view`、`workspace_explorer`。

全量包名从 `main` 改为 `navop` 会牵动 CI、`release.yml`、`script/*` 里的 `-p main`；建议 P2 先保留 package name `main`、只把目录挪到 `apps/navop`，P3 再统一改名，避免和领域迁移同时制造噪声。

### 8.4 `one-core` 的边界

`one-core` 继续做共享地基，但需要处理两个泄漏：

1. **`one-core` → `connection_tunnel` → `ssh`**：审计 `one-core` 使用 `connection_tunnel` 的位置。如果只是共享 `TunnelConfig` 数据模型，把模型上移到 `one-core`，让 `connection_tunnel` 依赖 `one-core`；如果涉及运行时代码，移到 `db`/`shell_domain`。目标是 `one-core` 不再传递依赖 `ssh`。
2. **`one-core::settings` 里的域设置**：`DatabaseOpenMode`、`LocalTerminalProfileKind` 等仍可留在 `AppSettings`（schema 超集），但读写与 UI 归域；不要为了“干净”拆设置表，否则三产品 schema 会分叉。

`one-core` 的 `cloud_sync`、`storage`、`account` 保持共享；`navopData` 与 `navopShell` 必须能登录同一账号、同步同一批连接（按能力过滤展示）。

### 8.5 `db` / `ssh` 的小清理

- `crates/db/Cargo.toml` 写了 `ssh.workspace = true`，但 `crates/db/src` 没有 `ssh::` 直接引用（隧道走 `connection_tunnel`）。P1 验证后删除该依赖；`navopData` 仍会经 `connection_tunnel` 链接 `ssh` 库，这是必要的。
- `db` 的 `builtin-duckdb`、`builtin-data-drivers` 等后端 feature 保留，但只由 data 产品选择；这些是“能力”，不是“产品”。
- `ssh`/`sftp` 不需要产品 feature；Shell/Data 的差异在 UI 层与工具层。

### 8.6 SSH 隧道表单（Data 的刚需）

`main/src/home_tab/connection_forms.rs` 的 DB 表单会遍历 `ConnectionType::SshSftp` 连接作为隧道下拉（`:144` 附近）。这意味着 `navopData` 不能只“导入 SSH 连接而不能新建”，否则用户在没有全量 app 的机器上无法建立第一条隧道。

`SshFormWindow` 现在在 `crates/terminal_view/src/ssh_form_window.rs`（4,500+ 行），若 Data 直接依赖 `terminal_view`，会重新拉入 58k 行终端栈，抵消拆分收益。做法：

- 优先新建 `crates/ssh_form`，把 `SshFormWindow`/`SshFormWindowConfig`、`host_key_dialog`、`ssh_form_mfa` 及测试搬过去；
- 但 `ssh_form_window.rs` 现在依赖 `terminal::{SshBackend, HostKeyVerificationReason}`（`crates/terminal_view/src/ssh_form_window.rs:52`），所以 `ssh_form` 要么依赖 `terminal` 后端（37k，仍比 `terminal_view` 小），要么把 `SshBackend`/host-key 类型下沉到 `ssh`；
- 若短期抽不干净，Data 侧改为**精简隧道表单**：用 `connection-form` 的字段 + `ssh` 建连，只保留 host/port/user/auth/host-key 策略，不搬终端 profile/MFA 全套；
- `terminal_view` 改为 re-export，Shell 行为不变；Data 创建出的 SSH 连接只出现在隧道下拉，不出现终端入口；
- `host_key_dialog`/known_hosts 校验 UI 必须随 `ssh_form` 共享（`known_hosts_view` 的完整管理页仍归 Shell），否则 Data 遇到 unknown host key 无法继续。

验收：`cargo tree -p navop-data | grep terminal_view` 为空，但 Data 能新建 SSH 连接并用于 DB 隧道。若 `ssh_form` 仍依赖 `terminal`，需在架构检查里显式豁免这一条并记录原因。

## 9. 身份、数据与共存

### 9.1 为什么必须分 data dir

三产品共用 `crates/core/src/storage` 的 SQLite（WAL + `busy_timeout=5000`，`migration.rs:4` 前向迁移）。两个独立发版的应用共用一个 data dir 会带来：

- schema 版本倒挂：新版本迁移过的库被旧版本打开；
- 缓存/索引不一致：`db::init_cache`、`GlobalNodeCache` 假设单写者；
- 两进程同时写同一 SQLite：WAL 只能缓解锁冲突，不能解决逻辑冲突；
- 备份/迁移/卸载边界不清。

因此默认：

| 产品 | 安装版 data | 开发版 data | keychain |
| --- | --- | --- | --- |
| navop | `navop` | `.navop-dev` | `com.navop.desktop` / `.dev` |
| navopShell | `navop-shell` | `.navop-shell-dev` | `com.navop.desktop.shell` / `.shell.dev` |
| navopData | `navop-data` | `.navop-data-dev` | `com.navop.desktop.data` / `.data.dev` |

全量 app 保留现有目录与 keychain；两个新产品首启进入“导入”流程。

### 9.2 首次启动迁移

`navopShell` / `navopData` 首启检测到本机存在 `navop` 数据目录时，显示可选导入：

1. “从 Navop 导入连接”：
   - 复制 `connections` 表中本产品支持的 `connection_type`；
   - 复制工作区、分组、颜色、排序；
   - 凭据：让用户输入原 master password（或从旧 keychain 读取后重新加密到新 keychain）；无法解密则跳过凭据并提示；
   - 不复制不支持的连接类型，但记录数量，展示“N 个数据库连接已隐藏”。
2. “登录账号，从云同步恢复”：
   - 走现有 auth + cloud sync；
   - 同步下来的连接按能力过滤；服务端不需要为 v1 改协议，客户端过滤即可；
   - 后续可在 sync 请求里带 `product`/`capabilities`，让服务端裁剪。
3. “跳过，从空白开始”。

导入必须是**只读源库**：以只读方式打开 `navop` 的 SQLite，复制到新库，绝不写回。导入后记录 marker，避免重复提示。

### 9.3 共享账号与 License

- License 目前只有 `PlanTier{Free,Pro}` 与 `Feature{CloudSync,TeamManagement}`（`crates/core/src/license/models.rs`），没有产品维度。三产品复用同一账号/权益，不改后端。
- 若要分 SKU（例如 Shell 免费、Data 付费），需要改离线 key、后端权益与升级页；本轮不做。
- 三产品是否共享 keychain：默认**不共享**。共享会减少一次登录，但任一产品被攻破即扩大凭据边界；导入流程已经能解决迁移体验。

### 9.4 文件关联、MCP、热键、单实例

- **文件关联**：全量 app 继续独占 `.db`、`.duckdb`、`.md`、`.cast`（`main/src/file_association.rs`、Info.plist）。两个新产品默认不抢这些后缀；Data 可注册 `.sql`（新），Shell 不注册。若用户只装 Data，可在设置里手动关联 `.db`/`.duckdb`。这样避免“装了 Shell 后双击 .db 打不开”的回归。
- **MCP**：v1 只有全量 app 开 Public MCP，避免 `public-mcp.json` 单文件互相覆盖、`validate_for_stdio_bridge` 只认 `navop`/`onetcli` 的问题。`navopShell`/`navopData` 的 `ProductFeatures.public_mcp = false`，不写发现文件、不注册 MCP 工具。v2 若要开，需要 discovery 按产品分文件 + 客户端选择器。
- **全局热键**：`app_init.rs:124-266` 的 toggle hotkey 默认值按产品区分（Navop 保持现状；Shell 默认 `Cmd/Ctrl+Shift+S`；Data 默认 `Cmd/Ctrl+Shift+D`），并允许设置覆盖。否则三个都装会抢同一热键。
- **单实例**：`windows_single_instance.rs` 的互斥名/窗口类名改为按 `identity.app_id` 生成；macOS/Linux 用 app_id 或 socket 名区分。

## 10. 发版、CI 与安装包

### 10.1 构建

- `.github/workflows/release.yml` 从单产品矩阵改为“产品 × 平台”矩阵：3 产品 × 6 平台 = 18 个构建。
- 每个构建显式 `cargo build --release -p navop` / `-p navop-shell` / `-p navop-data`，**禁止** `--workspace`，否则 feature/依赖会被求并集，失去瘦身意义。
- 版本号：三个包共用 workspace version（推荐），或各自独立版本（需要三套 tag/CHANGELOG）。推荐共用，减少支持矩阵。

### 10.2 安装包

| 平台 | 现状 | 三产品改法 |
| --- | --- | --- |
| macOS | `script/bundle-macos.sh` 固定 `Navop.app`/`navop` | 参数化 `APP_NAME`、`BUNDLE_ID`、`BINARY_NAME`、`DOC_TYPES`；产物 `Navop Shell.app` / `Navop Data.app` |
| Windows | `installer/windows/navop.wxs` 固定 UpgradeCode/`navop.exe` | 每个产品独立 `UpgradeCode`/`ProductCode`/ARPPRODUCTICON；MSI 名 `NavopShell-<version>.msi` |
| Linux | `resources/linux/navop.desktop` | 每个产品独立 `.desktop`（`Name`/`Exec`/`Icon`/`StartupWMClass`），包名 `navop-shell`/`navop-data` |

签名/公证：macOS 同一 Developer ID、按 bundle 分别公证；Windows 同一代码签名证书、分别签 MSI；Linux 维持现状。证书不需要三套，但 notarization 与 updater 签名要按产物分别做。

### 10.3 更新

- 每个产品独立 feed：`stable/navop/latest.json`、`stable/navop-shell/latest.json`、`stable/navop-data/latest.json`。
- manifest 增加 `product` 字段；客户端校验 `product == identity.app_id`，不匹配直接拒绝，避免 Shell 被全量包替换。
- `NAVOP_UPDATE_URL` 仍可作为全量 app 的覆盖；新产品用 `NAVOP_SHELL_UPDATE_URL` / `NAVOP_DATA_UPDATE_URL`（或统一 `NAVOP_UPDATE_URL` + 产品路径）。
- 更新检查 UI 显示产品名，不显示 “Navop 有新版本” 这种会让用户误装全量包的文案。

### 10.4 CI

- 保留全量 `cargo test --all` 作为“全量回归”；新增三个 `cargo test -p navop-shell` / `-p navop-data` / `-p navop` 的 job。
- 新增架构检查（见 §11.2），在 check 阶段失败即阻断。
- 预计 CI 时间从 ~25 分钟升到 35-45 分钟；可用缓存与并行 job 控制。若成本不可接受，至少保留全量 + 两个产品的 `cargo check`，夜间跑完整测试。

## 11. 迁移路线

### P0 基线与安全网（3-5 天）

1. 记录 release 二进制体积、启动时间、`cargo build -p main` 冷/热时间，作为瘦身验收基线。
2. 增加 `TabItem::from_dyn`，把现有 `TabItem::new` 调用逐步替换（行为不变）。
3. 把 `TabContentRegistry` 的 register 补起来：DB/终端/Redis/Mongo 各注册一个 builder，先只做“恢复时能 build”，不删旧路径。
4. 增加架构检查脚本（见 §11.2），先输出当前违规清单，不阻断。
5. 跑通 `RUSTFLAGS="-D warnings" cargo test -p main --bin navop`，记录基线。

验收：全量 app 行为不变；`TabItem::from_dyn` 有测试；架构脚本能列出 `extension-runtime→db` 等已知违规。

### P1 拆扩展层（8-10 天）

1. 新建 `extension-core`，把通用 manifest/registry/market/download/host 搬过去。
2. 新建 `extension-db`、`extension-rdp`，搬 DB/RDP 专属模块。
3. `extension-runtime` 暂时保留为 facade（`pub use extension_core::*; pub use extension_db::*;`），让消费者逐步改 import；**facade 不能长期存在**，P2 结束前删除。
4. 泛化 `DbTreeExtensionMenuRegistry`、`ExtensionActionContext`、`IpcDriverRegistry` 的边界。
5. 更新 `notes`、`resource_view`、`remote_file_editor`、`workspace_explorer`、`extension-plugin-adapter` 的依赖。

验收：`cargo tree -p extension-core` 无 `db*`/`remote_desktop`；`cargo tree -p remote_file_editor` 无 `db_view`；全量测试通过。

### P2 抽 `app_kit` + `domain_api`（15-20 天）

1. 新建 `domain_api`（trait/registry/descriptor，先不实现）。
2. 新建 `app_kit`，按 §8.3 搬 `main` 的共享模块；`main` 里用 `pub use app_kit::...` 过渡，保持旧路径编译。
3. 把 `HomePage` 的域字段换成 trait/服务（§7.2）；把 `home_strategy` 的 match 换成 `DomainRegistry::opener_for`。
4. 新建 `data_domain`、`shell_domain`，把域 opener/form/tab/设置页/MCP provider/文件 handler 搬进去。
5. 把 `navop_runtime` 拆成 `runtime_core`/`runtime_database`/`runtime_redis`/`runtime_sftp`（可与 P2 并行）。
6. 删除 `extension-runtime` facade；`main` 只剩 `apps/navop` 薄壳。
7. 处理 253 处 `include_str!` 测试：能改行为测试的改，不能改的随文件迁移并更新断言路径。

验收：`cargo tree -p app_kit` 无域 crate；全量 app 功能对等（手工清单 + 自动化测试）；`apps/navop` 的 `main.rs` < 100 行。

### P3 三产品与身份隔离（5-7 天）

1. 新建 `apps/navop-shell`、`apps/navop-data`，各自 `Product` 实现。
2. 把 `AppIdentity` 接入 `app_paths`/`app_dirs`/`key_storage`/`discovery`/`update`/`bundle`/`wxs`。
3. Shell 的 `init` 只注册 shell 域；Data 只注册 data 域；验证 Shell 启动日志无 DB 初始化、Data 无终端初始化。
4. 首次启动导入向导（§9.2）。
5. 文件关联、热键、单实例按产品隔离。

验收：三个 app 在同机共存；连接/工作区/凭据互不污染；全量 app 数据完全不受影响。

### P4 打包与发布（8-10 天）

1. `bundle-macos.sh` 参数化；两套新 Info.plist；三套签名/公证。
2. Windows 三套 WiX（不同 UpgradeCode/ProductCode/图标）。
3. Linux 三套 desktop/AppImage/deb/rpm。
4. 三套更新 feed 与 manifest 校验。
5. CI 矩阵、 smoke test（安装→启动→建连接→开 Tab→检查更新）。

验收：三产品在 6 平台产出可安装包；更新互不串包；卸载不互删数据。

### P5 收尾（3-5 天）

1. 删除 `main` 过渡 `pub use`、旧模块、死代码。
2. 更新 `CLAUDE.md`/`AGENTS.md`/docs/design 的 crate 表与启动流程。
3. 清理 `db` 的冗余 `ssh` 依赖、`one-core` 的 `connection_tunnel` 泄漏。
4. 记录最终体积/构建时间对比。

总工期：单人约 42-57 个工作日（9-12 周）；两人并行约 6-8 周。不含证书申请、商店审核、后端 SKU 改造。

## 12. 测试与验证

### 12.1 架构检查

新增 `script/check-architecture.py`（读 `cargo metadata --no-deps`），CI 强制执行：

```text
app_kit       不得依赖 db, db_view, db-value, redis_view, mongodb_view,
              terminal, terminal_view, sftp_view, ssh, remote_desktop_view,
              workspace_explorer
extension-core 不得依赖 db, db_view, db-value, remote_desktop, terminal
data_domain   不得依赖 terminal*, sftp*, remote_desktop*, workspace_explorer
shell_domain  不得依赖 db*, redis*, mongodb*, er_flow, ferrum-flow
domain_api    不得依赖任何域 crate
apps/*        不得直接依赖域 crate（只能经 *_domain）
```

另加两条 `cargo tree` 断言：

```bash
! cargo tree -p navop-shell -e normal | grep -q 'db_view'
! cargo tree -p navop-data  -e normal | grep -q 'terminal_view'
```

注意：`data_domain` 会经 `db → connection_tunnel → ssh` 传递链接 `ssh` 库，这是允许的；禁止的是 `data_domain` 直接依赖 `terminal`/`terminal_view` 或出现 SSH UI。

### 12.2 行为测试

- 域测试随 crate 迁移：`cargo test -p data_domain`、`cargo test -p shell_domain`。
- Tab 恢复：为每种 `TabIntent` 写 build/dump/restore 测试。
- 连接打开：注册表 `opener_for` 的单测；未知类型返回 None 并提示。
- 设置页：每个域至少一个“页面能构建”的测试。
- MCP：每个域的 provider 在 registry 中出现/不出现。
- 导入：用 fixture SQLite 测“只读源库 + 过滤类型 + 凭据失败降级”。
- i18n：现有 `main/tests/i18n_locale_keys_contract.rs` 改为按 crate 运行（每个含 `t!` 的 crate 都要有自己的 locale key）。

### 12.3 手工 smoke 清单

| 场景 | navop | navopShell | navopData |
| --- | --- | --- | --- |
| 新建 DB 连接 / 执行 SQL | ✓ | — | ✓ |
| 新建 SSH / 打开终端 / SFTP | ✓ | ✓ | — |
| Redis / MongoDB | ✓ | — | ✓ |
| 本地终端 / WSL | ✓ | ✓ | — |
| 端口转发 / RDP / VNC | ✓ | ✓ | — |
| AI Workbench / Notes / Team | ✓ | ✓ | ✓ |
| 云同步登录/恢复 | ✓ | ✓ | ✓ |
| 文件关联 `.db`/`.sql` | ✓ | — | 可选 |
| Public MCP | ✓ | — | — |
| 检查更新 | ✓ | ✓ | ✓ |

## 13. 风险与应对

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| `TabContentView` 对象化不彻底（`TabItem` 无 `from_dyn`） | 高 | P0 先补 API；所有域 opener 用 `Arc<dyn TabContentView>` |
| `HomePage` 的 `Context<HomePage>` 让 trait object 难以传递 | 高 | opener 用 `&mut dyn HomeContext`；必要时 `app_kit` 提供泛型 `open_with_home` 适配，领域只依赖 `domain_api` |
| 253 处 `include_str!` 源码断言 | 中 | 迁移时逐个改为行为测试；不能改的随文件移动并更新相对路径 |
| 设置页 `SettingPage` 构建依赖窗口/`AppSettings` | 中 | 用 `SettingSection` 工厂函数，域只返回页面；先在 P2 做 1 个 DB 页 + 1 个终端页试点 |
| 扩展系统默认假设 DB | 高 | P1 先拆 `extension-core`/`extension-db`；`extension-core` 不出现 `GlobalDbState` |
| `IpcDriverRegistry` 扩散到表单/图标/快速打开 | 中 | 先抽 `ExternalDriverCatalog`，DB 适配器放 data 域；Shell 空实现 |
| 全量 app 行为回归 | 高 | 每阶段跑全量测试 + smoke；`main` 的过渡 `pub use` 保持旧路径直到 P3 |
| 两产品共享 data dir 导致 schema 倒挂 | 高 | 默认分目录；导入向导只读源库 |
| 两个产品争文件关联/MCP 发现/热键 | 中 | 全量独占既有资源；新产品默认不抢；热键按产品默认值 |
| 更新 feed 串包 | 高 | manifest `product` 校验；三套 feed；CI 校验产物名 |
| 三套签名/公证/CI 成本 | 中 | 共用证书；并行矩阵；Linux 不签；夜间跑完整测试 |
| `one-core` 的 `connection_tunnel`/`ssh` 泄漏 | 低-中 | 只做模型上移/下沉，不拆 `one-core`；允许 data 经隧道链接 ssh |
| 团队/Notes/AI 仍由 `app_kit` 链接，体积下降有限 | 中 | 先接受；若后续要更小，再把它们做成 `NavigationProvider` 可选包 |

## 14. 工作量拆分

| 阶段 | 内容 | 人日（1 人） |
| --- | --- | ---: |
| P0 | 基线、`TabItem::from_dyn`、registry 试点、架构脚本 | 3-5 |
| P1 | extension-core/db/rdp | 8-10 |
| P2 | app_kit/domain_api/data_domain/shell_domain/runtime 拆分 | 15-20 |
| P3 | 三产品、身份、导入 | 5-7 |
| P4 | 打包、签名、更新、CI | 8-10 |
| P5 | 收尾、文档、清理 | 3-5 |
| **合计** | | **42-57（9-12 周）** |

两人并行（一人 P1+extension，一人 P2 前半 app_kit）约 6-8 周，但 P2 后期会汇合，需避免同时改 `HomePage`。建议顺序：P0 → P1 → P2（先 `app_kit` 骨架，再域迁移）→ P3 → P4。

## 15. 待决策

1. **产品范围**：`navopData` 是否保留 Redis/MongoDB？建议保留（都是“数据”），MQTT 归 Data 还是全量？
2. **RDP/VNC 归属**：建议归 `navopShell`（远程终端），但 `remote_desktop_view` 体量 23k，需确认。
3. **文件关联**：全量独占 `.db/.duckdb/.md/.cast`，新产品默认不抢；是否接受 Data 注册 `.sql`？
4. **Public MCP**：v1 是否只在全量开？若 Shell/Data 也要开，需要先改 discovery 为按产品分文件。
5. **版本策略**：三产品共用 workspace version，还是独立版本？建议共用。
6. **Data 的 SSH 隧道**：必须保留 `connection_tunnel` + `ssh` 库（无终端 UI）；是否接受这条传递依赖？
7. **共享 keychain**：默认不共享；是否提供“与 Navop 共用登录”开关？

## 16. 附：迁移检查表

- [ ] `TabItem::from_dyn` 合入，域 opener 不再依赖具体 `TabContent` 类型。
- [ ] `TabContentRegistry` 在每个域 `init` 时注册 builder。
- [ ] `app_kit` 的 `Cargo.toml` 无域 crate；CI 架构脚本通过。
- [ ] `extension-core` 无 `db*`/`remote_desktop`；`remote_file_editor` 无 `db_view`。
- [ ] `runtime_*` 按域拆分；`app_kit` 只收集 provider。
- [ ] `HomePage` 无 `IpcDriverRegistry`/`WslDistribution`/`PortForwardingRuntime` 字段。
- [ ] 三个 `Product` 的 `identity`/`features`/`domains` 独立。
- [ ] 三个 data dir/keychain/更新 feed/安装器 ID 独立。
- [ ] 导入向导只读源库，且有 fixture 测试。
- [ ] 全量 app 的 bundle id/数据目录/更新源未变。
- [ ] 253 处 `include_str!` 测试全部迁移或改写。
- [ ] 三产品在 6 平台安装/启动/更新 smoke 通过。

## 17. 实施进度（2026-10-29）

分支：`refactor/product-split`（基于 `dev@3dbe5f87a`）。

| 提交 | 内容 | 状态 |
| --- | --- | --- |
| `8093ed9d9` | 本文档 | 已完成 |
| `312aa8e8b` | `navop_runtime` 的 `database`/`redis`/`sftp` 可选特性 | 已完成，三种组合均通过 `cargo check`/`cargo test` |
| `d83316cb9` | `TabItem::from_dyn` + 单测 | 已完成 |
| `dc3c7e98d` | `domain_api`：`AppIdentity`/`Product`/`Domain`/`TabIntent`/`DomainRegistry` | 已完成，5 个单测 |
| `e4181254e` | `HomePage` 实现 `TabHost`/`HomeContext` | 已完成，`main` 编译通过 |
| `e12c7945b` | `extension-runtime` 的 `database`/`remote-desktop` 可选特性 | 已完成，默认/无域/仅 wasm 三种组合编译通过 |
| `62d38c9ce` | Shell 侧下游关闭扩展默认域特性 | 已完成，`terminal_view` 闭包已无 `db`/`db_view`/`redis-runtime`/`mongodb-runtime` |
| `4bc1361c8` | 抽出 `crates/ssh_form`（SSH 表单 + 主机密钥 UI + MFA） | 已完成，51 个测试通过 |
| `ca747569b` | `app_kit`：按产品身份配置数据目录/keychain；修复 ssh_form 移动后的源码断言路径 | 已完成，`app_kit` 3 个测试、`one-core` 962 个测试通过 |
| `09d8bd1c2` | `app_paths` 支持产品级便携标记/开发目录/缓存目录；`main` 启动前应用 `FULL` 身份 | 已完成，`main` 以 `-Dwarnings` 编译通过 |
| `fa0a70f9d` | 新增 `apps/navop-shell`、`apps/navop-data` 骨架可执行包，按身份打开占位窗口 | 已完成，两者以 `-Dwarnings` 编译通过 |
| `6ef0f60a6` | `main` 实现 `Product`，改走 `app_kit::bootstrap`；`Domain::init` 支持错误传播 | 已完成，`main` 以 `-Dwarnings` 编译通过 |
| `2c7ba2897` | 新增 `shell_domain`，KnownHosts 通过 `DomainRegistry` 打开；修正启动顺序源码断言 | 已完成，`main` 767 个测试通过 |
| `296b0f76c` | `apps/navop-shell`/`apps/navop-data` 改为 `Product` 启动；`TabOpener` 增加打开模式 | 两个 bin 以 `-Dwarnings` 编译通过 |
| `7295e415a` | 新增 `data_domain`，Redis 单连接经 `ConnectionRegistry` 打开；全量产品注册 DataDomain | 已完成，`main` 767 个测试通过 |
| `87b76d56f` | `HomeContext` 增加工作区查询与连接排序；Redis 工作区标签迁入 `data_domain` | 已完成，`main` 767 个测试通过 |
| `6a7f202bb` | `HomeContext` 增加数据库打开模式；Database 单连接/工作区标签迁入 `data_domain` | 已完成，`main` 767 个测试通过 |
| `93b073e56` | Mongo 单连接/工作区标签迁入 `data_domain` | 已完成，`main` 767 个测试通过 |
| `6f53fbecf` | 本地终端经 `DomainRegistry` 打开；`TabHost` 增加聚焦打开，`HomePage` 按 id 去重；`TabOpener` 返回 `bool` 并拿到 `HomeContext` | 已完成，`main` 767 个测试、`shell_domain` 3 个测试通过 |
| `f0cb91d75` | SSH 终端经 `ConnectionOpener` 打开；新增 `app_kit::ShellHost` 事件桥，SFTP/保存临时连接回到当前 `HomePage`；`HomeContext::keep_subscription` 保管事件订阅 | 已完成，`main` 767 个测试、`shell_domain` 4 个测试通过 |
| `36b4818ee` | SFTP 双栏视图经 `ConnectionOpener::open_sftp` 打开；`ShellHost` 增加 SFTP→本地终端/SSH 终端回调，`HomePage` 保留原标签序号与聚焦行为 | 已完成，`main` 767 个测试，改动 crate 以 `-Dwarnings` 通过 clippy |
| `9a5455c47` | 端口转发连接经 `ConnectionOpener` 打开；`HomeContext` 暴露 `port_forwarding_runtime`，领域与宿主共享隧道运行时 | 已完成，`main` 767 个测试、`shell_domain` 5 个测试通过 |
| `0209bdb91` | `TabHost::open_tab_lazy` 惰性创建；数据库/Redis/Mongo/端口转发已存在同 id 标签时只激活，不重复建连 | 已完成，`main` 767 个测试，`domain_api`/`shell_domain`/`data_domain` 测试通过 |
| `5796eafa3` | RDP/VNC 经 `RemoteDesktopConnectionOpener` 打开；凭据解析、选项构造、稳定标签 id 与惰性视图创建迁入 `shell_domain` | 已完成，`main` 767 个测试、`shell_domain` 7 个测试通过 |
| `cb6e54b4b` | 会话日志页面从 `main/src/session_logs` 迁入 `shell_domain`；`.cast` 打开经 `ShellHost::open_session_log_file` 回到宿主 | 已完成，`main` 760 个测试（7 个随模块迁移）、`shell_domain` 15 个测试通过 |
| `d3930398c` | 新增 `runtime_core`：应用信息工具与工作区工具从 `navop_runtime` 抽出，`navop_runtime` 保持兼容 re-export | 已完成，`runtime_core`/`navop_runtime` 测试通过 |
| `7a29ea95a` | 新增 `runtime_database`：`database_tools` 及其 metadata/schema/tests 整体迁出，`navop_runtime::database_tools` 改为 re-export | 已完成，`runtime_database`/`navop_runtime`/`main` 编译通过，`main` 760 个测试通过 |
| `e7d3d48f6` | 新增 `runtime_redis`、`runtime_sftp`；Redis/SFTP 工具与测试整体迁出，`navop_runtime` 去掉 `redis-runtime`/`sftp`/`ssh` 直接依赖，只保留 re-export 门面 | 已完成，两个新 crate 测试通过，`main` 760 个测试通过 |
| `4a9c41b57` | `main` 抽成 `navop_app` 库并删除自身 bin；新增 `apps/navop` 全量包，`default-members` 指向它；release 构建改 `-p navop`；补 `shell_domain` 的 `Common.cancel/delete` 语言包 | 已完成，`main` 760 个测试 + 3 个集成测试通过，`navop`/`navop-shell`/`navop-data` 以 `-Dwarnings` 编译通过 |
| `8480e16b7` | 打包校验脚本与文档命令更新到 `apps/navop` | 已完成，`node script/test-release-packaging.mjs` 34/34 |
| `05195c80a` | `navop-shell` 装配真实主页：连接列表、Shell 类型过滤、本地终端/KnownHosts 入口、`TabHost`/`HomeContext` 实现 | 已完成，`navop-shell` 以 `-Dwarnings` 编译并通过 clippy |
| `d572a14ed` | `HomeContext::port_forwarding_runtime` 改为 `Option`，Data 产品无需端口转发运行时 | 已完成，main 760 tests、shell/data/domain 测试通过 |
| `731d97209` | `navop-data` 装配真实主页：Database/Redis/MongoDB 连接列表、打开入口、按设置排序与打开模式 | 已完成，`navop-data` 以 `-Dwarnings` 编译、clippy 通过，8 个排序测试通过 |
| `8a30e7ab9` | `extension-runtime` 增加 `&mut App` 级安装流水线 `run_install_with_progress_prompt_app` 与 `prompt_install_database_driver_app` | 已完成，`extension-runtime` 227 个测试通过（1 个既有 fixture 失败） |
| `6b1efc09d` | `data_domain` 接管数据库驱动检查与安装提示；`navop-data` 初始化扩展运行时 | 已完成，`main` 760 tests、`data_domain` 4 tests，`navop-data` 启动冒烟通过 |
| `205d97f55` | `app_kit::bootstrap` 在路径未初始化时调用 `initialize_app_paths`，独立产品不再依赖 main 的启动路径 | 已完成，`main` 760 tests、`app_kit` 3 tests 通过 |
| `5974b0d3a` | macOS 应用包按产品生成：`navop`（Navop/navop/com.onetcli.app/保留文档类型）、`navop-shell`（Navop Shell/navop-shell/com.navop.shell/无文档类型）、`navop-data`（Navop Data/navop-data/com.navop.data/无文档类型）；DMG 文件名改按二进制名 | 已完成，`script/test-bundle-macos.sh` 三产品通过，`test-release-packaging.mjs` 34/34 |
| `04fdc8c95` | 新增 Shell/Data 的 Linux desktop、图标与 `script/package-linux-product.sh`；产出普通 tar 与带产品 portable marker 的便携包 | 已完成，脚本本地打包验证通过；包装测试 35/35 |
| `ed7614d22` | 新增参数化 WiX 安装器 `navop-product.wxs`/`navop-product-bundle.wxs` 与 `build-product-installer.ps1`；Shell/Data 使用独立 UpgradeCode、安装目录与注册表键；CI 矩阵加入 Windows x64 | 已完成，XML/YAML 与 38 项打包测试通过；本机无 `wix`/`pwsh`，未实际编译 MSI |

| `355be3cf7` | 新增 `.github/workflows/standalone-products.yml`：Shell/Data × macOS arm64/x64 × Linux x64 的 6 个构建任务，复用产品级 macOS/Linux 打包脚本 | 已完成本地 YAML 解析与包装测试 36/36；尚未在 GitHub Actions 实跑 |
| — | 三产品启动冒烟：`navop-shell`/`navop-data` 用临时 `HOME` 启动 8 秒，无 panic，分别创建 `.navop-shell-dev`/`.navop-data-dev` 目录，Data 完成 `one-hub.db` 迁移 | 通过；真实窗口交互与连接打开仍待人工验证 |

尚未开始：Windows 安装器参数化与 CI 实跑；独立产品 workflow 已增加但未实跑，Linux 包仍用 runner 原生 glibc；`DriverGate` 的 Data 路径已接管，Main 仍保留原驱动守卫；`navop-shell`/`navop-data` 的启动冒烟已通过，真实连接打开仍待人工验证；runtime 层四个包（core/database/redis/sftp）已拆完，`navop_runtime` 现在是薄门面，`apps/navop` 全量包已能依赖 `navop_app` 库。`shell_domain` 的域打开已覆盖 SSH/SFTP/端口转发/RDP/VNC/会话日志，剩余是扩展驱动守卫（`DriverGate`）与数据库驱动安装流程。注意：构建期间磁盘曾满，已清理 `target` 与 mbx 增量/action 缓存；后续构建缓存会重建。

已知问题：`extension-runtime` 的 `db_tree_action_runs_registered_wasm_component` 在本机因 fixture 指向 `.wat` 文本而失败（`bad magic number`），与本次改动无关；其余 227 个测试通过。`ssh_form` 暂用 `edition = "2021"` 以保持与原实现一致，后续随 `terminal_view` 一起升级。

下一步建议：参数化 Windows 安装器（独立 UpgradeCode/ProductCode），再补真实连接打开验证与最终交付说明。

## 18. 交付状态（2026-10-29）

### 18.1 已交付

- 分支：`refactor/product-split`，基于 `dev@3dbe5f87a`，未合并 dev。
- 三个可构建产品：
  - `apps/navop` → 全量 `navop`；
  - `apps/navop-shell` → `navop-shell`；
  - `apps/navop-data` → `navop-data`。
- 共享层：`app_kit`（启动、身份、领域注册表、宿主事件桥）、`domain_api`（Product/Domain/TabHost/HomeContext/ConnectionOpener/TabOpener/AppIdentity）。
- 领域层：`shell_domain`（SSH/SFTP/端口转发/RDP/VNC/本地终端/KnownHosts/会话日志）、`data_domain`（Database/Redis/MongoDB，含驱动检查与安装提示）。
- 运行时层：`runtime_core`、`runtime_database`、`runtime_redis`、`runtime_sftp`；`navop_runtime` 只保留兼容门面。
- `main` 已抽成 `navop_app` 库；原 bin 由 `apps/navop` 提供。
- 独立 SSH 表单：`crates/ssh_form`，`terminal_view` 通过 re-export 保持兼容。
- 身份隔离：`AppIdentity` 统一配置 app id、bundle id、数据目录、keychain、portable marker；三个产品分别使用 `navop`、`navop-shell`、`navop-data` 身份。
- 打包脚本：macOS `.app`/`.dmg` 三产品参数化；Linux desktop、图标、普通/便携 tar；Windows 参数化 WiX MSI + Burn bundle。

### 18.2 验证结果

| 验证 | 结果 |
|---|---|
| `cargo build -p navop -p navop-shell -p navop-data`（`-Dwarnings`） | 通过 |
| `cargo test -p main --lib` | 760 passed |
| `main` 集成测试 | 3 passed |
| `domain_api` / `app_kit` / `shell_domain` / `data_domain` | 5 / 3 / 15 / 4 passed |
| `runtime_core` / `runtime_database` / `runtime_redis` / `runtime_sftp` | 0 / 1 / 5 / 17 passed |
| `ssh_form` | 51 passed |
| `script/test-bundle-macos.sh` | 三产品通过 |
| `script/test-release-packaging.mjs` | 38 / 38 通过 |
| 三产品临时 `HOME` 启动冒烟 | 无 panic；Data 完成 SQLite 迁移 |

已知既有失败：`extension-runtime` 的 `db_tree_action_runs_registered_wasm_component` 因 fixture 指向 `.wat` 文本而失败（`bad magic number`），与拆分无关。

### 18.3 运行方式

```bash
cargo run -p navop        # 全量
cargo run -p navop-shell  # Shell
cargo run -p navop-data   # Data
```

开发模式下三个产品分别使用 `.navop-dev`、`.navop-shell-dev`、`.navop-data-dev` 数据目录。

### 18.4 已知限制

- Windows MSI/Burn 未在真实 WiX 环境编译；本机无 `wix`/`pwsh`，只做了 XML、参数和身份唯一性校验。
- `standalone-products.yml` 已提交到分支，但 GitHub 的 `workflow_dispatch` 要求 workflow 位于默认分支；分支上直接 dispatch 返回 404，需要合并后或临时放到默认分支才能实跑。
- macOS 包默认 ad-hoc 签名，未接 Developer ID/公证；三产品更新 feed 尚未隔离，独立产品不应复用全量的更新清单。
- Linux 包使用构建机 glibc，尚未接主 Release 的 `cargo zigbuild ... .2.28` 基线。
- `main` 仍保留旧域名打开路径作为未注册时的 fallback，以及旧的数据库驱动守卫；`navop-shell`/`navop-data` 已不再走这些 fallback。
- 两个独立产品已具备新建/编辑/复制/删除连接、搜索、基础设置页与会话日志入口，但仍不含 AI、Notes、Team、凭据库、云同步、扩展市场与主题级外观自定义；这些属于全量产品。
- 独立产品暂不支持「从全量产品导入连接」，首次使用需在新产品里重新建连接（或手工复制数据目录）。
- 启动冒烟只验证进程存活与目录创建，真实连接、窗口交互仍需人工验证。

### 18.5 后续建议

1. 把 `standalone-products.yml` 合入默认分支后跑一次 Windows/Linux/macOS 矩阵，按真实日志修 WiX 与 Linux 打包。
2. 为三个产品隔离更新 feed，并补 macOS 公证与 Windows 代码签名。
3. 逐步删除 `main` 中已被领域注册表覆盖的 fallback，防止两套路径长期漂移。
4. 给 `navop-shell`/`navop-data` 补“从全量导入连接”的迁移入口（设置页已完成，见 §20）。

## 19. Public MCP 三产品隔离（2026-10-29）

### 19.1 实现

- 新增 `crates/mcp_host`：
  - 从 `main/src/public_mcp_runtime/config.rs` 迁入 `PublicMcpStartConfig` / `PublicMcpEnvOverride`，main 改为 re-export，避免两端配置漂移。
  - `register_toolset(toolset, default_enabled, factory)`：领域 crate 只声明本域工具，`mcp_host` 负责按产品、设置和 `ToolExposureToolsetSettings` 组装。
  - `start(cx, identity)`：读取 `AppSettings`，按注册表构建 `PublicMcpToolRegistry`，经 `Tokio::spawn_result` 启动 `PublicMcpRuntime`；`stop`/`is_running`/`client_count`/`last_error` 提供生命周期查询。
  - 独立产品的核心工具集默认开启，避免没有完整设置页时 Data 产品默认暴露 0 个数据库工具。
- `public_mcp::discovery` 增加 `DiscoveryIdentity { app, legacy, file_name }` 与 `configure_discovery_identity`：
  - `DiscoveryDocument::new`、`legacy_compatible`、`validate_for_stdio_bridge`、`public_mcp_discovery_path*` 全部改为运行时身份。
  - 默认仍是 `navop` + `onetcli`，全量产品行为不变。
- `AppIdentity` 增加 `mcp_legacy_app_name`：Full 为 `Some("onetcli")`，Shell/Data 为 `None`。
- `app_kit`：
  - `configure_identity` 同时配置 discovery 身份。
  - `bootstrap` 在路径初始化后设置 canonical/legacy discovery 路径。
  - `Product::mcp_managed_by_app_kit` 默认 `true`；`FullProduct` 覆盖为 `false`，继续由 main 的 `public_mcp_runtime` 管理。
  - `bootstrap` 在产品 `init` 后读取 `ProductFeatures.public_mcp`：true 调 `mcp_host::start`，false 调 `stop`。`ProductFeatures` 不再是死配置。
- 工具下沉：
  - `shell_domain` 注册 Terminal（terminal_exec/read/control/write_keys/remote_ops）与 SFTP 工具集。
  - `data_domain` 注册 Database 工具集，以及 Redis/Mongo 的 live-state provider；`main/src/public_mcp_runtime/{redis,mongo}.rs` 已迁入 `data_domain::mcp`，main 改为复用，避免两套实现。
  - `shell_domain` 与 `data_domain` 都注册 connection 管理工具集（`McpToolset::Connections`，复用 `navop_runtime::connections`），`mcp_host` 按工具集去重，全量产品同时注册两个域时不会重复。
- 审批复用：
  - `main/src/public_mcp_approval/*` 抽成独立 `mcp_approval` crate，窗口解析改为 `active_window`/首个窗口，不再依赖 main 的 `resolve_navop_window`。
  - main 与 `navop-shell`/`navop-data` 都调用 `mcp_approval::init`；`mcp_host` 用它的 `PublicMcpApprovalManager` 启动 server，因此独立产品的 `Ask` 模式会弹同一个审批对话框。
  - 无窗口时仍走默认拒绝。
- 独立产品主页增加 `MCP 已连接 / 启动中 / 关闭` 切换按钮：直接持久化 `AppSettings.mcp.server_enabled` 并立即 start/stop；`is_starting` 防止重复点击启动两个 server。
- 工具暴露设置真正生效：
  - `McpSettings` 增加 `toolsets_initialized`；独立产品首次启动时，`mcp_host::enable_default_toolsets` 把本产品核心工具集写入 `AppSettings.tool_exposure.mcp`，之后以设置为准，不再用 `default_enabled` 强制打开。
  - `mcp_host::register_toolset` 去掉 `default_enabled` 参数；`build_registry` 只按 `ToolExposureToolsetSettings` 过滤。
  - 主页新增 `MCP 工具` 对话框，Terminal/SFTP/Connections（Shell）或 Database/Redis/Mongo/Connections（Data）可逐项开关；每次修改持久化并重启 server。

### 19.2 验证

| 验证 | 结果 |
|---|---|
| `navop-shell` + `NAVOP_PUBLIC_MCP=1` 启动 | `.navop-shell-dev/state/public-mcp.json`，`app=navop-shell`，loopback 随机端口 |
| `navop-data` + `NAVOP_PUBLIC_MCP=1` 启动 | `.navop-data-dev/state/public-mcp.json`，`app=navop-data` |
| `navop` 全量启动 | 仍由 main 启动，`app=navop`，未出现第二个 mcp_host server |
| `cargo test -p main --lib` | 750 passed（config 的 10 个测试随文件迁到 mcp_host） |
| `cargo test -p main --tests` | 3 个集成测试通过 |
| `mcp_host` / `mcp_approval` / `domain_api` / `app_kit` / `shell_domain` / `data_domain` | 12 / 10 / 5 / 3 / 15 / 12 passed |
| `cargo check -p navop -p navop-shell -p navop-data`（`-Dwarnings`） | 通过 |
| `cargo clippy --no-deps -p mcp_host -p app_kit -p domain_api -p shell_domain -p data_domain -- -D warnings` | 通过；`public_mcp` 仍命中既有 `tool_runtime_adapter.rs:134` 的 `too_many_arguments` 基线 lint |

### 19.3 启停与权限

- 默认仍关闭：需 `NAVOP_PUBLIC_MCP=1` 或设置里启用。
- 独立产品已复用通用审批弹窗；`PermissionMode::Ask` 会请求审批。无窗口或超时会拒绝。
- 无设置页时可通过主页的 MCP 按钮启停；也可用环境变量强制：
  - `NAVOP_PUBLIC_MCP=1`
  - `NAVOP_PUBLIC_MCP_PERMISSION=allow`（跳过审批，仅用于本地调试）
- 发现文件权限仍为 0600，只监听 loopback；token 每次启动重新生成。

### 19.4 尚未完成

- 独立产品没有完整 MCP 设置页；主页的 `MCP 工具` 对话框只能开关工具集，不能改权限模式/审批超时。
- `toolsets_initialized` 是进程级设置字段：首次启动后即写入 true；如果用户清空设置文件，下次启动会重新写入本产品默认工具集。
- 全量产品仍走 main 的 `public_mcp_runtime`，两条实现并存；后续应让 main 也改用 `mcp_host`，或明确保留 main 的高级能力（resource pool/access guard/审批）。
- 三产品的工具暴露设置目前共用 `AppSettings.tool_exposure.mcp`；独立产品默认值由 `default_enabled` 兜底，还没有产品级 UI。

提交：`0a5b4e472`、`cbb72d954`、`0c958ddcf`、`0c56a0b98 feat(mcp): 工具集开关接入设置并增加逐项 UI`。

## 20. 独立产品基础能力补齐（2026-10-29）

目标：保持三产品代码隔离的前提下，让 `navop-shell`/`navop-data` 从“已有连接查看器”变成可独立交付的产品。领域仍然只依赖 `domain_api` 与自己的领域 crate，`app_kit` 不反向依赖任何领域。

### 20.1 契约扩展（`domain_api`）

- `kind.rs`：`ConnectionKindDescriptor { id, label: fn(&App) -> SharedString, group, order }` 与 `ConnectionKindGroup::{Database, Terminal, Remote, Extension}`。侧栏筛选、新建连接左栏分组都读同一份种类清单。
- `form.rs`：`ConnectionFormProvider`，由领域声明本产品支持哪些连接种类（`kinds`）、已有连接属于哪个种类（`kind_for`）、如何构造表单（`build`）。`ConnectionFormRequest` 承载编辑/复制/预选工作区/保存回调。
- `settings.rs`：`SettingsSectionProvider::pages(&mut App) -> Vec<(i32, SettingPage)>`。返回 `SettingPage` 而不是 `AnyView`，因为 gpui-kit 的页面只能由 `Settings` 容器渲染（`SettingPage::render` 是 `pub(super)`），顺带复用内建侧栏与搜索。
- `DomainRegistry` 增加 `register_form_provider`/`register_settings_provider`，以及 `kinds`/`kind_for_connection`/`build_connection_form`/`settings_pages` 四个聚合入口。

### 20.2 共享外壳（`app_kit`）

| 模块 | 职责 |
|---|---|
| `new_connection.rs` | 连接向导：左栏按 `ConnectionKindGroup` 分组列出本产品种类，右栏渲染领域表单；支持新建、编辑（`editing_connection`）、复制（`initial_connection` + 清空 id/云端字段）与预选工作区 |
| `connection_actions.rs` | `open_new_connection`/`open_new_connection_in_workspace`/`edit_connection`/`duplicate_connection`/`delete_connection`；删除走确认弹窗，成功后广播 `ConnectionDataEvent::ConnectionDeleted` |
| `settings.rs` | 设置弹窗 = 通用页（外观：主题/语言；Public MCP：开关与权限模式；关于：产品名与数据目录）+ 各领域 `SettingsSectionProvider` 页面 |
| `host.rs`/`lib.rs` | 原有 `ShellHost`、身份配置、领域注册表保持不变；新增 `identity()` 供设置页“关于”读取产品身份 |

所有表单都承载在 `open_reusable_popup_window` 的独立弹窗里：表单保存后自身调用 `close_window_after_save`，若嵌在主窗口标签里会误关主窗口。

### 20.3 领域补齐

- `shell_domain`
  - `connection_forms/`：SSH/SFTP、FTP、Serial、Telnet、端口转发、RDP、VNC 七种 `kind`，`kind_for` 是纯函数便于单测。
  - `connection_openers.rs`：新增 `FtpConnectionOpener`（双栏 SFTP 视图）、`SerialConnectionOpener`/`TelnetConnectionOpener`（`TerminalWorkspace::new_serial_with_index`/`new_telnet_with_index`）。此前列表里能看到 Serial/Telnet/FTP，但点击“打开”会失败。
  - `settings.rs`：终端设置分区（字体、字号、回滚行数、自动复制、自动补全）。
- `data_domain`
  - `connection_forms.rs`：`db:{DatabaseType}` × `builtin_all()` + `redis` + `mongodb`，`kind_for` 用 `to_db_connection().db_type` 反查；`build` 复用 `ConnectionFormWindow`/`RedisFormWindow`/`MongoFormWindow`。
  - `settings.rs`：数据库设置分区（打开方式、连接排序等），读写 `AppSettings`。
- 两个产品主页：工具栏“新建连接/设置”、搜索框、行内编辑/复制/删除（右键菜单）、空列表引导；宿主订阅 `ConnectionDataEvent` 刷新列表，Shell 另补“会话日志”入口。

### 20.4 验证结果

| 验证 | 结果 |
|---|---|
| `cargo test -p domain_api -p app_kit -p shell_domain -p data_domain` | 10 / 3 / 20 / 13 passed |
| `cargo build -p navop` | 通过（仅既有 linker 警告） |
| `cargo build -p navop-shell -p navop-data` | 通过（无 rustc 警告，仅既有 linker 警告） |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| `navop-shell`/`navop-data` 临时 `HOME` 启动 | 进程存活 12 秒、无 panic、日志无报错 |

### 20.5 仍未覆盖

- 从全量产品导入连接（迁移向导）。
- 数据库 AI、AI Chat、Notes/Team/Toolbox、扩展市场、云同步与账户、主密钥解锁 UI。
- Tab 持久化、窗口状态、托盘、单实例、自动更新、文件关联、全局热键。
- 独立产品更新 feed 与 macOS 公证/Windows 签名（见 §18.4）。

## 21. 独立产品能力对齐第二批（2026-10-29）

目标变为「独立产品该有的都要有」之后，第二批把 `main` 里的系统级能力整体下沉到
`app_kit`，三产品共用同一份实现，`main` 只留薄 shim 保持既有调用点与行为不变。

### 21.1 下沉的能力（`crates/app_kit`）

| 模块 | 能力 | main 侧 |
|---|---|---|
| `actions.rs` | 可配置快捷键的动作清单（id / 标题 / 默认键位 / 平台差异） | 无 |
| `settings/` | 通用设置页：外观（主题、字体、缩放、语言）、快捷键、Public MCP、安全与凭据、关于 | `setting_tab.rs` 保留全量产品的完整设置 |
| `import/` | 连接导出/导入（JSON bundle）与从全量产品 `one-hub.db` 只读迁移 | 无 |
| `integration/window_state.rs` | 窗口位置/尺寸持久化（关闭时保存 + 20s 周期保存） | 无（全量走 `AppSettings.main_window_state`） |
| `integration/shortcuts.rs` | `cmd-n` 新建连接、`cmd-,` 设置、标签关闭/切换等绑定 | 各产品自行 `cx.bind_keys` |
| `integration/single_instance.rs` | 三平台单实例门禁（Windows 命名管道 / Unix socket），二次启动转发 `show` | `windows_single_instance.rs` shim |
| `integration/tray.rs` | 系统托盘：会话菜单、显示/隐藏主窗口、退出走既有确认流程 | `system_tray.rs` shim，注入 `TrayHost` |
| `tabs.rs` | Tab 会话快照保存/恢复（复用 `one_core` 的 `TabContainerState`） | 无 |
| `update/` | 自动更新：检查/下载/校验/安装，feed 按产品隔离 | `update/mod.rs` shim |
| `file_association.rs` | 文件关联注册/注销，扩展名由 `AppIdentity.file_associations` 派生 | `file_association.rs` shim |
| `vault/` | 主密钥首次设置/解锁/修改/重置、凭据库列表与表单 | `master_key_flow.rs`、`master_key_dialogs.rs` 与 `credential_vault/` shim |

### 21.2 关键决策

1. **Tab 恢复不复用 `TabContainer::load`**：`load` 走的是不订阅内容事件的私有路径，
   恢复出来的标签收不到 `TabContentEvent`。改为逐条 `add_tab_with_mode(_, Background, _)`
   （公开加法会订阅内容事件），全部加完后再激活快照里的当前标签；`PortForwarding`
   标签依赖主页 runtime，恢复时跳过。
2. **更新 feed 按产品隔离**：全量产品维持 `{base}/updates/latest.json`；独立产品走
   `{base}/updates/{binary_name}/{channel}/latest.json`，`NAVOP_UPDATE_URL` 仍可整体覆盖。
   版本号用 `update::set_current_version()` 由各 bin 注入，避免 `env!("CARGO_PKG_VERSION")`
   在 `app_kit` 里取到错的包版本。
3. **文件关联默认 no-op**：SHELL/DATA 的 `AppIdentity.file_associations` 为空数组，
   `windows_registry_plan` 在 `specs.is_empty()` 时直接返回空，不再写
   `Applications\{binary}.exe\shell\open\command`，避免独立产品抢占 `.db/.duckdb`。
4. **托盘/单实例靠宿主注入，不反向依赖 main**：`TrayHost` 注入主窗口句柄、原生显示/
   隐藏、标签激活与退出请求；`app_kit` 不依赖 `main` 的任何模块，独立产品注入默认实现。
5. **主密钥门禁前移到"新建/编辑/复制连接"入口**：与全量产品一致，先
   `vault::ensure_master_key_ready`，未设置或未解锁时弹设置/解锁弹窗，本次不打开表单。

### 21.3 独立产品接入点

- `apps/navop-shell/src/main.rs`、`apps/navop-data/src/main.rs`：单实例门禁 →
  `window_state::initial_bounds` → 开窗 → `set_global(GlobalTabContainer)` →
  `window_state::attach` + `shortcuts::install` + `single_instance.install` + `tray::install`。
- 两个主页 `Home::new` 末尾调 `app_kit::tabs::install`，恢复上次会话的标签。
- 侧栏工具栏新增「导入连接」；设置页新增「安全与凭据」（凭据库、修改/重置主密钥）
  与「快捷键」（可编辑、可一键恢复默认）。

### 21.4 验证结果（2026-10-29）

| 验证 | 结果 |
|---|---|
| `cargo test -p app_kit` | 141 passed |
| `cargo test -p domain_api -p shell_domain -p data_domain` | 18 / 11 / 24 passed |
| `cargo test -p main --lib` | 见交付说明（托盘/更新/文件关联源码守卫已按新实现改写） |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过 |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| `navop-shell` / `navop-data` 临时 `HOME` 启动 | 进程存活、无 panic |

### 21.5 仍未覆盖（下一批）

- 账户登录与个人云同步（`main/src/auth.rs`、`personal_sync_*` 尚未下沉）。
- Shell 工作区文件浏览/编辑器、会话日志完整入口已接但工作区浏览器未下沉。
- Data 的数据库 AI 能力与 AI Chat、Notes/Team、扩展市场、主题级外观自定义。
- 独立产品的更新 feed 实际发布、macOS 公证与 Windows 签名。

## 22. 统一到全量主页（2026-10-29，方向修正）

### 22.1 为什么改

§20、§21 的独立产品是「薄壳主页 + 领域开放入口」，功能永远追不上全量产品：
用户需要的设置、标签、AI、团队、扩展市场都要在薄壳里再实现一遍。方向修正为
**现有功能全部保留，包括主页；只把「产品差异」做成门控**：

- 三个产品共用同一套 `navop_app` 主页、设置、标签体系与系统集成；
- 差异来自 `ProductKind`（身份、领域包、能力集合），而不是另写一套 UI；
- 隔离更彻底：每个产品有自己的 `one-hub.db`、数据目录、keychain、MCP 发现
  文件与更新 feed，互不读取；连接通过迁移向导导入；
- 文件关联仍只归全量产品（`file_associations` 为空的产品不注册）。

### 22.2 改动

- `main/src/lib.rs`
  - 新增 `pub enum ProductKind { Full, Shell, Data }`，集中给出 `identity()`
    （`AppIdentity::{FULL,SHELL,DATA}`）、`features()`、`domains()`
    （Full = Shell + Data，Shell/Data 只装自己的领域包）、
    `mcp_managed_by_app_kit()`（全量产品仍走自己的 `public_mcp_runtime`）。
  - `run()` → `run_product(kind)` → `run_product_with_version(kind, version)`：
    原 `run()` 函数体参数化。身份在最前面配置（早于更新 CLI、数据目录、单实例
    与窗口创建）；`set_current_version`/`db::ipc::set_host_version`/
    `extension_runtime::set_current_host_version` 使用产品自己的包版本
    （独立产品 `0.1.0`，避免与 `main` 的 `0.22.0` 混用导致更新判断错误）。
  - 单实例：全量产品沿用 Windows 命名管道门禁；独立产品统一走
    `app_kit::integration::single_instance::acquire_for_product`（macOS/Linux
    用 unix socket，Windows 用命名管道），已有实例时直接退出。
  - 文件关联：`is_installed()` 且 `file_associations` 非空才注册。
  - 托盘：`TrayConfig` 标题取当前身份显示名，三产品同机可区分。
  - `public_mcp_runtime` 只对全量产品启动，独立产品用 `mcp_host`，避免两套 MCP
    抢同一个发现文件；`app_kit::bootstrap` 首次启动按领域包写入默认工具集
    （Shell = Terminal/Sftp/Connections，Data = Database/Redis/Mongo/Connections），
    之后由用户在设置里接管。
- `main/src/product.rs`（新）：`is_full_product()` 与
  `connection_type_supported(ConnectionType)`。内置类型以「当前产品的领域包是否
  注册了打开入口」为准（`app_kit::registry().has_connection_opener`），扩展连接
  始终保留（由扩展运行时打开）。
- 门控接入点：主页列表/最近/侧栏树（`home_tab::connection_filter::
  match_connection_type`）、类型筛选条与菜单（`connection_type_menu::
  builtin_filter_types`）、新建连接清单（`new_connection::connection_kind::
  all_with_registry` + `connection_type()`）、快速打开
  （`home::home_connection_quick_open::update_items`）。因此：
  `navop-shell` 只出现 SSH/SFTP/FTP/串口/Telnet/端口转发/RDP/VNC，
  `navop-data` 只出现数据库/Redis/MongoDB，全量产品不受影响。
- 三个 bin：`apps/navop-shell`、`apps/navop-data` 只调用
  `navop_app::run_product_with_version(ProductKind::Shell/Data, env!("CARGO_PKG_VERSION"))`，
  依赖收敛为 `main`；旧薄壳 `apps/*/src/home.rs`、`home/`、`sort.rs` 删除。
- 「从全量产品导入连接」入口：新建连接下拉菜单里对独立产品展示
  （`Home.import_from_full_product` → `app_kit::import::open_import_dialog`），
  设置页「迁移」组同样只在独立产品展示。

### 22.3 数据隔离

| 项目 | navop | navop-shell | navop-data |
|---|---|---|---|
| 数据目录（开发） | `.navop-dev` | `.navop-shell-dev` | `.navop-data-dev` |
| `one-hub.db` | 自己的 | 自己的（首次为空） | 自己的（首次为空） |
| keychain service | `com.navop.desktop` | `com.navop.desktop.shell` | `com.navop.desktop.data` |
| MCP 发现文件 | `public-mcp.json`（app=`navop`） | 同左（app=`navop-shell`） | 同左（app=`navop-data`） |
| 更新 feed | `updates/latest.json` | `updates/navop-shell/shell/latest.json` | `updates/navop-data/data/latest.json` |
| 文件关联 | 注册 `.db/.duckdb/.md/.cast` | 不注册 | 不注册 |

开发态若设置了 `NAVOP_DATA_DIR`（本仓库 `.env.local` 默认指向 `~/.navop-dev`），
它会覆盖上述开发目录——三产品会共享同一个数据目录，这仅是本地调试便利，
打包（release）构建不读取 `.env*`。

### 22.4 验证结果（2026-10-29）

| 验证 | 结果 |
|---|---|
| `cargo check -p main --lib` | 0 error / 0 warning |
| `cargo test -p main --lib` | 657 passed, 0 failed |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过 |
| `HOME=<临时> NAVOP_DATA_DIR=<临时> navop-shell` / `navop-data` 启动 | 均存活 9s、无 panic，`Settings saved to <临时>/data/config/settings.json` |

### 22.5 仍未覆盖

- 账户登录与个人云同步、Notes/Team、扩展市场在独立产品中按全量能力保留，
  这是产品要求（见 §23.3），不做入口收敛。
- Shell 工作区文件浏览/远程文件编辑器、Data 的数据库 AI 入口尚未按产品验证。
- 独立产品的更新 feed 实际发布、macOS 公证与 Windows 签名。

## 23. 主页按领域收敛（2026-10-29，第二轮）

### 23.1 判定标准

独立产品与全量产品的差异只按「领域」区分，不按产品名硬编码：

- `main/src/product.rs::supports_database_domain()`：领域注册表里有没有数据库
  连接 opener（全量产品恒为 `true`）。
- `main/src/product.rs::supports_shell_domain()`：有没有 SSH 连接 opener。

两个判定只用来裁剪「数据库专属」与「终端专属」的界面；其余能力三产品一致。

### 23.2 收敛点

| 位置 | 数据库产品 | 终端产品 |
|---|---|---|
| 连接列表 / 最近 / 侧栏树 / 快速打开 | 保留数据库类 | 保留终端类 |
| 类型筛选条、新建连接清单 | 只列数据库类 | 只列终端类 |
| 扩展贡献的连接（RDP provider）与空类目安装入口 | 隐藏 | 保留（空类目安装入口隐藏） |
| 设置 → 数据库页 | 保留 | 隐藏 |
| 设置 → 终端页 | 隐藏 | 保留 |
| 设置 → 通用页「远程文件编辑器」分组 | 隐藏 | 保留 |
| 主页应用导航「已知主机」「会话日志」 | 隐藏 | 保留 |
| 主页工具栏「本地终端」按钮与快捷键 | 隐藏 | 保留 |
| 设置页跳转（同步页序号） | 按实际页数动态计算 | 同左 |

### 23.3 明确不隔离的部分

AI 工作台、笔记、团队、扩展市场、Toolbox、凭据库、云同步、账户、主密钥、
主题/语言/快捷键、Tab 恢复、托盘、单实例、自动更新在三个产品中完全一致，
这是产品要求：独立产品只做「数据库 / 终端」的领域隔离。

### 23.4 验证结果（2026-10-29）

| 验证 | 结果 |
|---|---|
| `cargo check -p main --lib` | 0 error / 0 warning |
| `cargo test -p main --lib` | 657 passed, 0 failed |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过，无 rustc 警告 |
| 两个独立产品临时 `HOME` 启动 8s | 均存活、无 panic |

### 23.5 仍未覆盖

- 数据库驱动的扩展市场分类、Data 侧数据库 AI 入口的实际可用性验证。
- Shell 工作区文件浏览在独立产品中的入口验证。
- 更新 feed 的实际发布、macOS 公证与 Windows 签名。

## 24. main 拆分：应用自组装 + 领域标签页收敛（2026-10-29，第三轮）

### 24.1 方向

用户要求「把 main 拆为公共组件，然后每个应用自己创建自己的 crate 来组装」。本轮先落两步，
让「产品装配」从 main 内部移到各应用，再让 main 不再自带领域标签页实现。

### 24.2 应用自组装

| 文件 | 内容 |
|---|---|
| `main/src/lib.rs` | `pub fn run_with<P: domain_api::Product + 'static>(product, version)`；`pub struct AppAssets`；`pub use navop_app::init as init_app` |
| `apps/navop/src/main.rs` | `FullProduct`（FULL 身份、`mcp_managed_by_app_kit() = false`、domains = Shell + Data） |
| `apps/navop-shell/src/main.rs` | `ShellProduct`（SHELL 身份、domains = ShellDomain、MCP 由 app_kit 托管） |
| `apps/navop-data/src/main.rs` | `DataProduct`（DATA 身份、domains = DataDomain、MCP 由 app_kit 托管） |

`run_with` 内部按 `product.identity()` 决定身份、单实例、文件关联与窗口标题，因此应用自己
声明 `Product` 即可复用全量主页；`main` 不再内置产品枚举。

### 24.3 删除 main 的新建连接向导

`main/src/new_connection/`（约 1750 行）整目录删除，`HomePage::show_new_connection_dialog`
改为委托 `app_kit::connection_actions::open_new_connection_in_workspace`；连接类型清单与
表单全部由领域注册表（`ConnectionFormProvider`）提供。`crates/core/src/window_close.rs`
的源码契约测试同步移除该文件条目。

### 24.4 领域标签页收敛

`main/src/home/home_tabs.rs` 中 8 个入口（`open_ssh/serial/telnet/sftp/remote_desktop/
redis/mongodb` 与 `add_item_to_tab_with_mode`）改为只调用 `app_kit::registry()`；
未注册类型给出「当前产品不支持打开该连接」提示。随之删除 main 内直接构造
`DatabaseTabView`/`RedisTabView`/`MongoTabView`/`SftpView`/`RemoteDesktopView` 的
fallback 代码与 17 个只覆盖 fallback 的测试（对应行为已由 `shell_domain`/`data_domain`
的 opener 测试覆盖）。

### 24.5 验证结果

| 验证 | 结果 |
|---|---|
| `cargo check -p main --lib --tests` | 0 error / 0 warning |
| `cargo test -p main --lib` | 622 passed, 0 failed（较 657 少的是向导与 fallback 测试） |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过，无 rustc 警告 |

### 24.6 剩余拆分路线（按依赖移除顺序）

1. `main/src/home_tab/connection_forms.rs` 编辑/复制流程改走 `app_kit::connection_actions`，
   仅保留扩展连接表单，删掉 `db_view`/`redis_view`/`mongodb_view`/`terminal_view`/
   `port_forwarding_view`/`remote_desktop_view` 的表单构造。
2. 设置页数据库/终端页改由 `SettingsSectionProvider` 提供，删除 `main/src/settings/
   {database_settings,local_terminal_settings,sql_format_settings}.rs`。
3. 领域初始化搬到 `Domain::init`：`db::init_cache`/`db_view::*`/`redis_view::init`/
   `mongodb_view::init` → `DataDomain`；`terminal_view::init`/`remote_desktop_view::init`/
   SSH 会话服务 → `ShellDomain`；`Domain` 增加默认空实现的 `refresh_keybindings`。
4. 迁移 `main` 其余领域 UI：`home_tab/local_terminal.rs`、`resource_workbench_terminal.rs`、
   `home/ai_workbench*.rs`、`file_open/recording.rs`、`public_mcp_runtime/*`、
   `persistent_connection_sidebar`、`connection_visuals.rs`、`home/connection_import_*`。
5. 从 `main/Cargo.toml` 删除领域 crate 依赖，验收依赖树：
   `cargo tree -p navop-shell | grep -E "db_view|redis_view|mongodb_view"` 为空、
   `cargo tree -p navop-data | grep -E "terminal_view|sftp_view|remote_desktop_view"` 为空。

## 25. 领域 feature 拆分落地（2026-10-29，第三批）

第二十四节留下的「删领域 Cargo 依赖」没有直接删，而是改用**编译期 feature + 领域契约**
完成：`main` 仍是唯一公共应用壳，但默认构建才同时装配数据库与终端；独立产品按需打开。

### 25.1 `main` 的领域 feature

```toml
default  = ["wasm-components", "windows-native-rdp", "shell-plugins", "database", "shell"]
database = ["extension-runtime/database", "navop_runtime/database", "navop_runtime/redis", "dep:db", ...]
shell    = ["extension-runtime/remote-desktop", "navop_runtime/sftp", "dep:terminal", ...]
```

- 领域 crate（`db`/`db_view`/`redis_view`/`mongodb_view`/`terminal`/`terminal_view`/`sftp_view`/
  `ssh`/`known_hosts_view`/`port_forwarding_view`/`remote_desktop_view`/`data_domain`/`shell_domain`）
  在 `main/Cargo.toml` 里全部改成 `optional = true`，由 feature 引入。
- 三个应用各自选 feature：`navop` 用默认（both），`navop-shell` 用 `default-features = false,
  features = [..., "shell"]`，`navop-data` 用 `[..., "database"]`。
- 第三方能力同样门控：`extension-runtime` 的 `database`/`remote-desktop`、`navop_runtime` 的
  `database`/`redis`/`sftp`（workspace 级 `default-features = false`，各产品显式打开）。
  `windows-native-rdp` 改成弱依赖形式 `remote_desktop_view?/windows-native-rdp`，
  数据产品不再被 RDP 依赖拖入 `remote_desktop_view`。

### 25.2 门控落点（`#[cfg(feature = "database")]` / `shell`）

| 位置 | 处理 |
|---|---|
| `home/home_strategy.rs` | 数据库/Redis/Mongo 策略、RDP/VNC/SSH/FTP/Serial/Telnet 策略按领域 cfg；扩展终端启动拆出 `_shell` 版本 |
| `home/home_tabs.rs` | shell 终端/SFTP/远程桌面/本地终端/已知主机/会话日志入口按 shell cfg，Redis/Mongo 入口按 database cfg |
| `home/home_connection_quick_open.rs` | 临时连接「保存为连接」按 shell cfg |
| `home/ai_workbench.rs` | 终端面板与面板工厂按 shell cfg（工作台其余面板保留） |
| `home_tab/domain_host.rs` | `ShellHost` 实现整体 shell cfg，`port_forwarding_runtime()` 双分支 |
| `home_tab/connection_forms.rs` | 临时连接表单 shell cfg，其余表单早在第二十四节交还注册表 |
| `persistent_connection_sidebar` | SFTP/终端/全屏远程桌面菜单项与 `TerminalColors` 按 shell cfg（非 shell 给禁用项） |
| `public_mcp_runtime` | terminal/redis/mongo 工具集与资源池、访问守卫按领域 cfg |
| `settings/appearance.rs`、`home_tab/local_terminal.rs`、`local_terminal_profiles.rs` | 终端主题/本地终端 UI 按 shell cfg |
| `file_open.rs`、`navop_app.rs` | 会话日志/录制、SFTP 粘贴键位、mongo 原生驱动工厂按领域 cfg |
| `lib.rs`（`AppAssets`） | 驱动图标/资产改走 `app_kit::visuals::ConnectionVisuals`（由 `data_domain` 注入），公共壳不再依赖 `db::ipc` |
| `connection_visuals.rs` | 驱动图标改走 `app_kit::visuals` + `one_ui::fonts`（字体表下沉），SSH 自定义图标用通用 helper |

### 25.3 依赖树验收（真实输出）

| 命令 | 结果 |
|---|---|
| `cargo tree -p navop-shell -e normal \| grep -E "db_view v\|redis_view v\|mongodb_view v\| db v0"` | 0 行 |
| `cargo tree -p navop-data -e normal \| grep -E "terminal_view v\|sftp_view v\|remote_desktop_view v\|known_hosts_view v\|port_forwarding_view v"` | 0 行（`ssh` 仍保留：数据库 SSH 隧道 `connection_tunnel` 需要） |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过，0 rustc 警告 |
| `cargo test -p main --lib` | 611 passed, 0 failed |
| `cargo test -p app_kit -p domain_api -p shell_domain -p data_domain` | 141 / 18 / 28 / 11 passed |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| 临时 HOME 启动 `navop-shell` / `navop-data` | 均 RUNNING_OK，无 panic |

### 25.4 与「拆成多个 crate」的关系

`main` 仍然是单一 crate，但领域代码不再随产物编译：公共壳部分（HomePage、标签页、
设置容器、账户、AI、笔记、团队、扩展、凭据库、云同步）保持共享，领域实现留在家门
crate 或按 feature 排除。后续若要进一步拆 crate，可把 `home_tabs`/`home_strategy`
的领域分支整体搬进 `shell_domain`/`data_domain`（现在由 cfg 隔离，迁移风险更低）。

## 二十六、领域 UI 继续下沉：从「cfg 隔离」走向「注册表 + 领域钩子」（2026-10-30，第四批）

§25 让产物依赖树完成了隔离，但 `main/src` 里仍留着大量 `#[cfg(feature = ...)]` 的领域
UI（161 处）。本批把这些入口改造成「公共壳调注册表/领域钩子」，把实现搬进
`shell_domain` / `data_domain`，cfg 数量降到 100 处、领域引用文件 21 → 13 个。

### 26.1 新增/扩展的契约

| 契约 | 位置 | 作用 |
|---|---|---|
| `TabIntent::{SessionLogs, KnownHosts, Terminal, ...}` | `domain_api` | 主页导航、本地终端菜单统一按意图打开标签页 |
| `ShellExtras::wsl_distributions` / `refresh_wsl_distributions` | `domain_api::shell_extras` | 公共壳渲染本地终端菜单时查询 WSL 发行版，不再导入 `terminal` |
| `ShellExtras::port_forwarding_runtime` | 同上 | 端口转发运行时由 `shell_domain` 持有，`HomeContext` 只读钩子 |
| `ShellExtras::open_remote_desktop_fullscreen` / `launch_remote_desktop_external` | 同上 | 远程桌面全屏窗口与 Windows 系统客户端由领域实现 |
| `ConnectionFormRequest` + `app_kit::connection_actions::open_form` | `domain_api` / `app_kit` | 通用表单宿主：任何连接类型都通过注册表构建表单 |
| `WorkbenchTerminalProvider` | `ai_chat_view::workbench` | 工作台终端面板由 shell 领域注入，数据产品无终端面板 |
| `HomeContext::{touch_connection_last_used, set_selected_connection}` | `domain_api::home` | 领域打开连接后回报宿主状态 |

### 26.2 迁移落点

| 原位置（`main/src`） | 去向 |
|---|---|
| 会话日志 / 已知主机导航 | `home_tab/navigation.rs` 改为 `registry().open_tab`，删除 `home_tabs::add_*_tab` 与 `KnownHostsPage` 兜底 |
| 本地终端菜单与 WSL | 菜单只产出 `TerminalProfileDescriptor`；配置解析、WSL、自定义 profile 全在 `shell_domain::TerminalTabOpener` |
| 连接打开策略（Ssh/Database/Redis/Mongo/Serial/Telnet/PortForwarding/RDP/VNC） | 删除 9 个策略与 2 个 extension 守卫实现，统一 `RegistryOpenStrategy` |
| 导入窗口逐类型编辑器（SSH/DB/Redis/Mongo） | `connection_import_window/editor.rs` 改用 `open_form`，删除 `ImportEditorContext` 与 4 类编辑器 |
| 资源工作台终端宿主（`resource_workbench_terminal.rs`） | `shell_domain::workbench_terminal`（`NativeTerminalHost` + `WorkbenchTerminalProvider`） |
| 端口转发运行时 | `shell_domain::shell_extras` 持有，`HomePage` 字段删除 |
| 远程桌面窗口 + mstsc 凭据（547 行） | `shell_domain::remote_desktop_window`（含 Windows mstsc 启动与凭据解码，9 个测试随之迁移） |

### 26.3 验证（真实输出）

| 命令 | 结果 |
|---|---|
| `cargo test -p main --lib` | 595 passed, 0 failed |
| `cargo test -p shell_domain --lib` | 37 passed |
| `cargo test -p app_kit / domain_api / data_domain` | 141 / 18 / 11 passed |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| `cargo check -p main --lib`（默认 both） | 0 error / 0 warning |
| `cargo check -p main --lib --no-default-features --features "wasm-components,windows-native-rdp,shell-plugins,shell"` | 0 error / 0 warning |
| `cargo check -p main --lib --no-default-features --features "wasm-components,windows-native-rdp,shell-plugins,database"` | 0 error / 0 warning |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过，无 rustc 警告 |
| 依赖树互斥计数（shell 不含 db*/redis*/mongo*，data 不含 terminal*/sftp*/remote_desktop*/known_hosts*/port_forwarding_view） | 0 / 0 |
| 临时 HOME 启动两个独立产品 | RUNNING_OK |

### 26.4 仍未迁移（下一步）

- `home/home_tabs.rs` 中标签复制对 `TerminalWorkspace` 的下转型、`duplicate_active_tab` 的终端分支。
- `persistent_connection_sidebar` 的终端配色、`settings/appearance.rs` 的终端主题卡片
  （属共享 UI 里的小块领域渲染，可继续用 cfg 或抽 `ShellExtras::terminal_palette`）。
- `public_mcp_runtime` 的 terminal/redis/mongo 工具集实现（可搬进领域包的 MCP 注册函数）。
- `file_open/recording.rs`（终端录制）与 `home_tab/encryption.rs`（主密钥门禁）的 shell 分支。
- 之后的最终形态仍是 §24 的方案：把 HomePage 骨架抽成 `app_shell`，用 `HomeProfile` /
  `HomeSection` 让每个产品装配自己的主页微调。

## 二十七、main 去掉最后一处领域 `use`（2026-10-30，第五批）

§26 之后 `main/src` 仍在 6 个文件里直接 `use` 领域 crate（终端配色、终端主题卡片、
录制文件打开、标签复制、临时连接保存、SFTP 粘贴键位、Redis 活动连接）。本批把这些
全部改造成「注册表 + 领域钩子」，`main/src` 已**没有任何 `use 领域crate::…`**。

### 27.1 新增/扩展的契约

| 契约 | 位置 | 作用 |
|---|---|---|
| `FileOpenHandler { handles, open }` + `DomainRegistry::open_file` | `domain_api::file_open` | 文件关联打开先问领域（终端录制 `.cast` 归 shell） |
| `ShellExtras::duplicate_terminal_tab` | `domain_api::shell_extras` | 终端标签复制：领域自己读工作区连接并重开 |
| `ShellExtras::open_terminal_in_dir` | 同上 | SFTP 双栏「在本地终端打开 / 在此连接开终端」不再由公共壳构造终端 |
| `ShellExtras::open_session_log` | 同上 | 会话日志窗口的打开由 shell 领域实现 |
| `ShellExtras::save_temporary_connection` | 同上 | 临时连接「保存为连接」走领域内的 SSH 表单 |
| `ShellExtras::sidebar_colors` / `SidebarColors` | 同上 | 侧栏配色由领域按终端配色算好，公共壳只存 `SidebarPalette` |
| `ShellExtras::terminal_theme_section` | 同上 | 外观设置里的终端主题卡片由领域渲染成 `AnyElement` 插槽 |
| `Domain::keybindings` + `app_kit::domain_keybindings` | `domain_api::domain` | 领域自带键位（如 SFTP 粘贴），公共壳统一 bind |
| `ActiveResourceProvider` + `ActiveResourceIds` | `domain_api::resources` | 公共 MCP 资源池需要「当前活跃连接」时由领域提供快照（Redis 已接） |
| `DomainRegistry::active_resources` | 同上 | 聚合所有领域快照，按 `kind` 分类 |

### 27.2 迁移落点

| 原位置（`main/src`） | 去向 |
|---|---|
| `file_open/recording.rs`（338 行终端录制） | `shell_domain::recording`（含 `.cast` 判定、sha2 稳定键、`RecordingFileOpenHandler`） |
| `home_tabs::duplicate_active_tab` 的终端分支 | `shell_domain::ShellExtrasImpl::duplicate_terminal_tab` |
| `home_tabs::open_sftp_local_terminal` / `open_sftp_ssh_terminal` | `shell_domain::terminals::open_local_terminal_with_dir` + `SshConnectionOpener::open_ssh_terminal` |
| `home_tab/domain_host::open_session_log_file` | `ShellExtras::open_session_log` |
| `home_tab/connection_forms::show_save_temporary_connection_form` | `ShellExtras::save_temporary_connection`（SSH 表单 + 主密钥门禁） |
| `persistent_connection_sidebar` 的 `TerminalColors` 转换 | `ShellExtras::sidebar_colors` |
| `settings/appearance::render_terminal_palette` 等（3 个函数） | `shell_domain::terminal_theme_section` |
| `navop_app` 的 SFTP 粘贴 `KeyBinding` | `Domain::keybindings`（`sftp_view::PasteUpload` + `SFTP_VIEW_CONTEXT`） |
| `public_mcp_runtime::resource_pool` 的 `GlobalRedisState` | `data_domain::active_resources::RedisActiveResourceProvider` |

### 27.3 依赖与配置收口

- `main/Cargo.toml` 删除 6 个已无引用的 optional 依赖：`db`、`ssh`、`sftp_view`、
  `known_hosts_view`、`port_forwarding`、`port_forwarding_view`（`builtin-duckdb`
  改由 `db_view/builtin-duckdb` 转发）。
- 领域 cfg 从 161 处降到 **57 处**（shell 45 / database 12），全部集中在
  `public_mcp_runtime` 的工具集装配、Mongo 原生驱动工厂、扩展连接终端启动与
  远程桌面协议映射。

### 27.4 验证（真实输出）

| 命令 | 结果 |
|---|---|
| `cargo test -p main --lib` | 589 passed, 0 failed |
| `cargo test -p shell_domain --lib` | 42 passed |
| `cargo test -p app_kit / domain_api / data_domain` | 141 / 18 / 11 passed |
| `cargo test -p main --test i18n_locale_keys_contract` | 1 passed |
| `cargo build -p navop -p navop-shell -p navop-data` | 通过，无 rustc 警告 |
| 依赖树互斥计数 | 0 / 0 |
| 临时 HOME 启动 `navop-shell` / `navop-data` | RUNNING_OK |

### 27.5 剩余

- `public_mcp_runtime` 的 data/shell 工具集装配与 Mongo 原生驱动工厂（建议直接合并两套
  MCP 实现，让全量产品也走 `mcp_host`）。
- `home/home_strategy` 的扩展连接终端启动、`home_tab/connection_open` 的远程桌面协议映射。
- `main::ProductKind`（产品装配最终要移到 `apps/*`）。
- CI 增加依赖树 guard job。

## §26 产品装配移出 main（2026-10-30，dev）

### 26.1 改动

- 删除 `main/src/lib.rs` 的 `ProductKind` / `ProductProfile` / `run()` / `run_product()` /
  `run_product_with_version()`：产品装配（身份 + 领域包 + MCP 托管策略）现在**只**存在于
  各应用 crate 的 `Product` 实现里，`main` 只保留与产品无关的 `run_with(product, version)`。
- 删除 `main/src/main.rs`：`main` 包只提供 `navop_app` 库，不再有同名 bin。入口统一为
  `apps/navop|navop-shell|navop-data`（文档里的 `cargo run -p main` 已改 `cargo run -p navop`）。
- `shell_domain` 从 `main/Cargo.toml` 的 `shell` feature 与依赖表移除：`main` 不再构造
  `ShellDomain`。`data_domain` 暂留（公共 MCP 的 Redis/Mongo 工具集实现仍在
  `data_domain::mcp`）。

### 26.2 验证

- `cargo test -p main --lib` → 591 passed（`domain_features_only_pull_their_own_optional_dependencies`
  同步更新：shell feature 断言 `dep:terminal_view` 且**不含** `dep:shell_domain`）。
- `cargo build -p navop -p navop-shell -p navop-data` 无 error、无 rustc 警告。
- `cargo test -p main --test i18n_locale_keys_contract` 与
  `--test product_dependency_isolation` 均通过。

### 26.3 Mongo 原生驱动工厂跟进

`init_native_data_driver_factories` 也迁到了 `DataDomain::init`（`data_domain` 新增同名
`builtin-mongodb` feature，`main` 用弱依赖 `data_domain?/builtin-mongodb` 转发）。顺序不变：
领域初始化 → 公共 MCP 工具注册表。

### 26.4 剩余

`data_domain` 依赖（公共 MCP 的 Redis/Mongo 工具集实现）、`ProductFeatures`/`AppIdentity`
常量重复、两套 MCP 实现合并、`public_mcp_runtime` 的领域 cfg 收敛。
