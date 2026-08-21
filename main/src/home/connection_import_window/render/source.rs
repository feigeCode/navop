use connection_import_protocol::{ImportRecordKind, ImporterAvailability};
use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, button::Button, checkbox::Checkbox, h_flex, v_flex,
};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::super::ConnectionImportWindow;
use crate::connection_visuals::{ConnectionVisualSize, connection_type_icon};
use crate::home::connection_import_model::ImportSourceState;

pub(super) fn render_source_row(
    source: &ImportSourceState,
    scanning: bool,
    cx: &mut Context<ConnectionImportWindow>,
) -> AnyElement {
    let importer_id = source.descriptor.id.clone();
    let file_importer_id = importer_id.clone();
    let file_pick_prompt = source
        .descriptor
        .capabilities
        .manual_file_pick_prompt
        .clone()
        .unwrap_or_else(|| t!("Home.ConnectionImport.choose_import_file").to_string());
    let file_pick_tooltip = file_pick_prompt.clone();
    let status_text = source
        .preview_error
        .clone()
        .unwrap_or_else(|| availability_text(&source.availability));
    let has_error = source.preview_error.is_some()
        || matches!(source.availability, ImporterAvailability::Error { .. });
    h_flex()
        .items_center()
        .gap_3()
        .p_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            Checkbox::new(format!("import-source-{importer_id}"))
                .checked(source.selected)
                .disabled(scanning || !source.selectable)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.model.toggle_source(&importer_id);
                    cx.notify();
                })),
        )
        .child(connection_type_icon(
            source_connection_type(source),
            ConnectionVisualSize::Tree,
        ))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(source.descriptor.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if has_error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(status_text),
                ),
        )
        .when(
            source.descriptor.capabilities.supports_manual_file_pick,
            |this| {
                this.child(
                    Button::new(format!("import-source-file-{file_importer_id}"))
                        .small()
                        .icon(IconName::FolderOpen)
                        .tooltip(file_pick_tooltip)
                        .disabled(scanning || !source.selectable)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.import_source_file(
                                file_importer_id.clone(),
                                file_pick_prompt.clone(),
                                window,
                                cx,
                            );
                        })),
                )
            },
        )
        .into_any_element()
}

fn source_connection_type(source: &ImportSourceState) -> ConnectionType {
    if source
        .descriptor
        .output_kinds
        .contains(&ImportRecordKind::Ssh)
    {
        ConnectionType::SshSftp
    } else if source
        .descriptor
        .output_kinds
        .contains(&ImportRecordKind::PortForwarding)
    {
        ConnectionType::PortForwarding
    } else {
        ConnectionType::Database
    }
}

fn availability_text(availability: &ImporterAvailability) -> String {
    match availability {
        ImporterAvailability::Available { estimated_count } => estimated_count
            .map(|count| t!("Home.ConnectionImport.available_count", count = count).to_string())
            .unwrap_or_else(|| t!("Home.ConnectionImport.available").to_string()),
        ImporterAvailability::Installed => t!("Home.ConnectionImport.installed").to_string(),
        ImporterAvailability::NotInstalled => t!("Home.ConnectionImport.not_installed").to_string(),
        ImporterAvailability::NoData => t!("Home.ConnectionImport.no_data").to_string(),
        ImporterAvailability::PermissionRequired => {
            t!("Home.ConnectionImport.permission_required").to_string()
        }
        ImporterAvailability::UnsupportedPlatform => {
            t!("Home.ConnectionImport.unsupported_platform").to_string()
        }
        ImporterAvailability::Error { message } => message.clone(),
    }
}
