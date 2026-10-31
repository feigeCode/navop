//! 连接级 AI 访问策略的执行期校验（issue #437）。
//!
//! 用户在 Navop 里可以把某个连接设置为「只读」或「禁止 AI 操作」。真正的拦截点放在
//! [`tool_runtime::ToolRegistry::call`] 上：它同时服务外部 MCP 客户端和内置 AI，是
//! 所有工具执行的唯一汇聚点，参数里的 `target` 在进入 handler 之前已经被归一化回
//! `connection` / `session_id`，因此这里可以稳定地把参数映射回连接。
//!
//! 职责边界：
//!
//! - 本模块只做「执行期拒绝」，不负责从资源池里隐藏连接（那是可见性收窄的职责）；
//! - 只读级别依据工具自身的 `read_only` 注解判断，新增的写类工具默认被拒；
//! - 读不到策略时一律拒绝（fail closed），避免存储异常变成静默放开。

use std::sync::Arc;

use gpui::App;
use one_core::storage::{ConnectionRepository, GlobalStorageState};
use public_mcp::registry::PublicMcpRegistry;
use serde_json::Value;
use tool_runtime::RestrictedConnection;
use tool_runtime::{
    AiAccessLevel, RuntimeToolDescriptor, ToolAccessGuard, ToolError, access_denied_error,
};

/// 参数里可能承载「连接 / 会话」的字段名。
///
/// 覆盖 `connections.*` / `database.*` / `redis.*` / `mongo.*` / `sftp.*` 的
/// `connection`、远程执行类工具的 `session_id`，以及归一化后可能残留的 `target`。
const CONNECTION_FIELDS: [&str; 3] = ["connection", "connection_id", "target"];
const SESSION_FIELDS: [&str; 1] = ["session_id"];

/// 解析出的连接目标。
struct ResolvedTarget {
    connection_id: i64,
    label: String,
}

/// 连接级 AI 访问策略的执行期校验器。
pub(super) struct ConnectionAiAccessGuard {
    repo: Arc<ConnectionRepository>,
    /// 会话 id → 连接 id 的来源。终端会话没有关联连接时视为与策略无关。
    terminal_registry: Option<PublicMcpRegistry>,
}

/// 构造校验器；没有连接存储时返回 `None`（说明当前环境没有连接可访问）。
pub(super) fn connection_ai_access_guard(cx: &App) -> Option<Arc<dyn ToolAccessGuard>> {
    let repo = cx
        .try_global::<GlobalStorageState>()?
        .storage
        .get::<ConnectionRepository>()?;
    #[cfg(feature = "shell")]
    let terminal_registry = terminal_view::public_mcp::registry(cx);
    #[cfg(not(feature = "shell"))]
    let terminal_registry = None;
    Some(Arc::new(ConnectionAiAccessGuard {
        repo,
        terminal_registry,
    }))
}

impl ConnectionAiAccessGuard {
    fn authorize_inner(
        &self,
        tool: &RuntimeToolDescriptor,
        input: &Value,
    ) -> Result<(), ToolError> {
        let Some(target) = self.resolve_target(tool, input)? else {
            // 没有连接/会话参数（如 connections.list、workspaces.*），与连接策略无关。
            return Ok(());
        };
        let level = self.level_for(target.connection_id)?;
        if level.permits(&tool.annotations) {
            return Ok(());
        }
        Err(access_denied_error(
            tool.id.as_str(),
            level,
            Some(&target.label),
        ))
    }

