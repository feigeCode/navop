//! 扩展能力面的 nativeModule 形态（nsh 标准机制）。
//!
//! nsh 给页面两条 JS → Rust 的路：
//!
//! 1. `host.call(name, …)` / `host.post` / `host.on`——字符串名字的通道，
//!    navop 的 provider 面（`navop.resource.*` / `navop.job.*` /
//!    `navop.event.*`）走这条；
//! 2. `nativeModule(specifier)`——宿主注册的 **UI set** 里的 `#[module]`，
//!    页面拿到的是强类型对象（`nativeModule<{ Db: DbApi }>("navop").Db`），
//!    函数直接调 Rust，参数由 nsh 逐个检查、类型不对抛 `TypeError`。
//!
//! 能力面（数据库 / 终端 / 应用导航 / UI）走第二条：见本模块的 `Db`、
//! `Terminal`、`App`、`Ui` 四个模块。它们都从 [`SurfaceApi::current`] 取
//! 当前 surface 的上下文（扩展身份、声明的能力面、清单权限、主线程队列、
//! 会话），再复用宿主既有的实现（`ExtensionDbGateway`、终端注册表）。
//!
//! ## 为什么按 module 而不是按扩展注册
//!
//! nsh 的 UI set 是**进程级**注册（`nsh_platform_gpui::set_ui_sets`），不能
//! 逐 surface 差异化。所以注册的是同一套模块，鉴权放在函数里：每个函数
//! 先看当前 surface 的清单有没有声明对应能力面（`modules: ["db"]`）、
//! 再按清单权限（`db:read:*` …）判定，缺一即拒。这与 `host.call` 时代
//! 的 `gate()` 等价，只是位置从通道分发挪到了函数入口。
//!
//! ## 页面侧形状
//!
//! ```text
//! const { Db, Terminal, App, Ui } = nativeModule<{ … }>("navop");
//! await Db.listConnections();
//! await Db.execute({ sessionId, sql, maxRows });
//! await Terminal.read({ sessionId, lines });
//! await App.open({ route, context });
//! await Ui.notify({ level, title, message });
//! ```

use std::sync::Arc;

use nsh_core::element::{Registry, engine_context};
use serde_json::Value;

use super::dispatch::MainThread;
use super::session::NshSession;

/// 能力面注册的 UI set 名字：页面写 `nativeModule("navop")`。
pub(crate) const UI_SET: &str = "navop";

/// 一个 surface（tab / 面板）的宿主上下文，经 `AppOptions::context` 注入，
/// 模块函数用 [`SurfaceApi::current`] 取回。
///
/// 只放**与 surface 同生命周期**的东西：身份与清单（`HostAbi` 里已有）、
/// 主线程队列、会话。数据库状态是进程级的（`abi_db::set_db_state`）。
#[derive(Clone)]
pub(crate) struct SurfaceApi {
    pub(crate) abi: Arc<super::abi::HostAbi>,
    /// `"tab"` / `"panel"`：与 `host.params.surface` 一致。
    ///
    /// 页面从 `host.params` 已经能读到，这里留一份给将来的宿主侧分支用
    /// （例如按 surface 路由终端事件），暂时没人读。
    #[allow(dead_code)]
    pub(crate) surface: &'static str,
    pub(crate) panel: Option<String>,
    pub(crate) anchor: Option<String>,
    pub(crate) dispatcher: MainThread,
    /// 页面自己的 provider 会话；模块里的能力面目前都用不上它，留作
    /// cancellation / 资源订阅的入口。
    #[allow(dead_code)]
    pub(crate) session: Arc<NshSession>,
    /// 页面所在的页签容器（`None` = 主窗口页签栏）。
    #[allow(dead_code)]
    tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
    /// 本 surface 打开的数据库会话句柄（页面关了即失效）。
    #[cfg(feature = "nsh-db")]
    pub(crate) db_plane: Arc<super::abi_db::DbPlane>,
}

impl SurfaceApi {
    pub(crate) fn new(
        abi: Arc<super::abi::HostAbi>,
        surface: &'static str,
        dispatcher: MainThread,
        session: Arc<NshSession>,
    ) -> Self {
        Self {
            abi,
            surface,
            panel: None,
            anchor: None,
            dispatcher,
            session,
            tabs: None,
            #[cfg(feature = "nsh-db")]
            db_plane: Arc::new(super::abi_db::DbPlane::new()),
        }
    }

