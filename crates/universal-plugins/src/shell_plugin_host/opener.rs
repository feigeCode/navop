use std::{rc::Rc, sync::atomic::Ordering};

use gpui::{App, Window};
use one_core::tab_container::{GlobalTabContainer, TabItem};

use super::{NEXT_SHELL_TAB_ID, ShellPluginHost, TrackedPluginTab};
use crate::shell_plugin_tab::{ShellPluginLoad, ShellPluginTab};

impl extension_view::ShellViewOpener for ShellPluginHost {
    fn open(&self, extension_id: &str, view_id: &str, window: &mut Window, cx: &mut App) {
        if self.retiring.borrow().contains(extension_id) {
            return;
        }
        let Some(contribution) = self.contribution(extension_id, view_id) else {
            return;
        };
        let tab_id = if contribution.singleton {
            format!("shell:{}", contribution.view_key)
        } else {
            format!(
                "shell:{}:{}",
                contribution.view_key,
                NEXT_SHELL_TAB_ID.fetch_add(1, Ordering::Relaxed)
            )
        };
        let host = self.clone();
        let extension_key = extension_id.to_string();
        let tabs = cx.global::<GlobalTabContainer>().primary_pane();
        tabs.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy(
                tab_id.clone(),
                move |window, cx| {
                    let registry = host.clone();
                    let view = ShellPluginTab::load(
                        ShellPluginLoad {
                            host,
                            contribution,
                            connection: None,
                            title_override: None,
                        },
                        window,
                        cx,
                    );
                    registry.register_tab(extension_key, view.downgrade());
                    TabItem::new(tab_id, format!("shell:{extension_id}"), view)
                },
                window,
                cx,
            )
        });
    }

    fn close_extension(
        &self,
        extension_id: &str,
        _: &mut Window,
        cx: &mut App,
    ) -> gpui::Task<bool> {
        self.service.begin_extension_retire(extension_id);
        self.retiring.borrow_mut().insert(extension_id.to_string());
        let tasks = self
            .tabs
            .borrow()
            .get(extension_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tab| match tab {
                TrackedPluginTab::Shell { tab, .. } => tab
                    .upgrade()
                    .map(|tab| tab.update(cx, |tab, cx| tab.close_for_extension(cx))),
                TrackedPluginTab::Headless { tab, .. } => tab
                    .upgrade()
                    .map(|tab| tab.update(cx, |tab, cx| tab.close_for_extension(cx))),
            })
            .collect::<Vec<_>>();
        let service = self.service.clone();
        let extension_id = extension_id.to_string();
        let retiring = Rc::clone(&self.retiring);
        cx.spawn(async move |cx| {
            for task in tasks {
                if !task.await {
                    return finish_failed(&service, &retiring, &extension_id);
                }
            }
            let stop = one_core::gpui_tokio::Tokio::spawn_result(cx, {
                let service = service.clone();
                let extension_id = extension_id.clone();
                async move {
                    service.deactivate_extension(&extension_id).await;
                    Ok(())
                }
            });
            stop.await.is_ok() || finish_failed(&service, &retiring, &extension_id)
        })
    }

    fn finish_extension_change(&self, extension_id: &str) {
        self.retiring.borrow_mut().remove(extension_id);
        self.service.finish_extension_retire(extension_id);
    }
}

fn finish_failed(
    service: &crate::universal_plugins::UniversalPluginService,
    retiring: &Rc<std::cell::RefCell<std::collections::HashSet<String>>>,
    extension_id: &str,
) -> bool {
    retiring.borrow_mut().remove(extension_id);
    service.finish_extension_retire(extension_id);
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn singleton_open_delegates_reactivation_to_tab_container() {
        let source = include_str!("opener.rs");
        let obsolete_guard = ["contribution.singleton", "self.has_open_view"].join(" && ");
        assert!(!source.contains(&obsolete_guard));
        assert!(source.contains("tabs.activate_or_add_tab_lazy("));
    }
}
