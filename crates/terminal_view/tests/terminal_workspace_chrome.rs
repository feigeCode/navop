use std::fs;
use std::path::PathBuf;

fn workspace_source(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/workspace")
        .join(file);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn terminal_pane_uses_a_floating_tool_instead_of_a_layout_header() {
    let modules = workspace_source("mod.rs");
    let render = workspace_source("render.rs");
    let tool = workspace_source("pane_tool.rs");

    assert!(modules.contains("mod pane_tool;"));
    assert!(!modules.contains("mod pane_header;"));
    assert!(render.contains("render_pane_floating_tool"));
    assert!(!render.contains("render_pane_header"));
    assert!(tool.contains(".absolute()"));
    assert!(tool.contains(".top_2()"));
    assert!(tool.contains(".right_2()"));
}

#[test]
fn floating_tool_does_not_offer_button_driven_splitting() {
    let modules = workspace_source("mod.rs");
    let actions = workspace_source("actions.rs");
    let tool = workspace_source("pane_tool.rs");

    assert!(!modules.contains("mod connections;"));
    assert!(!actions.contains("fn split_pane("));
    assert!(!tool.contains("FolderColumns"));
    assert!(!tool.contains("dropdown_menu"));
    assert!(!tool.contains("TerminalWorkspace.split_"));
}

#[test]
fn active_terminal_pane_keeps_a_highlight_border() {
    let render = workspace_source("render.rs");

    assert!(render.contains("let border = if active"));
    assert!(render.contains("cx.theme().drag_border"));
    assert!(render.contains(".border_color(border)"));
}

#[test]
fn terminal_tab_drop_region_renders_direct_zones_above_terminal_content() {
    let tab_drag = workspace_source("tab_drag.rs");

    assert!(tab_drag.contains(".id((\"terminal-tab-drop-region\""));
    assert!(tab_drag.contains("self.render_tab_drop_zone"));
    assert!(tab_drag.contains(".drag_over::<DragTab>"));
    assert!(tab_drag.contains("show_drop_highlight"));
}

#[test]
fn shared_sidebar_does_not_add_a_workspace_header() {
    let render = workspace_source("render.rs");

    assert!(!render.contains("render_sidebar_target_header"));
    assert!(!render.contains("terminal-sidebar-target-pin"));
}

#[test]
fn a_single_terminal_does_not_render_split_chrome() {
    let render = workspace_source("render.rs");

    assert!(render.contains("let split = self.panes.len() > 1"));
    assert!(render.contains("when(split"));
    assert!(render.contains("render_pane_floating_tool"));
    assert!(render.contains("border_1().border_color(border)"));
}

#[test]
fn floating_title_has_stable_width_and_can_drag_a_pane_back_to_tabs() {
    let tool = workspace_source("pane_tool.rs");
    let transfer = workspace_source("pane_tab_transfer.rs");

    assert!(tool.contains(".min_w(px(190.0))"));
    assert!(tool.contains("DragTab::from_external"));
    assert!(transfer.contains("impl ExternalTabDragSource"));
    assert!(transfer.contains("detach_pane_as_tab"));
}

#[test]
fn floating_title_drag_does_not_select_terminal_content() {
    let tool = workspace_source("pane_tool.rs");

    assert!(tool.contains(".on_mouse_down(MouseButton::Left"));
    assert!(tool.contains(".on_mouse_move("));
    assert!(tool.matches("window.prevent_default();").count() >= 3);
    assert!(tool.matches("cx.stop_propagation();").count() >= 3);
}

#[test]
fn all_split_panes_are_equal_and_offer_cancel_split_without_pin() {
    let view = workspace_source("view.rs");
    let tool = workspace_source("pane_tool.rs");
    let actions = workspace_source("actions.rs");

    assert!(!view.contains("main_pane_id"));
    assert!(!tool.contains("IconName::Pin"));
    assert!(!actions.contains("toggle_sidebar_target"));
    assert!(tool.contains("terminal-pane-cancel-split"));
    assert!(tool.contains("restore_pane_to_tab"));
}
