use super::*;
use chrono::{DateTime, Local, Utc};
use gpui_component::StyledExt;
use std::collections::HashSet;
use terminal::operation_journal::{
    OperationGenerationId, OperationId, OperationJournal, OperationJournalPersistenceCorruption,
    OperationJournalRecoverySource, OperationJournalRecoveryWarningKind, OperationJournalSessionId,
    OperationKind, OperationPayloadCompleteness, OperationPayloadFormat, OperationStatus,
};
use terminal::{TerminalOperationHistoryLoad, TerminalOperationHistoryRequestKey};

const OPERATION_HISTORY_CURRENT_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OperationHistoryItemKey {
    session_id: OperationJournalSessionId,
    generation_id: OperationGenerationId,
    operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryPayloadSummary {
    preview: String,
    format: OperationPayloadFormat,
    completeness: OperationPayloadCompleteness,
    original_byte_len: u64,
    redaction_applied: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryTransitionRow {
    sequence: u64,
    status: OperationStatus,
    occurred_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryOperationRow {
    key: OperationHistoryItemKey,
    operation_id: OperationId,
    parent_operation_id: Option<OperationId>,
    kind: OperationKind,
    status: OperationStatus,
    payload: Option<OperationHistoryPayloadSummary>,
    transitions: Vec<OperationHistoryTransitionRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryGenerationRow {
    id: OperationGenerationId,
    started_at_unix_ms: u64,
    ended_at_unix_ms: Option<u64>,
    is_closed: bool,
    operations: Vec<OperationHistoryOperationRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistorySessionRow {
    session_id: OperationJournalSessionId,
    generations: Vec<OperationHistoryGenerationRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OperationHistorySessionSource {
    Current,
    Recovered {
        created_at_unix_ms: u64,
        updated_at_unix_ms: u64,
        recovery_source: Option<OperationJournalRecoverySource>,
        checkpoint_rejection: Option<OperationJournalPersistenceCorruption>,
        discarded_log_tail_bytes: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryProjectedSession {
    source: OperationHistorySessionSource,
    session: OperationHistorySessionRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OperationHistoryNotice {
    CurrentSnapshotUnavailable(String),
    RecoveryWarning {
        kind: OperationJournalRecoveryWarningKind,
        session_id: Option<OperationJournalSessionId>,
    },
}

struct OperationHistoryRecoveredJournal<'a> {
    journal: &'a OperationJournal,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
    recovery_source: Option<OperationJournalRecoverySource>,
    checkpoint_rejection: Option<OperationJournalPersistenceCorruption>,
    discarded_log_tail_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OperationHistoryProjection {
    sessions: Vec<OperationHistoryProjectedSession>,
    notices: Vec<OperationHistoryNotice>,
}

impl OperationHistoryProjection {
    fn from_load(
        load: &TerminalOperationHistoryLoad,
        status_filter: Option<OperationStatus>,
    ) -> Self {
        Self::from_parts(
            load.current_journal(),
            load.current_journal_error().map(ToString::to_string),
            load.recovered()
                .histories()
                .iter()
                .map(|history| OperationHistoryRecoveredJournal {
                    journal: history.journal(),
                    created_at_unix_ms: history.manifest().created_at_unix_ms(),
                    updated_at_unix_ms: history.manifest().updated_at_unix_ms(),
                    recovery_source: history.recovery_source(),
                    checkpoint_rejection: history.checkpoint_rejection(),
                    discarded_log_tail_bytes: history.discarded_log_tail_bytes(),
                }),
            load.recovered()
                .warnings()
                .iter()
                .map(|warning| (warning.kind(), warning.session_id().cloned())),
            status_filter,
        )
    }

    fn from_parts<'a>(
        current_journal: Option<&'a OperationJournal>,
        current_snapshot_error: Option<String>,
        recovered: impl IntoIterator<Item = OperationHistoryRecoveredJournal<'a>>,
        recovery_warnings: impl IntoIterator<
            Item = (
                OperationJournalRecoveryWarningKind,
                Option<OperationJournalSessionId>,
            ),
        >,
        status_filter: Option<OperationStatus>,
    ) -> Self {
        let mut seen_session_ids = HashSet::new();
        let mut sessions = Vec::new();

        if let Some(journal) = current_journal {
            seen_session_ids.insert(journal.session_id().clone());
            sessions.push(OperationHistoryProjectedSession {
                source: OperationHistorySessionSource::Current,
                session: project_journal(journal, status_filter),
            });
        }

        for recovered in recovered {
            if !seen_session_ids.insert(recovered.journal.session_id().clone()) {
                continue;
            }
            sessions.push(OperationHistoryProjectedSession {
                source: OperationHistorySessionSource::Recovered {
                    created_at_unix_ms: recovered.created_at_unix_ms,
                    updated_at_unix_ms: recovered.updated_at_unix_ms,
                    recovery_source: recovered.recovery_source,
                    checkpoint_rejection: recovered.checkpoint_rejection,
                    discarded_log_tail_bytes: recovered.discarded_log_tail_bytes,
                },
                session: project_journal(recovered.journal, status_filter),
            });
        }

        let mut notices = Vec::new();
        if let Some(message) = current_snapshot_error {
            notices.push(OperationHistoryNotice::CurrentSnapshotUnavailable(message));
        }
        notices.extend(recovery_warnings.into_iter().map(|(kind, session_id)| {
            OperationHistoryNotice::RecoveryWarning { kind, session_id }
        }));

        Self { sessions, notices }
    }
}

pub(super) struct OperationHistoryPanelState {
    open: bool,
    status_filter: Option<OperationStatus>,
    expanded_operations: HashSet<OperationHistoryItemKey>,
    scroll_handle: ScrollHandle,
}

impl Default for OperationHistoryPanelState {
    fn default() -> Self {
        Self {
            open: false,
            status_filter: None,
            expanded_operations: HashSet::new(),
            scroll_handle: ScrollHandle::new(),
        }
    }
}

impl OperationHistoryPanelState {
    fn toggle_open(&mut self) {
        self.open = !self.open;
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn set_status_filter(&mut self, status_filter: Option<OperationStatus>) {
        self.status_filter = status_filter;
    }

    fn status_filter(&self) -> Option<OperationStatus> {
        self.status_filter
    }

    fn toggle_expanded(&mut self, key: OperationHistoryItemKey) {
        if !self.expanded_operations.insert(key.clone()) {
            self.expanded_operations.remove(&key);
        }
    }

    fn expanded_operations(&self) -> &HashSet<OperationHistoryItemKey> {
        &self.expanded_operations
    }

    fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll_handle
    }

    fn reset_unavailable(&mut self) {
        self.close();
        self.status_filter = None;
        self.expanded_operations.clear();
        self.scroll_handle = ScrollHandle::new();
    }
}

fn operation_status_translation_key(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Queued => "TerminalOperationHistory.status.queued",
        OperationStatus::Sent => "TerminalOperationHistory.status.sent",
        OperationStatus::Acknowledged => "TerminalOperationHistory.status.acknowledged",
        OperationStatus::Succeeded => "TerminalOperationHistory.status.succeeded",
        OperationStatus::Failed => "TerminalOperationHistory.status.failed",
        OperationStatus::Unknown => "TerminalOperationHistory.status.unknown",
        OperationStatus::NeedsReview => "TerminalOperationHistory.status.needsReview",
        OperationStatus::Canceled => "TerminalOperationHistory.status.canceled",
    }
}

fn operation_history_status_filters() -> [Option<OperationStatus>; 9] {
    [
        None,
        Some(OperationStatus::Queued),
        Some(OperationStatus::Sent),
        Some(OperationStatus::Acknowledged),
        Some(OperationStatus::Succeeded),
        Some(OperationStatus::Failed),
        Some(OperationStatus::Unknown),
        Some(OperationStatus::NeedsReview),
        Some(OperationStatus::Canceled),
    ]
}

fn operation_kind_translation_key(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::UserInput => "TerminalOperationHistory.kind.userInput",
        OperationKind::Command => "TerminalOperationHistory.kind.command",
        OperationKind::Paste => "TerminalOperationHistory.kind.paste",
        OperationKind::ControlSequence => "TerminalOperationHistory.kind.controlSequence",
        OperationKind::FileOperation => "TerminalOperationHistory.kind.fileOperation",
        OperationKind::ApplicationOperation => "TerminalOperationHistory.kind.applicationOperation",
        OperationKind::Unconfirmable => "TerminalOperationHistory.kind.unconfirmable",
    }
}

fn operation_recovery_source_translation_key(
    source: OperationJournalRecoverySource,
) -> &'static str {
    match source {
        OperationJournalRecoverySource::AppendLog => {
            "TerminalOperationHistory.recoverySource.appendLog"
        }
        OperationJournalRecoverySource::Checkpoint => {
            "TerminalOperationHistory.recoverySource.checkpoint"
        }
        OperationJournalRecoverySource::CheckpointAndAppendLog => {
            "TerminalOperationHistory.recoverySource.checkpointAndAppendLog"
        }
    }
}

fn operation_persistence_corruption_translation_key(
    corruption: OperationJournalPersistenceCorruption,
) -> &'static str {
    match corruption {
        OperationJournalPersistenceCorruption::InvalidRecord => {
            "TerminalOperationHistory.corruption.invalidRecord"
        }
        OperationJournalPersistenceCorruption::ChecksumMismatch => {
            "TerminalOperationHistory.corruption.checksumMismatch"
        }
        OperationJournalPersistenceCorruption::UnsupportedFormat => {
            "TerminalOperationHistory.corruption.unsupportedFormat"
        }
        OperationJournalPersistenceCorruption::UnsupportedSchemaVersion => {
            "TerminalOperationHistory.corruption.unsupportedSchemaVersion"
        }
        OperationJournalPersistenceCorruption::SessionMismatch => {
            "TerminalOperationHistory.corruption.sessionMismatch"
        }
        OperationJournalPersistenceCorruption::InvalidSequence => {
            "TerminalOperationHistory.corruption.invalidSequence"
        }
        OperationJournalPersistenceCorruption::ConflictingSnapshot => {
            "TerminalOperationHistory.corruption.conflictingSnapshot"
        }
        OperationJournalPersistenceCorruption::RecordTooLarge => {
            "TerminalOperationHistory.corruption.recordTooLarge"
        }
        OperationJournalPersistenceCorruption::Unreadable => {
            "TerminalOperationHistory.corruption.unreadable"
        }
        OperationJournalPersistenceCorruption::FileChanged => {
            "TerminalOperationHistory.corruption.fileChanged"
        }
    }
}

fn operation_payload_format_translation_key(format: OperationPayloadFormat) -> &'static str {
    match format {
        OperationPayloadFormat::OpaqueSummary => {
            "TerminalOperationHistory.payloadFormat.opaqueSummary"
        }
        OperationPayloadFormat::StructuredJson => {
            "TerminalOperationHistory.payloadFormat.structuredJson"
        }
    }
}

fn operation_payload_completeness_translation_key(
    completeness: OperationPayloadCompleteness,
) -> &'static str {
    match completeness {
        OperationPayloadCompleteness::Complete => {
            "TerminalOperationHistory.payloadCompleteness.complete"
        }
        OperationPayloadCompleteness::Redacted => {
            "TerminalOperationHistory.payloadCompleteness.redacted"
        }
        OperationPayloadCompleteness::SummaryOnly => {
            "TerminalOperationHistory.payloadCompleteness.summaryOnly"
        }
    }
}

fn operation_recovery_warning_translation_key(
    kind: OperationJournalRecoveryWarningKind,
) -> &'static str {
    match kind {
        OperationJournalRecoveryWarningKind::DirectoryReadFailed => {
            "TerminalOperationHistory.warning.directoryReadFailed"
        }
        OperationJournalRecoveryWarningKind::DirectoryEntryReadFailed => {
            "TerminalOperationHistory.warning.directoryEntryReadFailed"
        }
        OperationJournalRecoveryWarningKind::DirectoryScanLimitReached => {
            "TerminalOperationHistory.warning.directoryScanLimitReached"
        }
        OperationJournalRecoveryWarningKind::ManifestNotRegularFile => {
            "TerminalOperationHistory.warning.manifestNotRegularFile"
        }
        OperationJournalRecoveryWarningKind::ManifestTooLarge => {
            "TerminalOperationHistory.warning.manifestTooLarge"
        }
        OperationJournalRecoveryWarningKind::ManifestReadFailed => {
            "TerminalOperationHistory.warning.manifestReadFailed"
        }
        OperationJournalRecoveryWarningKind::InvalidManifest => {
            "TerminalOperationHistory.warning.invalidManifest"
        }
        OperationJournalRecoveryWarningKind::DuplicateSessionId => {
            "TerminalOperationHistory.warning.repeatedSessionId"
        }
        OperationJournalRecoveryWarningKind::JournalMissing => {
            "TerminalOperationHistory.warning.journalMissing"
        }
        OperationJournalRecoveryWarningKind::JournalRecoveryFailed => {
            "TerminalOperationHistory.warning.journalRecoveryFailed"
        }
        OperationJournalRecoveryWarningKind::JournalSessionMismatch => {
            "TerminalOperationHistory.warning.journalSessionMismatch"
        }
        OperationJournalRecoveryWarningKind::CheckpointRejected => {
            "TerminalOperationHistory.warning.checkpointRejected"
        }
        OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered => {
            "TerminalOperationHistory.warning.truncatedLogTailRecovered"
        }
        OperationJournalRecoveryWarningKind::HistoryLimitReached => {
            "TerminalOperationHistory.warning.historyLimitReached"
        }
    }
}

fn project_journal(
    journal: &OperationJournal,
    status_filter: Option<OperationStatus>,
) -> OperationHistorySessionRow {
    let session_id = journal.session_id().clone();
    let generations = journal
        .generations()
        .iter()
        .rev()
        .filter_map(|generation| {
            let mut operations = generation
                .operations()
                .iter()
                .filter(|operation| status_filter.is_none_or(|status| operation.status() == status))
                .map(|operation| {
                    let payload = operation.redacted_payload().map(|payload| {
                        OperationHistoryPayloadSummary {
                            preview: payload.preview(),
                            format: payload.format(),
                            completeness: payload.completeness(),
                            original_byte_len: payload.original_byte_len(),
                            redaction_applied: payload.redaction_applied(),
                        }
                    });
                    OperationHistoryOperationRow {
                        key: OperationHistoryItemKey {
                            session_id: session_id.clone(),
                            generation_id: generation.id(),
                            operation_id: operation.operation_id().clone(),
                        },
                        operation_id: operation.operation_id().clone(),
                        parent_operation_id: operation.parent_operation_id().cloned(),
                        kind: operation.kind(),
                        status: operation.status(),
                        payload,
                        transitions: operation
                            .transitions()
                            .iter()
                            .map(|transition| OperationHistoryTransitionRow {
                                sequence: transition.sequence(),
                                status: transition.status(),
                                occurred_at_unix_ms: transition.occurred_at_unix_ms(),
                            })
                            .collect(),
                    }
                })
                .collect::<Vec<_>>();
            operations.sort_by(|left, right| {
                let left_transition = left
                    .transitions
                    .last()
                    .map(|transition| (transition.occurred_at_unix_ms, transition.sequence));
                let right_transition = right
                    .transitions
                    .last()
                    .map(|transition| (transition.occurred_at_unix_ms, transition.sequence));
                right_transition
                    .cmp(&left_transition)
                    .then_with(|| right.operation_id.as_str().cmp(left.operation_id.as_str()))
            });

            if status_filter.is_some() && operations.is_empty() {
                return None;
            }
            Some(OperationHistoryGenerationRow {
                id: generation.id(),
                started_at_unix_ms: generation.started_at_unix_ms(),
                ended_at_unix_ms: generation.ended_at_unix_ms(),
                is_closed: generation.is_closed(),
                operations,
            })
        })
        .collect();

    OperationHistorySessionRow {
        session_id,
        generations,
    }
}

fn format_operation_history_time(unix_ms: u64) -> String {
    let Ok(unix_ms) = i64::try_from(unix_ms) else {
        return unix_ms.to_string();
    };
    DateTime::<Utc>::from_timestamp_millis(unix_ms)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| unix_ms.to_string())
}

