use super::connection_import_actions::import_connection_sources;
use connection_importer::{ImportSourceKind, ImportSourceStatus, SourceAvailability, list_sources};
use gpui::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, WindowExt, dialog::DialogButtonProps, h_flex, v_flex,
};
use rust_i18n::t;

pub(crate) fn show_connection_import_dialog(window: &mut Window, cx: &mut App) {
    let sources = list_sources();
    window.open_dialog(cx, move |dialog, _window, cx| {
        let sources_for_ok = sources.clone();
        dialog
            .title(t!("Home.import").to_string())
            .w(px(520.0))
            .child(render_import_dialog_content(&sources, cx))
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Home.import"))
                    .cancel_text(t!("Common.cancel"))
                    .show_cancel(true),
            )
            .on_ok(move |_, window, cx| {
                let sources = importable_source_kinds(&sources_for_ok);
                match import_connection_sources(&sources, cx) {
                    Ok(0) => {
                        window.push_notification("没有可导入的连接".to_string(), cx);
                    }
                    Ok(count) => {
                        window.push_notification(
                            t!("Home.import_success", count = count).to_string(),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(format!("导入失败：{}", error), cx);
                    }
                }
                true
            })
    });
}

fn render_import_dialog_content(sources: &[ImportSourceStatus], cx: &mut App) -> impl IntoElement {
    let rows = sources
        .iter()
        .map(|source| render_source_row(source, cx))
        .collect::<Vec<_>>();

    v_flex()
        .gap_4()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(
                    "从已安装的数据库和 SSH 客户端导入连接配置，并尝试通过系统 keychain 导入密码。",
                ),
        )
        .child(v_flex().gap_2().children(rows))
}

fn render_source_row(source: &ImportSourceStatus, cx: &mut App) -> AnyElement {
    let available = matches!(
        source.availability,
        SourceAvailability::Available { .. }
            | SourceAvailability::Installed
            | SourceAvailability::NoConnections
    );
    h_flex()
        .items_center()
        .justify_between()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .min_w_0()
                .child(source_icon(source.kind))
                .child(
                    v_flex()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().foreground)
                                .child(source.display_name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(source_availability_summary(&source.availability)),
                        ),
                ),
        )
        .child(
            div()
                .text_xs()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .bg(if available {
                    cx.theme().success
                } else {
                    cx.theme().muted
                })
                .text_color(if available {
                    cx.theme().success_foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(if available { "可用" } else { "不可用" }),
        )
        .into_any_element()
}

fn source_icon(kind: ImportSourceKind) -> impl IntoElement {
    let icon = match kind {
        ImportSourceKind::DBeaver => IconName::Database,
        ImportSourceKind::TablePlus => IconName::Table,
        ImportSourceKind::SequelAce => IconName::Database,
        ImportSourceKind::BeekeeperStudio => IconName::Apps,
        ImportSourceKind::DataGrip => IconName::Database,
        ImportSourceKind::Xshell => IconName::TerminalColor,
        ImportSourceKind::FinalShell => IconName::TerminalColor,
        ImportSourceKind::Termius => IconName::TerminalColor,
    };
    Icon::new(icon).size_5()
}

fn importable_source_kinds(sources: &[ImportSourceStatus]) -> Vec<ImportSourceKind> {
    sources
        .iter()
        .filter(|source| is_supported_source(source.kind) && is_available_source(source))
        .map(|source| source.kind)
        .collect()
}

fn is_supported_source(kind: ImportSourceKind) -> bool {
    matches!(
        kind,
        ImportSourceKind::DBeaver
            | ImportSourceKind::TablePlus
            | ImportSourceKind::SequelAce
            | ImportSourceKind::BeekeeperStudio
            | ImportSourceKind::DataGrip
            | ImportSourceKind::Xshell
            | ImportSourceKind::FinalShell
            | ImportSourceKind::Termius
    )
}

fn is_available_source(source: &ImportSourceStatus) -> bool {
    matches!(
        source.availability,
        SourceAvailability::Available { .. }
            | SourceAvailability::Installed
            | SourceAvailability::NoConnections
    )
}

fn source_availability_summary(availability: &SourceAvailability) -> SharedString {
    match availability {
        SourceAvailability::Available { connection_count } => {
            format!("找到 {} 个连接", connection_count).into()
        }
        SourceAvailability::Installed => "已安装，未发现连接".into(),
        SourceAvailability::NoConnections => "未发现连接".into(),
        SourceAvailability::NotInstalled => "未检测到应用数据".into(),
        SourceAvailability::Unsupported => "暂不支持".into(),
        SourceAvailability::PermissionRequired => "需要文件访问权限".into(),
        SourceAvailability::Error { message } => format!("读取失败：{}", message).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connection_importer::{ImportSourceKind, ImportSourceStatus, SourceAvailability};

    #[test]
    fn source_availability_summary_formats_available_count() {
        let summary = source_availability_summary(&SourceAvailability::Available {
            connection_count: 3,
        });

        assert_eq!(summary, "找到 3 个连接");
    }

    #[test]
    fn source_availability_summary_marks_reserved_sources() {
        let summary = source_availability_summary(&SourceAvailability::Unsupported);

        assert_eq!(summary, "暂不支持");
    }

    #[test]
    fn importable_source_kinds_include_supported_available_sources() {
        let sources = vec![
            ImportSourceStatus::new(
                ImportSourceKind::TablePlus,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
            ImportSourceStatus::new(
                ImportSourceKind::DBeaver,
                SourceAvailability::Available {
                    connection_count: 2,
                },
            ),
            ImportSourceStatus::new(ImportSourceKind::SequelAce, SourceAvailability::Unsupported),
            ImportSourceStatus::new(
                ImportSourceKind::DataGrip,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
            ImportSourceStatus::new(
                ImportSourceKind::Xshell,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
            ImportSourceStatus::new(
                ImportSourceKind::FinalShell,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
            ImportSourceStatus::new(
                ImportSourceKind::Termius,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
        ];

        let kinds = importable_source_kinds(&sources);

        assert_eq!(
            vec![
                ImportSourceKind::TablePlus,
                ImportSourceKind::DBeaver,
                ImportSourceKind::DataGrip,
                ImportSourceKind::Xshell,
                ImportSourceKind::FinalShell,
                ImportSourceKind::Termius
            ],
            kinds
        );
    }
}
