//! Agent 会话持久化(集成胶水)。
//!
//! 把 `agent_runtime` 的 [`SessionSnapshot`] 与 `one-core` 的
//! [`AgentSessionRepository`] 接起来:快照 ↔ JSON 字符串的序列化在此完成,使
//! core 对快照内容保持不透明。所有函数在**缺少存储后端**(如未初始化
//! `GlobalStorageState` 的示例程序)时安全降级为 no-op / 空结果,绝不 panic。
//!
//! DB 操作为同步、低频(每轮结束保存一次 / 切换会话时读取一次)、载荷小,直接在
//! 调用线程执行,沿用项目既有的同步 Repository 调用风格。

use agent_runtime::{AcpSessionRef, HistoryItem, Session, SessionSnapshot};
use one_core::{
    llm::chat_history::{AgentSessionRepository, ChatMessage, MessageRepository},
    storage::GlobalStorageState,
    storage::traits::Repository,
};

use gpui::App;

use crate::session_sidebar::SessionSummary;

/// 标题最大字符数(超出截断)。
const MAX_TITLE_CHARS: usize = 40;

/// 带「工作区归属定格」的保存。
///
/// 归属规则：快照里已有 `workspace_root`（此前落过盘 / 从别处载入）则原样保留；
/// 否则写本次传入的当前工作区。之后无论外壳切到哪个工作区，该会话都留在
/// 首次落盘时的工作区（侧栏分组与底部新建对话下拉都以此为准）。
/// 不需要归属时传 `None`。
pub fn save_session_with_workspace(
    cx: &App,
    session: &Session,
    workspace_root: Option<&str>,
) -> Option<(String, i64)> {
    let mut snapshot = session.snapshot();
    if snapshot.history.is_empty() {
        return None;
    }
    if snapshot.workspace_root.is_none()
        && let Some(root) = workspace_root_non_empty(workspace_root)
    {
        snapshot.workspace_root = Some(root);
    }
    let title = derive_title(&snapshot);
    let snapshot_json = serde_json::to_string(&snapshot).ok()?;
    let repo = agent_session_repository(cx)?;
    let saved = repo
        .save_snapshot(&snapshot.id.to_string(), &title, &snapshot_json)
        .ok()?;
    Some((saved.title, saved.updated_at))
}

fn workspace_root_non_empty(root: Option<&str>) -> Option<String> {
    root.map(str::trim)
        .filter(|root| !root.is_empty())
        .map(str::to_string)
}

/// 追加一条上下文用量采样（历史趋势用）。
///
/// 与快照里的 `context_tokens` 分工：那份是「现在用了多少」的当前值，会被下一轮
/// 覆盖；这里是只追加的流水，画趋势、按天汇总靠它。
///
/// **去重靠「与最近一条相同就跳过」**：落盘路径一次轮次可能被触发多次
/// （切换会话 / 连接就绪 / 每轮结束都会调），没这层兜底就会把流水刷成一串重复点，
/// 趋势图除了变粗一无所获。上下文占用真的变了（涨或压缩后回落）才记。
///
/// 写完 / 跳过都返回 `true`；没存储后端时返回 `false`，不静默假装记下了。
pub fn record_usage_sample(
    cx: &App,
    uid: &str,
    used: u64,
    window: Option<u64>,
    model: Option<&str>,
) -> bool {
    let uid = uid.trim();
    if uid.is_empty() {
        return false;
    }
    let Some(repo) = agent_usage_repository(cx) else {
        return false;
    };
    // 窗口 0 等同于未知：先归一化再拿去比对，否则调用侧传一个 `Some(0)`
    // 会与库里存的 `NULL` 永远不相等，流水每轮都写一条。
    let window = window.filter(|window| *window > 0);
    let now = one_core::storage::manager::now();
    match repo.latest_for_uid(uid) {
        Ok(Some(latest))
            if latest.used == used
                && latest.window == window
                && latest.model.as_deref() == model =>
        {
            return true;
        }
        Ok(_) => {}
        // 读不到旧样本不该阻塞写入：让它当作第一条。
        Err(error) => tracing::warn!("读用量采样失败，按首次记录处理: {error}"),
    }
    let mut sample = one_core::llm::usage_history::AgentUsageSample::new(uid, used, now);
    sample.window = window;
    sample.model = model.map(str::to_string);
    match repo.record(&sample) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!("写用量采样失败: {error}");
            false
        }
    }
}

