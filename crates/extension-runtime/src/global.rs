use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};

#[cfg(feature = "wasm-components")]
use futures::FutureExt;
use gpui::BorrowAppContext;

use super::catalog::ExtensionRuntimeCatalog;
use crate::extension::manifest::Manifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopmentManifestFailure {
    pub extension_id: String,
    pub root: std::path::PathBuf,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct DevelopmentCatalogReport {
    pub failures: Vec<DevelopmentManifestFailure>,
}

#[derive(Clone, Default)]
pub struct GlobalExtensionRuntimeCatalog {
    catalog: Arc<RwLock<Option<Arc<ExtensionRuntimeCatalog>>>>,
    revision: Arc<AtomicU64>,
    /// Session-only contributions, retained when installed extensions refresh.
    development_manifests: Vec<Manifest>,
}

impl gpui::Global for GlobalExtensionRuntimeCatalog {}

impl GlobalExtensionRuntimeCatalog {
    /// Replaces session-only development contributions. Invalid development
    /// manifests are rejected individually without disturbing valid entries.
    pub fn set_development_manifests(
        &mut self,
        manifests: Vec<Manifest>,
    ) -> anyhow::Result<DevelopmentCatalogReport> {
        let installed = load_installed_manifests()?;
        let (catalog, accepted, report) = build_catalog_with_development(installed, &manifests)?;
        self.development_manifests = accepted;
        self.publish(catalog);
        Ok(report)
    }

    fn refresh(&self) -> anyhow::Result<()> {
        let installed = load_installed_manifests()?;
        let (catalog, _, report) =
            build_catalog_with_development(installed, &self.development_manifests)?;
        for failure in report.failures {
            tracing::warn!(
                extension_id = %failure.extension_id,
                root = %failure.root.display(),
                error = %failure.error,
                "development extension omitted while refreshing catalog"
            );
        }
        self.publish(catalog);
        Ok(())
    }

    fn publish(&self, catalog: ExtensionRuntimeCatalog) {
        let catalog = Arc::new(catalog);
        #[cfg(feature = "wasm-components")]
        install_html_preview_transform_provider(catalog.clone());
        self.replace_arc(catalog);
    }

    pub fn snapshot(&self) -> (u64, Option<Arc<ExtensionRuntimeCatalog>>) {
        loop {
            let before = self.revision.load(Ordering::Acquire);
            let catalog = self.catalog.read().ok().and_then(|catalog| catalog.clone());
            let after = self.revision.load(Ordering::Acquire);
            if before == after {
                return (after, catalog);
            }
        }
    }

    pub fn get(&self) -> Option<Arc<ExtensionRuntimeCatalog>> {
        self.catalog.read().ok()?.clone()
    }

    pub fn replace(&self, catalog: ExtensionRuntimeCatalog) {
        self.replace_arc(Arc::new(catalog));
    }

    pub fn replace_arc(&self, catalog: Arc<ExtensionRuntimeCatalog>) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = Some(catalog);
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = None;
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

fn load_installed_manifests() -> anyhow::Result<Vec<Manifest>> {
    let Some(root) = crate::extension::extensions_root() else {
        return Ok(Vec::new());
    };
    let root = root.join(crate::extension::ExtensionKind::Composite.dir_name());
    Ok(crate::registration::load_installed_composite_manifests(
        &root,
    )?)
}

fn build_catalog_with_development(
    installed: Vec<Manifest>,
    development: &[Manifest],
) -> anyhow::Result<(
    ExtensionRuntimeCatalog,
    Vec<Manifest>,
    DevelopmentCatalogReport,
)> {
    let mut accepted = Vec::new();
    let mut catalog = ExtensionRuntimeCatalog::from_manifests(installed.clone())?;
    let mut report = DevelopmentCatalogReport::default();
    for manifest in development {
        let mut candidate = installed.clone();
        candidate.extend(accepted.iter().cloned());
        candidate.push(manifest.clone());
        match ExtensionRuntimeCatalog::from_manifests(candidate) {
            Ok(next) => {
                accepted.push(manifest.clone());
                catalog = next;
            }
            Err(error) => report.failures.push(DevelopmentManifestFailure {
                extension_id: manifest.id.clone(),
                root: manifest.manifest_dir.clone(),
                error: error.to_string(),
            }),
        }
    }
    Ok((catalog, accepted, report))
}

/// 读取当前全局 runtime catalog(未初始化时 None)。
pub fn global_catalog(cx: &gpui::App) -> Option<Arc<ExtensionRuntimeCatalog>> {
    cx.try_global::<GlobalExtensionRuntimeCatalog>()
        .and_then(|global| global.get())
}

pub fn refresh_global_runtime_catalog(cx: &mut impl BorrowAppContext) {
    cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
        if let Err(err) = global.refresh() {
            tracing::warn!("加载扩展运行时 catalog 失败: {err:?}");
        }
    });
}

