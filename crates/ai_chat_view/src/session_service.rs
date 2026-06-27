//! AI Chat 会话持久化服务。

use one_core::{
    llm::chat_history::{ChatMessage, ChatSession, MessageRepository, SessionRepository},
    storage::{StorageManager, traits::Repository},
};

/// SessionService 错误类型。
#[derive(Debug, Clone)]
pub enum SessionError {
    RepositoryNotAvailable,
    SessionNotFound,
    StorageError(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::RepositoryNotAvailable => {
                write!(f, "session repository is not available")
            }
            SessionError::SessionNotFound => write!(f, "session not found"),
            SessionError::StorageError(msg) => {
                write!(f, "session storage error: {msg}")
            }
        }
    }
}

impl std::error::Error for SessionError {}

/// 会话持久化服务。
#[derive(Clone)]
pub struct SessionService {
    storage_manager: StorageManager,
}

impl SessionService {
    pub fn new(storage_manager: StorageManager) -> Self {
        Self { storage_manager }
    }

    pub fn create_session(&self, name: String, provider_id: String) -> Result<i64, SessionError> {
        let session_repo = self.session_repository()?;
        let mut session = ChatSession::new(name, provider_id);
        session_repo
            .insert(&mut session)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn get_session(&self, session_id: i64) -> Result<Option<ChatSession>, SessionError> {
        self.session_repository()?
            .get(session_id)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn list_sessions(&self) -> Result<Vec<ChatSession>, SessionError> {
        self.session_repository()?
            .list()
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn delete_session(&self, session_id: i64) -> Result<(), SessionError> {
        self.session_repository()?
            .delete(session_id)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn update_session_name(&self, session_id: i64, name: String) -> Result<(), SessionError> {
        let session_repo = self.session_repository()?;
        let mut session = session_repo
            .get(session_id)
            .map_err(|e| SessionError::StorageError(e.to_string()))?
            .ok_or(SessionError::SessionNotFound)?;

        session.name = name;
        session_repo
            .update(&session)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn add_user_message(&self, session_id: i64, content: String) -> Result<i64, SessionError> {
        let message_repo = self.message_repository()?;
        let mut message = ChatMessage::user(session_id, content);
        message_repo
            .insert(&mut message)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn add_assistant_message(
        &self,
        session_id: i64,
        content: String,
    ) -> Result<i64, SessionError> {
        let message_repo = self.message_repository()?;
        let mut message = ChatMessage::assistant(session_id, content);
        message_repo
            .insert(&mut message)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn get_messages(&self, session_id: i64) -> Result<Vec<ChatMessage>, SessionError> {
        self.message_repository()?
            .list_by_session(session_id)
            .map_err(|e| SessionError::StorageError(e.to_string()))
    }

    pub fn ensure_session(
        &self,
        session_id: Option<i64>,
        provider_id: &str,
        default_name: &str,
    ) -> Result<i64, SessionError> {
        if let Some(id) = session_id {
            if self.get_session(id)?.is_some() {
                return Ok(id);
            }
        }

        self.create_session(default_name.to_string(), provider_id.to_string())
    }

    pub fn storage_manager(&self) -> &StorageManager {
        &self.storage_manager
    }

    fn session_repository(&self) -> Result<std::sync::Arc<SessionRepository>, SessionError> {
        self.storage_manager
            .get::<SessionRepository>()
            .ok_or(SessionError::RepositoryNotAvailable)
    }

    fn message_repository(&self) -> Result<std::sync::Arc<MessageRepository>, SessionError> {
        self.storage_manager
            .get::<MessageRepository>()
            .ok_or(SessionError::RepositoryNotAvailable)
    }
}

/// 从消息内容提取会话名称，取前 20 个字符并移除空行。
pub fn extract_session_name(content: &str) -> String {
    let clean_content = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let clean_content = clean_content.trim();
    if clean_content.chars().count() <= 20 {
        clean_content.to_string()
    } else {
        format!("{}...", clean_content.chars().take(17).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::{
        llm::chat_history::{MessageRepository, SessionRepository},
        storage::{StorageManager, connection::SqliteConnection, migration::run_migrations},
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_storage(register_repositories: bool) -> StorageManager {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-ai-chat-view-session-service-{}-{unique}-{counter}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(run_migrations)
            .expect("run migrations");

        let storage = StorageManager::new_with_connection(conn.clone());
        if register_repositories {
            storage.register(SessionRepository::new(conn.clone()));
            storage.register(MessageRepository::new(conn));
        }
        storage
    }

    #[test]
    fn returns_repository_error_when_repositories_are_missing() {
        let service = SessionService::new(test_storage(false));

        assert!(matches!(
            service.list_sessions(),
            Err(SessionError::RepositoryNotAvailable)
        ));
    }

    #[test]
    fn creates_sessions_and_roundtrips_messages() {
        let service = SessionService::new(test_storage(true));

        let session_id = service
            .create_session("AI Chat".to_string(), "provider-a".to_string())
            .expect("create session");
        service
            .add_user_message(session_id, "show tables".to_string())
            .expect("add user message");
        service
            .add_assistant_message(session_id, "select * from users".to_string())
            .expect("add assistant message");

        let session = service
            .get_session(session_id)
            .expect("get session")
            .expect("session exists");
        assert_eq!("AI Chat", session.name);
        assert_eq!("provider-a", session.provider_id);

        let messages = service.get_messages(session_id).expect("get messages");
        assert_eq!(2, messages.len());
        assert_eq!("user", messages[0].role);
        assert_eq!("show tables", messages[0].content);
        assert_eq!("assistant", messages[1].role);
        assert_eq!("select * from users", messages[1].content);
    }

    #[test]
    fn ensure_session_reuses_existing_or_creates_missing_session() {
        let service = SessionService::new(test_storage(true));
        let existing_id = service
            .create_session("Existing".to_string(), "provider-a".to_string())
            .expect("create session");

        assert_eq!(
            existing_id,
            service
                .ensure_session(Some(existing_id), "provider-b", "New")
                .expect("reuse existing")
        );

        let created_id = service
            .ensure_session(Some(999_999), "provider-b", "New")
            .expect("create missing");
        let created = service
            .get_session(created_id)
            .expect("get created")
            .expect("created exists");
        assert_eq!("New", created.name);
        assert_eq!("provider-b", created.provider_id);
    }

    #[test]
    fn updates_and_deletes_sessions() {
        let service = SessionService::new(test_storage(true));
        let session_id = service
            .create_session("Old".to_string(), "provider-a".to_string())
            .expect("create session");

        service
            .update_session_name(session_id, "New".to_string())
            .expect("rename session");
        assert_eq!(
            "New",
            service
                .get_session(session_id)
                .expect("get renamed")
                .expect("renamed exists")
                .name
        );

        service.delete_session(session_id).expect("delete session");
        assert!(
            service
                .get_session(session_id)
                .expect("get deleted")
                .is_none()
        );
    }

    #[test]
    fn extract_session_name_trims_newlines_and_truncates_long_titles() {
        assert_eq!("Hello World", extract_session_name(" Hello\n\nWorld "));

        let title =
            extract_session_name("这是一个非常长的会话标题，需要被截断，并且应该带有省略号");
        assert!(title.ends_with("..."));
        assert!(title.chars().count() <= 20);
    }
}
