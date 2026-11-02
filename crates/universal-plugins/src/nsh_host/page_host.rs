//! 资源工作台嵌入页（`resource_view::CustomPageHost` 的 nsh 实现）。
//!
//! 与 tab 路径的差别只有一处：页面用的是**别人已经开好的连接**。所以
//! 主资源是借来的（`owned: false`，页面关掉不调 provider 的 close），
//! 数据要么走 `navop.workbench.dispatch`，要么用借来的句柄 invoke。
//!
//! 黑名单与 gpui-shell 版逐条对齐（见 `ensure_embeddable`）：嵌入页碰不到
//! 原始 resource/job/event/blob/runtime/dev 这些能自己开资源的模块。

use std::sync::Arc;

use gpui::{App, AppContext as _, Window};
use nsh_engine::HostChannel;
use resource_view::{CustomPageHost, ShellMountError, ShellPageMount, ShellPageMountRequest};

use super::{
    NshHost,
    abi::{self, HostAbi},
    bundle,
    session::NshSession,
    theme,
};
use extension_runtime::extension::manifest::ShellHostModule;

/// 嵌入页只允许这几个模块：读宿主事实（语言/主题/上下文）+ 工作台操作。
fn ensure_embeddable(
    view: &extension_runtime::RegisteredShellViewContribution,
) -> Result<(), ShellMountError> {
    if !view.modules.contains(&ShellHostModule::Context) {
        return Err(ShellMountError::ModuleUnavailable("context".into()));
    }
    if !view.modules.contains(&ShellHostModule::Workbench) {
        return Err(ShellMountError::ModuleUnavailable("workbench".into()));
    }
    // 黑名单而非白名单：`i18n` / `theme` 读的是宿主自己的两个事实，
    // 嵌入页本来就该能用 —— 别"顺手补上"。
    if view.modules.iter().any(|module| {
        matches!(
            module,
            ShellHostModule::Resource
                | ShellHostModule::Job
                | ShellHostModule::Event
                | ShellHostModule::Blob
                | ShellHostModule::Runtime
                | ShellHostModule::Dev
        )
    }) {
        return Err(ShellMountError::ModuleUnavailable(
            "raw resource/job/event/blob/runtime/dev modules are forbidden for embedded pages"
                .into(),
        ));
    }
    Ok(())
}

