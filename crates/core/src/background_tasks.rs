//! 全局后台任务状态与控制中心。
//!
//! 管理器本身不强制拥有业务 future。长期任务可以：
//! - 自行执行并通过 [`BackgroundTaskHandle`] 上报状态；
//! - 使用 [`spawn`] 让管理器统一注册、启动、取消和落终态。
//!
//! 所有更新都发生在 GPUI 前台线程，业务线程只持有一个轻量句柄，
//! 避免把 UI Entity 泄漏到后台执行逻辑中。

use chrono::{DateTime, Utc};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, Global, SharedString, Task,
    WeakEntity,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// 后台任务稳定标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackgroundTaskId(u64);

impl BackgroundTaskId {
    fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for BackgroundTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bg-task-{}", self.0)
    }
}

/// 任务生命周期状态。`Cancelling` 只表示取消请求已发出，
/// 任务必须等执行方确认后进入 `Cancelled` 终态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundTaskStatus {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl BackgroundTaskStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub fn can_request_cancel(&self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// 进度单位，用于面板和后续调用方做人性化展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskProgressUnit {
    Bytes,
    Items,
    Percent,
    Steps,
}

impl BackgroundTaskProgressUnit {
    pub fn format_value(&self, value: u64) -> String {
        match self {
            Self::Bytes => format_bytes(value),
            Self::Items | Self::Steps => value.to_string(),
            Self::Percent => format!("{value}%"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTaskProgress {
    pub current: u64,
    pub total: Option<u64>,
    pub unit: BackgroundTaskProgressUnit,
    pub message: Option<SharedString>,
}

impl BackgroundTaskProgress {
    pub fn bytes(current: u64, total: Option<u64>) -> Self {
        Self {
            current,
            total,
            unit: BackgroundTaskProgressUnit::Bytes,
            message: None,
        }
    }

    pub fn items(current: u64, total: Option<u64>) -> Self {
        Self {
            current,
            total,
            unit: BackgroundTaskProgressUnit::Items,
            message: None,
        }
    }

    pub fn percent(&self) -> u32 {
        match self.total {
            Some(total) if total > 0 => {
                (((u128::from(self.current) * 100) / u128::from(total)).min(100)) as u32
            }
            _ => 0,
        }
    }

    pub fn display(&self) -> String {
        match (self.total, self.message.as_deref()) {
            (Some(total), _) => format!(
                "{} / {} ({:.0}%)",
                self.unit.format_value(self.current),
                self.unit.format_value(total),
                self.percent()
            ),
            (None, Some(message)) => {
                format!("{} · {message}", self.unit.format_value(self.current))
            }
            (None, None) => self.unit.format_value(self.current),
        }
    }
}

/// 统一注册描述。
#[derive(Debug, Clone)]
pub struct BackgroundTaskSpec {
    pub kind: SharedString,
    pub group: Option<SharedString>,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub key: Option<SharedString>,
    pub open_folder: Option<PathBuf>,
    pub cancellable: bool,
    pub progress_unit: BackgroundTaskProgressUnit,
}

impl BackgroundTaskSpec {
    pub fn new(kind: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            kind: kind.into(),
            group: None,
            title: title.into(),
            detail: None,
            key: None,
            open_folder: None,
            cancellable: true,
            progress_unit: BackgroundTaskProgressUnit::Steps,
        }
    }

    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn open_folder(mut self, path: impl Into<PathBuf>) -> Self {
        self.open_folder = Some(path.into());
        self
    }

    pub fn cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }

    pub fn progress_unit(mut self, unit: BackgroundTaskProgressUnit) -> Self {
        self.progress_unit = unit;
        self
    }
}

/// 任务终态结果。成功和取消可以携带简短结果/原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundTaskOutcome {
    Succeeded(Option<SharedString>),
    Failed(String),
    Cancelled(Option<SharedString>),
}

/// 面板可见的任务快照。控制句柄故意不放入快照，避免 UI 持有执行逻辑。
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: BackgroundTaskId,
    pub group: Option<SharedString>,
    pub key: Option<SharedString>,
    pub kind: SharedString,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub open_folder: Option<PathBuf>,
    pub cancellable: bool,
    pub progress_unit: BackgroundTaskProgressUnit,
    pub status: BackgroundTaskStatus,
    pub progress: Option<BackgroundTaskProgress>,
    pub result: Option<SharedString>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl BackgroundTask {
    pub fn percent(&self) -> u32 {
        self.progress.as_ref().map(|p| p.percent()).unwrap_or(0)
    }

    pub fn can_cancel(&self) -> bool {
        self.cancellable && self.status.can_request_cancel()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackgroundTaskCounts {
    pub queued: usize,
    pub running: usize,
    pub cancelling: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundTaskEvent {
    Added(BackgroundTaskId),
    Updated(BackgroundTaskId),
    Removed(Vec<BackgroundTaskId>),
}

/// 面板过滤器。六个取值按状态互斥划分：等待中（排队）、进行中（含取消中）、
/// 已完成（成功）、已取消、失败，`All` 为全集。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskFilter {
    All,
    Queued,
    Running,
    Succeeded,
    Cancelled,
    Failed,
}

impl BackgroundTaskFilter {
    /// 过滤栏的固定展示顺序。
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Queued,
        Self::Running,
        Self::Succeeded,
        Self::Cancelled,
        Self::Failed,
    ];

    pub fn matches(self, task: &BackgroundTask) -> bool {
        match self {
            Self::All => true,
            Self::Queued => task.status == BackgroundTaskStatus::Queued,
            Self::Running => matches!(
                task.status,
                BackgroundTaskStatus::Running | BackgroundTaskStatus::Cancelling
            ),
            Self::Succeeded => task.status == BackgroundTaskStatus::Succeeded,
            Self::Cancelled => task.status == BackgroundTaskStatus::Cancelled,
            Self::Failed => task.status == BackgroundTaskStatus::Failed,
        }
    }

    /// 该过滤桶在给定聚合数量下的任务数，用于过滤 tab 上跟随的数量展示。
    pub fn count(self, counts: BackgroundTaskCounts) -> usize {
        match self {
            Self::All => {
                counts.queued
                    + counts.running
                    + counts.cancelling
                    + counts.succeeded
                    + counts.failed
                    + counts.cancelled
            }
            Self::Queued => counts.queued,
            Self::Running => counts.running + counts.cancelling,
            Self::Succeeded => counts.succeeded,
            Self::Cancelled => counts.cancelled,
            Self::Failed => counts.failed,
        }
    }
}

type CancelCallback = Arc<dyn Fn() + Send + Sync>;
type CancelResultCallback = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub struct BackgroundTaskCancellation {
    token: Option<CancellationToken>,
    callback: Option<CancelCallback>,
    result_callback: Option<CancelResultCallback>,
}

impl BackgroundTaskCancellation {
    pub fn token(token: CancellationToken) -> Self {
        Self {
            token: Some(token),
            callback: None,
            result_callback: None,
        }
    }

    pub fn callback(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            token: None,
            callback: Some(Arc::new(callback)),
            result_callback: None,
        }
    }

    pub fn token_and_callback(
        token: CancellationToken,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            token: Some(token),
            callback: Some(Arc::new(callback)),
            result_callback: None,
        }
    }

