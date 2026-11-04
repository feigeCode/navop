use std::rc::Rc;

use gpui::{App, Global, Task, Window};

/// 打开扩展的 nsh app。
///
/// `route` 是插件的初始路由（如 `containers`）；`None` 表示让 app 用自己的
/// 默认路由。插件内部的后续导航完全由 app 自己负责，宿主不参与。
pub trait ShellAppOpener {
    fn open(&self, extension_id: &str, route: Option<&str>, window: &mut Window, cx: &mut App);
    fn close_extension(&self, extension_id: &str, window: &mut Window, cx: &mut App) -> Task<bool>;
    fn finish_extension_change(&self, extension_id: &str);
}

pub fn finish_shell_extension(extension_id: &str, cx: &App) {
    if let Some(global) = cx.try_global::<GlobalShellAppOpener>() {
        global.opener.finish_extension_change(extension_id);
    }
}

pub fn close_shell_extension(extension_id: &str, window: &mut Window, cx: &mut App) -> Task<bool> {
    let Some(global) = cx.try_global::<GlobalShellAppOpener>() else {
        return Task::ready(true);
    };
    let opener = Rc::clone(&global.opener);
    opener.close_extension(extension_id, window, cx)
}

pub struct GlobalShellAppOpener {
    opener: Rc<dyn ShellAppOpener>,
}

impl Global for GlobalShellAppOpener {}

pub fn register_shell_app_opener(opener: Rc<dyn ShellAppOpener>, cx: &mut App) {
    cx.set_global(GlobalShellAppOpener { opener });
}

pub fn open_shell_app(extension_id: &str, route: Option<&str>, window: &mut Window, cx: &mut App) {
    let Some(global) = cx.try_global::<GlobalShellAppOpener>() else {
        tracing::warn!(extension_id, "shell app opener is not registered");
        return;
    };
    let opener = Rc::clone(&global.opener);
    opener.open(extension_id, route, window, cx);
}