#[derive(Clone, Copy)]
enum OperationHistoryMessageTone {
    Info,
    Warning,
    Danger,
    Muted,
}

fn render_operation_history_message(
    message: impl Into<SharedString>,
    tone: OperationHistoryMessageTone,
    cx: &App,
) -> AnyElement {
    let color = match tone {
        OperationHistoryMessageTone::Info => cx.theme().info,
        OperationHistoryMessageTone::Warning => cx.theme().warning,
        OperationHistoryMessageTone::Danger => cx.theme().danger,
        OperationHistoryMessageTone::Muted => cx.theme().muted_foreground,
    };
    div()
        .w_full()
        .min_w_0()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .p_2()
        .text_xs()
        .text_color(color)
        .whitespace_normal()
        .child(message.into())
        .into_any_element()
}

fn render_operation_history_detail_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(132.0))
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .whitespace_normal()
                .child(value.into()),
        )
        .into_any_element()
}

fn render_operation_history_status(status: OperationStatus, cx: &App) -> AnyElement {
    let color = match status {
        OperationStatus::Succeeded => cx.theme().success,
        OperationStatus::Failed => cx.theme().danger,
        OperationStatus::Unknown | OperationStatus::NeedsReview => cx.theme().warning,
        OperationStatus::Canceled => cx.theme().muted_foreground,
        OperationStatus::Queued | OperationStatus::Sent | OperationStatus::Acknowledged => {
            cx.theme().info
        }
    };
    div()
        .flex_shrink_0()
        .rounded_sm()
        .bg(cx.theme().secondary)
        .px_1()
        .text_xs()
        .font_semibold()
        .text_color(color)
        .child(t!(operation_status_translation_key(status)))
        .into_any_element()
}

