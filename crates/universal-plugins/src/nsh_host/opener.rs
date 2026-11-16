//! `ShellAppOpener` 的 nsh 实现：扩展页面从"gpui-shell 脚本视图"换成
//! "nsh 程序"，但对上层的契约不变（`extension_view` 只认这个 trait）。

use gpui::{App, Task, Window};

use super::{NshHost, Tracked};

impl extension_view::ShellAppOpener for NshHost {
    fn open(&self, extension_id: &str, route: Option<&str>, window: &mut Window, cx: &mut App) {
        // 正在退役的扩展不再接受新页面：它马上就要被卸载了。
        if self.retiring().contains(extension_id) {
            return;
        }
        let Some(contribution) = self.contribution(extension_id) else {
            tracing::warn!(extension_id, "扩展目录里没有这个 shell app，忽略打开请求");
            return;
        };
        super::open_view(self.clone(), contribution, route, None, window, cx);
    }

    fn open_with_context(
        &self,
        extension_id: &str,
        route: Option<&str>,
        context: serde_json::Value,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.retiring().contains(extension_id) {
            return;
        }
        let Some(contribution) = self.contribution(extension_id) else {
            tracing::warn!(extension_id, "扩展目录里没有这个 shell app，忽略打开请求");
            return;
        };
        super::open_view(self.clone(), contribution, route, Some(context), window, cx);
    }

    fn open_in_container(
        &self,
        extension_id: &str,
        route: Option<&str>,
        context: serde_json::Value,
        container: gpui::Entity<one_core::tab_container::TabContainer>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.retiring().contains(extension_id) {
            return;
        }
        let Some(contribution) = self.contribution(extension_id) else {
            tracing::warn!(extension_id, "扩展目录里没有这个 shell app，忽略打开请求");
            return;
        };
        super::open_view_in(
            self.clone(),
            contribution,
            route,
            Some(context),
            container,
            window,
            cx,
        );
    }

    /// 扩展被停用/卸载：关掉它的所有页面（tab 与面板），
    /// 等它们都关干净，再让能力平面卸掉这个扩展。
    fn close_extension(&self, extension_id: &str, _: &mut Window, cx: &mut App) -> Task<bool> {
        self.service().begin_extension_retire(extension_id);
        self.retiring_mut().insert(extension_id.to_string());
        let mut tasks = self.close_extension_tabs(extension_id, cx);
        tasks.extend(self.close_extension_panels(extension_id, cx));
        let service = self.service().clone();
        let extension_id = extension_id.to_string();
        let host = self.clone();
        cx.spawn(async move |cx| {
            for task in tasks {
                if !task.await {
                    return host.finish_failed_retire(&service, &extension_id);
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
            stop.await.is_ok() || host.finish_failed_retire(&service, &extension_id)
        })
    }

    fn finish_extension_change(&self, extension_id: &str) {
        self.retiring_mut().remove(extension_id);
        self.service().finish_extension_retire(extension_id);
    }
}

impl NshHost {
    /// 关掉一个扩展名下所有页面，返回每个页面的关闭任务。
    fn close_extension_tabs(&self, extension_id: &str, cx: &mut App) -> Vec<Task<bool>> {
        let tasks = self
            .open_tabs(extension_id)
            .into_iter()
            .map(|tab| {
                let (route, extension_name) = {
                    let tab = tab.read(cx);
                    (tab.route().cloned(), tab.extension_id().clone())
                };
                tracing::debug!(
                    extension_id,
                    route = route.as_deref().unwrap_or("<default>"),
                    "扩展停用：关闭 nsh app 页（所属 {extension_name}）"
                );
                tab.update(cx, |tab, cx| tab.close_for_extension(cx))
            })
            .collect::<Vec<_>>();
        self.forget(extension_id);
        tasks
    }

    /// 关掉一个扩展名下所有面板，返回每个面板的关闭任务。
    ///
    /// 面板实体由宿主锚点（终端侧栏）持有；`close_for_extension` 会让内部
    /// 页面发 `CloseRequested`，宿主锚点订阅后 drop 实体。这里先把弱引用
    /// 清出 monitor 记账，避免 provider 事件继续转发给半关的面板。
    fn close_extension_panels(&self, extension_id: &str, cx: &mut App) -> Vec<Task<bool>> {
        let panels = {
            let mut tracked = self.tracked.borrow_mut();
            let Some(entries) = tracked.get_mut(extension_id) else {
                return Vec::new();
            };
            let panels = entries
                .iter()
                .filter_map(|entry| match entry {
                    Tracked::Panel(panel) => panel.upgrade(),
                    Tracked::Page(_) => None,
                })
                .collect::<Vec<_>>();
            entries.retain(|entry| {
                !matches!(entry, Tracked::Panel(panel) if panel.upgrade().is_none())
            });
            entries.retain(|entry| !matches!(entry, Tracked::Panel(_)));
            panels
        };
        panels
            .into_iter()
            .map(|panel| {
                tracing::debug!(extension_id, "扩展停用：关闭 nsh 面板");
                panel.update(cx, |panel, cx| panel.close_for_extension(cx))
            })
            .collect()
    }

    /// 关闭失败也要把扩展从"退役中"状态放出来，否则它会永远打不开。
    fn finish_failed_retire(
        &self,
        service: &crate::UniversalPluginService,
        extension_id: &str,
    ) -> bool {
        self.retiring_mut().remove(extension_id);
        service.finish_extension_retire(extension_id);
        false
    }
}
