//! Shell borrowed-mount 实现(feature = "shell-plugins")。
//!
//! 实现契约:`mount` 只装载 view 并借用会话,绝不执行
//! resource/open 或复用 `ShellPluginHost::open_connection`。

use std::rc::Rc;

use gpui::{App, Window};

use resource_view::{CustomPageHost, ShellMountError, ShellPageMount, ShellPageMountRequest};

use crate::shell_plugin_host::ShellPluginHost;

/// 把现有 ShellPluginHost 适配为 CustomPageHost。
///
/// 页面级嵌入:只装载 shell view 的 JS 入口并暴露 AnyView;
/// 生命周期(host module、session 借用)由 mount handle 管理。
pub struct ShellPageHostAdapter {
    host: ShellPluginHost,
}

impl ShellPageHostAdapter {
    pub fn new(host: ShellPluginHost) -> Self {
        Self { host }
    }

    /// 检查 view 是否存在且声明了工作台所需模块。
    fn ensure_embeddable(
        &self,
        extension_id: &str,
        view_id: &str,
    ) -> Result<extension_runtime::RegisteredShellViewContribution, ShellMountError> {
        let view = self
            .host
            .contribution(extension_id, view_id)
            .ok_or_else(|| ShellMountError::ViewUnavailable(view_id.to_string()))?;
        // 嵌入页面必须声明 context 模块才能收到页面上下文。
        if !view
            .modules
            .contains(&extension_runtime::extension::manifest::ShellHostModule::Context)
        {
            return Err(ShellMountError::ModuleUnavailable("context".into()));
        }
        if !view
            .modules
            .contains(&extension_runtime::extension::manifest::ShellHostModule::Workbench)
        {
            return Err(ShellMountError::ModuleUnavailable("workbench".into()));
        }
        if view.modules.iter().any(|module| {
            matches!(
                module,
                extension_runtime::extension::manifest::ShellHostModule::Resource
                    | extension_runtime::extension::manifest::ShellHostModule::Job
                    | extension_runtime::extension::manifest::ShellHostModule::Event
                    | extension_runtime::extension::manifest::ShellHostModule::Blob
                    | extension_runtime::extension::manifest::ShellHostModule::Runtime
                    | extension_runtime::extension::manifest::ShellHostModule::Dev
            )
        }) {
            return Err(ShellMountError::ModuleUnavailable(
                "raw resource/job/event/blob/runtime/dev modules are forbidden for embedded pages"
                    .into(),
            ));
        }
        Ok(view)
    }
}

impl CustomPageHost for ShellPageHostAdapter {
    fn mount(
        &self,
        request: ShellPageMountRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<ShellPageMount, ShellMountError> {
        let view = self.ensure_embeddable(&request.extension_id, &request.view_id)?;
        let session_handle = request
            .session
            .ok_or_else(|| ShellMountError::MountFailed("borrowed session is required".into()))?;
        let workbench = request.workbench;
        let page_context = request.page_context;
        let snapshot = session_handle
            .resource_snapshot()
            .map_err(|error| ShellMountError::MountFailed(error.to_string()))?;
        let alias = view
            .backends
            .iter()
            .find(|(_, runtime_id)| *runtime_id == &session_handle.identity().runtime_id)
            .map(|(alias, _)| alias.clone())
            .ok_or_else(|| {
                ShellMountError::MountFailed(
                    "shell view has no backend for the borrowed session runtime".into(),
                )
            })?;
        let session = self.host.new_mount_session(view.backends.clone());
        let resource = session
            .register_borrowed_resource(alias, session_handle.managed_client(), snapshot)
            .map_err(|error| ShellMountError::MountFailed(error.to_string()))?;
        let connection = crate::shell_plugin_host::ShellConnectionContext {
            connection_id: 0,
            name: "borrowed connection".into(),
            contribution_id: request.view_id.clone(),
            resource_type: request.resource_type.clone(),
            resource,
        };
        let loaded = crate::shell_plugin_host::load_borrowed(
            &self.host,
            view,
            session,
            Some(connection),
            workbench,
            session_handle,
            page_context,
            window,
            cx,
        )
        .map_err(|error| ShellMountError::MountFailed(error.error.to_string()))?;
        let view = loaded.view().clone().into();
        let mut loaded = Some(loaded);
        Ok(ShellPageMount::new(view, move |cx| {
            if let Some(mut loaded) = loaded.take() {
                loaded.unload(cx);
            }
        }))
    }

    fn dispose(&self, mount: ShellPageMount, cx: &mut App) {
        mount.dispose(cx);
    }
}

/// 注入全局 CustomPageHost(仅在 shell-plugins 构建调用)。
pub fn install_custom_page_host(host: ShellPluginHost, cx: &mut App) {
    let adapter = Rc::new(ShellPageHostAdapter::new(host));
    cx.set_global(resource_view::GlobalCustomPageHost { host: adapter });
}
