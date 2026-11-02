//! `ShellViewOpener` 的 nsh 实现：扩展页面从"gpui-shell 脚本视图"换成
//! "nsh 程序"，但对上层的契约不变（`extension_view` 只认这个 trait）。

use gpui::{App, Task, Window};

use super::NshHost;

impl extension_view::ShellViewOpener for NshHost {
    fn open(&self, extension_id: &str, view_id: &str, window: &mut Window, cx: &mut App) {
        let Some(contribution) = self.contribution(extension_id, view_id) else {
            tracing::warn!(
                extension_id,
                view_id,
                "扩展目录里没有这个 shell view，忽略打开请求"
            );
            return;
        };
        super::open_view(self.clone(), contribution, window, cx);
    }

    fn close_extension(&self, extension_id: &str, _: &mut Window, cx: &mut App) -> Task<bool> {
        // 每个 tab 关掉时它自己的 `Entity<AppView>` 被 drop，`AppView` 的
        // `Drop` 会停掉引擎线程；这里只清掉登记表里的条目。
        let open = self.open_tabs(extension_id);
        for tab in &open {
            let tab = tab.read(cx);
            tracing::debug!(
                extension_id = %tab.extension_id(),
                view_id = %tab.view_id(),
                "扩展停用后仍有页面开着"
            );
        }
        if !open.is_empty() {
            tracing::debug!(
                extension_id,
                open_views = open.len(),
                "扩展停用后仍有页面开着，等它们自己关掉"
            );
        }
        self.forget(extension_id);
        Task::ready(true)
    }

    fn finish_extension_change(&self, _extension_id: &str) {}
}
