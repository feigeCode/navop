//! Agent 会话持久化(集成胶水)。
//!
//! 把 `agent_runtime` 的 [`SessionSnapshot`] 与 `one-core` 的
//! [`AgentSessionRepository`] 接起来:快照 ↔ JSON 字符串的序列化在此完成,使
//! core 对快照内容保持不透明。所有函数在**缺少存储后端**(如未初始化
//! `GlobalStorageState` 的示例程序)时安全降级为 no-op / 空结果,绝不 panic。
//!
//! DB 操作为同步、低频(每轮结束保存一次 / 切换会话时读取一次)、载荷小,直接在
//! 调用线程执行,沿用项目既有的同步 Repository 调用风格。

use agent_runtime::{HistoryItem, Session, SessionSnapshot};
use one_core::{
    llm::chat_history::{AgentSessionRepository, ChatMessage, MessageRepository},
    storage::GlobalStorageState,
};

use gpui::App;

use crate::session_sidebar::SessionSummary;

/// 标题最大字符数(超出截断)。
const MAX_TITLE_CHARS: usize = 40;

/// 保存会话快照。空会话(无任何历史)不落库。
///
/// 返回用于刷新侧边栏摘要的 `(标题, 更新时间秒)`;未保存时返回 `None`。
pub fn save_session(cx: &App, session: &Session) -> Option<(String, i64)> {
    let snapshot = session.snapshot();
    if snapshot.history.is_empty() {
        return None;
    }
    let title = derive_title(&snapshot);
    let snapshot_json = serde_json::to_string(&snapshot).ok()?;
    let repo = agent_session_repository(cx)?;
    let saved = repo
        .save_snapshot(&snapshot.id.to_string(), &title, &snapshot_json)
        .ok()?;
    Some((saved.title, saved.updated_at))
}

/// 列出全部**未归档**会话,按更新时间倒序映射为侧边栏摘要。
pub fn list_summaries(cx: &App) -> Vec<SessionSummary> {
    list_summaries_by_archived(cx, false)
}

/// 列出**已归档**会话,按更新时间倒序映射为侧边栏摘要。
pub fn list_archived_summaries(cx: &App) -> Vec<SessionSummary> {
    list_summaries_by_archived(cx, true)
}

/// 按会话 id 读取并反序列化快照。
pub fn load_snapshot(cx: &App, uid: &str) -> Option<SessionSnapshot> {
    let repo = agent_session_repository(cx)?;
    let session = repo.get_by_uid(uid).ok()??;
    if !session.snapshot_json.trim().is_empty()
        && let Ok(snapshot) = serde_json::from_str(&session.snapshot_json)
    {
        return Some(snapshot);
    }
    load_legacy_chat_snapshot(cx, uid)
}

/// 删除一条持久化会话(无存储后端时为 no-op)。
pub fn delete_session(cx: &App, uid: &str) {
    if let Some(repo) = agent_session_repository(cx) {
        let _ = repo.delete_by_uid(uid);
    }
}

/// 重命名一条持久化会话;成功返回 `true`(无存储后端 / 失败返回 `false`)。
pub fn rename_session(cx: &App, uid: &str, title: &str) -> bool {
    agent_session_repository(cx)
        .and_then(|repo| repo.rename_by_uid(uid, title).ok())
        .unwrap_or(false)
}

/// 归档 / 恢复一条会话(软删除);成功返回 `true`。
pub fn set_archived(cx: &App, uid: &str, archived: bool) -> bool {
    agent_session_repository(cx)
        .and_then(|repo| repo.set_archived_by_uid(uid, archived).ok())
        .unwrap_or(false)
}

/// 旧版 `chat_sessions` 记录没有 Agent snapshot,应按 Ask 会话恢复和续聊。
pub fn should_use_ask_mode(cx: &App, uid: &str) -> bool {
    agent_session_repository(cx)
        .and_then(|repo| repo.is_legacy_chat_uid(uid).ok())
        .unwrap_or(false)
}

