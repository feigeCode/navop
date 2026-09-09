//! Shell borrowed-mount 实现(feature = "shell-plugins")。
//!
//! 实现契约:`mount` 只装载 view 并借用会话,绝不执行
//! resource/open 或复用 `ShellPluginHost::open_connection`。

use std::rc::Rc;

use gpui::App;

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
        Ok(view)
    }
}

impl CustomPageHost for ShellPageHostAdapter {
    fn mount(
        &self,
        request: ShellPageMountRequest,
        _cx: &mut App,
    ) -> Result<ShellPageMount, ShellMountError> {
        // 只做声明校验;真正的 JS view 装载在 P3 借用 mount 管线中接通。
        let _view = self.ensure_embeddable(&request.extension_id, &request.view_id)?;
        Err(ShellMountError::MountFailed(format!(
            "shell page mount for `{}` requires the borrowed-mount pipeline",
            request.view_id
        )))
    }

    fn dispose(&self, _mount: ShellPageMount, _cx: &mut App) {
        // 借用 mount 尚无运行时资源;dispose 语义由 mount 管线补齐。
    }
}

/// 注入全局 CustomPageHost(仅在 shell-plugins 构建调用)。
pub fn install_custom_page_host(host: ShellPluginHost, cx: &mut App) {
    let adapter = Rc::new(ShellPageHostAdapter::new(host));
    cx.set_global(resource_view::GlobalCustomPageHost { host: adapter });
}
