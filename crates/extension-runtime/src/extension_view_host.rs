use std::{path::PathBuf, sync::Arc};

use futures::{FutureExt, future::BoxFuture};
use gpui::{App, http_client::HttpClient};

use crate::{
    extension as host_extension,
    extension::manifest::{build_permission_review, load_from_dir},
    extension_downloader as host_downloader,
    extension_package_layout::package_root,
};

#[derive(Clone, Default)]
pub struct MainExtensionViewHost;

impl extension_view::ExtensionViewHost for MainExtensionViewHost {
    fn list_installed(&self) -> anyhow::Result<Vec<extension_view::ExtensionSummary>> {
        let registry = registry()?;
        let registry = registry
            .read()
            .map_err(|err| anyhow::anyhow!("registry lock poisoned: {err}"))?;
        Ok(registry
            .list_all_installed()
            .into_iter()
            .map(to_view_summary)
            .collect())
    }

    fn load_marketplace_entries(
        &self,
        http_client: Arc<dyn HttpClient>,
    ) -> BoxFuture<'static, anyhow::Result<Vec<extension_view::MarketplaceEntry>>> {
        async move {
            let manifest = host_downloader::fetch_default_manifest_url(http_client).await?;
            Ok(manifest
                .into_entries()
                .into_iter()
                .map(to_view_entry)
                .collect())
        }
        .boxed()
    }

    fn load_marketplace_entries_from_url(
        &self,
        http_client: Arc<dyn HttpClient>,
        manifest_url: String,
    ) -> BoxFuture<'static, anyhow::Result<Vec<extension_view::MarketplaceEntry>>> {
        async move {
            let manifest = host_downloader::fetch_manifest_url(http_client, &manifest_url).await?;
            Ok(manifest
                .into_entries()
                .into_iter()
                .map(to_view_entry)
                .collect())
        }
        .boxed()
    }

    fn review_marketplace_entry(
        &self,
        http_client: Arc<dyn HttpClient>,
        entry: extension_view::MarketplaceEntry,
    ) -> BoxFuture<'static, anyhow::Result<extension_view::MarketplaceInstallOutcome>> {
        async move {
            let host_entry = to_host_entry(entry);
            let staging =
                host_downloader::download_marketplace_entry_to_staging(http_client, &host_entry)
                    .await?;
            review_downloaded_extension(staging, host_entry.kind, to_view_entry(host_entry))
        }
        .boxed()
    }

    fn review_local_tarball(
        &self,
        path: PathBuf,
    ) -> anyhow::Result<extension_view::MarketplaceInstallOutcome> {
        let staging = host_downloader::stage_local_tarball(&path)?;
        let kind = host_downloader::detect_package_kind(&staging)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("本地扩展")
            .to_string();
        let entry = extension_view::MarketplaceEntry {
            id: name.clone(),
            kind: to_view_kind(kind),
            name,
            version: String::new(),
            description: String::new(),
            file_extensions: Vec::new(),
            asset_url: path.display().to_string(),
            sha256: None,
            fallback_asset_url: None,
            manifest_url: None,
            manifest_fallback_url: None,
        };
        review_downloaded_extension(staging, kind, entry)
    }

    fn install_confirmed_staging(
        &self,
        staging: PathBuf,
    ) -> anyhow::Result<extension_view::ExtensionSummary> {
        let summary = install_staging_with_permission(&staging);
        let _ = std::fs::remove_dir_all(&staging);
        summary
    }

    fn uninstall(&self, summary: &extension_view::ExtensionSummary) -> anyhow::Result<String> {
        let registry = registry()?;
        let registry = registry
            .read()
            .map_err(|err| anyhow::anyhow!("registry lock poisoned: {err}"))?;
        registry.uninstall(to_host_kind(summary.kind), &summary.name)
    }

    fn refresh_after_extension_change(&self, cx: &mut App) {
        crate::refresh_global_runtime_catalog(cx);
        crate::extension::refresh_runtime_contributions(cx);
    }
}

fn registry() -> anyhow::Result<&'static std::sync::RwLock<host_extension::ExtensionRegistry>> {
    host_extension::ExtensionRegistry::global().ok_or_else(|| anyhow::anyhow!("扩展系统未初始化"))
}

