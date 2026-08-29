use gpui::{Anchor, AnyElement, IntoElement, Styled as _, prelude::FluentBuilder as _};
use gpui_component::{
    Icon, IconName, IconSize, Selectable as _, Sizable as _,
    button::{IconButton, IconButtonRole},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::connection_visuals::connection_type_rail_icon;
use crate::home_tab::HomePage;

/// Filter control for the connection tree search row.
impl PersistentConnectionSidebar {
    pub(super) fn render_tree_filter_button(
        &self,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let selected_filter = self.home_page.read(cx).selected_filter;
        let navigation = TreeFilterNavigation {
            home: self.home_page.clone(),
        };
        let is_filtered = selected_filter != ConnectionType::All;
        let selected = selected_filter;

        IconButton::new(
            "persistent-filter-button",
            Icon::new(IconName::Filter)
                .mono()
                .with_size(IconSize::Small),
        )
        .role(IconButtonRole::Compact)
        .selected(is_filtered)
        .text_color(if is_filtered {
            palette.accent
        } else {
            palette.muted_foreground
        })
        .when(is_filtered, |button| button.bg(palette.selected))
        .tooltip(t!("Home.connection_filter").to_string())
        .accessible_label(t!("Home.connection_filter").to_string())
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            build_filter_menu(
                menu,
                FilterMenuContext {
                    navigation: navigation.clone(),
                    selected_filter: selected,
                    palette,
                },
            )
        })
        .into_any_element()
    }
}

#[derive(Clone)]
struct TreeFilterNavigation {
    home: gpui::Entity<HomePage>,
}

struct FilterMenuContext {
    navigation: TreeFilterNavigation,
    selected_filter: ConnectionType,
    palette: SidebarPalette,
}

impl TreeFilterNavigation {
    fn activate(&self, filter: ConnectionType, cx: &mut gpui::App) {
        let is_new_filter = self.home.read(cx).selected_filter != filter;
        if is_new_filter {
            self.home
                .update(cx, |home, cx| home.set_selected_filter(filter, cx));
        }
    }
}

fn build_filter_menu(menu: PopupMenu, context: FilterMenuContext) -> PopupMenu {
    let FilterMenuContext {
        navigation,
        selected_filter,
        palette,
    } = context;
    ConnectionType::all()
        .into_iter()
        .fold(menu, |menu, filter| {
            let navigation = navigation.clone();
            menu.item(
                PopupMenuItem::new(filter.label().to_string())
                    .icon(connection_type_rail_icon(filter).text_color(palette.muted_foreground))
                    .checked(selected_filter == filter)
                    .on_click(move |_, _, cx| navigation.activate(filter, cx)),
            )
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn filter_control_is_rendered_inside_the_search_row() {
        let tree = include_str!("tree.rs").replace("\r\n", "\n");
        let filter = tree
            .find("self.render_tree_filter_button(palette, cx)")
            .unwrap();
        let search = tree.find("self.render_tree_search(palette, cx)").unwrap();

        assert!(search < filter);
    }

    #[test]
    fn selecting_a_filter_does_not_collapse_the_connection_tree() {
        let source = include_str!("filter_bar.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();

        assert!(implementation.contains("if is_new_filter"));
        assert!(!implementation.contains("next_tree_expanded_after_filter_click"));
    }
}