    pub(crate) fn with_panel(mut self, panel: Option<String>, anchor: Option<String>) -> Self {
        self.panel = panel;
        self.anchor = anchor;
        self
    }

    /// 记下页面所在的页签容器（`App.open` 用它保持在同一标签页里）。
    pub(crate) fn with_tabs(
        mut self,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
    ) -> Self {
        self.tabs = tabs;
        self
    }

    pub(crate) fn extension_id(&self) -> &str {
        &self.abi.extension_id
    }

    /// 当前引擎线程上的 surface 上下文（nsh 在引擎线程进入）。
    pub(crate) fn current() -> Result<Arc<Self>, String> {
        engine_context::<Self>()
            .ok_or_else(|| "navop: 当前引擎没有宿主上下文（不是 navop 打开的页面）".to_string())
    }

    /// 第一道闸门：清单声明了这个能力面没有（`modules: ["db"]` → `navop.db`）。
    pub(crate) fn require(&self, module: &str) -> Result<(), String> {
        if self.abi.has_module(module) {
            return Ok(());
        }
        Err(format!(
            "PERMISSION_DENIED: 清单没有声明 `{module}` 能力面（contributes.shellApp.modules）"
        ))
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.abi.cancelled()
    }
}

/// 能力面的 UI set（注册进 `nsh_platform_gpui::set_ui_sets`）。
pub(crate) fn registry() -> Registry {
    let mut registry = Registry::ui_set(UI_SET);
    registry.module::<App>();
    registry.module::<Ui>();
    #[cfg(feature = "nsh-db")]
    registry.module::<Db>();
    #[cfg(feature = "nsh-terminal")]
    registry.module::<Terminal>();
    registry
}

/// 装上 navop 的能力面（连同 `nsh/base`；UI set 是整表替换）。
///
/// 在创建任何 `AppView` 之前调用一次（navop app 的 init 里）。
pub fn install() {
    nsh_platform_gpui::set_ui_sets(vec![nsh_base::registry(), registry()]);
}

/// 跨 surface 导航：`App.open({ extensionId?, route?, context? })`。
#[nsh_core::element::module]
pub(crate) struct App;

#[nsh_core::element::module]
impl App {
    /// 打开一个扩展页面（缺省是本扩展）。
    ///
    /// 与 `host.call("navop.app.open")` 同义：`(extensionId, route)` 相同则
    /// 复用已开的 tab，`context` 只在新建时注入（`host.params.context`）。
    pub(crate) async fn open(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.app")?;
        if api.cancelled() {
            return Err("CANCELLED: 页面正在关闭".to_string());
        }
        let opts = opts.unwrap_or(Value::Null);
        let extension_id = opts
            .get("extensionId")
            .or_else(|| opts.get("appId"))
            .and_then(Value::as_str)
            .unwrap_or(api.extension_id())
            .to_string();
        let route = opts
            .get("route")
            .and_then(Value::as_str)
            .filter(|route| !route.is_empty())
            .map(str::to_string);
        let context = opts.get("context").cloned().unwrap_or(Value::Null);
        // 页面在数据库标签页里时，跳转也留在同一个页签栏；容器已经关掉就退回主窗口。
        let tabs = api.tabs.as_ref().and_then(|tabs| tabs.upgrade());
        api.dispatcher
            .run(move |cx| match tabs {
                Some(tabs) => super::abi::open_app_in_container(
                    tabs,
                    &extension_id,
                    route,
                    context,
                    cx,
                ),
                None => super::abi::open_app_on_main(&extension_id, route, context, cx),
            })
            .await?;
        Ok(serde_json::json!({ "ok": true }))
    }
}

/// 宿主 UI：`Ui.notify({ level?, title, message? })`。
#[nsh_core::element::module]
pub(crate) struct Ui;