    pub fn callback_with_result(callback: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            token: None,
            callback: None,
            result_callback: Some(Arc::new(callback)),
        }
    }

    fn trigger(&self) -> bool {
        let token_cancelled = self.token.as_ref().is_some_and(|t| {
            t.cancel();
            true
        });
        let callback_called = self.callback.as_ref().is_some_and(|cb| {
            cb();
            true
        });
        let result_callback_called = self.result_callback.as_ref().is_some_and(|cb| cb());
        token_cancelled || callback_called || result_callback_called
    }

    fn is_configured(&self) -> bool {
        self.token.is_some() || self.callback.is_some() || self.result_callback.is_some()
    }
}

#[derive(Clone)]
pub struct GlobalBackgroundTaskManager(pub Entity<BackgroundTaskManager>);

impl Global for GlobalBackgroundTaskManager {}

pub struct BackgroundTaskManager {
    next_id: u64,
    tasks: Vec<BackgroundTask>,
    cancellations: Arc<std::sync::Mutex<Vec<(BackgroundTaskId, BackgroundTaskCancellation)>>>,
    max_finished: usize,
}

impl EventEmitter<BackgroundTaskEvent> for BackgroundTaskManager {}

impl BackgroundTaskManager {
    const DEFAULT_MAX_FINISHED: usize = 200;

    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            next_id: 0,
            tasks: Vec::new(),
            cancellations: Arc::new(std::sync::Mutex::new(Vec::new())),
            max_finished: Self::DEFAULT_MAX_FINISHED,
        })
    }

    pub fn register(
        &mut self,
        spec: BackgroundTaskSpec,
        cx: &mut Context<Self>,
    ) -> BackgroundTaskId {
        let id = self.allocate_id();
        let now = Utc::now();
        self.tasks.push(BackgroundTask {
            id,
            group: spec.group,
            key: spec.key,
            kind: spec.kind,
            title: spec.title,
            detail: spec.detail,
            open_folder: spec.open_folder,
            cancellable: spec.cancellable,
            progress_unit: spec.progress_unit,
            status: BackgroundTaskStatus::Queued,
            progress: None,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
        });
        let removed = self.trim_finished();
        cx.emit(BackgroundTaskEvent::Added(id));
        if !removed.is_empty() {
            cx.emit(BackgroundTaskEvent::Removed(removed));
        }
        cx.notify();
        id
    }

    /// 兼容快速注册的旧式 API；新代码建议使用 [`BackgroundTaskSpec`]。
    pub fn register_simple(
        &mut self,
        kind: impl Into<SharedString>,
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        key: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> BackgroundTaskId {
        let mut spec = BackgroundTaskSpec::new(kind, title);
        spec.detail = detail;
        spec.key = key;
        self.register(spec, cx)
    }

    /// 只查找进行中的 key。终态历史可重复使用同一 key 创建新任务。
    pub fn find_by_key(&self, key: &str) -> Option<BackgroundTaskId> {
        self.tasks
            .iter()
            .find(|task| task.status.is_active() && task.key.as_deref() == Some(key))
            .map(|task| task.id)
    }

    pub fn find_latest_by_key(&self, key: &str) -> Option<BackgroundTaskId> {
        self.tasks
            .iter()
            .rev()
            .find(|task| task.key.as_deref() == Some(key))
            .map(|task| task.id)
    }

    pub fn ensure_by_key(
        &mut self,
        spec: BackgroundTaskSpec,
        key: SharedString,
        cx: &mut Context<Self>,
    ) -> BackgroundTaskId {
        if let Some(existing) = self.find_by_key(&key) {
            return existing;
        }
        self.register(spec.key(key), cx)
    }

    /// 为任务绑定取消控制。通常由 `spawn` 或业务注册入口调用。
    pub fn set_cancellation(
        &mut self,
        id: BackgroundTaskId,
        cancellation: BackgroundTaskCancellation,
        _cx: &mut Context<Self>,
    ) {
        if !self
            .tasks
            .iter()
            .any(|task| task.id == id && task.cancellable && task.status.can_request_cancel())
            || !cancellation.is_configured()
        {
            return;
        }
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.retain(|(task_id, _)| *task_id != id);
            cancellations.push((id, cancellation));
        }
    }

    /// 读取任务的取消 token。业务自行执行的任务可用它监听取消请求。
    pub fn cancellation_token(&self, id: BackgroundTaskId) -> Option<CancellationToken> {
        if !self
            .tasks
            .iter()
            .any(|task| task.id == id && task.cancellable && task.status.can_request_cancel())
        {
            return None;
        }
        self.cancellations.lock().ok().and_then(|cancellations| {
            cancellations
                .iter()
                .find(|(task_id, _)| *task_id == id)
                .and_then(|(_, cancellation)| cancellation.token.clone())
        })
    }

    /// 发出取消请求。这里不会直接伪造终态；执行方收到 token/callback 后
    /// 必须调用 `finish(..., Cancelled(...))` 确认停止。
    pub fn request_cancel(&mut self, id: BackgroundTaskId, cx: &mut Context<Self>) -> bool {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return false;
        };
        if !task.can_cancel() {
            return false;
        }

        let cancellation = self.cancellations.lock().ok().and_then(|cancellations| {
            cancellations
                .iter()
                .find(|(task_id, _)| *task_id == id)
                .map(|(_, cancellation)| cancellation.clone())
        });
        let Some(cancellation) = cancellation.filter(BackgroundTaskCancellation::is_configured)
        else {
            // 可取消任务必须在注册后绑定控制句柄；否则这是调用方编程错误。
            tracing::warn!(task = %id, "cancellable background task has no cancellation controller");
            return false;
        };

        // 先验证并触发真实控制器，再对外暴露 Cancelling，避免任务永远卡在取消中。
        if !cancellation.trigger() {
            return false;
        }
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return false;
        };
        task.status = BackgroundTaskStatus::Cancelling;
        task.updated_at = Utc::now();
        cx.emit(BackgroundTaskEvent::Updated(id));
        cx.notify();
        true
    }

    pub fn cancel_all_active(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| task.can_cancel())
            .map(|task| task.id)
            .collect();
        for id in ids {
            self.request_cancel(id, cx);
        }
    }

    pub fn tasks(&self) -> Vec<BackgroundTask> {
        let mut tasks = self.tasks.clone();
        tasks.reverse();
        tasks
    }

    pub fn filtered_tasks(&self, filter: BackgroundTaskFilter) -> Vec<BackgroundTask> {
        self.tasks()
            .into_iter()
            .filter(|t| filter.matches(t))
            .collect()
    }

    pub fn counts(&self) -> BackgroundTaskCounts {
        let mut counts = BackgroundTaskCounts::default();
        for task in &self.tasks {
            match task.status {
                BackgroundTaskStatus::Queued => counts.queued += 1,
                BackgroundTaskStatus::Running => counts.running += 1,
                BackgroundTaskStatus::Cancelling => counts.cancelling += 1,
                BackgroundTaskStatus::Succeeded => counts.succeeded += 1,
                BackgroundTaskStatus::Failed => counts.failed += 1,
                BackgroundTaskStatus::Cancelled => counts.cancelled += 1,
            }
        }
        counts.active = counts.queued + counts.running + counts.cancelling;
        counts
    }

    pub fn mark_running(&mut self, id: BackgroundTaskId, cx: &mut Context<Self>) {
        self.update_task(
            id,
            |task| {
                if task.status == BackgroundTaskStatus::Queued {
                    task.status = BackgroundTaskStatus::Running;
                    true
                } else {
                    false
                }
            },
            cx,
        );
    }

    pub fn update_progress(
        &mut self,
        id: BackgroundTaskId,
        current: u64,
        total: Option<u64>,
        detail: Option<SharedString>,
        message: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.update_task(
            id,
            |task| {
                if !matches!(
                    task.status,
                    BackgroundTaskStatus::Running | BackgroundTaskStatus::Cancelling
                ) {
                    return false;
                }
                let unit = task
                    .progress
                    .as_ref()
                    .map(|progress| progress.unit)
                    .unwrap_or(task.progress_unit);
                let progress = BackgroundTaskProgress {
                    current,
                    total,
                    unit,
                    message,
                };
                let mut changed = task.progress.as_ref() != Some(&progress);
                task.progress = Some(progress);
                if let Some(detail) = detail {
                    if task.detail.as_ref() != Some(&detail) {
                        task.detail = Some(detail);
                        changed = true;
                    }
                }
                changed
            },
            cx,
        );
    }

    pub fn finish(
        &mut self,
        id: BackgroundTaskId,
        outcome: BackgroundTaskOutcome,
        cx: &mut Context<Self>,
    ) {
        let changed = self.update_task(
            id,
            |task| {
                let allowed = match (&task.status, &outcome) {
                    (
                        BackgroundTaskStatus::Running,
                        BackgroundTaskOutcome::Succeeded(_)
                        | BackgroundTaskOutcome::Failed(_)
                        | BackgroundTaskOutcome::Cancelled(_),
                    ) => true,
                    (
                        BackgroundTaskStatus::Queued,
                        BackgroundTaskOutcome::Failed(_) | BackgroundTaskOutcome::Cancelled(_),
                    ) => true,
                    (BackgroundTaskStatus::Cancelling, BackgroundTaskOutcome::Cancelled(_)) => true,
                    _ => false,
                };
                if !allowed {
                    return false;
                }
                match outcome {
                    BackgroundTaskOutcome::Succeeded(result) => {
                        task.status = BackgroundTaskStatus::Succeeded;
                        task.result = result;
                        task.error = None;
                        if let Some(progress) = &mut task.progress {
                            // 成功即视为完成：把进度推进到总量，避免停留在最后一次
                            // 更新的百分比（如下载 98/100 即成功时仍显示 98%）。
                            if let Some(total) = progress.total {
                                progress.current = total;
                            } else if progress.current > 0 {
                                // 总量未知的任务也要在成功时显示 100%，而不是停在 0。
                                progress.total = Some(progress.current);
                            } else {
                                progress.current = 1;
                                progress.total = Some(1);
                            }
                            progress.message = None;
                        }
                    }
                    BackgroundTaskOutcome::Failed(error) => {
                        task.status = BackgroundTaskStatus::Failed;
                        task.error = Some(error);
                        task.result = None;
                    }
                    BackgroundTaskOutcome::Cancelled(reason) => {
                        task.status = BackgroundTaskStatus::Cancelled;
                        task.result = reason;
                        task.error = None;
                    }
                }
                true
            },
            cx,
        );
        if changed && let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.retain(|(task_id, _)| *task_id != id);
        }
    }

    pub fn succeed(
        &mut self,
        id: BackgroundTaskId,
        result: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.finish(id, BackgroundTaskOutcome::Succeeded(result), cx);
    }

    pub fn fail(&mut self, id: BackgroundTaskId, error: impl Into<String>, cx: &mut Context<Self>) {
        self.finish(id, BackgroundTaskOutcome::Failed(error.into()), cx);
    }

    pub fn cancel_confirmed(
        &mut self,
        id: BackgroundTaskId,
        reason: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.finish(id, BackgroundTaskOutcome::Cancelled(reason), cx);
    }

    pub fn clear_finished(&mut self, cx: &mut Context<Self>) {
        let before = self.tasks.len();
        let finished: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .map(|task| task.id)
            .collect();
        self.tasks.retain(|task| task.status.is_active());
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.retain(|(id, _)| !finished.contains(id));
        }
        if self.tasks.len() != before {
            cx.emit(BackgroundTaskEvent::Removed(finished));
            cx.notify();
        }
    }

    /// 执行一次可判定是否实际修改的更新。no-op 不更新时间戳，也不触发 UI 刷新。
    fn update_task(
        &mut self,
        id: BackgroundTaskId,
        mutate: impl FnOnce(&mut BackgroundTask) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return false;
        };
        if !mutate(task) {
            return false;
        }
        task.updated_at = Utc::now();
        cx.emit(BackgroundTaskEvent::Updated(id));
        cx.notify();
        true
    }

    fn allocate_id(&mut self) -> BackgroundTaskId {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        BackgroundTaskId::new(self.next_id)
    }

    fn trim_finished(&mut self) -> Vec<BackgroundTaskId> {
        let finished_count = self
            .tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .count();
        if finished_count <= self.max_finished {
            return Vec::new();
        }
        let excess = finished_count - self.max_finished;
        let mut removed = 0;
        let mut removed_ids = Vec::with_capacity(excess);
        self.tasks.retain(|task| {
            if removed < excess && task.status.is_terminal() {
                removed += 1;
                removed_ids.push(task.id);
                false
            } else {
                true
            }
        });
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.retain(|(id, _)| !removed_ids.contains(id));
        }
        removed_ids
    }
}

