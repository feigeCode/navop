use connection_import_protocol::{ImportRecordKind, ImporterAvailability};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, checkbox::Checkbox, h_flex, v_flex,
};

use super::super::ConnectionImportWindow;
use crate::home::connection_import_model::ImportSourceState;

pub(super) fn render_source_row(
    source: &ImportSourceState,
    scanning: bool,
    cx: &mut Context<ConnectionImportWindow>,
) -> AnyElement {
    let importer_id = source.descriptor.id.clone();
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
        .child(
            Icon::new(source_icon_name(source))
                .color()
                .with_size(Size::Small),
        )
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
                        .text_color(cx.theme().muted_foreground)
                        .child(availability_text(&source.availability)),
                ),
        )
        .into_any_element()
}

fn source_icon_name(source: &ImportSourceState) -> IconName {
    if source
        .descriptor
        .output_kinds
        .contains(&ImportRecordKind::Ssh)
        || source
            .descriptor
            .output_kinds
            .contains(&ImportRecordKind::PortForwarding)
    {
        IconName::TerminalColor
    } else {
        IconName::Database
    }
}

fn availability_text(availability: &ImporterAvailability) -> String {
    match availability {
        ImporterAvailability::Available { estimated_count } => estimated_count
            .map(|count| format!("可导入 {count} 条"))
            .unwrap_or_else(|| "可导入".to_string()),
        ImporterAvailability::Installed => "已安装".to_string(),
        ImporterAvailability::NotInstalled => "未安装".to_string(),
        ImporterAvailability::NoData => "未发现数据".to_string(),
        ImporterAvailability::PermissionRequired => "需要授权".to_string(),
        ImporterAvailability::UnsupportedPlatform => "当前平台不支持".to_string(),
        ImporterAvailability::Error { message } => message.clone(),
    }
}
