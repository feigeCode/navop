use connection_importer::ImportSourceKind;
use gpui::{IntoElement, Styled};
use gpui_component::{Icon, IconName};

pub(super) fn source_icon(kind: ImportSourceKind) -> impl IntoElement {
    let icon = match kind {
        ImportSourceKind::TablePlus => IconName::Table,
        ImportSourceKind::BeekeeperStudio => IconName::Apps,
        ImportSourceKind::Xshell | ImportSourceKind::FinalShell | ImportSourceKind::Termius => {
            IconName::TerminalColor
        }
        ImportSourceKind::DBeaver
        | ImportSourceKind::SequelAce
        | ImportSourceKind::DataGrip
        | ImportSourceKind::HeidiSQL
        | ImportSourceKind::Navicat => IconName::Database,
    };
    Icon::new(icon).size_5()
}
