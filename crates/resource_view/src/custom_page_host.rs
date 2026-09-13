//! Shell 页面挂载契约:资源工作台向可选 Shell 渲染器暴露的嵌入接口。
//!
//! 接口由 resource_view 定义、universal-plugins(feature = "shell-plugins")
//! 实现。核心约束:`mount` 只借用页面 scope,不打开主连接、不复用旧
//! `ShellPluginHost::open_connection` 路径;dispose 只回收本 mount 的
//! 订阅与子资源。

use extension_plugin_adapter::ResourceSessionHandle;
use gpui::{AnyView, App, WeakEntity, Window};

use crate::NativeResourceWorkbench;

/// 页面级 Shell 覆盖的输入。
pub struct ShellPageMountRequest {
    /// 目标扩展 id(view 必须在该扩展的 shellViews 中声明)。
    pub extension_id: String,
    /// 要挂载的 shell view contribution id(同扩展 shellViews 中声明)。
    pub view_id: String,
    /// 页面 route/selection 快照,借给 Shell 页面做初始上下文。
    pub page_context: serde_json::Value,
    /// Workbench resource type exposed in `navop.context`.
    pub resource_type: String,
    /// Borrowed primary session. The Shell host must not close it.
    pub session: Option<ResourceSessionHandle>,
    /// Installed workbench descriptor pinned to this connection session.
    pub workbench: extension_runtime::RegisteredResourceWorkbenchContribution,
}

/// 挂载产物:可嵌入的视图。
pub struct ShellPageMount {
    pub view: AnyView,
    dispose: Option<Box<dyn FnOnce(&mut App)>>,
}

impl ShellPageMount {
    pub fn new(view: AnyView, dispose: impl FnOnce(&mut App) + 'static) -> Self {
        Self {
            view,
            dispose: Some(Box::new(dispose)),
        }
    }

    pub fn dispose(mut self, cx: &mut App) {
        if let Some(dispose) = self.dispose.take() {
            dispose(cx);
        }
    }
}

/// Shell 借用挂载宿主。实现方持有真实 gpui-shell 运行时。
/// GPUI 应用是单线程 UI 模型,这里不要求 Send + Sync。
pub trait CustomPageHost {
    /// 挂载一个 shell 页面视图。失败时宿主工作台按 manifest fallback
    /// 声明降级到原生模板。本接口不执行 resource.open。
    fn mount(
        &self,
        request: ShellPageMountRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<ShellPageMount, ShellMountError>;

    /// 处置一个挂载:只回收该 mount 的订阅和子资源,
    /// 不触碰连接主会话与 provider activation。
    fn dispose(&self, mount: ShellPageMount, cx: &mut App);
}

#[derive(Debug, thiserror::Error)]
pub enum ShellMountError {
    #[error("shell view `{0}` was not found or is not embeddable")]
    ViewUnavailable(String),
    #[error("shell module `{0}` is required by this view but not available in this build")]
    ModuleUnavailable(String),
    #[error("shell mount failed: {0}")]
    MountFailed(String),
}

/// 工作台页面 renderer 选择:manifest 声明 + 运行时可用性的合成结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRenderer {
    Native,
    Shell { view_id: String },
}

/// 依据 manifest renderer 声明和 host 可用性决定页面 renderer。
/// 规则:
/// - native 声明 → Native;
/// - shell 声明 + host 存在 → Shell(实际装载失败再走页面级降级);
/// - shell 声明 + host 不存在 + fallback=native → Native(构建级降级);
/// - shell-only → Shell,页面渲染“此构建不可用”说明。
pub fn resolve_renderer(
    declared: &extension_runtime::extension::manifest::ResourceWorkbenchRenderer,
    host_available: bool,
) -> PageRenderer {
    use extension_runtime::extension::manifest::ResourceWorkbenchRendererKind;
    match declared.kind {
        ResourceWorkbenchRendererKind::Native => PageRenderer::Native,
        ResourceWorkbenchRendererKind::Shell => {
            let view_id = declared.view_id.clone().unwrap_or_default();
            if host_available {
                PageRenderer::Shell { view_id }
            } else if declared.fallback.as_deref() == Some("native") {
                PageRenderer::Native
            } else {
                PageRenderer::Shell { view_id }
            }
        }
    }
}

/// 供宿主 tab 注入的 Shell host 全局。
pub struct GlobalCustomPageHost {
    pub host: std::rc::Rc<dyn CustomPageHost>,
}

impl gpui::Global for GlobalCustomPageHost {}

/// 读取当前注入的 Shell host(无 shell-plugins 构建为 None)。
pub fn custom_page_host(cx: &App) -> Option<std::rc::Rc<dyn CustomPageHost>> {
    cx.try_global::<GlobalCustomPageHost>()
        .map(|global| global.host.clone())
}

/// 工作台弱引用 + 页面 id:Shell mount dispose 时回调宿主清理。
pub struct MountHandle {
    pub workbench: WeakEntity<NativeResourceWorkbench>,
    pub page_id: String,
}

impl MountHandle {
    pub fn new(workbench: WeakEntity<NativeResourceWorkbench>, page_id: impl Into<String>) -> Self {
        Self {
            workbench,
            page_id: page_id.into(),
        }
    }
}

/// 页面切换前检查 dirty 状态的守卫结果。
pub enum DirtyGuardDecision {
    Proceed,
    Cancel,
}

/// 请求宿主确认未保存内容(首版:无编辑器页面直接 Proceed)。
pub fn confirm_dirty(_mount: &MountHandle) -> DirtyGuardDecision {
    DirtyGuardDecision::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::extension::manifest::{
        ResourceWorkbenchRenderer, ResourceWorkbenchRendererKind,
    };

    fn renderer(
        kind: ResourceWorkbenchRendererKind,
        fallback: Option<&str>,
    ) -> ResourceWorkbenchRenderer {
        ResourceWorkbenchRenderer {
            kind,
            view_id: Some("search-editor".into()),
            fallback: fallback.map(Into::into),
        }
    }

    #[test]
    fn native_declaration_always_uses_native() {
        assert_eq!(
            PageRenderer::Native,
            resolve_renderer(
                &renderer(ResourceWorkbenchRendererKind::Native, None),
                false
            )
        );
    }

    #[test]
    fn shell_without_host_falls_back_to_native_when_declared() {
        assert_eq!(
            PageRenderer::Native,
            resolve_renderer(
                &renderer(ResourceWorkbenchRendererKind::Shell, Some("native")),
                false
            )
        );
    }

    #[test]
    fn shell_only_without_host_stays_shell_for_unavailable_notice() {
        assert_eq!(
            PageRenderer::Shell {
                view_id: "search-editor".into()
            },
            resolve_renderer(&renderer(ResourceWorkbenchRendererKind::Shell, None), false)
        );
    }

    #[test]
    fn shell_with_host_uses_shell() {
        assert_eq!(
            PageRenderer::Shell {
                view_id: "search-editor".into()
            },
            resolve_renderer(&renderer(ResourceWorkbenchRendererKind::Shell, None), true)
        );
    }
}
