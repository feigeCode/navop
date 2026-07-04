use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex, v_flex,
};

use super::super::{ConnectionImportWindow, is_save_candidate};
use crate::home::connection_import_draft::{EditableImportDraft, ImportDraftKind};
use crate::home::connection_import_model::{ImportPreviewRow, ImportRowSaveStatus};

pub(super) fn render_preview_row(
    row: &ImportPreviewRow,
    cx: &mut Context<ConnectionImportWindow>,
) -> AnyElement {
    let record_id = row.record_id().to_string();
    h_flex()
        .items_center()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .child(
            Checkbox::new(format!("import-row-{record_id}"))
                .checked(row.selected)
                .disabled(!is_save_candidate(&row.save_status))
                .on_click(cx.listener({
                    let record_id = record_id.clone();
                    move |this, _, _, cx| {
                        this.model.toggle_row(&record_id);
                        cx.notify();
                    }
                })),
        )
        .child(
            Icon::new(row_icon_name(&row.draft))
                .color()
                .with_size(Size::Small),
        )
        .child(render_row_text(row, cx))
        .child(render_row_status(row, cx))
        .child(render_row_actions(row, &record_id, cx))
        .into_any_element()
}

fn render_row_actions(
    row: &ImportPreviewRow,
    record_id: &str,
    cx: &mut Context<ConnectionImportWindow>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(
            Button::new(format!("edit-import-{record_id}"))
                .xsmall()
                .icon(IconName::Edit)
                .disabled(matches!(row.save_status, ImportRowSaveStatus::Saving))
                .on_click(cx.listener({
                    let record_id = record_id.to_string();
                    move |this, _, _, cx| this.edit_row(record_id.clone(), cx)
                })),
        )
        .child(
            Button::new(format!("save-import-{record_id}"))
                .xsmall()
                .primary()
                .icon(IconName::Upload)
                .label("保存")
                .disabled(!is_save_candidate(&row.save_status))
                .on_click(cx.listener({
                    let record_id = record_id.to_string();
                    move |this, _, _, cx| this.save_row(record_id.clone(), cx)
                })),
        )
}

fn render_row_text(
    row: &ImportPreviewRow,
    cx: &mut Context<ConnectionImportWindow>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(row.draft.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(kind_text(row.draft.kind())),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(row_detail_text(&row.draft)),
        )
        .when_some(row.draft.warning_text(), |this, warning| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(warning),
            )
        })
}

fn row_detail_text(draft: &EditableImportDraft) -> String {
    let mut parts = vec![draft.source_name().to_string()];
    if let Some(endpoint) = endpoint_text(draft) {
        parts.push(endpoint);
    }
    if let Some(username) = trimmed_text(&draft.username) {
        parts.push(username);
    }
    parts.push(draft.password_status_text().to_string());
    parts.join(" · ")
}

fn endpoint_text(draft: &EditableImportDraft) -> Option<String> {
    let host = trimmed_text(&draft.host)?;
    Some(match trimmed_text(&draft.port) {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn trimmed_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn render_row_status(
    row: &ImportPreviewRow,
    cx: &mut Context<ConnectionImportWindow>,
) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(status_color(&row.save_status, cx))
        .child(status_text(&row.save_status))
}

fn row_icon_name(draft: &EditableImportDraft) -> IconName {
    match draft.kind() {
        ImportDraftKind::Database => IconName::Database,
        ImportDraftKind::Ssh | ImportDraftKind::Unsupported => IconName::TerminalColor,
    }
}

fn kind_text(kind: ImportDraftKind) -> &'static str {
    match kind {
        ImportDraftKind::Database => "数据库",
        ImportDraftKind::Ssh => "SSH",
        ImportDraftKind::Unsupported => "暂不支持",
    }
}

fn status_text(status: &ImportRowSaveStatus) -> String {
    match status {
        ImportRowSaveStatus::Pending => "待保存".to_string(),
        ImportRowSaveStatus::Saving => "保存中".to_string(),
        ImportRowSaveStatus::Saved { .. } => "已保存".to_string(),
        ImportRowSaveStatus::Failed { message } => format!("失败：{message}"),
        ImportRowSaveStatus::SkippedDuplicate { existing_name } => {
            format!("已跳过重复：{existing_name}")
        }
    }
}

fn status_color(
    status: &ImportRowSaveStatus,
    cx: &mut Context<ConnectionImportWindow>,
) -> gpui::Hsla {
    match status {
        ImportRowSaveStatus::Saved { .. } => cx.theme().success,
        ImportRowSaveStatus::Failed { .. } => cx.theme().danger,
        ImportRowSaveStatus::SkippedDuplicate { .. } => cx.theme().warning,
        ImportRowSaveStatus::Pending | ImportRowSaveStatus::Saving => cx.theme().muted_foreground,
    }
}