fn render_operation_history_filters(
    active_filter: Option<OperationStatus>,
    cx: &mut Context<TerminalView>,
) -> AnyElement {
    let mut buttons = Vec::new();
    for (index, filter) in operation_history_status_filters().into_iter().enumerate() {
        let label = filter
            .map(operation_status_translation_key)
            .map(|key| t!(key))
            .unwrap_or_else(|| t!("TerminalOperationHistory.filter.all"));
        buttons.push(
            Button::new(SharedString::from(format!(
                "terminal-operation-history-filter-{index}"
            )))
            .label(label)
            .xsmall()
            .when(active_filter == filter, |button| button.primary())
            .when(active_filter != filter, |button| button.ghost())
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.operation_history_panel.set_status_filter(filter);
                cx.notify();
            })),
        );
    }

    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .flex_wrap()
        .gap_1()
        .children(buttons)
        .into_any_element()
}

fn render_operation_history_notice(notice: OperationHistoryNotice, cx: &App) -> AnyElement {
    match notice {
        OperationHistoryNotice::CurrentSnapshotUnavailable(error) => {
            render_operation_history_message(
                t!(
                    "TerminalOperationHistory.current_snapshot_unavailable",
                    error = error
                ),
                OperationHistoryMessageTone::Warning,
                cx,
            )
        }
        OperationHistoryNotice::RecoveryWarning { kind, session_id } => {
            let warning = t!(operation_recovery_warning_translation_key(kind));
            let message = session_id.map_or(warning.clone(), |session_id| {
                t!(
                    "TerminalOperationHistory.recovery_warning_for_session",
                    warning = warning,
                    session = session_id.as_str()
                )
            });
            render_operation_history_message(message, OperationHistoryMessageTone::Warning, cx)
        }
    }
}

fn render_operation_history_transition(
    transition: OperationHistoryTransitionRow,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .rounded_sm()
        .bg(cx.theme().secondary)
        .px_2()
        .py_1()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!(
                    "TerminalOperationHistory.sequence",
                    sequence = transition.sequence
                )),
        )
        .child(render_operation_history_status(transition.status, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format_operation_history_time(
                    transition.occurred_at_unix_ms,
                )),
        )
        .into_any_element()
}

