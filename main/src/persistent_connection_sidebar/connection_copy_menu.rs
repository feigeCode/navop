use gpui::ClipboardItem;
use gpui_component::{
    IconName,
    menu::{PopupMenu, PopupMenuItem},
};
use one_core::storage::StoredConnection;
use rust_i18n::t;

use super::connection_copy::{ConnectionCopyTarget, connection_copy_targets};

pub(super) fn append_copy_targets(mut menu: PopupMenu, connection: &StoredConnection) -> PopupMenu {
    for (target, value) in connection_copy_targets(connection) {
        let (label, icon) = copy_target_presentation(target);
        menu = menu.item(copy_text_item(label, value).icon(icon));
    }
    menu
}

pub(super) fn copy_text_item(label: String, text: String) -> PopupMenuItem {
    PopupMenuItem::new(label)
        .icon(IconName::Copy)
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        })
}

fn copy_target_presentation(target: ConnectionCopyTarget) -> (String, IconName) {
    match target {
        ConnectionCopyTarget::DatabaseAddress => label("copy_database_target", IconName::Network),
        ConnectionCopyTarget::SshTarget => label("copy_ssh_target", IconName::Network),
        ConnectionCopyTarget::RedisAddress => label("copy_redis_target", IconName::Network),
        ConnectionCopyTarget::MongoDbAddress => label("copy_mongodb_target", IconName::Network),
        ConnectionCopyTarget::Username => label("copy_username", IconName::User),
        ConnectionCopyTarget::SerialPort => label("copy_serial_port", IconName::Network),
        ConnectionCopyTarget::ForwardingRule => label("copy_forwarding_rule", IconName::Network),
        ConnectionCopyTarget::RemoteDesktopAddress => {
            label("copy_remote_desktop_target", IconName::Network)
        }
    }
}

fn label(key: &str, icon: IconName) -> (String, IconName) {
    let key = format!("Connection.{key}");
    (t!(&key).to_string(), icon)
}