fn agent_session_repository(cx: &App) -> Option<std::sync::Arc<AgentSessionRepository>> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<AgentSessionRepository>())
}

fn message_repository(cx: &App) -> Option<std::sync::Arc<MessageRepository>> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<MessageRepository>())
}

fn list_summaries_by_archived(cx: &App, archived: bool) -> Vec<SessionSummary> {
    agent_session_repository(cx)
        .and_then(|repo| repo.list_by_archived(archived).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|session| SessionSummary::new(session.uid, session.title, session.updated_at))
        .collect()
}

fn load_legacy_chat_snapshot(cx: &App, uid: &str) -> Option<SessionSnapshot> {
    let session_id = uid.parse::<i64>().ok()?;
    let messages = message_repository(cx)?.list_by_session(session_id).ok()?;
    Some(SessionSnapshot {
        id: agent_runtime::SessionId::from_string(uid.to_string()),
        resources: agent_runtime::ResourceContext::new(),
        history: messages
            .into_iter()
            .filter_map(chat_message_to_history)
            .collect(),
        plan: None,
        system_instruction: None,
        skills: agent_runtime::SkillContext::new(),
    })
}

fn chat_message_to_history(message: ChatMessage) -> Option<HistoryItem> {
    match message.role.as_str() {
        "user" => Some(HistoryItem::User {
            text: message.content,
            images: Vec::new(),
        }),
        "assistant" => Some(HistoryItem::Assistant(message.content)),
        "system" => Some(HistoryItem::System(message.content)),
        _ => None,
    }
}