/// 注册全局后台任务管理器。应在应用初始化早期调用。
pub fn init(cx: &mut App) {
    if try_global(cx).is_some() {
        return;
    }
    let manager = BackgroundTaskManager::new(cx);
    cx.set_global(GlobalBackgroundTaskManager(manager));
}

pub fn try_global(cx: &App) -> Option<Entity<BackgroundTaskManager>> {
    cx.try_global::<GlobalBackgroundTaskManager>()
        .map(|global| global.0.clone())
}

pub fn global(cx: &mut App) -> Entity<BackgroundTaskManager> {
    try_global(cx).unwrap_or_else(|| {
        let manager = BackgroundTaskManager::new(cx);
        cx.set_global(GlobalBackgroundTaskManager(manager.clone()));
        manager
    })
}

/// 轻量任务句柄。可在 UI/业务线程间克隆传递；管理器消失时更新自动 no-op。
#[derive(Clone)]
pub struct BackgroundTaskHandle {
    manager: WeakEntity<BackgroundTaskManager>,
    id: BackgroundTaskId,
}

impl BackgroundTaskHandle {
    pub fn new(manager: WeakEntity<BackgroundTaskManager>, id: BackgroundTaskId) -> Self {
        Self { manager, id }
    }

    pub fn id(&self) -> BackgroundTaskId {
        self.id
    }

