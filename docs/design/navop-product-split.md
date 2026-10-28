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
| — | 三产品启动冒烟：`navop-shell`/`navop-data` 用临时 `HOME` 启动 8 秒，无 panic，分别创建 `.navop-shell-dev`/`.navop-data-dev` 目录，Data 完成 `one-hub.db` 迁移 | 通过；真实窗口交互与连接打开仍待人工验证 |

尚未开始：三产品打包与 CI 矩阵；`DriverGate` 的 Data 路径已接管，Main 仍保留原驱动守卫；`navop-shell`/`navop-data` 的启动冒烟已通过，真实连接打开仍待人工验证；runtime 层四个包（core/database/redis/sftp）已拆完，`navop_runtime` 现在是薄门面，`apps/navop` 全量包已能依赖 `navop_app` 库。`shell_domain` 的域打开已覆盖 SSH/SFTP/端口转发/RDP/VNC/会话日志，剩余是扩展驱动守卫（`DriverGate`）与数据库驱动安装流程。注意：构建期间磁盘曾满，已清理 `target` 与 mbx 增量/action 缓存；后续构建缓存会重建。

已知问题：`extension-runtime` 的 `db_tree_action_runs_registered_wasm_component` 在本机因 fixture 指向 `.wat` 文本而失败（`bad magic number`），与本次改动无关；其余 227 个测试通过。`ssh_form` 暂用 `edition = "2021"` 以保持与原实现一致，后续随 `terminal_view` 一起升级。

下一步建议：三产品主页均已装配；下一步补 `DriverGate`（数据库驱动安装/提示从 main 解耦），再做三产品 GUI 冒烟、打包与 CI 矩阵。
