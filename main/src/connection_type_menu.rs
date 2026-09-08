//! Stateless menu shared by independent connection views.
use crate::connection_visuals::{connection_type_label, connection_type_rail_icon};
use gpui::{App, Window};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use one_core::storage::ConnectionType;
use std::rc::Rc;

pub(crate) fn build_filter_menu(
    menu: PopupMenu,
    selected: ConnectionType,
    activate: Rc<dyn Fn(ConnectionType, &mut Window, &mut App)>,
) -> PopupMenu {
    ConnectionType::all()
        .into_iter()
        .fold(menu, |menu, filter| {
            let activate = activate.clone();
            menu.item(
                PopupMenuItem::new(connection_type_label(filter))
                    .icon(connection_type_rail_icon(filter))
                    .checked(selected == filter)
                    .on_click(move |_, window, cx| activate(filter, window, cx)),
            )
        })
}