fn render_operation_history_operation(
    operation: OperationHistoryOperationRow,
    expanded: bool,
    cx: &mut Context<TerminalView>,
) -> AnyElement {
    let chevron = if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let toggle_tooltip = if expanded {
        t!("TerminalOperationHistory.collapse")
    } else {
        t!("TerminalOperationHistory.expand")
    };
    let key = operation.key.clone();
    let toggle_id = SharedString::from(format!(
        "terminal-operation-history-operation-{}-{}-{}",
        key.session_id.as_str(),
        key.generation_id.get(),
        key.operation_id.as_str()
    ));
    let payload_preview = operation
        .payload
        .as_ref()
        .map(|payload| payload.preview.clone());

    let mut details = Vec::new();
    if expanded {
        details.push(render_operation_history_detail_row(
            t!("TerminalOperationHistory.operation_id"),
            operation.operation_id.as_str().to_string(),
            cx,
        ));
        details.push(render_operation_history_detail_row(
            t!("TerminalOperationHistory.parent_operation"),
            operation
                .parent_operation_id
                .as_ref()
                .map(|operation_id| operation_id.as_str().to_string())
                .unwrap_or_else(|| t!("TerminalOperationHistory.not_available").to_string()),
            cx,
        ));
        if let Some(payload) = operation.payload.as_ref() {
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.payload_preview"),
                payload.preview.clone(),
                cx,
            ));
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.payload_format"),
                t!(operation_payload_format_translation_key(payload.format)),
                cx,
            ));
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.payload_completeness"),
                t!(operation_payload_completeness_translation_key(
                    payload.completeness
                )),
                cx,
            ));
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.original_byte_len"),
                t!(
                    "TerminalOperationHistory.bytes",
                    bytes = payload.original_byte_len
                ),
                cx,
            ));
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.redaction_applied"),
                if payload.redaction_applied {
                    t!("TerminalOperationHistory.yes")
                } else {
                    t!("TerminalOperationHistory.no")
                },
                cx,
            ));
        } else {
            details.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.payload"),
                t!("TerminalOperationHistory.no_payload"),
                cx,
            ));
        }
        details.push(
            div()
                .mt_1()
                .text_xs()
                .font_semibold()
                .child(t!("TerminalOperationHistory.transitions"))
                .into_any_element(),
        );
        details.extend(
            operation
                .transitions
                .iter()
                .cloned()
                .map(|transition| render_operation_history_transition(transition, cx)),
        );
    }

    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .p_2()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    Button::new(toggle_id)
                        .icon(chevron)
                        .ghost()
                        .xsmall()
                        .tooltip(toggle_tooltip)
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.operation_history_panel.toggle_expanded(key.clone());
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_semibold()
                        .child(t!(operation_kind_translation_key(operation.kind))),
                )
                .child(render_operation_history_status(operation.status, cx)),
        )
        .when_some(payload_preview, |operation, preview| {
            operation.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(preview),
            )
        })
        .children(details)
        .into_any_element()
}

fn render_operation_history_generation(
    generation: OperationHistoryGenerationRow,
    expanded_operations: &HashSet<OperationHistoryItemKey>,
    cx: &mut Context<TerminalView>,
) -> AnyElement {
    let mut operations = Vec::new();
    for operation in generation.operations {
        let expanded = expanded_operations.contains(&operation.key);
        operations.push(render_operation_history_operation(operation, expanded, cx));
    }

    let generation_state = if generation.is_closed {
        t!("TerminalOperationHistory.closed_generation")
    } else {
        t!("TerminalOperationHistory.current_generation")
    };
    let ended_at = generation
        .ended_at_unix_ms
        .map(format_operation_history_time)
        .unwrap_or_else(|| t!("TerminalOperationHistory.not_available").to_string());

    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .rounded_md()
        .bg(cx.theme().secondary)
        .p_2()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().min_w_0().text_sm().font_semibold().child(t!(
                    "TerminalOperationHistory.generation",
                    generation = generation.id.get()
                )))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(generation_state),
                ),
        )
        .child(render_operation_history_detail_row(
            t!("TerminalOperationHistory.started_at"),
            format_operation_history_time(generation.started_at_unix_ms),
            cx,
        ))
        .child(render_operation_history_detail_row(
            t!("TerminalOperationHistory.ended_at"),
            ended_at,
            cx,
        ))
        .children(operations)
        .into_any_element()
}

