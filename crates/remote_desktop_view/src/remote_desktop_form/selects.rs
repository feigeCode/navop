use gpui::{AppContext, Context, Entity, SharedString, Window};
use gpui_component::select::{SelectItem, SelectState};
use one_core::storage::Workspace;
use rust_i18n::t;

use super::{RemoteDesktopFormWindow, RemoteDesktopFormWindowConfig};

#[derive(Clone, Default, PartialEq)]
pub struct WorkspaceSelectItem {
    pub id: Option<i64>,
    name: String,
}

impl WorkspaceSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    fn from_workspace(workspace: &Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub fn create_workspace_select(
    config: &RemoteDesktopFormWindowConfig,
    window: &mut Window,
    cx: &mut Context<RemoteDesktopFormWindow>,
) -> Entity<SelectState<Vec<WorkspaceSelectItem>>> {
    let mut items = vec![WorkspaceSelectItem::none()];
    items.extend(
        config
            .workspaces
            .iter()
            .map(WorkspaceSelectItem::from_workspace),
    );
    cx.new(|cx| SelectState::new(items, Some(Default::default()), window, cx))
}