/// 删一条会话时顺手清掉它的用量流水。
pub fn clear_usage_samples(cx: &App, uid: &str) {
    if let Some(repo) = agent_usage_repository(cx) {
        let _ = repo.clear_uid(uid);
    }
}

/// `since`（含）之后的全部用量采样，按时间正序。
pub fn usage_samples_since(
    cx: &App,
    since: i64,
) -> Vec<one_core::llm::usage_history::AgentUsageSample> {
    agent_usage_repository(cx)
        .and_then(|repo| repo.list_since(since).ok())
        .unwrap_or_default()
}

fn agent_usage_repository(cx: &App) -> Option<std::sync::Arc<one_core::llm::AgentUsageRepository>> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<one_core::llm::AgentUsageRepository>())
}

/// 保存一条**由外部 agent 承载**的会话（ACP）。
///
/// 与 [`save_session_with_workspace`] 的关键差别：历史在外部 agent 那边，本地
/// `history` 就是空的，所以这里**不**按「空历史就跳过」处理——按那条规则走，
/// ACP 会话永远进不了侧栏（用户在会话列表里看不到自己刚聊过的一切）。
/// 本地只存「这个会话指向哪个 agent 的哪条协议会话」加一个可读标题。
///
/// 已有快照会被当作基底：工作区归属继续定格（首存优先）、输入草稿等本地字段保留，
/// 这里只更新外部地址与标题。标题留空时沿用首条用户消息的推导结果。
/// 内容没变化时仓储层自会跳过写入，不会因此把会话在侧栏里顶到最上面。
pub fn save_acp_session(
    cx: &App,
    uid: &str,
    title: &str,
    workspace_root: Option<&str>,
    acp: AcpSessionRef,
) -> Option<(String, i64)> {
    let uid = uid.trim();
    if uid.is_empty() || acp.agent_id.trim().is_empty() || acp.session_id.trim().is_empty() {
        return None;
    }
    let mut snapshot = load_snapshot(cx, uid).unwrap_or_else(|| empty_snapshot(uid));
    snapshot.acp = Some(acp);
    if snapshot.workspace_root.is_none()
        && let Some(root) = workspace_root_non_empty(workspace_root)
    {
        snapshot.workspace_root = Some(root);
    }
    let title = match title.trim() {
        "" => derive_title(&snapshot),
        text => truncate_title(text),
    };
    let snapshot_json = serde_json::to_string(&snapshot).ok()?;
    let repo = agent_session_repository(cx)?;
    let saved = repo.save_snapshot(uid, &title, &snapshot_json).ok()?;
    // 外部会话的快照内容几乎不变（地址一存就定），仓储层会跳过写入，更新时间就会
    // 永远停在首次落盘那一刻；而列表按时间倒序排，刚聊过的对话反而沉到底部。
    // 这里显式提一次时间：调这个函数的地方都是「真有过动静」。
    if let Ok(Some(row)) = repo.get_by_uid(uid) {
        let _ = repo.update(&row);
    }
    Some((saved.title, saved.updated_at))
}

/// 只有标识、没有对话内容的空快照；工作区与外部地址由调用方补。
fn empty_snapshot(uid: &str) -> SessionSnapshot {
    SessionSnapshot {
        id: agent_runtime::SessionId::from_string(uid.to_string()),
        resources: agent_runtime::ResourceContext::new(),
        history: Vec::new(),
        plan: None,
        system_instruction: None,
        skills: agent_runtime::SkillContext::new(),
        workspace_root: None,
        draft: None,
        context_tokens: None,
        acp: None,
    }
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

fn agent_session_repository(cx: &App) -> Option<std::sync::Arc<AgentSessionRepository>> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<AgentSessionRepository>())
}

fn message_repository(cx: &App) -> Option<std::sync::Arc<MessageRepository>> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<MessageRepository>())
}