    pub fn upgrade(&self, _cx: &App) -> Option<Entity<BackgroundTaskManager>> {
        self.manager.upgrade()
    }

    pub fn cancellation_token(&self, cx: &App) -> Option<CancellationToken> {
        self.manager.upgrade()?.read(cx).cancellation_token(self.id)
    }

    pub fn request_cancel(&self, cx: &mut App) -> bool {
        let Some(manager) = self.manager.upgrade() else {
            return false;
        };
        manager.update(cx, |manager, cx| manager.request_cancel(self.id, cx))
    }

    pub fn mark_running(&self, cx: &mut App) {
        self.with_manager(cx, |m, cx| m.mark_running(self.id, cx));
    }

    pub fn update_progress(
        &self,
        current: u64,
        total: Option<u64>,
        detail: Option<SharedString>,
        message: Option<SharedString>,
        cx: &mut App,
    ) {
        self.with_manager(cx, |m, cx| {
            m.update_progress(self.id, current, total, detail, message, cx);
        });
    }

    pub fn succeed(&self, result: Option<SharedString>, cx: &mut App) {
        self.with_manager(cx, |m, cx| m.succeed(self.id, result, cx));
    }

    pub fn fail(&self, error: impl Into<String>, cx: &mut App) {
        self.with_manager(cx, |m, cx| m.fail(self.id, error, cx));
    }