fn render_operation_history_session(
    projected_session: OperationHistoryProjectedSession,
    expanded_operations: &HashSet<OperationHistoryItemKey>,
    cx: &mut Context<TerminalView>,
) -> AnyElement {
    let OperationHistoryProjectedSession { source, session } = projected_session;
    let mut metadata = vec![render_operation_history_detail_row(
        t!("TerminalOperationHistory.session_id"),
        session.session_id.as_str().to_string(),
        cx,
    )];
    let source_label = match source {
        OperationHistorySessionSource::Current => {
            t!("TerminalOperationHistory.current_session")
        }
        OperationHistorySessionSource::Recovered {
            created_at_unix_ms,
            updated_at_unix_ms,
            recovery_source,
            checkpoint_rejection,
            discarded_log_tail_bytes,
        } => {
            metadata.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.created_at"),
                format_operation_history_time(created_at_unix_ms),
                cx,
            ));
            metadata.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.updated_at"),
                format_operation_history_time(updated_at_unix_ms),
                cx,
            ));
            metadata.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.recovery_source"),
                recovery_source
                    .map(operation_recovery_source_translation_key)
                    .map(|key| t!(key))
                    .unwrap_or_else(|| t!("TerminalOperationHistory.not_available")),
                cx,
            ));
            metadata.push(render_operation_history_detail_row(
                t!("TerminalOperationHistory.discarded_tail"),
                t!(
                    "TerminalOperationHistory.bytes",
                    bytes = discarded_log_tail_bytes
                ),
                cx,
            ));
            if let Some(checkpoint_rejection) = checkpoint_rejection {
                metadata.push(render_operation_history_detail_row(
                    t!("TerminalOperationHistory.checkpoint_rejection"),
                    t!(operation_persistence_corruption_translation_key(
                        checkpoint_rejection
                    )),
                    cx,
                ));
            }
            t!("TerminalOperationHistory.recovered_session")
        }
    };

    let mut generations = Vec::new();
    for generation in session.generations {
        generations.push(render_operation_history_generation(
            generation,
            expanded_operations,
            cx,
        ));
    }

    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .p_3()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().foreground)
                .child(source_label),
        )
        .children(metadata)
        .children(generations)
        .into_any_element()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryLoadError {
    key: TerminalOperationHistoryRequestKey,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationHistoryCompletion {
    AppliedSuccess,
    AppliedError,
    IgnoredStale,
}

pub(super) struct OperationHistoryLoadState<T> {
    in_flight_key: Option<TerminalOperationHistoryRequestKey>,
    last_completed_key: Option<TerminalOperationHistoryRequestKey>,
    current_load: Option<T>,
    last_error: Option<OperationHistoryLoadError>,
}

impl<T> Default for OperationHistoryLoadState<T> {
    fn default() -> Self {
        Self {
            in_flight_key: None,
            last_completed_key: None,
            current_load: None,
            last_error: None,
        }
    }
}

impl<T> OperationHistoryLoadState<T> {
    fn is_loading(&self) -> bool {
        self.in_flight_key.is_some()
    }

    fn is_showing_stale_snapshot(&self) -> bool {
        self.is_loading() && self.current_load.is_some()
    }

    /// Starts a load only when the terminal exposes a new request key.
    ///
    /// Ordinary PTY wakeups are intentionally no-ops for an in-flight or
    /// completed key. A future explicit refresh action can clear the completed
    /// key without turning every output event into another disk scan.
    fn begin(&mut self, request_key: Option<&TerminalOperationHistoryRequestKey>) -> bool {
        let Some(request_key) = request_key else {
            *self = Self::default();
            return false;
        };

        if self.in_flight_key.as_ref() == Some(request_key)
            || self.last_completed_key.as_ref() == Some(request_key)
        {
            return false;
        }

        self.in_flight_key = Some(request_key.clone());
        self.last_error = None;
        true
    }

    /// Applies a background result only when both the terminal's current key
    /// and the latest in-flight key still match the completed request.
    fn complete(
        &mut self,
        current_request_key: Option<&TerminalOperationHistoryRequestKey>,
        completed_key: &TerminalOperationHistoryRequestKey,
        result: Result<T, String>,
    ) -> OperationHistoryCompletion {
        let Some(current_request_key) = current_request_key else {
            *self = Self::default();
            return OperationHistoryCompletion::IgnoredStale;
        };
        let in_flight_matches = self.in_flight_key.as_ref() == Some(completed_key);
        let current_request_matches = current_request_key == completed_key;
        if !in_flight_matches || !current_request_matches {
            if in_flight_matches {
                self.in_flight_key = None;
            }
            return OperationHistoryCompletion::IgnoredStale;
        }

        self.in_flight_key = None;
        self.last_completed_key = Some(completed_key.clone());
        match result {
            Ok(load) => {
                self.current_load = Some(load);
                self.last_error = None;
                OperationHistoryCompletion::AppliedSuccess
            }
            Err(message) => {
                self.last_error = Some(OperationHistoryLoadError {
                    key: completed_key.clone(),
                    message,
                });
                OperationHistoryCompletion::AppliedError
            }
        }
    }
}

impl TerminalView {
    pub(super) fn operation_history_is_available(&self, cx: &App) -> bool {
        self.terminal.read(cx).operation_history_request().is_some()
    }

    pub(super) fn operation_history_panel_is_open(&self) -> bool {
        self.operation_history_panel.is_open()
    }

    pub(super) fn toggle_operation_history_panel(&mut self, cx: &mut Context<Self>) {
        if !self.operation_history_is_available(cx) {
            self.operation_history.begin(None);
            self.operation_history_panel.reset_unavailable();
            cx.notify();
            return;
        }

        self.operation_history_panel.toggle_open();
        if self.operation_history_panel.is_open() {
            self.sync_operation_history(cx);
        }
        cx.notify();
    }

    pub(super) fn close_operation_history_panel(&mut self, cx: &mut Context<Self>) {
        self.operation_history_panel.close();
        cx.notify();
    }

    pub(super) fn should_render_operation_history(&self, cx: &App) -> bool {
        self.operation_history_is_available(cx) && self.operation_history_panel.is_open()
    }

    pub(super) fn render_operation_history_drawer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let is_loading = self.operation_history.is_loading();
        let is_stale = self.operation_history.is_showing_stale_snapshot();
        let load_error = self
            .operation_history
            .last_error
            .as_ref()
            .map(|error| error.message.clone());
        let active_filter = self.operation_history_panel.status_filter();
        let projection = self
            .operation_history
            .current_load
            .as_ref()
            .map(|load| OperationHistoryProjection::from_load(load, active_filter))
            .unwrap_or_default();
        let has_visible_operations = projection.sessions.iter().any(|projected_session| {
            projected_session
                .session
                .generations
                .iter()
                .any(|generation| !generation.operations.is_empty())
        });
        let notices = projection.notices;
        let sessions = projection.sessions;
        let expanded_operations = self.operation_history_panel.expanded_operations().clone();
        let scroll_handle = self.operation_history_panel.scroll_handle().clone();

        let mut content = Vec::new();
        if is_loading {
            content.push(render_operation_history_message(
                if is_stale {
                    t!("TerminalOperationHistory.refreshing")
                } else {
                    t!("TerminalOperationHistory.loading")
                },
                OperationHistoryMessageTone::Info,
                cx,
            ));
        }
        if is_stale {
            content.push(render_operation_history_message(
                t!("TerminalOperationHistory.stale"),
                OperationHistoryMessageTone::Warning,
                cx,
            ));
        }
        if let Some(error) = load_error {
            content.push(render_operation_history_message(
                t!("TerminalOperationHistory.load_failed", error = error),
                OperationHistoryMessageTone::Danger,
                cx,
            ));
        }
        content.extend(
            notices
                .into_iter()
                .map(|notice| render_operation_history_notice(notice, cx)),
        );

        if !is_loading && sessions.is_empty() {
            content.push(render_operation_history_message(
                t!("TerminalOperationHistory.empty"),
                OperationHistoryMessageTone::Muted,
                cx,
            ));
        } else if !has_visible_operations && !sessions.is_empty() {
            content.push(render_operation_history_message(
                if active_filter.is_some() {
                    t!("TerminalOperationHistory.no_matches")
                } else {
                    t!("TerminalOperationHistory.empty")
                },
                OperationHistoryMessageTone::Muted,
                cx,
            ));
        }
        content.extend(
            sessions
                .into_iter()
                .map(|session| render_operation_history_session(session, &expanded_operations, cx)),
        );

        v_flex()
            .debug_selector(|| "terminal-operation-history-drawer".to_string())
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(440.0))
            .max_w(relative(0.92))
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .occlude()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .px_3()
                    .py_2()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(t!("TerminalOperationHistory.title")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(t!("TerminalOperationHistory.read_only")),
                            ),
                    )
                    .child(
                        Button::new("terminal-operation-history-close")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip(t!("TerminalOperationHistory.close"))
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.close_operation_history_panel(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .p_2()
                    .child(render_operation_history_filters(active_filter, cx)),
            )
            .child(
                div().flex_1().min_h_0().min_w_0().overflow_hidden().child(
                    div()
                        .id("terminal-operation-history-scroll")
                        .size_full()
                        .track_scroll(&scroll_handle)
                        .overflow_y_scroll()
                        .child(v_flex().w_full().min_w_0().gap_3().p_3().children(content)),
                ),
            )
            .into_any_element()
    }

    pub(super) fn sync_operation_history(&mut self, cx: &mut Context<Self>) {
        let request = self.terminal.read(cx).operation_history_request();
        let Some(request) = request else {
            self.operation_history.begin(None);
            self.operation_history_panel.reset_unavailable();
            return;
        };
        let request_key = request.key().clone();
        if !self.operation_history.begin(Some(&request_key)) {
            return;
        }

        let task =
            cx.background_spawn(
                async move { request.load(OPERATION_HISTORY_CURRENT_SNAPSHOT_TIMEOUT) },
            );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let load = task.await;
            let _ = this.update(cx, |this, cx| {
                let current_request_key = this
                    .terminal
                    .read(cx)
                    .operation_history_request()
                    .map(|request| request.key().clone());
                let completion = this.operation_history.complete(
                    current_request_key.as_ref(),
                    &request_key,
                    Ok(load),
                );
                if completion != OperationHistoryCompletion::IgnoredStale {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OperationHistoryCompletion, OperationHistoryItemKey, OperationHistoryLoadState,
        OperationHistoryNotice, OperationHistoryPanelState, OperationHistoryProjection,
        OperationHistoryRecoveredJournal, OperationHistorySessionSource,
        operation_history_status_filters, operation_kind_translation_key,
        operation_recovery_warning_translation_key, operation_status_translation_key,
        project_journal,
    };
    use serde_json::json;
    use std::collections::HashSet;
    use terminal::TerminalOperationHistoryRequestKey;
    use terminal::operation_journal::{
        OperationGenerationId, OperationJournal, OperationJournalRecoverySource,
        OperationJournalRecoveryWarningKind, OperationJournalScope, OperationJournalSessionId,
        OperationKind, OperationPayloadCompleteness, OperationPayloadFormat, OperationStatus,
        SensitiveOperationPayload,
    };

    fn request_key(connection_generation: u64) -> TerminalOperationHistoryRequestKey {
        TerminalOperationHistoryRequestKey::new(
            OperationJournalScope::local(),
            OperationJournalSessionId::from("history-state-test"),
            connection_generation,
        )
    }

    fn generation(value: u64) -> OperationGenerationId {
        OperationGenerationId::new(value).expect("generation must be non-zero")
    }

    fn journal(session_id: &str) -> OperationJournal {
        OperationJournal::new(
            OperationJournalSessionId::from(session_id),
            generation(1),
            1_000,
        )
    }

    #[test]
    fn matching_success_applies_and_same_key_does_not_rescan() {
        let key = request_key(1);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&key)));
        assert_eq!(
            state.complete(Some(&key), &key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert_eq!(state.current_load, Some("generation-one"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&key));
        assert!(state.last_error.is_none());
        assert!(state.in_flight_key.is_none());
        assert!(!state.begin(Some(&key)));
    }

    #[test]
    fn matching_task_failure_applies_without_discarding_previous_history() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&old_key), &old_key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert!(state.begin(Some(&new_key)));
        assert_eq!(
            state.complete(
                Some(&new_key),
                &new_key,
                Err("history worker stopped".to_string()),
            ),
            OperationHistoryCompletion::AppliedError
        );

        assert_eq!(state.current_load, Some("generation-one"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&new_key));
        assert_eq!(
            state
                .last_error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("history worker stopped")
        );
        assert_eq!(
            state.last_error.as_ref().map(|error| &error.key),
            Some(&new_key)
        );
        assert!(state.in_flight_key.is_none());
        assert!(!state.begin(Some(&new_key)));
    }

    #[test]
    fn stale_success_and_failure_cannot_overwrite_newer_request() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));

        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("stale-success")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert_eq!(
            state.complete(Some(&new_key), &old_key, Err("stale-error".to_string()),),
            OperationHistoryCompletion::IgnoredStale
        );

        assert_eq!(state.in_flight_key.as_ref(), Some(&new_key));
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn late_stale_completion_cannot_overwrite_completed_newer_history() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));
        assert_eq!(
            state.complete(Some(&new_key), &new_key, Ok("generation-two")),
            OperationHistoryCompletion::AppliedSuccess
        );

        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("late-stale-success")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert_eq!(
            state.complete(
                Some(&new_key),
                &old_key,
                Err("late-stale-error".to_string()),
            ),
            OperationHistoryCompletion::IgnoredStale
        );

        assert_eq!(state.current_load, Some("generation-two"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&new_key));
        assert!(state.last_error.is_none());
        assert!(state.in_flight_key.is_none());
    }

    #[test]
    fn duplicate_in_flight_is_rejected_but_newer_key_supersedes_it() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::<()>::default();

        assert!(state.begin(Some(&old_key)));
        assert!(!state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));
        assert_eq!(state.in_flight_key.as_ref(), Some(&new_key));
    }

    #[test]
    fn current_terminal_key_is_rechecked_before_applying_completion() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("stale-before-wakeup")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert!(state.in_flight_key.is_none());
        assert!(state.begin(Some(&new_key)));
    }

    #[test]
    fn unavailable_history_cancels_state_and_never_starts_loading() {
        let key = request_key(1);
        let mut state = OperationHistoryLoadState::<()>::default();

        assert!(state.begin(Some(&key)));
        assert!(!state.begin(None));
        assert!(state.in_flight_key.is_none());
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn unavailable_history_completion_clears_existing_snapshot() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&old_key), &old_key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert!(state.begin(Some(&new_key)));

        assert_eq!(
            state.complete(None, &new_key, Ok("must-not-apply")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert!(state.in_flight_key.is_none());
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn projection_groups_generations_newest_first_and_operations_by_latest_transition() {
        let mut journal = journal("projection-order");
        let older_operation = journal
            .queue_operation(OperationKind::Command, None, 1_010)
            .expect("queue older operation");
        journal
            .transition_operation(&older_operation, OperationStatus::Failed, 1_020)
            .expect("fail older operation");
        journal
            .begin_generation(generation(2), 2_000)
            .expect("begin generation two");
        let earlier_in_generation = journal
            .queue_operation(OperationKind::Paste, None, 2_010)
            .expect("queue earlier operation");
        let later_in_generation = journal
            .queue_operation(OperationKind::ControlSequence, None, 2_020)
            .expect("queue later operation");

        let projection = project_journal(&journal, None);

        assert_eq!(
            projection
                .generations
                .iter()
                .map(|generation| generation.id.get())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert_eq!(
            projection.generations[0]
                .operations
                .iter()
                .map(|operation| operation.operation_id.as_str())
                .collect::<Vec<_>>(),
            vec![later_in_generation.as_str(), earlier_in_generation.as_str()]
        );
        assert_eq!(
            projection.generations[1].operations[0]
                .operation_id
                .as_str(),
            older_operation.as_str()
        );
    }

    #[test]
    fn current_projection_keeps_pre_reconnect_operations_in_the_closed_generation() {
        let session_id = OperationJournalSessionId::from("reconnect-history-session");
        let mut journal = OperationJournal::new(session_id.clone(), generation(1), 1_000);
        let previous_operation = journal
            .queue_operation(OperationKind::UserInput, None, 1_010)
            .expect("queue pre-reconnect input");
        journal
            .transition_operation(&previous_operation, OperationStatus::Sent, 1_020)
            .expect("mark pre-reconnect input sent");
        journal
            .begin_generation(generation(2), 2_000)
            .expect("begin reconnect generation");

        let projection = OperationHistoryProjection::from_parts(Some(&journal), None, [], [], None);

        assert_eq!(projection.sessions.len(), 1);
        let projected_session = &projection.sessions[0];
        assert_eq!(
            projected_session.source,
            OperationHistorySessionSource::Current
        );
        assert_eq!(projected_session.session.session_id, session_id);
        assert_eq!(
            projected_session
                .session
                .generations
                .iter()
                .map(|generation| generation.id.get())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );

        let reconnect_generation = &projected_session.session.generations[0];
        assert!(!reconnect_generation.is_closed);
        assert!(
            reconnect_generation.operations.is_empty(),
            "the new SSH generation must not receive copied historical input"
        );

        let previous_generation = &projected_session.session.generations[1];
        assert!(previous_generation.is_closed);
        assert_eq!(previous_generation.operations.len(), 1);
        let projected_operation = &previous_generation.operations[0];
        assert_eq!(projected_operation.operation_id, previous_operation);
        assert_eq!(projected_operation.status, OperationStatus::Unknown);
        assert_eq!(projected_operation.key.session_id, session_id);
        assert_eq!(projected_operation.key.generation_id, generation(1));
        assert_eq!(
            projected_operation.key.operation_id,
            projected_operation.operation_id
        );
    }

    #[test]
    fn projection_status_filter_is_exact_and_none_includes_every_operation() {
        let mut journal = journal("projection-filter");
        let succeeded = journal
            .queue_operation(OperationKind::Command, None, 1_010)
            .expect("queue succeeded operation");
        journal
            .transition_operation(&succeeded, OperationStatus::Sent, 1_011)
            .expect("send operation");
        journal
            .transition_operation(&succeeded, OperationStatus::Succeeded, 1_012)
            .expect("succeed operation");
        let failed = journal
            .queue_operation(OperationKind::Paste, None, 1_020)
            .expect("queue failed operation");
        journal
            .transition_operation(&failed, OperationStatus::Failed, 1_021)
            .expect("fail operation");
        let unknown = journal
            .queue_operation(OperationKind::UserInput, None, 1_030)
            .expect("queue unknown operation");
        journal
            .transition_operation(&unknown, OperationStatus::Unknown, 1_031)
            .expect("mark operation unknown");
        let needs_review = journal
            .queue_operation(OperationKind::Unconfirmable, None, 1_040)
            .expect("queue review operation");
        journal
            .transition_operation(&needs_review, OperationStatus::NeedsReview, 1_041)
            .expect("mark operation for review");
        let queued = journal
            .queue_operation(OperationKind::ControlSequence, None, 1_050)
            .expect("queue active operation");

        let all = project_journal(&journal, None);
        assert_eq!(all.generations[0].operations.len(), 5);

        for (status, expected_operation_id) in [
            (OperationStatus::Succeeded, &succeeded),
            (OperationStatus::Failed, &failed),
            (OperationStatus::Unknown, &unknown),
            (OperationStatus::NeedsReview, &needs_review),
            (OperationStatus::Queued, &queued),
        ] {
            let filtered = project_journal(&journal, Some(status));
            assert_eq!(filtered.generations.len(), 1);
            assert_eq!(filtered.generations[0].operations.len(), 1);
            assert_eq!(
                filtered.generations[0].operations[0].operation_id,
                *expected_operation_id
            );
            assert_eq!(filtered.generations[0].operations[0].status, status);
        }
    }

    #[test]
    fn expansion_uses_session_generation_and_operation_as_a_stable_key() {
        let operation_id = terminal::operation_journal::OperationId::from("shared-operation");
        let key = OperationHistoryItemKey {
            session_id: OperationJournalSessionId::from("session-one"),
            generation_id: generation(1),
            operation_id: operation_id.clone(),
        };
        let other_generation = OperationHistoryItemKey {
            session_id: OperationJournalSessionId::from("session-one"),
            generation_id: generation(2),
            operation_id: operation_id.clone(),
        };
        let other_session = OperationHistoryItemKey {
            session_id: OperationJournalSessionId::from("session-two"),
            generation_id: generation(1),
            operation_id,
        };
        let mut panel = OperationHistoryPanelState::default();

        panel.toggle_expanded(key.clone());
        assert!(panel.expanded_operations().contains(&key));
        assert!(!panel.expanded_operations().contains(&other_generation));
        assert!(!panel.expanded_operations().contains(&other_session));

        panel.toggle_expanded(key.clone());
        assert!(!panel.expanded_operations().contains(&key));
    }

    #[test]
    fn confirmed_and_uncertain_statuses_have_distinct_translation_keys() {
        let succeeded = operation_status_translation_key(OperationStatus::Succeeded);
        let failed = operation_status_translation_key(OperationStatus::Failed);
        let unknown = operation_status_translation_key(OperationStatus::Unknown);
        let needs_review = operation_status_translation_key(OperationStatus::NeedsReview);

        assert_eq!(succeeded, "TerminalOperationHistory.status.succeeded");
        assert_eq!(failed, "TerminalOperationHistory.status.failed");
        assert_eq!(unknown, "TerminalOperationHistory.status.unknown");
        assert_eq!(needs_review, "TerminalOperationHistory.status.needsReview");
        assert_ne!(succeeded, failed);
        assert_ne!(succeeded, unknown);
        assert_ne!(succeeded, needs_review);
        assert_ne!(failed, unknown);
        assert_ne!(failed, needs_review);
        assert_ne!(unknown, needs_review);
    }

    #[test]
    fn operation_history_status_filters_are_exact_and_complete() {
        assert_eq!(
            operation_history_status_filters(),
            [
                None,
                Some(OperationStatus::Queued),
                Some(OperationStatus::Sent),
                Some(OperationStatus::Acknowledged),
                Some(OperationStatus::Succeeded),
                Some(OperationStatus::Failed),
                Some(OperationStatus::Unknown),
                Some(OperationStatus::NeedsReview),
                Some(OperationStatus::Canceled),
            ]
        );
    }

    #[test]
    fn projection_keeps_only_redacted_preview_and_safe_payload_metadata() {
        let mut journal = journal("projection-redaction");
        let redacted_payload = SensitiveOperationPayload::structured(json!({
            "path": "/tmp/report.txt",
            "token": "plain-secret"
        }))
        .redact();
        let expected_preview = redacted_payload.preview();
        let expected_original_byte_len = redacted_payload.original_byte_len();
        let operation_id = journal
            .queue_operation_with_payload(
                OperationKind::FileOperation,
                None,
                redacted_payload,
                1_010,
            )
            .expect("queue redacted file operation");

        let projection = project_journal(&journal, None);
        let operation = &projection.generations[0].operations[0];
        let payload = operation.payload.as_ref().expect("payload summary");

        assert_eq!(operation.operation_id, operation_id);
        assert_eq!(payload.preview, expected_preview);
        assert!(!payload.preview.contains("plain-secret"));
        assert_eq!(payload.format, OperationPayloadFormat::StructuredJson);
        assert_eq!(payload.completeness, OperationPayloadCompleteness::Redacted);
        assert_eq!(payload.original_byte_len, expected_original_byte_len);
        assert!(payload.redaction_applied);
    }

    #[test]
    fn projection_preserves_parent_relationship_without_mutating_the_original_operation() {
        let mut journal = journal("projection-parent");
        let parent = journal
            .queue_operation(OperationKind::Command, None, 1_010)
            .expect("queue parent");
        journal
            .transition_operation(&parent, OperationStatus::Failed, 1_020)
            .expect("finish parent");
        let retry = journal
            .queue_operation(OperationKind::Command, Some(&parent), 1_030)
            .expect("queue child");

        let projection = project_journal(&journal, None);
        let child = projection.generations[0]
            .operations
            .iter()
            .find(|operation| operation.operation_id == retry)
            .expect("projected child");

        assert_eq!(child.parent_operation_id.as_ref(), Some(&parent));
        assert_eq!(
            journal
                .operation(&parent)
                .expect("original parent")
                .status(),
            OperationStatus::Failed
        );
    }

    #[test]
    fn load_projection_keeps_current_first_and_deduplicates_recovered_sessions() {
        let current = journal("current-session");
        let duplicate_current = journal("current-session");
        let newest_recovered = journal("newest-recovered");
        let duplicate_recovered = journal("newest-recovered");
        let oldest_recovered = journal("oldest-recovered");

        let projection = OperationHistoryProjection::from_parts(
            Some(&current),
            None,
            [
                OperationHistoryRecoveredJournal {
                    journal: &duplicate_current,
                    created_at_unix_ms: 100,
                    updated_at_unix_ms: 500,
                    recovery_source: Some(OperationJournalRecoverySource::AppendLog),
                    checkpoint_rejection: None,
                    discarded_log_tail_bytes: 0,
                },
                OperationHistoryRecoveredJournal {
                    journal: &newest_recovered,
                    created_at_unix_ms: 200,
                    updated_at_unix_ms: 400,
                    recovery_source: Some(OperationJournalRecoverySource::CheckpointAndAppendLog),
                    checkpoint_rejection: None,
                    discarded_log_tail_bytes: 0,
                },
                OperationHistoryRecoveredJournal {
                    journal: &duplicate_recovered,
                    created_at_unix_ms: 300,
                    updated_at_unix_ms: 300,
                    recovery_source: Some(OperationJournalRecoverySource::Checkpoint),
                    checkpoint_rejection: None,
                    discarded_log_tail_bytes: 0,
                },
                OperationHistoryRecoveredJournal {
                    journal: &oldest_recovered,
                    created_at_unix_ms: 50,
                    updated_at_unix_ms: 100,
                    recovery_source: Some(OperationJournalRecoverySource::AppendLog),
                    checkpoint_rejection: None,
                    discarded_log_tail_bytes: 0,
                },
            ],
            [],
            None,
        );

        assert_eq!(
            projection
                .sessions
                .iter()
                .map(|session| session.session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["current-session", "newest-recovered", "oldest-recovered"]
        );
        assert_eq!(
            projection.sessions[0].source,
            OperationHistorySessionSource::Current
        );
        assert!(matches!(
            projection.sessions[1].source,
            OperationHistorySessionSource::Recovered {
                created_at_unix_ms: 200,
                updated_at_unix_ms: 400,
                recovery_source: Some(OperationJournalRecoverySource::CheckpointAndAppendLog),
                checkpoint_rejection: None,
                discarded_log_tail_bytes: 0,
            }
        ));
    }

    #[test]
    fn load_projection_keeps_snapshot_and_recovery_failures_as_explicit_notices() {
        let session_id = OperationJournalSessionId::from("damaged-session");
        let projection = OperationHistoryProjection::from_parts(
            None,
            Some("current snapshot timed out".to_string()),
            [],
            [
                (
                    OperationJournalRecoveryWarningKind::CheckpointRejected,
                    Some(session_id.clone()),
                ),
                (
                    OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered,
                    Some(session_id.clone()),
                ),
                (
                    OperationJournalRecoveryWarningKind::HistoryLimitReached,
                    None,
                ),
            ],
            None,
        );

        assert_eq!(
            projection.notices,
            vec![
                OperationHistoryNotice::CurrentSnapshotUnavailable(
                    "current snapshot timed out".to_string()
                ),
                OperationHistoryNotice::RecoveryWarning {
                    kind: OperationJournalRecoveryWarningKind::CheckpointRejected,
                    session_id: Some(session_id.clone()),
                },
                OperationHistoryNotice::RecoveryWarning {
                    kind: OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered,
                    session_id: Some(session_id),
                },
                OperationHistoryNotice::RecoveryWarning {
                    kind: OperationJournalRecoveryWarningKind::HistoryLimitReached,
                    session_id: None,
                },
            ]
        );
    }

    #[test]
    fn all_operation_kinds_have_distinct_translation_keys() {
        let keys = [
            operation_kind_translation_key(OperationKind::UserInput),
            operation_kind_translation_key(OperationKind::Command),
            operation_kind_translation_key(OperationKind::Paste),
            operation_kind_translation_key(OperationKind::ControlSequence),
            operation_kind_translation_key(OperationKind::FileOperation),
            operation_kind_translation_key(OperationKind::ApplicationOperation),
            operation_kind_translation_key(OperationKind::Unconfirmable),
        ];

        assert_eq!(
            keys,
            [
                "TerminalOperationHistory.kind.userInput",
                "TerminalOperationHistory.kind.command",
                "TerminalOperationHistory.kind.paste",
                "TerminalOperationHistory.kind.controlSequence",
                "TerminalOperationHistory.kind.fileOperation",
                "TerminalOperationHistory.kind.applicationOperation",
                "TerminalOperationHistory.kind.unconfirmable",
            ]
        );
        let mut unique = HashSet::new();
        assert!(keys.into_iter().all(|key| unique.insert(key)));
    }

    #[test]
    fn recovery_warning_translation_keys_never_alias_confirmed_success() {
        let succeeded = operation_status_translation_key(OperationStatus::Succeeded);
        for kind in [
            OperationJournalRecoveryWarningKind::DirectoryReadFailed,
            OperationJournalRecoveryWarningKind::DirectoryEntryReadFailed,
            OperationJournalRecoveryWarningKind::DirectoryScanLimitReached,
            OperationJournalRecoveryWarningKind::ManifestNotRegularFile,
            OperationJournalRecoveryWarningKind::ManifestTooLarge,
            OperationJournalRecoveryWarningKind::ManifestReadFailed,
            OperationJournalRecoveryWarningKind::InvalidManifest,
            OperationJournalRecoveryWarningKind::DuplicateSessionId,
            OperationJournalRecoveryWarningKind::JournalMissing,
            OperationJournalRecoveryWarningKind::JournalRecoveryFailed,
            OperationJournalRecoveryWarningKind::JournalSessionMismatch,
            OperationJournalRecoveryWarningKind::CheckpointRejected,
            OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered,
            OperationJournalRecoveryWarningKind::HistoryLimitReached,
        ] {
            let warning = operation_recovery_warning_translation_key(kind);
            assert_ne!(warning, succeeded);
            assert!(warning.starts_with("TerminalOperationHistory.warning."));
        }
    }

    #[test]
    fn unavailable_history_resets_panel_to_fail_closed_state() {
        let key = OperationHistoryItemKey {
            session_id: OperationJournalSessionId::from("session-one"),
            generation_id: generation(1),
            operation_id: terminal::operation_journal::OperationId::from("operation-one"),
        };
        let mut panel = OperationHistoryPanelState::default();
        panel.toggle_open();
        panel.set_status_filter(Some(OperationStatus::Unknown));
        panel.toggle_expanded(key);

        panel.reset_unavailable();

        assert!(!panel.is_open());
        assert_eq!(panel.status_filter(), None);
        assert!(panel.expanded_operations().is_empty());
    }

    #[test]
    fn newer_in_flight_request_keeps_previous_snapshot_visible_as_stale_loading_data() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&old_key), &old_key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert!(state.begin(Some(&new_key)));

        assert!(state.is_loading());
        assert!(state.is_showing_stale_snapshot());
        assert_eq!(state.current_load, Some("generation-one"));
    }
}
