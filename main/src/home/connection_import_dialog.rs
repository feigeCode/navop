use super::connection_import_actions::{preview_import_drafts, save_selected_import_drafts};
use super::connection_import_preview_view::ConnectionImportPreview;
use connection_importer::{ImportSourceKind, ImportSourceStatus, SourceAvailability, list_sources};
#[cfg(test)]
use gpui::SharedString;
use gpui::{App, AppContext, ParentElement, Window, px};
use gpui_component::{WindowExt, dialog::DialogButtonProps};
use rust_i18n::t;

pub(crate) fn show_connection_import_dialog(window: &mut Window, cx: &mut App) {
    let sources = list_sources();
    let source_kinds = importable_source_kinds(&sources);
    let (drafts, preview_error) = match preview_import_drafts(&source_kinds) {
        Ok(drafts) => (drafts, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let preview = cx.new(|cx| ConnectionImportPreview::new(drafts, preview_error, window, cx));
    let preview_for_render = preview.clone();
    let preview_for_ok = preview.clone();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Home.import").to_string())
            .w(px(760.0))
            .child(preview_for_render.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Home.import"))
                    .cancel_text(t!("Common.cancel"))
                    .show_cancel(true),
            )
            .on_ok({
                let preview_for_ok = preview_for_ok.clone();
                move |_, window, cx| {
                    let drafts = match preview_for_ok.read(cx).collect_drafts(cx) {
                        Ok(drafts) => drafts,
                        Err(error) => {
                            window.push_notification(format!("导入失败：{}", error), cx);
                            return false;
                        }
                    };
                    match save_selected_import_drafts(&drafts, cx) {
                        Ok(0) => {
                            window.push_notification("没有选择要导入的连接".to_string(), cx);
                            false
                        }
                        Ok(count) => {
                            window.push_notification(
                                t!("Home.import_success", count = count).to_string(),
                                cx,
                            );
                            true
                        }
                        Err(error) => {
                            window.push_notification(format!("导入失败：{}", error), cx);
                            false
                        }
                    }
                }
            })
    });
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
            | ImportSourceKind::HeidiSQL
            | ImportSourceKind::Navicat
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

#[cfg(test)]
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
        let sources = supported_source_statuses();

        let kinds = importable_source_kinds(&sources);

        assert_eq!(
            vec![
                ImportSourceKind::TablePlus,
                ImportSourceKind::DBeaver,
                ImportSourceKind::DataGrip,
                ImportSourceKind::Xshell,
                ImportSourceKind::HeidiSQL,
                ImportSourceKind::Navicat,
                ImportSourceKind::FinalShell,
                ImportSourceKind::Termius
            ],
            kinds
        );
    }

    fn supported_source_statuses() -> Vec<ImportSourceStatus> {
        vec![
            available(ImportSourceKind::TablePlus, 1),
            available(ImportSourceKind::DBeaver, 2),
            ImportSourceStatus::new(ImportSourceKind::SequelAce, SourceAvailability::Unsupported),
            available(ImportSourceKind::DataGrip, 1),
            available(ImportSourceKind::Xshell, 1),
            available(ImportSourceKind::HeidiSQL, 1),
            available(ImportSourceKind::Navicat, 1),
            available(ImportSourceKind::FinalShell, 1),
            available(ImportSourceKind::Termius, 1),
        ]
    }

    fn available(kind: ImportSourceKind, connection_count: usize) -> ImportSourceStatus {
        ImportSourceStatus::new(kind, SourceAvailability::Available { connection_count })
    }
}