fn review_downloaded_extension(
    staging: PathBuf,
    kind: host_extension::ExtensionKind,
    entry: extension_view::MarketplaceEntry,
) -> anyhow::Result<extension_view::MarketplaceInstallOutcome> {
    let review = match permission_review_for_staging(&staging, kind) {
        Ok(review) => review,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err);
        }
    };
    if review.high_risk_count > 0 {
        return Ok(extension_view::MarketplaceInstallOutcome::NeedsPermission(
            extension_view::DownloadedMarketplaceExtension {
                entry,
                staging,
                review,
            },
        ));
    }
    let summary = match install_staging_without_permission(&staging, kind) {
        Ok(summary) => summary,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err);
        }
    };
    let _ = std::fs::remove_dir_all(&staging);
    Ok(extension_view::MarketplaceInstallOutcome::Installed(
        summary,
    ))
}

fn permission_review_for_staging(
    staging: &std::path::Path,
    kind: host_extension::ExtensionKind,
) -> anyhow::Result<extension_view::PermissionReviewModel> {
    let review = if kind == host_extension::ExtensionKind::Composite {
        let root = package_root(staging)?;
        let manifest = load_from_dir(&root)?;
        build_permission_review(&manifest.permissions)?
    } else {
        build_permission_review(&[])?
    };
    Ok(extension_view::PermissionReviewModel {
        high_risk_count: review.high_risk_count,
        summary: review.summary,
    })
}

fn install_staging_without_permission(
    staging: &std::path::Path,
    kind: host_extension::ExtensionKind,
) -> anyhow::Result<extension_view::ExtensionSummary> {
    let registry = registry()?;
    let registry = registry
        .read()
        .map_err(|err| anyhow::anyhow!("registry lock poisoned: {err}"))?;
    host_downloader::install_from_staging_generic(staging, &registry, Some(kind))
        .map(to_view_summary)
}

fn install_staging_with_permission(
    staging: &std::path::Path,
) -> anyhow::Result<extension_view::ExtensionSummary> {
    let registry = registry()?;
    let registry = registry
        .read()
        .map_err(|err| anyhow::anyhow!("registry lock poisoned: {err}"))?;
    host_downloader::install_from_staging_with_high_risk_permissions(
        staging,
        &registry,
        Some(host_extension::ExtensionKind::Composite),
    )
    .map(to_view_summary)
}

fn to_view_summary(summary: host_extension::ExtensionSummary) -> extension_view::ExtensionSummary {
    extension_view::ExtensionSummary::new(
        to_view_kind(summary.kind),
        summary.name,
        summary.version,
        summary.path,
    )
    .with_description(summary.description)
    .with_file_extensions(summary.file_extensions)
    .with_icon(summary.icon)
    .with_driver_id(summary.driver_id)
    .with_default_port(summary.default_port)
}

fn to_view_entry(entry: host_downloader::MarketplaceEntry) -> extension_view::MarketplaceEntry {
    let asset_url = entry.asset_url().unwrap_or_default();
    let fallback_asset_url = entry.fallback_asset_url();
    let sha256 = entry.sha256();
    let manifest_urls = entry.extension_manifest_urls();
    let manifest_url = manifest_urls.first().cloned();
    let manifest_fallback_url = manifest_urls.get(1).cloned();
    extension_view::MarketplaceEntry {
        id: entry.id,
        kind: to_view_kind(entry.kind),
        name: entry.name,
        version: entry.version,
        description: entry.description,
        file_extensions: entry.file_extensions,
        asset_url,
        sha256,
        fallback_asset_url,
        manifest_url,
        manifest_fallback_url,
    }
}