#[nsh_core::element::module]
impl Ui {
    /// 弹一条通知（主线程派发到活动窗口，与 `App.open` 同一套窗口解析）。
    pub(crate) async fn notify(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.ui")?;
        let level = opts
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or("info")
            .to_string();
        if !matches!(level.as_str(), "info" | "success" | "warning" | "error") {
            return Err("INVALID_ARGUMENT: level 只认 info/success/warning/error".to_string());
        }
        let title = opts
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| "INVALID_ARGUMENT: Ui.notify 需要 title".to_string())?
            .to_string();
        let message = opts
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        api.dispatcher
            .run(move |cx| {
                super::abi_ui::notify_on_main(&level, &title, &message, cx).map(|_| Value::Null)
            })
            .await?;
        Ok(serde_json::json!({ "ok": true }))
    }
}

/// 数据库：`Db.listConnections` / `openSession` / `execute` / `listDatabases`
/// / `listSchemas` / `closeSession` / `refreshTree`。
///
/// 权限与语句分级沿用宿主既有实现（`ExtensionDbGateway` + `db:*` 清单权限）：
/// 读写面与 WASM 扩展共用同一份判定，这里只是换了调用形态。
#[cfg(feature = "nsh-db")]
#[nsh_core::element::module]
pub(crate) struct Db;

