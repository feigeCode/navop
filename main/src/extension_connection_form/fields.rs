use gpui::{App, AppContext, Context, Entity, Window};
use gpui_component::{input::InputState, select::SelectState};
use one_core::storage::Workspace;

use super::{ExtensionConnectionForm, WorkspaceItem};

pub(super) fn optional_input_text(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input.read(cx).text().to_string().trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub(super) fn create_name_input(
    value: String,
    window: &mut Window,
    cx: &mut Context<ExtensionConnectionForm>,
) -> Entity<InputState> {
    create_input(value, "Connection name", window, cx)
}

pub(super) fn create_input(
    value: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<ExtensionConnectionForm>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(placeholder);
        input.set_value(value, window, cx);
        input
    })
}

pub(super) fn create_workspace_select(
    workspaces: &[Workspace],
    selected: Option<i64>,
    window: &mut Window,
    cx: &mut Context<ExtensionConnectionForm>,
) -> Entity<SelectState<Vec<WorkspaceItem>>> {
    let items = std::iter::once(WorkspaceItem {
        id: None,
        label: "None".into(),
    })
    .chain(workspaces.iter().map(|workspace| WorkspaceItem {
        id: workspace.id,
        label: workspace.name.clone(),
    }))
    .collect();
    cx.new(|cx| {
        let mut select = SelectState::new(items, Some(Default::default()), window, cx);
        select.set_selected_value(&selected, window, cx);
        select
    })
}
