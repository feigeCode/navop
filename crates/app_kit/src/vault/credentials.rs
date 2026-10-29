//! 凭据库弹窗（三产品共享）。
//!
//! 弹窗形态而不是 Tab 标签页：独立产品没有 tab 容器，弹窗在两种宿主里都能直接
//! 打开。锁定状态也允许打开——摘要（名称 / 用户名 / 有哪些敏感字段）不加密，
//! 只有编辑到密文时才要求主密钥。

use gpui::{App, AppContext as _, Context, FocusHandle, Focusable, Subscription, Window};
use gpui_component::WindowExt as _;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::notification::Notification;
use one_core::cloud_sync::personal::PersonalSyncConflictRepository;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::crypto;
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::storage::{
    CredentialRepository, CredentialSummary, DeleteCredentialOutcome, GlobalStorageState,
    StorageManager,
};

use crate::vault::credential_form;

const REUSE_KEY: &str = "app-kit-credential-vault";

/// 打开凭据库弹窗（已打开则复用同一个窗口）。
pub fn open_credential_vault(window: &mut Window, cx: &mut App) {
    open_reusable_popup_window(
        PopupWindowOptions::new("钥匙串").size(860., 640.),
        REUSE_KEY,
        move |window, cx| cx.new(|cx| CredentialVaultView::new(window, cx)),
        Some(window),
        cx,
    );
}