#[cfg(feature = "nsh-db")]
#[nsh_core::element::module]
impl Db {
    /// 清单可见的数据库连接（需要 `db:connections:list`）。
    pub(crate) async fn list_connections() -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        super::abi_db::gateway(&api.abi, db_state)
            .list_connections()
            .map(super::abi_db::connections_json)
            .map_err(super::abi_db::db_failure)
    }

    /// 打开一个会话，返回不透明的 `sessionId`（`{ connectionId, database? }`）。
    pub(crate) async fn open_session(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let connection_id = required_str(&opts, "connectionId", "Db.openSession")?;
        let database = opts
            .get("database")
            .and_then(Value::as_str)
            .map(str::to_string);
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let resource = super::abi_db::gateway(&api.abi, db_state)
            .open_session(extension_component::protocol::OpenSessionRequest {
                connection_id,
                database,
            })
            .await
            .map_err(super::abi_db::db_failure)?;
        let session_id = api.db_plane.insert(resource);
        Ok(serde_json::json!({ "sessionId": session_id }))
    }

    /// 执行一条 SQL（`{ sessionId, sql, maxRows? }`）；写/结构变更成功后向
    /// 宿主事件 `db.treeChanged` 推一条，订阅方自行决定要不要重载树。
    pub(crate) async fn execute(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let session_id = required_str(&opts, "sessionId", "Db.execute")?;
        let sql = required_str(&opts, "sql", "Db.execute")?;
        let max_rows = opts
            .get("maxRows")
            .and_then(Value::as_u64)
            .map(|rows| rows.min(u64::from(u32::MAX)) as u32);
        let resource = api
            .db_plane
            .get(&session_id)
            .ok_or_else(|| "会话不存在或已关闭".to_string())?;
        let connection_id = resource.connection_id().to_string();
        let session_id_for_event = resource.session_id().to_string();
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let request = extension_component::protocol::ExecuteSqlRequest {
            session_id: resource.session_id().to_string(),
            connection_id: resource.connection_id().to_string(),
            sql,
            options: extension_component::protocol::ExecOptions {
                max_rows,
                timeout_ms: None,
                stream: false,
            },
        };
        let access = request.access();
        let batch = super::abi_db::gateway(&api.abi, db_state)
            .execute(request)
            .await
            .map_err(super::abi_db::db_failure)?;
        if matches!(
            access,
            extension_component::SqlAccess::Write
                | extension_component::SqlAccess::Schema
                | extension_component::SqlAccess::Admin
        ) {
            super::host_events::publish(
                "db.treeChanged",
                serde_json::json!({
                    "connectionId": connection_id,
                    "sessionId": session_id_for_event,
                    "reason": "execute",
                }),
            );
        }
        Ok(super::abi_db::batch_json(batch))
    }

    /// 连接可见的数据库名（`{ sessionId? , connectionId? }`）。
    pub(crate) async fn list_databases(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let opts = opts.unwrap_or(Value::Null);
        let (connection_id, _) = metadata_source(&api, &opts, "Db.listDatabases")?;
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let databases = super::abi_db::gateway(&api.abi, db_state)
            .list_databases(connection_id)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({ "databases": databases }))
    }

    /// 某个库里的 schema（`{ sessionId? , connectionId?, database }`）。
    pub(crate) async fn list_schemas(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let (connection_id, database) = metadata_source(&api, &opts, "Db.listSchemas")?;
        let database = database.ok_or_else(|| "INVALID_ARGUMENT: Db.listSchemas 需要 database".to_string())?;
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let schemas = super::abi_db::gateway(&api.abi, db_state)
            .list_schemas(connection_id, database)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({ "schemas": schemas }))
    }

    /// 列表/视图（`{ sessionId? , connectionId?, database?, schema? }`）。
    pub(crate) async fn list_tables(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let opts = opts.unwrap_or(Value::Null);
        let (connection_id, database) = metadata_source(&api, &opts, "Db.listTables")?;
        let schema = optional_str(&opts, "schema");
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let tables = super::abi_db::gateway(&api.abi, db_state)
            .list_tables(connection_id, database, schema)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({
            "tables": tables.into_iter().map(table_json).collect::<Vec<_>>(),
        }))
    }

    /// 描述一张表（`{ sessionId? , connectionId?, database?, schema?, table }`）：
    /// 列、索引、外键一次取全。
    pub(crate) async fn describe_table(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let (connection_id, database) = metadata_source(&api, &opts, "Db.describeTable")?;
        let schema = optional_str(&opts, "schema");
        let table = required_str(&opts, "table", "Db.describeTable")?;
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let metadata = super::abi_db::gateway(&api.abi, db_state)
            .describe_table(connection_id, database, schema, table)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({
            "columns": metadata.columns.into_iter().map(column_json).collect::<Vec<_>>(),
            "indexes": metadata.indexes.into_iter().map(index_json).collect::<Vec<_>>(),
            "foreignKeys": metadata.foreign_keys.into_iter().map(foreign_key_json).collect::<Vec<_>>(),
        }))
    }

    /// 库里的函数/过程（`{ sessionId? , connectionId?, database?, schema? }`）。
    pub(crate) async fn list_functions(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let opts = opts.unwrap_or(Value::Null);
        let (connection_id, database) = metadata_source(&api, &opts, "Db.listFunctions")?;
        let schema = optional_str(&opts, "schema");
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        let functions = super::abi_db::gateway(&api.abi, db_state)
            .list_functions(connection_id, database, schema)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({
            "functions": functions.into_iter().map(function_json).collect::<Vec<_>>(),
        }))
    }

    /// 关闭会话（`{ sessionId }`）。
    pub(crate) async fn close_session(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let session_id = required_str(&opts, "sessionId", "Db.closeSession")?;
        let mut resource = api
            .db_plane
            .take(&session_id)
            .ok_or_else(|| "会话不存在或已关闭".to_string())?;
        let db_state = super::abi_db::current_db_state()
            .ok_or_else(|| "数据库状态未初始化".to_string())?;
        super::abi_db::gateway(&api.abi, db_state)
            .close_session(&mut resource)
            .await
            .map_err(super::abi_db::db_failure)?;
        Ok(serde_json::json!({ "ok": true }))
    }

    /// 让宿主刷新已打开的数据库树（`{ connectionId? }`）。
    ///
    /// 返回刷新到的连接根数量；0 表示这棵树没打开（不是错误）。
    pub(crate) async fn refresh_tree(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.db")?;
        let connection_id = opts
            .as_ref()
            .and_then(|opts| opts.get("connectionId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let event_connection = connection_id.clone();
        let refreshed = api
            .dispatcher
            .run(move |cx| {
                Ok(serde_json::json!({
                    "ok": true,
                    "refreshed": super::abi_db::refresh_db_trees(cx, connection_id.as_deref()),
                }))
            })
            .await?;
        if let Some(connection_id) = event_connection {
            super::host_events::publish(
                "db.treeChanged",
                serde_json::json!({ "connectionId": connection_id, "reason": "refresh" }),
            );
        }
        Ok(refreshed)
    }
}

/// 终端：`Terminal.list` / `read` / `write` / `wait`。
///
/// 底层复用 Public MCP 的终端注册表（与 AI/MCP 工具同一套会话快照与输入
/// 通道）。`sessionId` 缺省取第一个已连接的本地/SSH 会话。
#[cfg(feature = "nsh-terminal")]
#[nsh_core::element::module]
pub(crate) struct Terminal;

#[cfg(feature = "nsh-terminal")]
#[nsh_core::element::module]
impl Terminal {
    /// 所有终端会话的元信息。
    pub(crate) async fn list() -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        api.dispatcher
            .run(|cx| {
                let Some(registry) = terminal_view::public_mcp::registry(cx) else {
                    return Err("终端注册表未初始化".to_string());
                };
                let sessions = registry
                    .list_sessions()
                    .into_iter()
                    .map(|session| {
                        serde_json::json!({
                            "sessionId": session.session_id,
                            "connectionId": session.connection_id,
                            "title": session.title,
                            "host": session.host_label,
                            "kind": format!("{:?}", session.connection_kind),
                            "connected": session.connected,
                            "cwd": session.cwd,
                            "rows": session.rows,
                            "cols": session.cols,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::json!({ "sessions": sessions }))
            })
            .await
    }

    /// 读终端快照（`{ sessionId?, lines? }`，默认 200 行）。
    pub(crate) async fn read(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        let session_id = opts
            .as_ref()
            .and_then(|opts| opts.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let lines = opts
            .as_ref()
            .and_then(|opts| opts.get("lines"))
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 2000) as usize;
        api.dispatcher
            .run(move |cx| super::abi_terminal::read_on_main(session_id.as_deref(), lines, cx))
            .await
    }

    /// 向终端写内容（`{ sessionId?, text }`）。
    pub(crate) async fn write(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        let text = required_str(&opts, "text", "Terminal.write")?;
        if text.is_empty() {
            return Err("INVALID_ARGUMENT: Terminal.write 的 text 不能为空".to_string());
        }
        let session_id = opts
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
        api.dispatcher
            .run(move |cx| super::abi_terminal::write_on_main(session_id.as_deref(), &text, cx))
            .await
    }

    /// 等终端输出变化（长轮询，`{ sessionId?, previous?, lines?, timeoutMs? }`）。
    ///
    /// 给了 `previous` 就等它变；没给就把当前快照当基线再等（`tail -f`）。
    pub(crate) async fn wait(opts: Option<Value>) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        let session_id = opts
            .as_ref()
            .and_then(|opts| opts.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let lines = opts
            .as_ref()
            .and_then(|opts| opts.get("lines"))
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 2000) as usize;
        let timeout = opts
            .as_ref()
            .and_then(|opts| opts.get("timeoutMs"))
            .and_then(Value::as_u64)
            .map(|ms| ms.clamp(200, 15_000))
            .unwrap_or(2_000);
        let mut baseline = opts
            .as_ref()
            .and_then(|opts| opts.get("previous"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let started = std::time::Instant::now();
        loop {
            let target = session_id.clone();
            let snapshot = api
                .dispatcher
                .run(move |cx| super::abi_terminal::read_on_main(target.as_deref(), lines, cx))
                .await?;
            let text = snapshot
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let changed = match baseline.as_deref() {
                Some(seen) => seen != text,
                None => {
                    baseline = Some(text.clone());
                    false
                }
            };
            if changed {
                return Ok(super::abi_terminal::with_change_flag(snapshot, true));
            }
            if started.elapsed() >= std::time::Duration::from_millis(timeout) {
                return Ok(super::abi_terminal::with_change_flag(snapshot, false));
            }
            if api.cancelled() {
                return Err("CANCELLED: 页面正在关闭".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }

    /// 在活动终端里执行一条命令并等输出。
    ///
    /// `{ command, sessionId?, submit?, waitForOutput?, readyTimeoutMs?, timeoutMs? }`
    ///
    /// 这是「把命令打进一个已经开着的终端」：会经过用户的 shell、复用它的
    /// 环境与 cwd，也能在 SSH 终端上跑。要拿结构化 stdout/stderr、或在
    /// 远端跑后台任务，用 [`Terminal::ssh_exec`]。
    pub(crate) async fn exec(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        let command = required_str(&opts, "command", "Terminal.exec")?;
        let registry = terminal_registry(&api).await?;

        // 不给会话时挑第一个已连接的：页面上最常见的是「往当前终端跑一条」。
        let target = match optional_str(&opts, "sessionId").or_else(|| optional_str(&opts, "target"))
        {
            Some(target) => target,
            None => registry
                .list_sessions()
                .into_iter()
                .find(|session| session.connected)
                .map(|session| session.session_id)
                .ok_or_else(|| "没有已连接的终端会话".to_string())?,
        };

        let request = public_mcp::terminal_exec::TerminalExecRequest {
            target: target.clone(),
            command,
            submit: opts.get("submit").and_then(Value::as_bool).unwrap_or(true),
            wait_for_output: opts
                .get("waitForOutput")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            ready_timeout_ms: opts
                .get("readyTimeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            timeout_ms: opts.get("timeoutMs").and_then(Value::as_u64),
        };
        // 页面调用被取消（关闭 tab/停用扩展）时，执行也跟着中止：
        // 宿主的取消令牌要桥接到 public_mcp 的 tokio 令牌上。
        let cancellation = tokio_util::sync::CancellationToken::new();
        let bridge = {
            let session_token = api.session.call_token();
            let bridge_token = cancellation.clone();
            api.session.tokio().spawn(async move {
                session_token.cancelled().await;
                bridge_token.cancel();
            })
        };
        let result = registry
            .terminal_exec(&target, request, cancellation)
            .await
            .map_err(|error| format!("终端执行失败: {error}"));
        bridge.abort();
        let result = result?;

        Ok(serde_json::json!({
            "sessionId": result.target,
            "command": result.command,
            "submitted": result.submitted,
            "completion": result.completion,
            "exitCode": result.exit_code,
            "output": result.output,
            "truncated": result.truncated,
            "capturedBytes": result.captured_bytes,
            "discardedBytes": result.discarded_bytes,
            "durationMs": result.duration_ms,
            "commandId": result.command_id,
        }))
    }

    /// 在 SSH 会话上执行命令（`{ sessionId, command, cwd?, env?, timeoutMs?, mode? }`）。
    ///
    /// 与 `exec` 的区别：这是一次单独的远端调用，stdout / stderr 分开返回，
    /// 可以带 cwd/env，也可以 `mode: "background"` 起后台命令（拿 `commandId`
    /// 轮询输出）。只支持暴露出来的 SSH 会话。
    pub(crate) async fn ssh_exec(opts: Value) -> Result<Value, String> {
        let api = SurfaceApi::current()?;
        api.require("navop.terminal")?;
        let session_id = required_str(&opts, "sessionId", "Terminal.sshExec")?;
        let command = required_str(&opts, "command", "Terminal.sshExec")?;
        let registry = terminal_registry(&api).await?;

        let env = match opts.get("env") {
            None | Some(Value::Null) => std::collections::BTreeMap::new(),
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
                .collect(),
            Some(_) => {
                return Err("INVALID_ARGUMENT: Terminal.sshExec 的 env 必须是字符串对象".to_string())
            }
        };
        let mode = match opts.get("mode").and_then(Value::as_str) {
            Some("background") => public_mcp::remote_ops::RemoteCommandMode::Background,
            Some("foreground") | None => public_mcp::remote_ops::RemoteCommandMode::Foreground,
            Some(other) => {
                return Err(format!(
                    "INVALID_ARGUMENT: Terminal.sshExec 的 mode 只能是 foreground/background（收到 {other}）"
                ))
            }
        };
        let request = public_mcp::remote_ops::RemoteExecRequest {
            session_id: session_id.clone(),
            command,
            cwd: optional_str(&opts, "cwd"),
            env,
            timeout_ms: opts.get("timeoutMs").and_then(Value::as_u64),
            mode,
        };

        // 前台执行会阻塞到命令结束：一定挪到 blocking 线程，不能占引擎线程。
        let target = session_id.clone();
        let result = api
            .session
            .tokio()
            .spawn_blocking(move || registry.remote_exec(&session_id, request))
            .await
            .map_err(|error| format!("终端执行任务失败: {error}"))?
            .map_err(|error| format!("终端执行失败: {error}"))?;

        Ok(serde_json::json!({
            "sessionId": target,
            "status": result.status,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exitCode": result.exit_code,
            "durationMs": result.duration_ms,
            "timedOut": result.timed_out,
            "commandId": result.command_id,
            "startedAtMs": result.started_at_ms,
        }))
    }
}

/// 取终端注册表：它只挂在主线程的 GPUI 全局上，取完就能在 tokio 线程使用。
#[cfg(feature = "nsh-terminal")]
async fn terminal_registry(
    api: &SurfaceApi,
) -> Result<public_mcp::registry::PublicMcpRegistry, String> {
    let registry = api
        .dispatcher
        .run_with(|cx| Ok(terminal_view::public_mcp::registry(cx)))
        .await?;
    registry.ok_or_else(|| "终端注册表未初始化".to_string())
}

/// 取必填字符串参数；缺失时给出带方法名的错误。
#[cfg(any(feature = "nsh-db", feature = "nsh-terminal"))]
fn required_str(opts: &Value, key: &str, method: &str) -> Result<String, String> {
    opts.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("INVALID_ARGUMENT: {method} 需要 {key}"))
}

/// 取可选字符串参数（空串按缺省处理）。
#[cfg(any(feature = "nsh-db", feature = "nsh-terminal"))]
fn optional_str(opts: &Value, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 解析元数据调用的数据源：显式 `connectionId`，或由 `sessionId` 推出连接。
///
/// 返回 `(connection_id, database)`；`database` 缺省时交给网关回落到连接配置。
#[cfg(feature = "nsh-db")]
fn metadata_source(
    api: &SurfaceApi,
    opts: &Value,
    method: &str,
) -> Result<(String, Option<String>), String> {
    let database = optional_str(opts, "database");
    if let Some(connection_id) = optional_str(opts, "connectionId") {
        return Ok((connection_id, database));
    }
    let Some(session_id) = optional_str(opts, "sessionId") else {
        return Err(format!(
            "INVALID_ARGUMENT: {method} 需要 sessionId 或 connectionId"
        ));
    };
    let resource = api
        .db_plane
        .get(&session_id)
        .ok_or_else(|| "会话不存在或已关闭".to_string())?;
    Ok((resource.connection_id().to_string(), database))
}

/// 表/视图的元数据形状。
#[cfg(feature = "nsh-db")]
fn table_json(table: db::types::TableInfo) -> Value {
    let kind = match table.object_type {
        db::types::TableObjectType::Table => "table",
        db::types::TableObjectType::View => "view",
        db::types::TableObjectType::ForeignTable => "foreign_table",
    };
    serde_json::json!({
        "name": table.name,
        "kind": kind,
        "schema": table.schema,
        "comment": table.comment,
        "engine": table.engine,
        "charset": table.charset,
        "collation": table.collation,
    })
}

/// 列的元数据形状。
#[cfg(feature = "nsh-db")]
fn column_json(column: db::types::ColumnInfo) -> Value {
    serde_json::json!({
        "name": column.name,
        "typeName": column.data_type,
        "nullable": column.is_nullable,
        "primaryKey": column.is_primary_key,
        "autoIncrement": column.is_auto_increment,
        "defaultValue": column.default_value,
        "comment": column.comment,
        "charset": column.charset,
        "collation": column.collation,
    })
}

/// 索引的元数据形状。
#[cfg(feature = "nsh-db")]
fn index_json(index: db::types::IndexInfo) -> Value {
    serde_json::json!({
        "name": index.name,
        "columns": index.columns,
        "unique": index.is_unique,
        "primary": index.is_primary,
        "typeName": index.index_type,
    })
}

/// 外键的元数据形状。
#[cfg(feature = "nsh-db")]
fn foreign_key_json(foreign_key: db::types::ForeignKeyDefinition) -> Value {
    serde_json::json!({
        "name": foreign_key.name,
        "columns": foreign_key.columns,
        "refTable": foreign_key.ref_table,
        "refSchema": foreign_key.ref_schema,
        "refColumns": foreign_key.ref_columns,
        "onDelete": foreign_key.on_delete,
        "onUpdate": foreign_key.on_update,
    })
}

/// 函数/过程的元数据形状。
#[cfg(feature = "nsh-db")]
fn function_json(function: db::types::FunctionInfo) -> Value {
    serde_json::json!({
        "name": function.name,
        "schema": function.schema,
        "returnType": function.return_type,
        "parameters": function.parameters,
        "identityArguments": function.identity_arguments,
        "definition": function.definition,
        "comment": function.comment,
    })
}

#[cfg(all(test, feature = "nsh-db"))]
mod tests {
    use super::*;
    use db::types::{
        ColumnInfo, ForeignKeyDefinition, FunctionInfo, IndexInfo, TableInfo, TableObjectType,
    };

    fn table() -> TableInfo {
        TableInfo {
            name: "orders".to_string(),
            object_type: TableObjectType::View,
            schema: Some("public".to_string()),
            comment: Some("订单".to_string()),
            engine: None,
            create_time: None,
            charset: None,
            collation: None,
        }
    }

    fn column() -> ColumnInfo {
        ColumnInfo {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: Some("0".to_string()),
            comment: None,
            charset: None,
            collation: None,
            is_auto_increment: true,
        }
    }

    #[test]
    fn table_json_keeps_kind_and_schema() {
        let value = table_json(table());
        assert_eq!(value["name"], "orders");
        assert_eq!(value["kind"], "view");
        assert_eq!(value["schema"], "public");
        assert_eq!(value["comment"], "订单");
    }

    #[test]
    fn column_json_maps_the_flags_the_page_reads() {
        let value = column_json(column());
        assert_eq!(value["name"], "id");
        assert_eq!(value["typeName"], "bigint");
        assert_eq!(value["nullable"], false);
        assert_eq!(value["primaryKey"], true);
        assert_eq!(value["autoIncrement"], true);
        assert_eq!(value["defaultValue"], "0");
    }

    #[test]
    fn index_and_foreign_key_json_use_the_page_facing_names() {
        let index = index_json(IndexInfo {
            name: "orders_pkey".to_string(),
            columns: vec!["id".to_string()],
            is_unique: true,
            is_primary: true,
            index_type: Some("btree".to_string()),
        });
        assert_eq!(index["unique"], true);
        assert_eq!(index["primary"], true);
        assert_eq!(index["typeName"], "btree");

        let foreign_key = foreign_key_json(ForeignKeyDefinition {
            name: "orders_user_fk".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: Some("public".to_string()),
            ref_columns: vec!["id".to_string()],
            on_delete: "CASCADE".to_string(),
            on_update: "NO ACTION".to_string(),
        });
        assert_eq!(foreign_key["refTable"], "users");
        assert_eq!(foreign_key["refColumns"][0], "id");
        assert_eq!(foreign_key["onDelete"], "CASCADE");
    }

    #[test]
    fn function_json_lists_parameters() {
        let value = function_json(FunctionInfo {
            name: "recount".to_string(),
            schema: Some("public".to_string()),
            return_type: Some("integer".to_string()),
            parameters: vec!["days integer".to_string()],
            identity_arguments: Some("days integer".to_string()),
            object_id: None,
            definition: None,
            comment: None,
        });
        assert_eq!(value["name"], "recount");
        assert_eq!(value["returnType"], "integer");
        assert_eq!(value["parameters"][0], "days integer");
    }
}

#[cfg(all(test, feature = "nsh-terminal"))]
mod terminal_api_tests {
    /// 执行类能力面挂在 `navop.terminal` 授权下，结果字段与 kit 的类型对齐；
    /// 前台 ssh 执行必须走 blocking 线程（否则会卡住引擎线程）。
    #[test]
    fn terminal_execution_methods_are_gated_and_mapped() {
        let source = include_str!("native_api.rs");
        for needle in [
            "pub(crate) async fn exec(",
            "pub(crate) async fn ssh_exec(",
            "registry.terminal_exec(&target, request, cancellation)",
            "spawn_blocking(move || registry.remote_exec(&session_id, request))",
            "\"completion\": result.completion",
            "\"timedOut\": result.timed_out",
        ] {
            assert!(source.contains(needle), "native_api.rs 缺少：{needle}");
        }
    }
}
