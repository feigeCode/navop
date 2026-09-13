#[cfg(not(test))]
use std::rc::Rc;

#[cfg(not(test))]
use extension_plugin_adapter::RuntimeMonitorEvent;
#[cfg(not(test))]
use gpui::App;

use super::ShellPluginHost;
#[cfg(not(test))]
use super::TrackedPluginTab;

impl ShellPluginHost {
    #[cfg(not(test))]
    pub(crate) fn start_monitor_bridge(&self, cx: &mut App) {
        // 桥接必须转发完整事件:tab 需要依据 generation / session 关闭状态
        // 判断是"瞬时抖动"还是"provider 已换进程",仅传 runtime id 会丢失语义。
        let (sender, receiver) = smol::channel::bounded::<RuntimeMonitorEvent>(32);
        let mut events = self.service.subscribe();
        self.tokio.spawn(async move {
            while let Ok(event) = events.recv().await {
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        let tabs = Rc::clone(&self.tabs);
        cx.spawn(async move |cx| {
            while let Ok(event) = receiver.recv().await {
                let runtime_id = event.runtime_id().to_owned();
                let tracked = tabs
                    .borrow()
                    .values()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>();
                for tab in tracked {
                    match tab {
                        TrackedPluginTab::Shell { tab } => {
                            let _ = tab.update(cx, |tab, cx| tab.runtime_changed(&event, cx));
                        }
                        TrackedPluginTab::Headless {
                            runtime_id: tracked,
                            tab,
                        } if tracked == runtime_id => {
                            let _ = tab.update(cx, |tab, cx| tab.runtime_changed(&event, cx));
                        }
                        TrackedPluginTab::Headless { .. } => {}
                    }
                }
            }
        })
        .detach();
    }
}
