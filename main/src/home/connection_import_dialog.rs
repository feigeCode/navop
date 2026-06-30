use crate::setting_tab::GlobalCurrentUser;
use connection_importer::{
    ImportOptions, ImportSourceKind, ImportSourceStatus, SourceAvailability, list_sources,
    preview_connections, to_db_connection_config,
};
use gpui::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, WindowExt, dialog::DialogButtonProps, h_flex, v_flex,
};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StoredConnection, traits::Repository,
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
                .child("从已安装的数据库客户端导入连接配置，并尝试通过系统 keychain 导入密码。"),
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
        ImportSourceKind::DBeaver | ImportSourceKind::TablePlus | ImportSourceKind::SequelAce
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

fn import_connection_sources(sources: &[ImportSourceKind], cx: &mut App) -> Result<usize, String> {
    sources.iter().try_fold(0usize, |count, source| {
        import_connections(*source, cx).map(|n| count + n)
    })
}

fn import_connections(kind: ImportSourceKind, cx: &mut App) -> Result<usize, String> {
    let imported = preview_connections(
        kind,
        ImportOptions {
            include_passwords: true,
        },
    )
    .map_err(|error| error.to_string())?;
    if imported.is_empty() {
        return Ok(0);
    }

    let owner_id = GlobalCurrentUser::get_user(cx).map(|user| user.id);
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| "ConnectionRepository not found".to_string())?;

    let mut saved = Vec::with_capacity(imported.len());
    for imported_connection in imported {
        let config =
            to_db_connection_config(imported_connection).map_err(|error| error.to_string())?;
        let mut stored = StoredConnection::from_db_connection(config);
        stored.owner_id = owner_id.clone();
        repo.insert(&mut stored)
            .map_err(|error| error.to_string())?;
        saved.push(stored);
    }

    notify_connections_created(saved.clone(), cx);
    Ok(saved.len())
}

fn notify_connections_created(connections: Vec<StoredConnection>, cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    for connection in connections {
        notifier.update(cx, |_, cx| {
            cx.emit(ConnectionDataEvent::ConnectionCreated { connection });
        });
    }
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
        ];

        let kinds = importable_source_kinds(&sources);

        assert_eq!(
            vec![ImportSourceKind::TablePlus, ImportSourceKind::DBeaver],
            kinds
        );
    }
}