impl CustomPageHost for NshHost {
    fn mount(
        &self,
        request: ShellPageMountRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<ShellPageMount, ShellMountError> {
        let contribution = self
            .contribution(&request.extension_id, &request.view_id)
            .ok_or_else(|| ShellMountError::ViewUnavailable(request.view_id.clone()))?;
        ensure_embeddable(&contribution)?;
        let handle = request
            .session
            .ok_or_else(|| ShellMountError::MountFailed("嵌入页需要一个借来的会话".into()))?;
        let snapshot = handle
            .resource_snapshot()
            .map_err(|error| ShellMountError::MountFailed(error.to_string()))?;
        // 借来的会话必须有一个别名指向它的 runtime，否则页面根本不知道
        // 该把调用发给谁 —— 这是清单写错，不是运行时故障。
        let alias = contribution
            .backends
            .iter()
            .find(|(_, runtime_id)| *runtime_id == &handle.identity().runtime_id)
            .map(|(alias, _)| alias.clone())
            .ok_or_else(|| {
                ShellMountError::MountFailed(
                    "shell view has no backend for the borrowed session runtime".into(),
                )
            })?;
        let program = bundle::load(&contribution, self.trust())
            .map_err(|error| ShellMountError::MountFailed(format!("{error:#}")))?;
        let session = Arc::new(NshSession::new(
            self.service.clone(),
            contribution.backends.clone(),
            one_core::gpui_tokio::Tokio::handle(cx),
        ));
        session
            .register_borrowed_resource(handle.managed_client(), snapshot)
            .map_err(|error| ShellMountError::MountFailed(error.message.clone()))?;
        session.set_workbench(request.workbench, handle, request.page_context);
        tracing::debug!(extension_id = %contribution.extension_id, alias, "嵌入 nsh 页面");

        let host_abi = HostAbi::new(&contribution);
        let channel = HostChannel::default();
        abi::register(
            &channel,
            Arc::clone(&host_abi),
            Arc::clone(&session),
            &self.dispatcher(),
        );
        let options = nsh_platform_gpui::AppOptions {
            app_id: program.app_id.clone(),
            asset_base: program.asset_base.clone(),
            host: channel,
            ..Default::default()
        };
        let view = cx.new(|cx| nsh_platform_gpui::AppView::new(options, window, cx));
        theme::push(&view, cx);
        host_abi.set_theme(theme::snapshot(cx));
        view.update(cx, |view, _| view.load(program.app));

        let dispose_abi = Arc::clone(&host_abi);
        let dispose_session = Arc::clone(&session);
        Ok(ShellPageMount::new(view.into(), move |_cx| {
            dispose_abi.cancel();
            dispose_session.cancel();
        }))
    }

    fn dispose(&self, mount: ShellPageMount, cx: &mut App) {
        mount.dispose(cx);
    }
}

/// 注入全局 CustomPageHost（nsh 宿主接管时调用）。
pub(crate) fn install(host: NshHost, cx: &mut App) {
    cx.set_global(resource_view::GlobalCustomPageHost {
        host: std::rc::Rc::new(host),
    });
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use extension_runtime::RegisteredShellViewContribution;
    use extension_runtime::extension::manifest::ShellHostModule;

    use super::ensure_embeddable;

    fn view(modules: &[ShellHostModule]) -> RegisteredShellViewContribution {
        RegisteredShellViewContribution {
            extension_id: "dev.navop.embed-test".into(),
            extension_version: "0.1.0".into(),
            id: "workbench-page".into(),
            view_key: "dev.navop.embed-test::workbench-page".into(),
            title: "工作台页".into(),
            description: None,
            icon_path: None,
            entry_path: PathBuf::from("ui/page.js"),
            extension_root: PathBuf::from("."),
            surface: extension_runtime::extension::manifest::ShellSurface::Tab,
            category: None,
            keywords: Vec::new(),
            singleton: false,
            backends: BTreeMap::new(),
            modules: modules.iter().copied().collect::<BTreeSet<_>>(),
            permissions: Vec::new(),
            shell_api_version: "1.0".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[test]
    fn embedded_pages_require_context_and_workbench() {
        // 只声明 workbench、没声明 context：页面拿不到 route/selection，
        // 开了也是空白页，所以这里必须挡住。
        let error = ensure_embeddable(&view(&[ShellHostModule::Workbench])).unwrap_err();
        assert!(error.to_string().contains("context"), "{error}");

        let error = ensure_embeddable(&view(&[ShellHostModule::Context])).unwrap_err();
        assert!(error.to_string().contains("workbench"), "{error}");

        ensure_embeddable(&view(&[
            ShellHostModule::Context,
            ShellHostModule::Workbench,
            // 读宿主事实的两个模块与嵌入页并不冲突。
            ShellHostModule::I18n,
            ShellHostModule::Theme,
        ]))
        .expect("context + workbench + i18n/theme 是合法组合");
    }

    #[test]
    fn embedded_pages_cannot_open_their_own_resources() {
        for forbidden in [
            ShellHostModule::Resource,
            ShellHostModule::Job,
            ShellHostModule::Event,
            ShellHostModule::Blob,
            ShellHostModule::Runtime,
            ShellHostModule::Dev,
        ] {
            let result = ensure_embeddable(&view(&[
                ShellHostModule::Context,
                ShellHostModule::Workbench,
                forbidden,
            ]));
            assert!(result.is_err(), "{forbidden:?} 必须被挡在嵌入页之外");
        }
    }
}
