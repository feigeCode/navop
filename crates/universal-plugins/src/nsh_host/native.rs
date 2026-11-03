//! 原生组件注入（nsh 的 `nativeComponent`）。
//!
//! 页面是 TSX，凡是「必须是 Rust 实体」的东西 —— 终端、编辑器、图表面板 ——
//! 都只能以原生组件出现：TSX 里写 `<Terminal … />`，宿主在那一格里画自己的
//! GPUI 视图。
//!
//! 注入方向刻意做成**注册表在宿主、实现在宿主上层**：本 crate 不认识
//! `terminal_view`（扩展宿主不该依赖应用层 UI），由 `navop_app` 在启动时
//! 调 [`register_native_component`] 注册真正的实现。这样：
//!
//! - 每视图一份注册表（`AppOptions.components`），不是进程级全局；
//! - 组件工厂拿得到「这是哪个扩展的哪个页面」以及清单给的权限，
//!   需要起进程的组件自己看 [`NativeViewContext::allow_process`]；
//! - 没有注入时页面里写 `<Terminal/>` 会走 nsh 自己的「未知组件」路径，
//!   不会渲染成空白。

use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

use extension_runtime::RegisteredShellViewContribution;
use gpui::App;
use nsh_platform_gpui::{ComponentContext, Components, NativeComponent};

/// 建一个原生组件实例时的上下文。
pub struct NativeViewContext {
    /// 扩展 id（`extension.json` 的 `id`）。
    pub extension_id: Arc<str>,
    /// 页面 id（`contributes.shellViews[].id`）。
    pub view_id: Arc<str>,
    /// 这个页面的清单声明了 `shell:exec`：允许组件启动本地进程。
    pub allow_process: bool,
    /// nsh 给的组件上下文（重绘、键盘焦点都靠它）。
    pub component: ComponentContext,
}

/// 原生组件工厂。名字由页面用 `nativeComponent("<name>")` 引用。
pub type NativeFactory = Rc<dyn Fn(NativeViewContext) -> Box<dyn NativeComponent>>;

#[derive(Default)]
struct Registry {
    factories: RefCell<BTreeMap<Arc<str>, NativeFactory>>,
}

impl gpui::Global for Registry {}

/// 注册一个原生组件。应用启动时调用一次即可，重复注册以后者为准。
pub fn register_native_component(name: impl Into<Arc<str>>, factory: NativeFactory, cx: &mut App) {
    let name = name.into();
    let registry = cx.default_global::<Registry>();
    registry.factories.borrow_mut().insert(name, factory);
}

/// 清单是否允许这个页面起本地进程（终端之类）。
pub(crate) fn allows_process(contribution: &RegisteredShellViewContribution) -> bool {
    contribution
        .permissions
        .iter()
        .any(|permission| permission.trim() == "shell:exec")
}

/// 为一个页面拼出注册表：工厂包一层，把「哪个扩展/哪个页面/有没有执行权限」
/// 钉死在闭包里，页面自己改不了。
pub(crate) fn components_for(
    contribution: &RegisteredShellViewContribution,
    cx: &App,
) -> Components {
    let components = Components::default();
    let Some(registry) = cx.try_global::<Registry>() else {
        return components;
    };
    let extension_id: Arc<str> = contribution.extension_id.as_str().into();
    let view_id: Arc<str> = contribution.id.as_str().into();
    let allow_process = allows_process(contribution);
    let factories: Vec<(Arc<str>, NativeFactory)> = registry
        .factories
        .borrow()
        .iter()
        .map(|(name, factory)| (Arc::clone(name), Rc::clone(factory)))
        .collect();
    for (name, factory) in factories {
        let extension_id = Arc::clone(&extension_id);
        let view_id = Arc::clone(&view_id);
        // 每次 nsh 建这个组件时都会调这里：上下文里只带本页面的身份与权限。
        components.register(
            &name,
            Rc::new(move |component| {
                factory(NativeViewContext {
                    extension_id: Arc::clone(&extension_id),
                    view_id: Arc::clone(&view_id),
                    allow_process,
                    component,
                })
            }),
        );
    }
    components
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::allows_process;
    use extension_runtime::RegisteredShellViewContribution;

    fn view(permissions: &[&str]) -> RegisteredShellViewContribution {
        RegisteredShellViewContribution {
            extension_id: "dev.navop.native-test".into(),
            extension_version: "0.1.0".into(),
            id: "terminal".into(),
            view_key: "dev.navop.native-test::terminal".into(),
            title: "终端".into(),
            description: None,
            icon_path: None,
            entry_path: "ui/pages/terminal/dist/bundle.js".into(),
            extension_root: ".".into(),
            surface: extension_runtime::extension::manifest::ShellSurface::Tab,
            category: None,
            keywords: Vec::new(),
            singleton: false,
            backends: BTreeMap::new(),
            modules: BTreeSet::new(),
            permissions: permissions.iter().map(|p| (*p).to_string()).collect(),
            shell_api_version: "1.0".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[test]
    fn only_exec_permission_allows_processes() {
        assert!(allows_process(&view(&["shell:exec"])));
        assert!(allows_process(&view(&[
            "spawn:./bin/provider",
            "shell:exec"
        ])));
        assert!(!allows_process(&view(&["spawn:./bin/provider"])));
        assert!(!allows_process(&view(&[])));
    }
}
