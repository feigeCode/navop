#[cfg(not(test))]
use std::rc::Rc;

#[cfg(not(test))]
use gpui::App;

use super::ShellPluginHost;
#[cfg(not(test))]
use super::TrackedPluginTab;

impl ShellPluginHost {
    #[cfg(not(test))]
    pub(crate) fn start_monitor_bridge(&self, cx: &mut App) {
        let (sender, receiver) = smol::channel::bounded::<String>(32);
        let mut events = self.service.subscribe();
        self.tokio.spawn(async move {
            while let Ok(event) = events.recv().await {
                let runtime_id = match event {
                    extension_plugin_adapter::RuntimeMonitorEvent::HealthChanged {
                        runtime_id,
                        ..
                    }
                    | extension_plugin_adapter::RuntimeMonitorEvent::RuntimeRemoved {
                        runtime_id,
                    }
                    | extension_plugin_adapter::RuntimeMonitorEvent::CheckFailed {
                        runtime_id,
                        ..
                    } => runtime_id,
                };
                if sender.send(runtime_id).await.is_err() {
                    break;
                }
            }
        });
        let tabs = Rc::clone(&self.tabs);
        cx.spawn(async move |cx| {
            while let Ok(runtime_id) = receiver.recv().await {
                let tracked = tabs
                    .borrow()
                    .values()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>();
                for tab in tracked {
                    match tab {
                        TrackedPluginTab::Shell { tab, .. } => {
                            let _ = tab.update(cx, |tab, cx| tab.runtime_changed(&runtime_id, cx));
                        }
                        TrackedPluginTab::Headless {
                            runtime_id: tracked,
                            tab,
                        } if tracked == runtime_id => {
                            let _ = tab.update(cx, |tab, cx| tab.runtime_changed(&runtime_id, cx));
                        }
                        TrackedPluginTab::Headless { .. } => {}
                    }
                }
            }
        })
        .detach();
    }
}