fn to_host_entry(entry: extension_view::MarketplaceEntry) -> host_downloader::MarketplaceEntry {
    let mut download_urls = Vec::new();
    if !entry.asset_url.trim().is_empty() {
        download_urls.push(entry.asset_url);
    }
    if let Some(fallback_asset_url) = entry.fallback_asset_url {
        if !fallback_asset_url.trim().is_empty()
            && !download_urls.iter().any(|url| url == &fallback_asset_url)
        {
            download_urls.push(fallback_asset_url);
        }
    }
    let mut manifest_urls = Vec::new();
    if let Some(manifest_url) = entry.manifest_url {
        if !manifest_url.trim().is_empty() {
            manifest_urls.push(manifest_url);
        }
    }
    if let Some(manifest_fallback_url) = entry.manifest_fallback_url {
        if !manifest_fallback_url.trim().is_empty()
            && !manifest_urls
                .iter()
                .any(|url| url == &manifest_fallback_url)
        {
            manifest_urls.push(manifest_fallback_url);
        }
    }
    host_downloader::MarketplaceEntry::from_resolved_urls(
        entry.id,
        to_host_kind(entry.kind),
        entry.name,
        entry.version,
        entry.description,
        entry.file_extensions,
        download_urls,
        entry.sha256,
    )
    .with_manifest_urls(manifest_urls)
}

fn to_view_kind(kind: host_extension::ExtensionKind) -> extension_view::ExtensionKind {
    match kind {
        host_extension::ExtensionKind::Language => extension_view::ExtensionKind::Language,
        host_extension::ExtensionKind::DatabaseDriver => {
            extension_view::ExtensionKind::DatabaseDriver
        }
        host_extension::ExtensionKind::Composite => extension_view::ExtensionKind::Composite,
    }
}

fn to_host_kind(kind: extension_view::ExtensionKind) -> host_extension::ExtensionKind {
    match kind {
        extension_view::ExtensionKind::Language => host_extension::ExtensionKind::Language,
        extension_view::ExtensionKind::DatabaseDriver => {
            host_extension::ExtensionKind::DatabaseDriver
        }
        extension_view::ExtensionKind::Composite => host_extension::ExtensionKind::Composite,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn to_view_summary_preserves_database_driver_metadata() {
        let summary = host_extension::ExtensionSummary::new(
            host_extension::ExtensionKind::DatabaseDriver,
            "fake_pg",
            "1.2.3",
            PathBuf::from("/tmp/fake_pg"),
        )
        .with_description("PostgreSQL compatible driver")
        .with_file_extensions(vec!["sql".to_string()])
        .with_icon("Database")
        .with_driver_id("fake_pg")
        .with_default_port(15432);

        let view = to_view_summary(summary);

        assert_eq!(extension_view::ExtensionKind::DatabaseDriver, view.kind);
        assert_eq!("fake_pg", view.name);
        assert_eq!("1.2.3", view.version);
        assert_eq!("PostgreSQL compatible driver", view.description);
        assert_eq!(vec!["sql".to_string()], view.file_extensions);
        assert_eq!(Some("Database"), view.icon.as_deref());
        assert_eq!(Some("fake_pg"), view.driver_id.as_deref());
        assert_eq!(Some(15432), view.default_port);
    }

    #[test]
    fn marketplace_entry_conversion_preserves_extension_manifest_url() {
        let mut manifest: host_downloader::MarketplaceManifest = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "extensions": [{
                    "id": "fake_pg",
                    "kind": "database_driver",
                    "name": "Fake PostgreSQL",
                    "version": "1.2.3",
                    "release_tag": "fake_pg-v1.2.3",
                    "manifest": "fake_pg/manifest.json"
                }]
            }"#,
        )
        .unwrap();
        manifest.resolve_downloads(
            "https://onetcli.test.cn/extensions/manifest.json",
            "https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json",
        );
        let host_entry = manifest.into_entries().remove(0);

        let view_entry = to_view_entry(host_entry);
        let round_tripped = to_host_entry(view_entry);

        assert!(round_tripped.needs_extension_manifest());
        assert_eq!(
            Some("https://onetcli.test.cn/extensions/fake_pg/manifest.json".to_string()),
            round_tripped.extension_manifest_url()
        );
        assert_eq!(
            vec![
                "https://onetcli.test.cn/extensions/fake_pg/manifest.json".to_string(),
                "https://github.com/feigeCode/onetcli-extensions/releases/download/fake_pg-v1.2.3/extension-manifest.json".to_string(),
            ],
            round_tripped.extension_manifest_urls()
        );
    }
}
