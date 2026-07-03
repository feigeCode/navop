use crate::tab_container::TabContentView;
use gpui::App;

#[test]
fn tab_content_view_exposes_sidebar_contracts() {
    let _can_split: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::can_split;
    let _sidebar_contributions = <dyn TabContentView>::sidebar_contributions;
}
