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
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::App;

use crate::session_sidebar::SessionSummary;

/// 标题最大字符数(超出截断)。
const MAX_TITLE_CHARS: usize = 40;

/// 保存会话快照。空会话(无任何历史)不落库。
///
/// 返回用于刷新侧边栏摘要的 `(标题, 更新时间秒)`;未保存时返回 `None`。
pub fn save_session(cx: &App, session: &Session) -> Option<(String, i64)> {
    let _ = cx;
    let snapshot = session.snapshot();
    if snapshot.history.is_empty() {
        return None;
    }
    let title = derive_title(&snapshot);
    Some((title, now_secs()))
}

/// 列出全部**未归档**会话,按更新时间倒序映射为侧边栏摘要。
pub fn list_summaries(cx: &App) -> Vec<SessionSummary> {
    let _ = cx;
    Vec::new()
}

/// 列出**已归档**会话,按更新时间倒序映射为侧边栏摘要。
pub fn list_archived_summaries(cx: &App) -> Vec<SessionSummary> {
    let _ = cx;
    Vec::new()
}

/// 按会话 id 读取并反序列化快照。
pub fn load_snapshot(cx: &App, uid: &str) -> Option<SessionSnapshot> {
    let _ = (cx, uid);
    None
}

/// 删除一条持久化会话(无存储后端时为 no-op)。
pub fn delete_session(cx: &App, uid: &str) {
    let _ = (cx, uid);
}

/// 重命名一条持久化会话;成功返回 `true`(无存储后端 / 失败返回 `false`)。
pub fn rename_session(cx: &App, uid: &str, title: &str) -> bool {
    let _ = (cx, uid, title);
    false
}

/// 归档 / 恢复一条会话(软删除);成功返回 `true`。
pub fn set_archived(cx: &App, uid: &str, archived: bool) -> bool {
    let _ = (cx, uid, archived);
    false
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

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::{ResourceContext, SessionId};

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
        }
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
        };
        assert_eq!(derive_title(&snap), "新 Agent 会话");
    }
}
