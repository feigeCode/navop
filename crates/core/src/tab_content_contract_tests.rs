use crate::tab_container::{TabContentEvent, TabContentView};
use gpui::App;

#[test]
fn tab_content_view_exposes_sidebar_contracts() {
    let _sidebar_contributions = <dyn TabContentView>::sidebar_contributions;
}

#[test]
fn tab_content_view_exposes_tab_action_contracts() {
    let _can_rename: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::can_rename;
    let _rename: fn(&dyn TabContentView, &str, &mut gpui::Window, &mut App) -> bool =
        <dyn TabContentView>::rename;
    let _can_duplicate: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::can_duplicate;
    let _duplicate: fn(
        &dyn TabContentView,
        &mut gpui::Window,
        &mut App,
    ) -> Option<std::sync::Arc<dyn TabContentView>> = <dyn TabContentView>::duplicate;
}

#[test]
fn tab_content_event_exposes_content_changed_contract() {
    let _event = TabContentEvent::ContentChanged;
}
