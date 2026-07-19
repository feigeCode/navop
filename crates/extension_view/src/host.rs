use std::{path::PathBuf, sync::Arc};

use futures::future::BoxFuture;
use gpui::{App, http_client::HttpClient};

use crate::{ExtensionKind, ExtensionSummary, MarketplaceEntry, MarketplaceInstallOutcome};

pub trait ExtensionViewHost: Send + Sync {
    fn list_installed(&self) -> anyhow::Result<Vec<ExtensionSummary>>;

    fn load_marketplace_entries(
        &self,
        http_client: Arc<dyn HttpClient>,
    ) -> BoxFuture<'static, anyhow::Result<Vec<MarketplaceEntry>>>;

    fn load_marketplace_entries_from_url(
        &self,
        http_client: Arc<dyn HttpClient>,
        manifest_url: String,
    ) -> BoxFuture<'static, anyhow::Result<Vec<MarketplaceEntry>>>;

    fn review_marketplace_entry(
        &self,
        http_client: Arc<dyn HttpClient>,
        entry: MarketplaceEntry,
    ) -> BoxFuture<'static, anyhow::Result<MarketplaceInstallOutcome>>;

    fn review_local_tarball(&self, path: PathBuf) -> anyhow::Result<MarketplaceInstallOutcome>;

    fn install_confirmed_staging(&self, staging: PathBuf) -> anyhow::Result<ExtensionSummary>;

    fn uninstall(&self, summary: &ExtensionSummary) -> anyhow::Result<String>;

    fn reload(
        &self,
        summary: &ExtensionSummary,
        cx: &mut App,
    ) -> anyhow::Result<Vec<ExtensionSummary>>;

    fn refresh_after_extension_change(&self, kind: ExtensionKind, cx: &mut App);
}
