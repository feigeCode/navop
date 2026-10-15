//! Auto 模式粘性会话：每个 SQL 编辑器页签在 Auto 事务模式下复用同一条
//! 物理连接，使 `SET @var` 等连接级状态跨执行存活（issue #312）。
//!
//! 与 Manual 事务的 `ManualTransactionSession` 平行：复用 `SessionSqlRun`
//! 执行管线与 `execute_session` 的 pin 语义（`in_flight` 不归零，连接不会
//! 被 idle 回收），由编辑器持有引用并在 scope/连接/模式变化时释放。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::sql_editor_view::SqlExecutionScope;

/// Auto 模式编辑器持有的粘性会话。
#[derive(Clone, Debug)]
pub(crate) struct AutoExecutionSession {
    session_id: String,
    database: Option<String>,
    schema: Option<String>,
    /// 会话级连接已失效（断连/被回收）。置位后下次执行前丢弃重建。
    connection_lost: Arc<AtomicBool>,
}

impl AutoExecutionSession {
    pub(crate) fn new(
        session_id: String,
        database: Option<String>,
        schema: Option<String>,
    ) -> Self {
        Self {
            session_id,
            database,
            schema,
            connection_lost: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn connection_lost(&self) -> &Arc<AtomicBool> {
        &self.connection_lost
    }

    pub(crate) fn matches_scope(&self, scope: &SqlExecutionScope) -> bool {
        self.database.as_deref() == scope.database.as_deref()
            && self.schema.as_deref() == scope.schema.as_deref()
    }
}

/// Auto 执行前对粘性会话的处置决策。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoSessionAction {
    /// 已有会话且 scope 匹配、连接健康：直接复用。
    RunInstalledSession,
    /// 没有会话（首次执行）或会话已失效：丢弃旧会话并新建。
    StartSession,
}

/// 纯决策：installed 会话能否服务本次执行。
///
/// scope 不匹配时选择重建而不是报错（Auto 模式下用户随时切库，旧会话的
/// 连接级状态随之作废，静默重建符合“自动”语义）。
pub(crate) fn auto_session_action_for(
    installed: Option<&AutoExecutionSession>,
    scope: &SqlExecutionScope,
) -> AutoSessionAction {
    match installed {
        Some(session) if session.matches_scope(scope) && !session_is_dead(session) => {
            AutoSessionAction::RunInstalledSession
        }
        _ => AutoSessionAction::StartSession,
    }
}

fn session_is_dead(session: &AutoExecutionSession) -> bool {
    session.connection_lost.load(Ordering::SeqCst)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn scope(db: Option<&str>, schema: Option<&str>) -> SqlExecutionScope {
        SqlExecutionScope::new(db.map(str::to_string), schema.map(str::to_string))
    }

    fn live_session(db: Option<&str>, schema: Option<&str>) -> AutoExecutionSession {
        AutoExecutionSession::new(
            "s1".to_string(),
            db.map(str::to_string),
            schema.map(str::to_string),
        )
    }

    #[test]
    fn no_session_starts() {
        assert_eq!(
            auto_session_action_for(None, &scope(Some("db"), None)),
            AutoSessionAction::StartSession
        );
    }

    #[test]
    fn matching_session_reuses() {
        let session = live_session(Some("db"), Some("sch"));
        assert_eq!(
            auto_session_action_for(Some(&session), &scope(Some("db"), Some("sch"))),
            AutoSessionAction::RunInstalledSession
        );
    }

    #[test]
    fn scope_mismatch_rebuilds() {
        let session = live_session(Some("db1"), None);
        assert_eq!(
            auto_session_action_for(Some(&session), &scope(Some("db2"), None)),
            AutoSessionAction::StartSession
        );
    }

    #[test]
    fn lost_connection_rebuilds() {
        let session = live_session(Some("db"), None);
        session.connection_lost().store(true, Ordering::SeqCst);
        assert_eq!(
            auto_session_action_for(Some(&session), &scope(Some("db"), None)),
            AutoSessionAction::StartSession
        );
    }
}
