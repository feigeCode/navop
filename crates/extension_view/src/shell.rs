use std::rc::Rc;

use gpui::{App, Global, Task, Window};

pub trait ShellViewOpener {
    fn open(&self, extension_id: &str, view_id: &str, window: &mut Window, cx: &mut App);
    fn close_extension(&self, extension_id: &str, window: &mut Window, cx: &mut App) -> Task<bool>;
    fn finish_extension_change(&self, extension_id: &str);
}

pub(crate) fn finish_shell_extension(extension_id: &str, cx: &App) {
    if let Some(global) = cx.try_global::<GlobalShellViewOpener>() {
        global.opener.finish_extension_change(extension_id);
    }
}

pub(crate) fn close_shell_extension(
    extension_id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Task<bool> {
    let Some(global) = cx.try_global::<GlobalShellViewOpener>() else {
        return Task::ready(true);
    };
    let opener = Rc::clone(&global.opener);
    opener.close_extension(extension_id, window, cx)
}

pub struct GlobalShellViewOpener {
    opener: Rc<dyn ShellViewOpener>,
}

impl Global for GlobalShellViewOpener {}

pub fn register_shell_view_opener(opener: Rc<dyn ShellViewOpener>, cx: &mut App) {
    cx.set_global(GlobalShellViewOpener { opener });
}

pub(crate) fn open_shell_view(
    extension_id: &str,
    view_id: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(global) = cx.try_global::<GlobalShellViewOpener>() else {
        tracing::warn!(extension_id, view_id, "shell view opener is not registered");
        return;
    };
    let opener = Rc::clone(&global.opener);
    opener.open(extension_id, view_id, window, cx);
}