    pub fn cancel_confirmed(&self, reason: Option<SharedString>, cx: &mut App) {
        self.with_manager(cx, |m, cx| m.cancel_confirmed(self.id, reason, cx));
    }

    pub fn finish(&self, outcome: BackgroundTaskOutcome, cx: &mut App) {
        self.with_manager(cx, |m, cx| m.finish(self.id, outcome, cx));
    }

    fn with_manager(
        &self,
        cx: &mut App,
        action: impl FnOnce(&mut BackgroundTaskManager, &mut Context<BackgroundTaskManager>),
    ) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        manager.update(cx, |m, cx| action(m, cx));
    }
}

/// 业务后台执行时可用的轻量进度上报器。
///
/// 只保留最新快照，并用容量为 1 的唤醒通道通知前台。高频 I/O 不会形成
/// 无界积压；发送失败表示前台已停止消费，此时上报自动 no-op。
#[derive(Clone)]
pub struct BackgroundProgressReporter {
    latest: Arc<Mutex<Option<BackgroundProgressUpdate>>>,
    wakeup: tokio::sync::mpsc::Sender<()>,
}

#[derive(Debug, Clone)]
struct BackgroundProgressUpdate {
    current: u64,
    total: Option<u64>,
    detail: Option<SharedString>,
    message: Option<SharedString>,
}

impl BackgroundProgressReporter {
    pub fn update(
        &self,
        current: u64,
        total: Option<u64>,
        detail: Option<SharedString>,
        message: Option<SharedString>,
    ) {
        if self.wakeup.is_closed() {
            return;
        }
        let update = BackgroundProgressUpdate {
            current,
            total,
            detail,
            message,
        };
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        *latest = Some(update);
        drop(latest);
        // Full 表示已经有一次前台唤醒排队，最新值会在那次唤醒中被读取。
        let _ = self.wakeup.try_send(());
    }
}

fn progress_channel() -> (BackgroundProgressReporter, tokio::sync::mpsc::Receiver<()>) {
    let (wakeup, receiver) = tokio::sync::mpsc::channel(1);
    (
        BackgroundProgressReporter {
            latest: Arc::new(Mutex::new(None)),
            wakeup,
        },
        receiver,
    )
}

fn spawn_progress_bridge(
    cx: &mut AsyncApp,
    mut receiver: tokio::sync::mpsc::Receiver<()>,
    latest: Arc<Mutex<Option<BackgroundProgressUpdate>>>,
    handle: BackgroundTaskHandle,
) -> Task<()> {
    cx.spawn(async move |cx| {
        while receiver.recv().await.is_some() {
            // 合并短时间内的密集上报，避免每个网络分片都触发一次 GPUI notify。
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            while receiver.try_recv().is_ok() {}
            let update = latest.lock().ok().and_then(|mut latest| latest.take());
            let Some(update) = update else {
                continue;
            };
            let _ = cx.update(|cx| {
                handle.update_progress(
                    update.current,
                    update.total,
                    update.detail,
                    update.message,
                    cx,
                );
            });
        }
    })
}

/// 统一异步接入入口。
///
/// `progress` 是一个很小的前台线程桥：后台任务通过 sender 上报进度，
/// GPUI task 负责落到 manager。`run` 返回 `Ok(_)` 表示成功，`Err(_)` 失败；
/// 任务应在每个可中断点 `select!` 监听 cancellation token。
pub struct BackgroundTaskRunner<'a> {
    cx: &'a mut AsyncApp,
    spec: BackgroundTaskSpec,
}

impl<'a> BackgroundTaskRunner<'a> {
    pub fn new(cx: &'a mut AsyncApp, spec: BackgroundTaskSpec) -> Self {
        Self { cx, spec }
    }

    pub fn run<F, R>(
        self,
        run: impl FnOnce(CancellationToken, BackgroundProgressReporter) -> F + Send + 'static,
    ) -> Task<BackgroundTaskHandle>
    where
        F: Future<Output = anyhow::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let token = CancellationToken::new();
        let (reporter, progress_rx) = progress_channel();
        let progress_latest = reporter.latest.clone();

        let (handle, task) = self.cx.update(|cx| {
            let manager = global(cx);
            let id = manager.update(cx, |manager, cx| {
                let id = manager.register(self.spec, cx);
                manager.set_cancellation(id, BackgroundTaskCancellation::token(token.clone()), cx);
                manager.mark_running(id, cx);
                id
            });
            let handle = BackgroundTaskHandle::new(manager.downgrade(), id);
            let task = crate::gpui_tokio::Tokio::spawn(cx, run(token.clone(), reporter));
            (handle, task)
        });

        spawn_progress_bridge(self.cx, progress_rx, progress_latest, handle.clone()).detach();