pub struct CredentialVaultView {
    pub(super) focus_handle: FocusHandle,
    storage: StorageManager,
    pub(super) search_input: gpui::Entity<InputState>,
    pub(super) summaries: Vec<CredentialSummary>,
    pub(super) load_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CredentialVaultView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("按名称、类型或用户名搜索"));
        let subscription = cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            storage,
            summaries: Vec::new(),
            search_input,
            load_error: None,
            _subscriptions: vec![subscription],
        };
        view.reload(cx);
        view
    }

    fn repository(&self) -> Result<std::sync::Arc<CredentialRepository>, String> {
        self.storage
            .get::<CredentialRepository>()
            .ok_or_else(|| "凭据仓库尚未注册".to_string())
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        match self
            .repository()
            .and_then(|repo| repo.list_summaries().map_err(|e| e.to_string()))
        {
            Ok(summaries) => {
                self.summaries = summaries;
                self.load_error = None;
            }
            Err(error) => self.load_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn filtered(&self, cx: &App) -> Vec<CredentialSummary> {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        if query.is_empty() {
            return self.summaries.clone();
        }
        self.summaries
            .iter()
            .filter(|summary| {
                summary.name.to_lowercase().contains(&query)
                    || summary
                        .username
                        .as_deref()
                        .is_some_and(|user| user.to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    pub(super) fn open_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !require_unlocked(window, cx) {
            return;
        }
        let view = cx.entity();
        credential_form::show_credential_form(
            window,
            cx,
            None,
            Some(Box::new(move |cx: &mut App| {
                let _ = view.update(cx, |view, cx| view.reload(cx));
            })),
        );
    }

    pub(super) fn open_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !require_unlocked(window, cx) {
            return;
        }
        let loaded = self
            .repository()
            .and_then(|repo| repo.get_plaintext(id).map_err(|error| error.to_string()));
        match loaded {
            Ok(Some(entry)) => {
                let view = cx.entity();
                credential_form::show_credential_form(
                    window,
                    cx,
                    Some(entry),
                    Some(Box::new(move |cx: &mut App| {
                        let _ = view.update(cx, |view, cx| view.reload(cx));
                    })),
                );
            }
            Ok(None) => push_error(window, cx, "凭据不存在，可能已被删除。"),
            Err(error) => push_error(
                window,
                cx,
                &format!("无法打开凭据：{error}。若钥匙串已锁定，请先解锁主密钥。"),
            ),
        }
    }

    pub(super) fn confirm_delete(
        &mut self,
        id: i64,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = self.storage.clone();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let storage = storage.clone();
            let view = view.clone();
            alert
                .title("删除凭据")
                .description(format!(
                    "确定删除“{name}”吗？如果有连接仍在引用它，删除会被安全拒绝。"
                ))
                .confirm()
                .on_ok(move |_, window, cx: &mut App| {
                    delete_credential(id, &storage, &view, window, cx)
                })
        });
    }
}

/// 需要主密钥的操作：未解锁时先弹解锁框并提示。
fn require_unlocked(window: &mut Window, cx: &mut App) -> bool {
    if crypto::has_master_key() {
        return true;
    }
    crate::vault::unlock::show_master_key_dialog(window, cx);
    window.push_notification(
        Notification::warning("请先解锁主密钥后再编辑凭据").autohide(true),
        cx,
    );
    false
}

fn delete_credential(
    id: i64,
    storage: &StorageManager,
    view: &gpui::Entity<CredentialVaultView>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    match delete_and_collect(storage, id) {
        Ok((DeleteCredentialOutcome::Deleted, cloud_id, last_synced_at)) => {
            finish_deleted(id, cloud_id, last_synced_at, storage, view, window, cx);
            true
        }
        Ok((DeleteCredentialOutcome::NotFound, _, _)) => {
            let _ = view.update(cx, |view, cx| view.reload(cx));
            window.push_notification(Notification::warning("凭据已不存在").autohide(true), cx);
            true
        }
        Ok((DeleteCredentialOutcome::Referenced(hits), _, _)) => {
            window.push_notification(
                Notification::error(referenced_message(&hits)).autohide(false),
                cx,
            );
            false
        }
        Err(error) => {
            push_error(window, cx, &format!("删除凭据失败：{error}"));
            false
        }
    }
}

/// 删之前把摘要读出来：`cloud_id` 用于清理云端与同步冲突，`last_synced_at` 是
/// 个人同步判断"远端是否在本地删除后被改过"的基线。
fn delete_and_collect(
    storage: &StorageManager,
    id: i64,
) -> Result<(DeleteCredentialOutcome, Option<String>, Option<i64>), String> {
    let repo = storage
        .get::<CredentialRepository>()
        .ok_or_else(|| "凭据仓库尚未注册".to_string())?;
    let summary = repo.get_summary(id).map_err(|error| error.to_string())?;
    let cloud_id = summary
        .as_ref()
        .and_then(|summary| summary.cloud_id.clone());
    let last_synced_at = summary.and_then(|summary| summary.last_synced_at);
    let outcome = repo.delete_checked(id).map_err(|error| error.to_string())?;
    Ok((outcome, cloud_id, last_synced_at))
}

/// 删除成功：清理同步冲突、广播事件、刷新列表
fn finish_deleted(
    id: i64,
    cloud_id: Option<String>,
    last_synced_at: Option<i64>,
    storage: &StorageManager,
    view: &gpui::Entity<CredentialVaultView>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(cloud_id) = &cloud_id {
        forget_conflict(storage, cloud_id);
    }
    emit_connection_event_from_app(
        ConnectionDataEvent::CredentialDeleted {
            credential_id: id,
            cloud_id,
            last_synced_at,
        },
        cx,
    );
    let _ = view.update(cx, |view, cx| view.reload(cx));
    window.push_notification(Notification::success("凭据已删除").autohide(true), cx);
}

/// 被引用时列出前 5 个连接，剩下的用数量概括
fn referenced_message(hits: &[one_core::storage::CredentialReferenceHit]) -> String {
    let mut message = String::from("该凭据仍被以下连接引用，请先取消引用：");
    for hit in hits.iter().take(5) {
        message.push_str(&format!(
            "\n· {}（连接 #{}）",
            hit.connection_name, hit.connection_id
        ));
    }
    if hits.len() > 5 {
        message.push_str(&format!("\n… 另有 {} 个", hits.len() - 5));
    }
    message
}

/// 本地条目消失后，遗留的同步冲突再也解不开，跟着删除。
fn forget_conflict(storage: &StorageManager, cloud_id: &str) {
    let Some(conflicts) = storage.get::<PersonalSyncConflictRepository>() else {
        return;
    };
    if let Err(error) = conflicts.delete(
        "personal",
        one_core::cloud_sync::data_type::CREDENTIAL,
        cloud_id,
    ) {
        tracing::warn!(%error, %cloud_id, "删除凭据后清理个人同步冲突失败");
    }
}

fn push_error(window: &mut Window, cx: &mut App, message: &str) {
    window.push_notification(Notification::error(message.to_string()).autohide(true), cx);
}

impl Focusable for CredentialVaultView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