fn list_summaries_by_archived(cx: &App, archived: bool) -> Vec<SessionSummary> {
    // 只取摘要行：`snapshot_json` 本体（实测本机未归档 356 行合计 76.8MB、单条
    // 最大 19.2MB）不再进入进程内存，工作区归属与外部 agent 来源这两个来自快照的
    // 字段由 SQL 层的 `json_extract` 直接给出。
    //
    // 这里以前是把整列快照读上来、再逐行整体反序列化两次（一次取 workspace_root、
    // 一次取 acp.agent_id）。列表刷新跑在 UI 线程上，且切会话 / 每轮结束 / ACP
    // 连接就绪都会触发，debug 构建下单次光解析就要 1.6s——界面会明显冻住。
    agent_session_repository(cx)
        .and_then(|repo| repo.list_summary_rows_by_archived(archived).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            SessionSummary::new(row.uid, row.title, row.updated_at)
                .with_workspace_root(snapshot_workspace_root(row.workspace_root))
                .with_external_agent(external_agent_label(row.acp_agent_id))
        })
        .collect()
}

/// 快照里的工作区归属：空白视为没有。
///
/// 与旧实现（解析 JSON 后取字段）保持同一判据：空串或纯空白都不算归属，
/// 否则侧栏会多出一个空的工作区分组。
fn snapshot_workspace_root(root: Option<String>) -> Option<String> {
    root.filter(|root| !root.trim().is_empty())
}

/// 会话来源标记：快照里记了外部 agent 时，用它的短标识标出这行归谁管。
///
/// 只从 id 的末段取名（`builtin.codex` → `codex`）：展示名在宿主的 agent 注册表里，
/// 持久化层拿不到；写进快照又会因为改名而过期。
fn external_agent_label(agent_id: Option<String>) -> Option<gpui::SharedString> {
    let agent_id = agent_id?;
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return None;
    }
    let label = agent_id
        .rsplit('.')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(agent_id);
    Some(gpui::SharedString::from(label.to_string()))
}