#[cfg(feature = "wasm-components")]
fn install_html_preview_transform_provider(catalog: Arc<ExtensionRuntimeCatalog>) {
    html_preview::set_html_preview_transform_provider(move |language, html| {
        let catalog = catalog.clone();
        async move {
            catalog
                .transform_html_preview(&language, &html)
                .await
                .map_err(|error| error.to_string())
        }
        .boxed()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::manifest::{
        ApiVersions, ContributesManifest, Engines, RuntimeSection, ShellHostModule, ShellSurface,
        ShellViewContrib,
    };

    fn development_manifest(id: &str, root: &str, view_id: &str) -> Manifest {
        Manifest {
            schema_version: 1,
            id: id.into(),
            name: id.into(),
            version: "0.1.0".into(),
            publisher: String::new(),
            license: String::new(),
            homepage: String::new(),
            repository: String::new(),
            icon: String::new(),
            description_i18n: String::new(),
            description: String::new(),
            categories: Vec::new(),
            keywords: Vec::new(),
            engines: Engines {
                onetcli: ">=0.1.0".into(),
                gpui_shell: "0.2.0".into(),
            },
            api: ApiVersions::default(),
            activation: Vec::new(),
            permissions: vec!["shell:exec".into()],
            runtime: RuntimeSection::default(),
            contributes: ContributesManifest {
                shell_views: vec![ShellViewContrib {
                    id: view_id.into(),
                    title: view_id.into(),
                    description: None,
                    icon: None,
                    entry: "ui/tool.js".into(),
                    surface: ShellSurface::Toolbox,
                    singleton: true,
                    backends: Default::default(),
                    modules: vec![ShellHostModule::Log],
                    category: Some("test".into()),
                    keywords: None,
                }],
                ..Default::default()
            },
            manifest_dir: root.into(),
        }
    }

    #[test]
    fn invalid_development_manifest_does_not_remove_valid_views() {
        let valid = development_manifest("dev.valid", "/tmp/valid", "tool");
        let mut invalid = development_manifest("dev.invalid", "/tmp/invalid", "tool");
        invalid.contributes.shell_views[0].category = None;

        let (catalog, accepted, report) =
            build_catalog_with_development(Vec::new(), &[valid, invalid]).unwrap();

        assert!(catalog.shell_view("dev.valid", "tool").is_some());
        assert!(catalog.shell_view("dev.invalid", "tool").is_none());
        assert_eq!(accepted.len(), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            report.failures[0].root,
            std::path::PathBuf::from("/tmp/invalid")
        );
    }

    #[test]
    fn development_views_are_rebuilt_with_installed_views() {
        let installed = development_manifest("installed.tools", "/tmp/installed", "installed");
        let development = development_manifest("dev.tools", "/tmp/dev", "dev");

        let (catalog, accepted, report) =
            build_catalog_with_development(vec![installed], &[development]).unwrap();

        assert!(report.failures.is_empty());
        assert_eq!(accepted.len(), 1);
        assert!(catalog.shell_view("installed.tools", "installed").is_some());
        assert!(catalog.shell_view("dev.tools", "dev").is_some());
    }
}
