use db::{DbNode, DbNodeType};
use gpui::ClipboardItem;
use gpui_component::{
    IconName, WindowExt,
    menu::{PopupMenu, PopupMenuItem},
    notification::Notification,
};
use rust_i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableCopyTarget {
    Name,
    Comment,
}

pub(crate) fn append_table_copy_items(mut menu: PopupMenu, node: &DbNode) -> PopupMenu {
    if node.node_type != DbNodeType::Table {
        return menu;
    }

    menu = menu.item(copy_item(TableCopyTarget::Name, node));
    menu.item(copy_item(TableCopyTarget::Comment, node))
}

fn copy_item(target: TableCopyTarget, node: &DbNode) -> PopupMenuItem {
    let (label, text) = match target {
        TableCopyTarget::Name => (
            t!("Table.copy_table_name").to_string(),
            table_copy_text(node, target),
        ),
        TableCopyTarget::Comment => (
            t!("Table.copy_table_comment").to_string(),
            table_copy_text(node, target),
        ),
    };

    PopupMenuItem::new(label)
        .icon(IconName::Copy)
        .disabled(text.is_none())
        .on_click(move |_, window, cx| {
            let Some(text) = text.clone() else {
                return;
            };
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            window.push_notification(
                Notification::success(t!("Table.copy_success").to_string()).autohide(true),
                cx,
            );
        })
}

fn table_copy_text(node: &DbNode, target: TableCopyTarget) -> Option<String> {
    if node.node_type != DbNodeType::Table {
        return None;
    }

    let value = match target {
        TableCopyTarget::Name => node.get_table_name(),
        TableCopyTarget::Comment => node.metadata.get("comment").cloned(),
    }?;

    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;
    use std::collections::HashMap;

    fn table_node(comment: Option<&str>) -> DbNode {
        let mut metadata = HashMap::new();
        metadata.insert("table".to_string(), "users".to_string());
        if let Some(comment) = comment {
            metadata.insert("comment".to_string(), comment.to_string());
        }

        DbNode::new(
            "conn:db:table:users",
            "users",
            DbNodeType::Table,
            "conn".to_string(),
            DatabaseType::PostgreSQL,
        )
        .with_metadata(metadata)
    }

    #[test]
    fn table_copy_text_returns_table_name_and_comment() {
        let node = table_node(Some("Application users"));

        assert_eq!(
            Some("users".to_string()),
            table_copy_text(&node, TableCopyTarget::Name)
        );
        assert_eq!(
            Some("Application users".to_string()),
            table_copy_text(&node, TableCopyTarget::Comment)
        );
    }

    #[test]
    fn table_copy_text_disables_missing_or_blank_comment() {
        assert_eq!(
            None,
            table_copy_text(&table_node(None), TableCopyTarget::Comment)
        );
        assert_eq!(
            None,
            table_copy_text(&table_node(Some("  ")), TableCopyTarget::Comment)
        );
    }

    #[test]
    fn table_copy_text_is_only_available_for_tables() {
        let mut node = table_node(Some("Application users"));
        node.node_type = DbNodeType::View;

        assert_eq!(None, table_copy_text(&node, TableCopyTarget::Name));
        assert_eq!(None, table_copy_text(&node, TableCopyTarget::Comment));
    }
}
