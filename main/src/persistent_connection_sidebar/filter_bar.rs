use super::{PersistentConnectionSidebar, SidebarPalette};
use gpui::{Anchor, AnyElement, IntoElement};
use gpui_component::{IconName, Selectable as _, menu::DropdownMenu as _};
use one_core::storage::ConnectionType;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_filter_button(
        &self,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_filter;
        let view = cx.entity();
        IconButton::new("persistent-filter-button", IconName::Filter)
            .role(IconButtonRole::Compact)
            .selected(selected != ConnectionType::All)
            .text_color(palette.muted_foreground)
            .tooltip(t!("Home.connection_filter").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let view = view.clone();
                crate::connection_type_menu::build_filter_menu(
                    menu,
                    selected,
                    std::rc::Rc::new(move |filter, _, cx| {
                        view.update(cx, |tree, cx| {
                            tree.selected_filter = filter;
                            cx.notify();
                        });
                    }),
                )
            })
            .into_any_element()
    }
}