fn load_legacy_chat_snapshot(cx: &App, uid: &str) -> Option<SessionSnapshot> {
    let session_id = uid.parse::<i64>().ok()?;
    let messages = message_repository(cx)?.list_by_session(session_id).ok()?;
    Some(SessionSnapshot {
        history: messages
            .into_iter()
            .filter_map(chat_message_to_history)
            .collect(),
        ..empty_snapshot(uid)
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
            workspace_root: None,
            draft: None,
            context_tokens: None,
            acp: None,
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
        storage.register(one_core::llm::AgentUsageRepository::new(conn.clone()));
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

    #[gpui::test]
    fn usage_samples_are_appended_deduped_and_read_back(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });

        assert!(
            cx.update(|cx| record_usage_sample(cx, "sess_a", 12_000, Some(200_000), Some("gpt-5"))),
            "有存储后端时应该写进去"
        );
        // 同一读数重复上报（切会话 / 连接就绪都会触发落盘）不该刷流水。
        assert!(cx.update(|cx| record_usage_sample(
            cx,
            "sess_a",
            12_000,
            Some(200_000),
            Some("gpt-5")
        )));
        assert!(cx.update(|cx| record_usage_sample(cx, "sess_b", 3_000, None, None)));
        assert!(cx.update(|cx| record_usage_sample(
            cx,
            "sess_a",
            40_000,
            Some(200_000),
            Some("gpt-5")
        )));
        // 窗口从「未知」变成已知也算变化。
        assert!(cx.update(|cx| record_usage_sample(cx, "sess_b", 3_000, Some(128_000), None)));

        let all = cx.update(|cx| usage_samples_since(cx, 0));
        assert_eq!(4, all.len(), "重复读数被去重，其余都留下");
        assert_eq!(
            vec![12_000, 3_000, 40_000, 3_000],
            all.iter().map(|sample| sample.used).collect::<Vec<_>>(),
            "读回来按时间正序"
        );

        cx.update(|cx| clear_usage_samples(cx, "sess_a"));
        let left = cx.update(|cx| usage_samples_since(cx, 0));
        assert_eq!(2, left.len(), "删会话只清它自己的流水");
        assert!(left.iter().all(|sample| sample.uid == "sess_b"));
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
            workspace_root: None,
            draft: None,
            context_tokens: None,
            acp: None,
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
            .update(|cx| save_session_with_workspace(cx, &session, None))
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
    fn saving_unchanged_session_preserves_sidebar_updated_at(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });

        let session = test_session();
        session.record_user_input("不要因为点击会话而重新排序");
        let first_saved_at = cx
            .update(|cx| save_session_with_workspace(cx, &session, None))
            .expect("initial session save")
            .1;

        std::thread::sleep(std::time::Duration::from_secs(1));

        let second_saved_at = cx
            .update(|cx| save_session_with_workspace(cx, &session, None))
            .expect("unchanged session save")
            .1;

        assert_eq!(
            first_saved_at, second_saved_at,
            "saving an unchanged snapshot during navigation must not make the conversation look newly active"
        );
    }

    #[gpui::test]
    fn draft_round_trips_through_the_session_repository(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });

        let session = test_session();
        session.record_user_input("已有消息");
        session.set_draft(Some("还没发送的半句话".into()));
        cx.update(|cx| save_session_with_workspace(cx, &session, None))
            .expect("save session");

        let loaded = cx
            .update(|cx| load_snapshot(cx, "sess_persist"))
            .expect("load snapshot");
        assert_eq!(loaded.draft.as_deref(), Some("还没发送的半句话"));
    }

    #[gpui::test]
    fn legacy_chat_sessions_are_listed_and_loaded_as_agent_history(cx: &mut TestAppContext) {
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

    fn acp_ref(agent_id: &str, session_id: &str) -> AcpSessionRef {
        AcpSessionRef {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
        }
    }

    /// 外部 agent 会话的历史为空，但**必须**进侧栏，而且要标出来源：
    /// 「历史在别处」不能让用户在会话列表里找不到它。
    #[gpui::test]
    fn acp_sessions_are_listed_with_their_external_source(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });

        let saved = cx
            .update(|cx| {
                save_acp_session(
                    cx,
                    "sess_acp",
                    "查一下连接数",
                    Some("/w/project"),
                    acp_ref("builtin.codex", "acp-1"),
                )
            })
            .expect("保存外部会话");
        assert_eq!("查一下连接数", saved.0);

        let summaries = cx.update(|cx| list_summaries(cx));
        assert_eq!(1, summaries.len(), "空历史不该让外部会话从侧栏消失");
        assert_eq!("sess_acp", summaries[0].id);
        assert_eq!(Some("/w/project"), summaries[0].workspace_root.as_deref());
        assert_eq!(Some("codex"), summaries[0].external_agent.as_deref());

        let snapshot = cx
            .update(|cx| load_snapshot(cx, "sess_acp"))
            .expect("读回快照");
        assert!(snapshot.history.is_empty());
        assert_eq!(
            Some("acp-1".to_string()),
            snapshot.acp.as_ref().map(|acp| acp.session_id.clone())
        );
    }

    /// 再次落盘不能把会话的原有信息抹掉：工作区归属首存定格、草稿留在本地、
    /// 地址换成新那条会话。
    #[gpui::test]
    fn resaving_an_acp_session_keeps_the_local_shell_fields(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });
        let uid = "sess_acp";
        cx.update(|cx| {
            save_acp_session(
                cx,
                uid,
                "第一句话",
                Some("/w/first"),
                acp_ref("builtin.codex", "acp-1"),
            )
        })
        .expect("首次保存");

        // 用着用着输入框里存了半句话（本地字段，只在快照里）。
        cx.update(|cx| {
            let mut snapshot = load_snapshot(cx, uid).expect("读回快照");
            snapshot.draft = Some("还没发出去".into());
            let json = serde_json::to_string(&snapshot).expect("序列化");
            agent_session_repository(cx)
                .expect("仓储")
                .save_snapshot(uid, "第一句话", &json)
                .expect("写入草稿");
        });

        // 外壳换了工作区、用户在 agent 那边切了另一条会话（地址变了）。
        cx.update(|cx| {
            save_acp_session(
                cx,
                uid,
                "第一句话",
                Some("/w/second"),
                acp_ref("builtin.codex", "acp-2"),
            )
        })
        .expect("再次保存");

        let snapshot = cx.update(|cx| load_snapshot(cx, uid)).expect("读回快照");
        assert_eq!(Some("/w/first"), snapshot.workspace_root.as_deref());
        assert_eq!(Some("还没发出去"), snapshot.draft.as_deref());
        assert_eq!(
            Some("acp-2".to_string()),
            snapshot.acp.as_ref().map(|acp| acp.session_id.clone())
        );
    }

    /// 外部会话的快照一存就定，仓储层会跳过重复写入；更新时间必须由这次调用推上去，
    /// 否则刚聊过的对话反而沉在侧栏底部。
    #[gpui::test]
    fn resaving_an_acp_session_marks_it_active(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalStorageState {
                storage: test_storage(),
            });
        });
        let uid = "sess_acp";
        let first = cx
            .update(|cx| save_acp_session(cx, uid, "第一句话", None, acp_ref("agent-1", "acp-1")))
            .expect("首次保存")
            .1;

        std::thread::sleep(std::time::Duration::from_secs(1));
        cx.update(|cx| save_acp_session(cx, uid, "第一句话", None, acp_ref("agent-1", "acp-1")))
            .expect("重复保存");

        let summaries = cx.update(|cx| list_summaries(cx));
        assert!(
            summaries[0].updated_at > first,
            "同一轮对话再次活跃时应当提到列表前面"
        );
    }

    /// 历史数据里存在 `snapshot_json` 为空串 / 非合法 JSON 的行（早期 chat 会话）。
    ///
    /// 摘要现在由 SQL 层的 `json_extract` 取字段，缺了 `json_valid` 守卫就会
    /// `malformed JSON` 报错——**整条查询失败**，侧栏会话列表会整体变空。
    /// 本测试同时钉住「脏行不丢」与「好行照常提取」。
    #[gpui::test]
    fn malformed_snapshots_do_not_blank_the_session_list(cx: &mut TestAppContext) {
        let storage = test_storage();
        let conn = storage.connection();
        cx.update(|cx| {
            cx.set_global(GlobalStorageState { storage });
        });
        cx.update(|cx| {
            save_acp_session(
                cx,
                "sess_ok",
                "正常会话",
                Some("/w/ok"),
                acp_ref("builtin.codex", "acp-1"),
            )
            .expect("保存正常会话");
        });
        conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO chat_sessions
                   (name, provider_id, session_kind, uid, snapshot_json, archived, created_at, updated_at)
                 VALUES
                   ('空快照', 'legacy', 'chat', '9001', '', 0, 1, 1),
                   ('坏快照', 'legacy', 'chat', '9002', 'not json', 0, 2, 2)",
                [],
            )?;
            Ok(())
        })
        .expect("插入脏快照行");

        let summaries = cx.update(|cx| list_summaries(cx));

        assert_eq!(
            3,
            summaries.len(),
            "脏快照不能把整条列表查询带崩（json_valid 守卫）: {summaries:?}"
        );
        let ok = summaries
            .iter()
            .find(|summary| summary.id == "sess_ok")
            .expect("正常会话仍应在列表里");
        assert_eq!(Some("/w/ok"), ok.workspace_root.as_deref());
        assert_eq!(Some("codex"), ok.external_agent.as_deref());
        let broken = summaries
            .iter()
            .find(|summary| summary.id == "9002")
            .expect("坏快照行也要保留，只是没有工作区与来源");
        assert_eq!(None, broken.workspace_root);
        assert_eq!(None, broken.external_agent);
    }

    /// 工作区归属的判据：空串 / 纯空白都不算归属，否则侧栏会多出一个空分组。
    ///
    /// 这条以前挂在 `session_sidebar::workspace_root_from_snapshot_json` 上（从整份
    /// JSON 里取字段）。摘要改成 SQL 层提取后字段已是解好的字符串，判据挪到这里。
    #[test]
    fn blank_workspace_roots_are_not_a_workspace() {
        assert_eq!(None, snapshot_workspace_root(None));
        assert_eq!(None, snapshot_workspace_root(Some(String::new())));
        assert_eq!(None, snapshot_workspace_root(Some("   ".into())));
        assert_eq!(
            Some("/w".to_string()),
            snapshot_workspace_root(Some("/w".into()))
        );
    }

    /// 来源标记只取 id 末段；空 / 纯空白不算来源。
    #[test]
    fn external_agent_labels_use_the_last_segment() {
        assert_eq!(
            Some("codex".to_string()),
            external_agent_label(Some("builtin.codex".into())).map(|l| l.to_string())
        );
        assert_eq!(
            Some("opencode-acp".to_string()),
            external_agent_label(Some("opencode-acp.opencode-acp".into())).map(|l| l.to_string())
        );
        assert_eq!(None, external_agent_label(None));
        assert_eq!(None, external_agent_label(Some("  ".into())));
    }
}