    /// 把工具参数映射回连接 id。
    ///
    /// 返回 `Ok(None)` 表示「这个调用不针对具体连接」；无法识别成已知连接时也返回
    /// `Ok(None)`，由 handler 自己报告「未知连接」，这里不抢它的错误语义。
    fn resolve_target(
        &self,
        tool: &RuntimeToolDescriptor,
        input: &Value,
    ) -> Result<Option<ResolvedTarget>, ToolError> {
        let Some(object) = input.as_object() else {
            return Ok(None);
        };

        for field in SESSION_FIELDS {
            let Some(session_id) = object.get(field).and_then(Value::as_str) else {
                continue;
            };
            return Ok(self.session_connection_id(session_id).map(|connection_id| {
                ResolvedTarget {
                    connection_id,
                    label: session_id.to_string(),
                }
            }));
        }

        // `connections.save` / `connections.delete` 用数字 id 定位连接，不经过 target 归一化，
        // 所以这里单独覆盖一次，避免 AI 绕过资源池直接改删被禁止的连接。
        if tool.id.as_str().starts_with("connections.")
            && let Some(connection_id) = object.get("id").and_then(Value::as_i64)
        {
            return Ok(self
                .restricted_connections()?
                .iter()
                .find(|entry| entry.connection_id == connection_id)
                .map(|entry| ResolvedTarget {
                    connection_id,
                    label: entry.name.clone(),
                }));
        }

        for field in CONNECTION_FIELDS {
            let Some(value) = object.get(field) else {
                continue;
            };
            let Some(reference) = reference_string(value) else {
                continue;
            };
            if reference.trim().is_empty() {
                continue;
            }
            return self.resolve_reference(&reference);
        }
        Ok(None)
    }

    /// 连接引用既可能是数字 id，也可能是连接名（与 `connections.*` 的解析规则一致）。
    fn resolve_reference(&self, reference: &str) -> Result<Option<ResolvedTarget>, ToolError> {
        let restricted = self.restricted_connections()?;
        if let Ok(connection_id) = reference.trim().parse::<i64>() {
            if let Some(entry) = restricted
                .iter()
                .find(|entry| entry.connection_id == connection_id)
            {
                return Ok(Some(ResolvedTarget {
                    connection_id,
                    label: entry.name.clone(),
                }));
            }
            // 其它连接要么是默认级别（放行），要么根本不存在（交给 handler 报错）。
            return Ok(None);
        }
        Ok(restricted
            .iter()
            .find(|entry| entry.name == reference)
            .map(|entry| ResolvedTarget {
                connection_id: entry.connection_id,
                label: entry.name.clone(),
            }))
    }

    fn session_connection_id(&self, session_id: &str) -> Option<i64> {
        let registry = self.terminal_registry.as_ref()?;
        registry
            .list_sessions()
            .into_iter()
            .find(|session| session.session_id == session_id)
            .and_then(|session| session.connection_id)
    }

    fn level_for(&self, connection_id: i64) -> Result<AiAccessLevel, ToolError> {
        self.repo
            .ai_access_level(connection_id)
            .map_err(|error| ToolError::Failed {
                message: format!("无法读取连接的 AI 访问策略，已按拒绝处理：{error}"),
            })
    }

    /// 非默认级别的连接：只需要这几十行数据就能完成 id / 名称两种引用方式的判定。
    fn restricted_connections(&self) -> Result<Vec<RestrictedConnection>, ToolError> {
        self.repo
            .ai_access_restricted()
            .map_err(|error| ToolError::Failed {
                message: format!("无法读取连接的 AI 访问策略，已按拒绝处理：{error}"),
            })
    }
}

impl ToolAccessGuard for ConnectionAiAccessGuard {
    fn authorize(&self, tool: &RuntimeToolDescriptor, input: &Value) -> Result<(), ToolError> {
        self.authorize_inner(tool, input)
    }
}