        self.cx.spawn(async move |cx: &mut AsyncApp| {
            let result = task.await;
            let outcome = match result {
                Ok(Ok(_)) if token.is_cancelled() => BackgroundTaskOutcome::Cancelled(None),
                Ok(Ok(_)) => BackgroundTaskOutcome::Succeeded(None),
                Ok(Err(error)) if token.is_cancelled() => {
                    BackgroundTaskOutcome::Cancelled(Some(error.to_string().into()))
                }
                Ok(Err(error)) => BackgroundTaskOutcome::Failed(format!("{error:#}")),
                Err(error) if token.is_cancelled() => {
                    BackgroundTaskOutcome::Cancelled(Some(error.to_string().into()))
                }
                Err(error) => BackgroundTaskOutcome::Failed(error.to_string()),
            };
            let _ = cx.update(|cx| handle.finish(outcome, cx));
            handle
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn new_manager(cx: &mut gpui::TestAppContext) -> Entity<BackgroundTaskManager> {
        cx.update(|cx| BackgroundTaskManager::new(cx))
    }

    fn task(
        manager: &Entity<BackgroundTaskManager>,
        id: BackgroundTaskId,
        cx: &gpui::TestAppContext,
    ) -> BackgroundTask {
        manager
            .read_with(cx, |manager, _| {
                manager.tasks().into_iter().find(|task| task.id == id)
            })
            .expect("background task should exist")
    }

    #[gpui::test]
    fn init_is_idempotent(cx: &mut gpui::TestAppContext) {
        let first = cx.update(|cx| {
            init(cx);
            try_global(cx).expect("manager should be initialized")
        });
        let second = cx.update(|cx| {
            init(cx);
            try_global(cx).expect("manager should still be initialized")
        });
        assert_eq!(first.entity_id(), second.entity_id());
    }

    #[gpui::test]
    fn active_key_is_reused_until_task_finishes(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let key = SharedString::from("upload:1");
        let first = manager.update(cx, |m, cx| {
            m.ensure_by_key(BackgroundTaskSpec::new("kind", "one"), key.clone(), cx)
        });
        let duplicate = manager.update(cx, |m, cx| {
            m.ensure_by_key(
                BackgroundTaskSpec::new("kind", "duplicate"),
                key.clone(),
                cx,
            )
        });
        assert_eq!(first, duplicate);
        assert_eq!(1, manager.read_with(cx, |m, _| m.tasks().len()));

        manager.update(cx, |m, cx| {
            m.mark_running(first, cx);
            m.succeed(first, None, cx);
        });
        let second = manager.update(cx, |m, cx| {
            m.ensure_by_key(BackgroundTaskSpec::new("kind", "two"), key, cx)
        });
        assert_ne!(first, second);
        assert_eq!(2, manager.read_with(cx, |m, _| m.tasks().len()));
    }

    #[gpui::test]
    fn valid_running_success_and_failure_transitions(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let (success_id, failure_id) = manager.update(cx, |m, cx| {
            let success_id = m.register(BackgroundTaskSpec::new("kind", "success"), cx);
            let failure_id = m.register(BackgroundTaskSpec::new("kind", "failure"), cx);
            m.mark_running(success_id, cx);
            m.mark_running(failure_id, cx);
            (success_id, failure_id)
        });

        manager.update(cx, |m, cx| {
            m.succeed(success_id, Some("done".into()), cx);
            m.fail(failure_id, "broken", cx);
        });

        let succeeded = task(&manager, success_id, cx);
        assert_eq!(BackgroundTaskStatus::Succeeded, succeeded.status);
        assert_eq!(Some(SharedString::from("done")), succeeded.result);
        assert_eq!(None, succeeded.error);

        let failed = task(&manager, failure_id, cx);
        assert_eq!(BackgroundTaskStatus::Failed, failed.status);
        assert_eq!(Some("broken".to_string()), failed.error);
        assert_eq!(None, failed.result);
    }

    #[gpui::test]
    fn queued_task_can_fail_but_cannot_succeed(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let (success_id, failure_id) = manager.update(cx, |m, cx| {
            (
                m.register(BackgroundTaskSpec::new("kind", "invalid-success"), cx),
                m.register(BackgroundTaskSpec::new("kind", "failure"), cx),
            )
        });
        let before = task(&manager, success_id, cx).updated_at;

        manager.update(cx, |m, cx| {
            m.succeed(success_id, None, cx);
            m.fail(failure_id, "failed before start", cx);
        });

        let unchanged = task(&manager, success_id, cx);
        assert_eq!(BackgroundTaskStatus::Queued, unchanged.status);
        assert_eq!(before, unchanged.updated_at);
        assert_eq!(
            BackgroundTaskStatus::Failed,
            task(&manager, failure_id, cx).status
        );
    }

    #[gpui::test]
    fn cancel_requests_trigger_controller_and_wait_for_confirmation(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            m.register(BackgroundTaskSpec::new("kind", "task"), cx)
        });
        let token = CancellationToken::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_for_cb = called.clone();
        manager.update(cx, |m, cx| {
            m.set_cancellation(
                id,
                BackgroundTaskCancellation {
                    token: Some(token.clone()),
                    callback: Some(Arc::new(move || {
                        called_for_cb.store(true, Ordering::SeqCst);
                    })),
                    result_callback: None,
                },
                cx,
            )
        });

        manager.update(cx, |m, cx| {
            m.mark_running(id, cx);
            assert!(m.request_cancel(id, cx));
        });
        assert!(token.is_cancelled());
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(
            BackgroundTaskStatus::Cancelling,
            task(&manager, id, cx).status
        );

        manager.update(cx, |m, cx| {
            m.cancel_confirmed(id, Some("user cancelled".into()), cx)
        });
        assert_eq!(
            BackgroundTaskStatus::Cancelled,
            task(&manager, id, cx).status
        );
    }

