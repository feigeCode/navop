use std::rc::Rc;

use gpui::{App, Global, Task, Window};
use serde_json::Value;

/// 打开扩展的 nsh app。
///
/// `route` 是插件的初始路由（如 `containers`）；`None` 表示让 app 用自己的
/// 默认路由。插件内部的后续导航完全由 app 自己负责，宿主不参与。
pub trait ShellAppOpener {
    fn open(&self, extension_id: &str, route: Option<&str>, window: &mut Window, cx: &mut App);
    /// 打开扩展 app 并携带宿主侧启动上下文（如 DB 树右键菜单的节点信息）。
    /// 上下文会注入页面 `host.params.context`。默认实现丢弃上下文、退化为普通 open。
    fn open_with_context(
        &self,
        extension_id: &str,
        route: Option<&str>,
        _context: Value,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.open(extension_id, route, window, cx);
    }
    /// 打开扩展 app 到指定 tab 容器（数据库标签页自己的页签栏），
    /// 而不是主窗口的页签栏。默认退化为带上下文的普通打开。
    fn open_in_container(
        &self,
        extension_id: &str,
        route: Option<&str>,
        context: Value,
        _container: gpui::Entity<one_core::tab_container::TabContainer>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.open_with_context(extension_id, route, context, window, cx);
    }
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

/// 打开扩展的 nsh app，并携带宿主侧启动上下文（注入 `host.params.context`）。
pub fn open_shell_app_with_context(
    extension_id: &str,
    route: Option<&str>,
    context: Value,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(global) = cx.try_global::<GlobalShellAppOpener>() else {
        tracing::warn!(extension_id, "shell app opener is not registered");
        return;
    };
    let opener = Rc::clone(&global.opener);
    opener.open_with_context(extension_id, route, context, window, cx);
}

/// 打开扩展的 nsh app 到指定 tab 容器（数据库标签页用自己的页签栏承载扩展页）。
pub fn open_shell_app_in_container(
    extension_id: &str,
    route: Option<&str>,
    context: Value,
    container: gpui::Entity<one_core::tab_container::TabContainer>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(global) = cx.try_global::<GlobalShellAppOpener>() else {
        tracing::warn!(extension_id, "shell app opener is not registered");
        return;
    };
    let opener = Rc::clone(&global.opener);
    opener.open_in_container(extension_id, route, context, container, window, cx);
}