/// 由快照推导标题:取首条非空用户消息,截断到 [`MAX_TITLE_CHARS`]。
fn derive_title(snapshot: &SessionSnapshot) -> String {
    snapshot
        .history
        .iter()
        .find_map(|item| match item {
            HistoryItem::User { text, .. } if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
        .map(|text| truncate_title(&text))
        .unwrap_or_else(|| "新 Agent 会话".to_string())
}

/// 取首行并按字符数截断,作为会话标题。
fn truncate_title(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= MAX_TITLE_CHARS {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(MAX_TITLE_CHARS).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::{ResourceContext, SessionId, TurnId};
    use gpui::TestAppContext;
    use one_core::llm::chat_history::{ChatSession, SessionRepository};
    use one_core::storage::{
        GlobalStorageState, StorageManager, connection::SqliteConnection,
        migration::run_migrations, traits::Repository,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn snapshot_with_first_user(text: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: SessionId::from_string("sess_x"),
            resources: ResourceContext::new(),
            history: vec![
                HistoryItem::System("sys".into()),
                HistoryItem::User {
                    text: text.into(),
                    images: Vec::new(),
                },
                HistoryItem::Assistant("hi".into()),
            ],
            plan: None,
            system_instruction: None,
            skills: agent_runtime::SkillContext::new(),
        }
    }

    fn test_storage() -> StorageManager {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-ai-agent-session-persistence-{}-{unique}-{counter}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(run_migrations)
            .expect("run migrations");

        let storage = StorageManager::new_with_connection(conn.clone());
        storage.register(AgentSessionRepository::new(conn.clone()));
        storage.register(SessionRepository::new(conn.clone()));
        storage.register(MessageRepository::new(conn));
        storage
    }

    fn test_session() -> std::sync::Arc<Session> {
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        Session::new(
            SessionId::from_string("sess_persist"),
            ResourceContext::new(),
            tx,
        )
    }

    fn seed_legacy_chat(storage: &StorageManager) -> i64 {
        let session_repo = storage.get::<SessionRepository>().expect("session repo");
        let message_repo = storage.get::<MessageRepository>().expect("message repo");
        let mut session = ChatSession::new("旧 Ask 会话".into(), "provider-a".into());
        let session_id = session_repo.insert(&mut session).expect("insert session");
        let mut user = ChatMessage::user(session_id, "旧问题".into());
        message_repo.insert(&mut user).expect("insert user");
        let mut assistant = ChatMessage::assistant(session_id, "旧回答".into());
        message_repo
            .insert(&mut assistant)
            .expect("insert assistant");
        session_id
    }

    #[test]
    fn title_uses_first_user_message() {
        let snap = snapshot_with_first_user("查询连接数");
        assert_eq!(derive_title(&snap), "查询连接数");
    }

    #[test]
    fn title_truncates_long_first_line() {
        let long = "一".repeat(60);
        let snap = snapshot_with_first_user(&long);
        let title = derive_title(&snap);
        assert!(title.ends_with('…'));
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS + 1);
    }

    #[test]
    fn title_falls_back_when_no_user_message() {
        let snap = SessionSnapshot {
            id: SessionId::from_string("sess_x"),
            resources: ResourceContext::new(),
            history: vec![HistoryItem::System("only system".into())],
            plan: None,
            system_instruction: None,
            skills: agent_runtime::SkillContext::new(),
        };
        assert_eq!(derive_title(&snap), "新 Agent 会话");
    }

    #[gpui::test]
    fn agent_session_persistence_round_trips_and_manages_lifecycle(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });

        let session = test_session();
        let turn_id = TurnId::from_string("turn_persist");
        session.set_system_instruction(Some("始终用 DBA 视角回答。".into()));
        session.record_user_input("查询连接数");
        session.record_assistant_message(&turn_id, "好的,我来查询");

        let saved = cx
            .update(|cx| save_session(cx, &session))
            .expect("save session");
        assert_eq!("查询连接数", saved.0);

        let summaries = cx.update(|cx| list_summaries(cx));
        assert_eq!(1, summaries.len());
        assert_eq!("sess_persist", summaries[0].id);
        assert_eq!("查询连接数", summaries[0].name.as_ref());

        let loaded = cx
            .update(|cx| load_snapshot(cx, "sess_persist"))
            .expect("load snapshot");
        assert_eq!(SessionId::from_string("sess_persist"), loaded.id);
        assert_eq!(2, loaded.history.len());
        assert_eq!(
            Some("始终用 DBA 视角回答。"),
            loaded.system_instruction.as_deref()
        );

        assert!(cx.update(|cx| rename_session(cx, "sess_persist", "连接数排查")));
        let renamed = cx.update(|cx| list_summaries(cx));
        assert_eq!("连接数排查", renamed[0].name.as_ref());

        assert!(cx.update(|cx| set_archived(cx, "sess_persist", true)));
        assert!(cx.update(|cx| list_summaries(cx)).is_empty());
        let archived = cx.update(|cx| list_archived_summaries(cx));
        assert_eq!(1, archived.len());
        assert_eq!("连接数排查", archived[0].name.as_ref());

        cx.update(|cx| delete_session(cx, "sess_persist"));
        assert!(
            cx.update(|cx| load_snapshot(cx, "sess_persist")).is_none(),
            "delete_session should remove persisted snapshots"
        );
    }

    #[gpui::test]
    fn legacy_chat_sessions_are_listed_and_loaded_as_ask_history(cx: &mut TestAppContext) {
        let storage = test_storage();
        let legacy_id = seed_legacy_chat(&storage);
        let legacy_uid = legacy_id.to_string();
        cx.update(|cx| {
            cx.set_global(GlobalStorageState { storage });
        });

        let summaries = cx.update(|cx| list_summaries(cx));
        assert!(
            summaries
                .iter()
                .any(|summary| summary.id == legacy_uid && summary.name.as_ref() == "旧 Ask 会话")
        );
        assert!(cx.update(|cx| should_use_ask_mode(cx, &legacy_uid)));

        let snapshot = cx
            .update(|cx| load_snapshot(cx, &legacy_uid))
            .expect("legacy snapshot");
        assert_eq!(SessionId::from_string(legacy_uid), snapshot.id);
        assert_eq!(2, snapshot.history.len());
        assert!(matches!(
            &snapshot.history[0],
            HistoryItem::User { text, .. } if text == "旧问题"
        ));
        assert!(matches!(
            &snapshot.history[1],
            HistoryItem::Assistant(text) if text == "旧回答"
        ));
    }
}