    #[gpui::test]
    fn trimming_finished_tasks_removes_their_cancellations(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let token = CancellationToken::new();
        let finished_id = manager.update(cx, |m, cx| {
            m.max_finished = 0;
            let id = m.register(BackgroundTaskSpec::new("kind", "finished"), cx);
            m.set_cancellation(id, BackgroundTaskCancellation::token(token.clone()), cx);
            m.tasks
                .iter_mut()
                .find(|task| task.id == id)
                .expect("registered task should exist")
                .status = BackgroundTaskStatus::Succeeded;
            id
        });

        let new_id = manager.update(cx, |m, cx| {
            m.register(BackgroundTaskSpec::new("kind", "new"), cx)
        });

        manager.read_with(cx, |m, _| {
            assert!(!m.tasks.iter().any(|task| task.id == finished_id));
            assert!(m.tasks.iter().any(|task| task.id == new_id));
            assert!(
                m.cancellations
                    .lock()
                    .expect("cancellations mutex should not be poisoned")
                    .iter()
                    .all(|(task_id, _)| *task_id != finished_id)
            );
        });
    }

    #[gpui::test]
    fn handle_request_cancel_delegates_to_manager(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let called = Arc::new(AtomicBool::new(false));
        let called_for_callback = called.clone();
        let id = manager.update(cx, |m, cx| {
            let id = m.register(BackgroundTaskSpec::new("kind", "task"), cx);
            m.set_cancellation(
                id,
                BackgroundTaskCancellation::callback(move || {
                    called_for_callback.store(true, Ordering::SeqCst);
                }),
                cx,
            );
            m.mark_running(id, cx);
            id
        });
        let handle = BackgroundTaskHandle::new(manager.downgrade(), id);

        assert!(cx.update(|cx| handle.request_cancel(cx)));
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(
            BackgroundTaskStatus::Cancelling,
            task(&manager, id, cx).status
        );
    }

