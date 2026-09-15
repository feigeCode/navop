//! Stateless menu shared by independent connection views.
use crate::connection_visuals::{connection_type_label, connection_type_rail_icon};
use gpui::{App, Window};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use one_core::storage::ConnectionType;
use std::rc::Rc;

/// 为一组连接类型构建筛选菜单；调用方决定展示哪些类型（全部类型或「更多」里的隐藏项）。
pub(crate) fn build_filter_menu(
    menu: PopupMenu,
    kinds: &[ConnectionType],
    selected: ConnectionType,
    activate: Rc<dyn Fn(ConnectionType, &mut Window, &mut App)>,
) -> PopupMenu {
    kinds.iter().fold(menu, |menu, filter| {
        let filter = *filter;
        let activate = activate.clone();
        menu.item(
            PopupMenuItem::new(connection_type_label(filter))
                .icon(connection_type_rail_icon(filter))
                .checked(selected == filter)
                .on_click(move |_, window, cx| activate(filter, window, cx)),
        )
    })
}
