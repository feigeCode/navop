use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{App, AppContext as _, http_client::HttpClient};

use crate::{
    ExtensionKind, ExtensionManagerMode, ExtensionSummary, MarketplaceEntry,
    MarketplaceInstallOutcome,
};

struct EmptyHost;

impl crate::ExtensionViewHost for EmptyHost {
    fn list_installed(&self) -> anyhow::Result<Vec<ExtensionSummary>> {
        Ok(Vec::new())
    }

    fn load_marketplace_entries(
        &self,
        _http_client: Arc<dyn HttpClient>,
    ) -> BoxFuture<'static, anyhow::Result<Vec<MarketplaceEntry>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn load_marketplace_entries_from_url(
        &self,
        _http_client: Arc<dyn HttpClient>,
        _manifest_url: String,
    ) -> BoxFuture<'static, anyhow::Result<Vec<MarketplaceEntry>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn review_marketplace_entry(
        &self,
        _http_client: Arc<dyn HttpClient>,
        entry: MarketplaceEntry,
    ) -> BoxFuture<'static, anyhow::Result<MarketplaceInstallOutcome>> {
        Box::pin(async move { Err(anyhow::anyhow!("测试不安装扩展：{}", entry.id)) })
    }

    fn review_local_tarball(&self, _path: PathBuf) -> anyhow::Result<MarketplaceInstallOutcome> {
        Err(anyhow::anyhow!("测试不安装扩展"))
    }

    fn install_confirmed_staging(&self, _staging: PathBuf) -> anyhow::Result<ExtensionSummary> {
        Err(anyhow::anyhow!("测试不安装扩展"))
    }

    fn uninstall(&self, _summary: &ExtensionSummary) -> anyhow::Result<String> {
        Ok(String::new())
    }

    fn reload(
        &self,
        _summary: &ExtensionSummary,
        _cx: &mut App,
    ) -> anyhow::Result<Vec<ExtensionSummary>> {
        Ok(Vec::new())
    }

    fn refresh_after_extension_change(&self, _kind: ExtensionKind, _cx: &mut App) {}
}

#[gpui::test]
fn plus_entry_opens_the_marketplace_filtered_to_composite(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        let window = cx
            .open_window(gpui::WindowOptions::default(), |_window, cx| {
                cx.new(|cx| crate::ExtensionManagerView::new(Arc::new(EmptyHost), _window, cx))
            })
            .expect("open window");
        window
            .update(cx, |view, window, cx| {
                view.show_composite_marketplace(window, cx);
                assert_eq!(ExtensionManagerMode::Marketplace, view.mode);
                assert_eq!(Some(ExtensionKind::Composite), view.selected_kind);
                assert!(!view.updates_only, "侧栏入口不能继承「有更新」过滤");
            })
            .expect("apply composite filter");
    });
}
