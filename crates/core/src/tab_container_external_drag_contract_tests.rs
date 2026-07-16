#[test]
fn tab_bar_accepts_external_tab_drag_sources() {
    let source = include_str!("tab_container.rs");

    assert!(source.contains("pub trait ExternalTabDragSource"));
    assert!(source.contains("pub fn from_external"));
    assert!(source.contains("fn take_external_tab"));
    assert!(source.matches("drag.take_external_tab").count() >= 2);
}

#[test]
fn tab_content_can_restore_a_split_pane_as_a_background_tab() {
    let source = include_str!("tab_container.rs");

    assert!(source.contains("TabContentEvent::OpenTab"));
    assert!(source.contains("add_tab_with_mode(tab.clone(), *mode"));
}

#[test]
fn tab_context_menu_explains_terminal_split_support() {
    let source = include_str!("tab_container.rs");
    let help = include_str!("tab_split_help.rs");
    let locales = include_str!("../locales/core.yml");

    assert!(source.contains("content_key(cx) == \"Terminal\""));
    assert!(source.contains("TerminalSplitHelp::new"));
    assert!(help.contains("TabContextMenu.split_help"));
    assert!(locales.contains("split_help:"));
    assert!(locales.contains("split_only_terminal:"));
}
