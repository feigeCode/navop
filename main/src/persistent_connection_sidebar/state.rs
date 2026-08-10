use one_core::settings::{AppSettings, ConnectionSidebarTreeState};

use super::PersistentConnectionSidebar;

impl PersistentConnectionSidebar {
    pub(super) fn set_workspace_collapsed(
        &mut self,
        workspace_id: i64,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let current = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .map(|workspace| workspace.sidebar_collapsed);
        if current.is_some_and(|current| current != collapsed) {
            self.home_page.update(cx, |home, cx| {
                home.set_workspace_sidebar_collapsed(workspace_id, collapsed, cx);
            });
        }
    }

    pub(super) fn toggle_workspace_collapsed(
        &mut self,
        workspace_id: i64,
        cx: &mut gpui::Context<Self>,
    ) {
        let collapsed = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .map(|workspace| !workspace.sidebar_collapsed);
        if let Some(collapsed) = collapsed {
            self.set_workspace_collapsed(workspace_id, collapsed, cx);
        }
    }

    pub(super) fn set_hide_empty_workspaces(&mut self, hide: bool, cx: &mut gpui::Context<Self>) {
        if self.hide_empty_workspaces != hide {
            self.hide_empty_workspaces = hide;
            self.persist_tree_state(cx);
            cx.notify();
        }
    }

    pub(super) fn collapse_all_groups(&mut self, cx: &mut gpui::Context<Self>) {
        self.home_page.update(cx, |home, cx| {
            home.set_all_workspaces_sidebar_collapsed(true, cx);
        });
    }

    fn persist_tree_state(&self, cx: &mut gpui::Context<Self>) {
        let tree_state = stored_tree_state(self.hide_empty_workspaces);
        AppSettings::update_and_save(cx, |settings| {
            settings.connection_sidebar_tree_state = tree_state;
        });
    }
}

fn stored_tree_state(hide_empty_workspaces: bool) -> ConnectionSidebarTreeState {
    ConnectionSidebarTreeState {
        hide_empty_workspaces,
    }
}

#[cfg(test)]
mod tests {
    use super::stored_tree_state;

    #[test]
    fn stored_tree_state_preserves_local_sidebar_preferences() {
        let state = stored_tree_state(true);

        assert!(state.hide_empty_workspaces);
    }
}
