use connection_import_protocol::{ImportRecordKind, ImporterDescriptor};
use gpui::{IntoElement, Styled};
use gpui_component::{Icon, IconName};

use super::connection_import_draft::ImportDraftKind;

pub(super) fn source_icon(kind: ImportDraftKind, hint: &str) -> impl IntoElement {
    let icon = match kind {
        ImportDraftKind::Database if hint.contains("tableplus") => IconName::Table,
        ImportDraftKind::Database => IconName::Database,
        ImportDraftKind::Ssh => IconName::TerminalColor,
    };
    Icon::new(icon).color().size_5()
}

pub(super) fn importer_icon(importer: &ImporterDescriptor) -> impl IntoElement {
    let icon = if importer.output_kinds.contains(&ImportRecordKind::Ssh) {
        IconName::TerminalColor
    } else {
        IconName::Database
    };
    Icon::new(icon).color().size_5()
}
