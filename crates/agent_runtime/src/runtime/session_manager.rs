//! 会话管理器:按 ID 持有所有活跃会话。

use crate::ids::SessionId;
use crate::runtime::session::Session;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 会话注册表。
#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<SessionId, Arc<Session>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: Arc<Session>) {
        self.sessions
            .lock()
            .expect("session manager 锁中毒")
            .insert(session.id().clone(), session);
    }

    pub fn get(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .expect("session manager 锁中毒")
            .get(id)
            .cloned()
    }

    pub fn remove(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .expect("session manager 锁中毒")
            .remove(id)
    }

    pub fn list(&self) -> Vec<SessionId> {
        self.sessions
            .lock()
            .expect("session manager 锁中毒")
            .keys()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.sessions.lock().expect("session manager 锁中毒").len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions
            .lock()
            .expect("session manager 锁中毒")
            .is_empty()
    }
}