    #[gpui::test]
    fn cancel_without_controller_is_rejected_and_state_is_unchanged(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(BackgroundTaskSpec::new("kind", "task"), cx);
            m.mark_running(id, cx);
            id
        });
        let before = task(&manager, id, cx).updated_at;

        manager.update(cx, |m, cx| {
            assert!(!m.request_cancel(id, cx));
        });

        let unchanged = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Running, unchanged.status);
        assert_eq!(before, unchanged.updated_at);
    }

    #[gpui::test]
    fn cancellation_controller_is_triggered_exactly_once(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let id = manager.update(cx, |m, cx| {
            let id = m.register(BackgroundTaskSpec::new("kind", "task"), cx);
            m.set_cancellation(
                id,
                BackgroundTaskCancellation::callback(move || {
                    callback_calls.fetch_add(1, Ordering::SeqCst);
                }),
                cx,
            );
            m.mark_running(id, cx);
            id
        });

        manager.update(cx, |m, cx| {
            assert!(m.request_cancel(id, cx));
            assert!(!m.request_cancel(id, cx));
        });
        assert_eq!(1, calls.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn rejected_cancellation_callback_keeps_task_running(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(BackgroundTaskSpec::new("kind", "task"), cx);
            m.set_cancellation(
                id,
                BackgroundTaskCancellation::callback_with_result(|| false),
                cx,
            );
            m.mark_running(id, cx);
            id
        });

        manager.update(cx, |m, cx| assert!(!m.request_cancel(id, cx)));
        assert_eq!(BackgroundTaskStatus::Running, task(&manager, id, cx).status);
    }

    #[gpui::test]
    fn cancellation_wins_over_late_success_or_failure(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(BackgroundTaskSpec::new("kind", "task"), cx);
            m.set_cancellation(
                id,
                BackgroundTaskCancellation::token(CancellationToken::new()),
                cx,
            );
            m.mark_running(id, cx);
            assert!(m.request_cancel(id, cx));
            id
        });
        let cancelling_at = task(&manager, id, cx).updated_at;

        manager.update(cx, |m, cx| {
            m.succeed(id, Some("late success".into()), cx);
            m.fail(id, "late failure", cx);
        });
        let cancelling = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Cancelling, cancelling.status);
        assert_eq!(cancelling_at, cancelling.updated_at);
        assert_eq!(None, cancelling.result);
        assert_eq!(None, cancelling.error);

        manager.update(cx, |m, cx| {
            m.cancel_confirmed(id, Some("cancelled".into()), cx)
        });
        let cancelled = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Cancelled, cancelled.status);
        assert_eq!(Some(SharedString::from("cancelled")), cancelled.result);
    }

    #[gpui::test]
    fn progress_uses_task_default_unit_and_formats_bytes(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            m.register(
                BackgroundTaskSpec::new("kind", "task")
                    .progress_unit(BackgroundTaskProgressUnit::Bytes),
                cx,
            )
        });
        manager.update(cx, |m, cx| {
            m.mark_running(id, cx);
            m.update_progress(id, 1024, Some(2048), None, None, cx)
        });
        let progress = task(&manager, id, cx).progress.unwrap();
        assert_eq!(50, progress.percent());
        assert_eq!(BackgroundTaskProgressUnit::Bytes, progress.unit);
        assert!(progress.display().contains("1.0 KB"));
    }

    #[gpui::test]
    fn task_preserves_optional_open_folder(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let open_folder = std::path::PathBuf::from("/tmp");
        let id = manager.update(cx, |m, cx| {
            m.register(
                BackgroundTaskSpec::new("download", "archive").open_folder(open_folder.clone()),
                cx,
            )
        });

        assert_eq!(task(&manager, id, cx).open_folder, Some(open_folder));
    }

    #[gpui::test]
    fn succeeded_task_pins_progress_to_total(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(
                BackgroundTaskSpec::new("kind", "upload")
                    .progress_unit(BackgroundTaskProgressUnit::Bytes),
                cx,
            );
            m.mark_running(id, cx);
            // 任务在 98/100 时即宣布成功，进度必须被钉到 100%。
            m.update_progress(id, 98, Some(100), None, None, cx);
            m.succeed(id, None, cx);
            id
        });

        let task = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Succeeded, task.status);
        let progress = task.progress.expect("progress should be kept");
        assert_eq!(100, progress.percent());
        assert_eq!(Some(progress.current), progress.total);
    }

    #[gpui::test]
    fn succeeded_task_with_unknown_total_pins_to_one_hundred(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(
                BackgroundTaskSpec::new("kind", "download")
                    .progress_unit(BackgroundTaskProgressUnit::Bytes),
                cx,
            );
            m.mark_running(id, cx);
            // 下载开始时总大小未知（total = None），成功时也应显示 100%。
            m.update_progress(id, 5_897, None, None, None, cx);
            m.succeed(id, None, cx);
            id
        });

        let task = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Succeeded, task.status);
        let progress = task.progress.expect("progress should be kept");
        assert_eq!(100, progress.percent());
        assert_eq!(Some(progress.current), progress.total);
        assert_eq!(5_897, progress.current);
    }

    #[gpui::test]
    fn terminal_task_rejects_progress_and_repeated_finish(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let id = manager.update(cx, |m, cx| {
            let id = m.register(
                BackgroundTaskSpec::new("kind", "task")
                    .progress_unit(BackgroundTaskProgressUnit::Bytes),
                cx,
            );
            m.mark_running(id, cx);
            m.update_progress(id, 10, Some(100), None, None, cx);
            m.succeed(id, Some("first".into()), cx);
            id
        });
        let before = task(&manager, id, cx);

        manager.update(cx, |m, cx| {
            m.update_progress(
                id,
                100,
                Some(100),
                Some("late detail".into()),
                Some("late progress".into()),
                cx,
            );
            m.fail(id, "late failure", cx);
        });

        let after = task(&manager, id, cx);
        assert_eq!(BackgroundTaskStatus::Succeeded, after.status);
        assert_eq!(before.updated_at, after.updated_at);
        assert_eq!(before.progress, after.progress);
        assert_eq!(before.detail, after.detail);
        assert_eq!(before.result, after.result);
        assert_eq!(None, after.error);
    }

    #[gpui::test]
    fn filter_buckets_partition_task_statuses(cx: &mut gpui::TestAppContext) {
        let manager = new_manager(cx);
        let (queued, running, cancelling, succeeded, failed, cancelled) =
            manager.update(cx, |m, cx| {
                let queued = m.register(BackgroundTaskSpec::new("kind", "queued"), cx);
                let running = m.register(BackgroundTaskSpec::new("kind", "running"), cx);
                let cancelling = m.register(BackgroundTaskSpec::new("kind", "cancelling"), cx);
                let succeeded = m.register(BackgroundTaskSpec::new("kind", "succeeded"), cx);
                let failed = m.register(BackgroundTaskSpec::new("kind", "failed"), cx);
                let cancelled = m.register(BackgroundTaskSpec::new("kind", "cancelled"), cx);
                m.mark_running(running, cx);
                m.mark_running(cancelling, cx);
                m.mark_running(succeeded, cx);
                m.mark_running(failed, cx);
                m.mark_running(cancelled, cx);
                m.set_cancellation(
                    cancelling,
                    BackgroundTaskCancellation::token(CancellationToken::new()),
                    cx,
                );
                assert!(m.request_cancel(cancelling, cx));
                m.succeed(succeeded, None, cx);
                m.fail(failed, "broken", cx);
                m.cancel_confirmed(cancelled, None, cx);
                (queued, running, cancelling, succeeded, failed, cancelled)
            });

        manager.read_with(cx, |m, _| {
            let counts = m.counts();
            let tasks = m.tasks();
            let ids_for = |filter: BackgroundTaskFilter| -> Vec<u64> {
                let mut ids: Vec<u64> = tasks
                    .iter()
                    .filter(|task| filter.matches(task))
                    .map(|task| task.id.as_u64())
                    .collect();
                ids.sort_unstable();
                ids
            };

            for filter in BackgroundTaskFilter::ALL {
                assert_eq!(
                    filter.count(counts),
                    ids_for(filter).len(),
                    "filter count must match matched tasks for {filter:?}"
                );
            }
            assert_eq!(
                6,
                BackgroundTaskFilter::All.count(counts),
                "all buckets must cover every task"
            );
            assert_eq!(vec![queued.as_u64()], ids_for(BackgroundTaskFilter::Queued));
            assert_eq!(
                vec![running.as_u64(), cancelling.as_u64()],
                ids_for(BackgroundTaskFilter::Running)
            );
            assert_eq!(
                vec![succeeded.as_u64()],
                ids_for(BackgroundTaskFilter::Succeeded)
            );
            assert_eq!(
                vec![cancelled.as_u64()],
                ids_for(BackgroundTaskFilter::Cancelled)
            );
            assert_eq!(vec![failed.as_u64()], ids_for(BackgroundTaskFilter::Failed));
        });
    }

    #[test]
    fn progress_reporter_coalesces_bursts_to_latest_snapshot() {
        let (reporter, mut receiver) = progress_channel();
        for current in 1..=1_000 {
            reporter.update(current, Some(1_000), None, None);
        }

        assert_eq!(Ok(()), receiver.try_recv());
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        let latest = reporter
            .latest
            .lock()
            .expect("progress snapshot mutex should not be poisoned")
            .take()
            .expect("latest progress should be retained");
        assert_eq!(1_000, latest.current);
        assert_eq!(Some(1_000), latest.total);
    }
}

fn format_bytes(value: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if value >= GB {
        format!("{:.1} GB", value as f64 / GB as f64)
    } else if value >= MB {
        format!("{:.1} MB", value as f64 / MB as f64)
    } else if value >= KB {
        format!("{:.1} KB", value as f64 / KB as f64)
    } else {
        format!("{value} B")
    }
}
