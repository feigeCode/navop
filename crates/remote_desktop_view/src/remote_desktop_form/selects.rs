use gpui::{AppContext, Context, Entity, SharedString, Window};
use gpui_component::IndexPath;
use gpui_component::select::{SelectItem, SelectState};
use one_core::storage::{RemoteDesktopBackendPreference, Workspace};
use rust_i18n::t;

use super::backend_preference::backend_preferences;
use super::{RemoteDesktopFormWindow, RemoteDesktopFormWindowConfig};

#[derive(Clone, PartialEq)]
pub(super) struct BackendPreferenceSelectItem {
    preference: RemoteDesktopBackendPreference,
    label: String,
}

impl BackendPreferenceSelectItem {
    fn all() -> Vec<Self> {
        backend_preferences()
            .into_iter()
            .map(Self::from_preference)
            .collect()
    }

    fn from_preference(preference: RemoteDesktopBackendPreference) -> Self {
        let label = match preference {
            RemoteDesktopBackendPreference::Auto => t!("RemoteDesktopForm.backend_auto"),
            RemoteDesktopBackendPreference::WindowsNative => {
                t!("RemoteDesktopForm.backend_windows_native")
            }
            RemoteDesktopBackendPreference::Canvas => t!("RemoteDesktopForm.backend_canvas"),
        };
        Self {
            preference,
            label: label.to_string(),
        }
    }
}

impl SelectItem for BackendPreferenceSelectItem {
    type Value = RemoteDesktopBackendPreference;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.preference
    }
}

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

pub(super) fn create_backend_preference_select(
    window: &mut Window,
    cx: &mut Context<RemoteDesktopFormWindow>,
) -> Entity<SelectState<Vec<BackendPreferenceSelectItem>>> {
    let items = BackendPreferenceSelectItem::all();
    let selected_index = items
        .iter()
        .position(|item| item.preference == RemoteDesktopBackendPreference::default())
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(items, selected_index, window, cx))
}

#[cfg(test)]
mod tests {
    use one_core::storage::RemoteDesktopBackendPreference;

    use super::BackendPreferenceSelectItem;

    #[test]
    fn backend_select_items_keep_auto_native_and_canvas_distinct() {
        let preferences = BackendPreferenceSelectItem::all()
            .into_iter()
            .map(|item| item.preference)
            .collect::<Vec<_>>();

        assert_eq!(
            vec![
                RemoteDesktopBackendPreference::Auto,
                RemoteDesktopBackendPreference::WindowsNative,
                RemoteDesktopBackendPreference::Canvas,
            ],
            preferences
        );
    }
}