/// 把数字或字符串形式的引用统一成字符串。
fn reference_string(value: &Value) -> Option<String> {
    match value {
        Value::String(reference) => Some(reference.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use one_core::storage::connection::SqliteConnection;
    use one_core::storage::migration::run_migrations;
    use one_core::storage::traits::Repository;
    use one_core::storage::{DatabaseType, DbConnectionConfig, StorageManager, StoredConnection};
    use public_mcp::registry::{
        ConnectionState, TerminalConnectionKind, TerminalSessionHandle, TerminalSessionSnapshot,
    };
    use serde_json::json;
    use tool_runtime::{
        ToolAdapter, ToolAnnotations, ToolId, ToolMode, ToolOrigin, ToolTargetSpec,
    };

    const CONNECTION_NAME: &str = "生产库";

    #[test]
    fn reference_string_accepts_numbers_and_strings() {
        assert_eq!(reference_string(&Value::from(7)), Some("7".to_string()));
        assert_eq!(
            reference_string(&Value::from(CONNECTION_NAME)),
            Some(CONNECTION_NAME.to_string())
        );
        assert_eq!(reference_string(&Value::Null), None);
    }

    #[gpui::test]
    fn denied_connection_rejects_every_tool(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::Denied);

        for read_only in [true, false] {
            let error = guard
                .authorize(
                    &descriptor(
                        if read_only {
                            "database.query"
                        } else {
                            "database.execute"
                        },
                        read_only,
                    ),
                    &json!({ "connection": connection_id.to_string() }),
                )
                .expect_err("禁止级别的连接必须拒绝所有工具");
            assert!(
                error.to_string().contains("禁止 AI 操作"),
                "错误信息应说明是用户策略拦截：{error}"
            );
        }
    }

    #[gpui::test]
    fn read_only_connection_allows_read_tools_only(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::ReadOnly);
        let input = json!({ "connection": connection_id });

        guard
            .authorize(&descriptor("database.query", true), &input)
            .expect("只读级别应放行只读工具");
        let error = guard
            .authorize(&descriptor("database.execute", false), &input)
            .expect_err("只读级别必须拒绝写类工具");
        assert!(
            error.to_string().contains("只读"),
            "错误信息应说明连接是只读：{error}"
        );
    }

    #[gpui::test]
    fn allowed_connection_is_never_blocked(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::Allowed);
        for read_only in [true, false] {
            guard
                .authorize(
                    &descriptor(
                        if read_only {
                            "database.query"
                        } else {
                            "database.execute"
                        },
                        read_only,
                    ),
                    &json!({ "connection": connection_id }),
                )
                .expect("默认级别不应拦截任何工具");
        }
    }

    #[gpui::test]
    fn connection_name_reference_is_resolved(cx: &mut TestAppContext) {
        let (guard, _) = setup(cx, AiAccessLevel::Denied);
        let error = guard
            .authorize(
                &descriptor("connections.show", true),
                &json!({ "connection": CONNECTION_NAME }),
            )
            .expect_err("按连接名引用时也必须命中策略");
        assert!(error.to_string().contains("禁止 AI 操作"));
    }

    #[gpui::test]
    fn unrestricted_and_unknown_targets_pass_through(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::Denied);

        // 不带连接参数的工具（connections.list 等）与连接策略无关。
        guard
            .authorize(&descriptor("connections.list", true), &json!({}))
            .expect("不涉及连接的工具不应被拦截");
        // 不存在的连接交给 handler 自己报「未知连接」，这里不抢错误语义。
        guard
            .authorize(
                &descriptor("database.execute", false),
                &json!({ "connection": connection_id + 1000 }),
            )
            .expect("未知连接不应由策略层拦截");
    }

    #[gpui::test]
    fn id_based_connection_tools_respect_the_policy(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::Denied);

        let denied = guard
            .authorize(
                &descriptor("connections.delete", false),
                &json!({ "id": connection_id }),
            )
            .expect_err("被禁止的连接不允许 AI 删除");
        assert!(denied.to_string().contains("禁止 AI 操作"));

        guard
            .authorize(
                &descriptor("connections.save", false),
                &json!({ "id": connection_id, "patch": { "remark": "x" } }),
            )
            .expect_err("被禁止的连接不允许 AI 修改");

        // 其它命名空间的同名 id 字段（例如 workspace id）不应被误判成连接。
        guard
            .authorize(
                &descriptor("workspaces.delete", false),
                &json!({ "id": connection_id }),
            )
            .expect("非 connections.* 工具不按连接 id 匹配");

        // 新建连接（不带 id）不受影响。
        guard
            .authorize(
                &descriptor("connections.save", false),
                &json!({ "kind": "mysql", "values": {} }),
            )
            .expect("新建连接不涉及既有策略");
    }

    #[gpui::test]
    fn terminal_session_inherits_its_connection_policy(cx: &mut TestAppContext) {
        let (guard, connection_id) = setup(cx, AiAccessLevel::Denied);
        let registry = cx
            .update(|cx| terminal_view::public_mcp::registry(cx))
            .expect("registry");
        registry.register(FakeSession::new("session-1", Some(connection_id)));

        let error = guard
            .authorize(
                &descriptor("terminal.exec", false),
                &json!({ "session_id": "session-1" }),
            )
            .expect_err("被禁连接的会话也必须拒绝");
        assert!(error.to_string().contains("禁止 AI 操作"));
        assert!(
            error.to_string().contains("session-1"),
            "错误信息应指出被拒的目标：{error}"
        );
    }

    #[gpui::test]
    fn terminal_session_without_connection_is_unrestricted(cx: &mut TestAppContext) {
        let (guard, _) = setup(cx, AiAccessLevel::Denied);
        let registry = cx
            .update(|cx| terminal_view::public_mcp::registry(cx))
            .expect("registry");
        registry.register(FakeSession::new("local-1", None));

        guard
            .authorize(
                &descriptor("terminal.exec", false),
                &json!({ "session_id": "local-1" }),
            )
            .expect("本地终端没有连接，不应被连接策略拦截");
    }

    /// 准备一个内存数据库、一条连接，并把连接设置为指定级别。
    fn setup(cx: &mut TestAppContext, level: AiAccessLevel) -> (Arc<dyn ToolAccessGuard>, i64) {
        let (guard, connection_id) = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            let storage = StorageManager::new_with_connection(memory_connection());
            let repo = ConnectionRepository::new(storage.connection());
            storage.register(repo.clone());
            cx.set_global(GlobalStorageState { storage });

            let mut connection =
                StoredConnection::new_database(CONNECTION_NAME.to_string(), db_config(), None);
            repo.insert(&mut connection).expect("连接应能插入");
            let connection_id = connection.id.expect("插入后应有 id");
            repo.set_ai_access_level(connection_id, level)
                .expect("策略应能写入");

            let guard = connection_ai_access_guard(cx).expect("有连接存储时校验器必须存在");
            (guard, connection_id)
        });
        (guard, connection_id)
    }

    fn descriptor(id: &str, read_only: bool) -> RuntimeToolDescriptor {
        RuntimeToolDescriptor {
            id: ToolId::new(id),
            title: id.to_string(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({}),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: if read_only {
                ToolAnnotations::read_only(id)
            } else {
                ToolAnnotations::mutating(id)
            },
            target: ToolTargetSpec::default(),
            origin: ToolOrigin::Database,
            aliases: Vec::new(),
        }
    }

    fn db_config() -> DbConnectionConfig {
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::SQLite,
            name: String::new(),
            host: "127.0.0.1".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            credential_reference: None,
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: Default::default(),
        }
    }

    fn memory_connection() -> SqliteConnection {
        let connection =
            SqliteConnection::open_with_pool_size(":memory:", 1).expect("内存数据库应能打开");
        connection
            .with_connection(|db| run_migrations(db))
            .expect("迁移应能执行");
        connection
    }

    struct FakeSession {
        session_id: String,
        connection_id: Option<i64>,
    }

    impl FakeSession {
        fn new(session_id: &str, connection_id: Option<i64>) -> Self {
            Self {
                session_id: session_id.to_string(),
                connection_id,
            }
        }

        fn snapshot(&self) -> TerminalSessionSnapshot {
            TerminalSessionSnapshot {
                session_id: self.session_id.clone(),
                connection_id: self.connection_id,
                title: "root@prod:~".to_string(),
                host_label: "prod".to_string(),
                cwd: Some("/root".to_string()),
                rows: 24,
                cols: 120,
                connection_kind: TerminalConnectionKind::Ssh,
                connection_state: ConnectionState::Connected,
            }
        }
    }

    impl TerminalSessionHandle for FakeSession {
        fn snapshot(&self) -> TerminalSessionSnapshot {
            self.snapshot()
        }
    }
}
