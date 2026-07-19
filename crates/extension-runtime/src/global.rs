use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};

#[cfg(feature = "wasm-components")]
use futures::FutureExt;
use gpui::BorrowAppContext;

use super::catalog::ExtensionRuntimeCatalog;

#[derive(Clone, Default)]
pub struct GlobalExtensionRuntimeCatalog {
    catalog: Arc<RwLock<Option<Arc<ExtensionRuntimeCatalog>>>>,
    revision: Arc<AtomicU64>,
}

impl gpui::Global for GlobalExtensionRuntimeCatalog {}

impl GlobalExtensionRuntimeCatalog {
    pub fn get(&self) -> Option<Arc<ExtensionRuntimeCatalog>> {
        self.catalog.read().ok()?.clone()
    }

    pub fn replace(&self, catalog: ExtensionRuntimeCatalog) {
        self.replace_arc(Arc::new(catalog));
    }

    pub fn replace_arc(&self, catalog: Arc<ExtensionRuntimeCatalog>) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = Some(catalog);
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = None;
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
}

pub fn refresh_global_runtime_catalog(cx: &mut impl BorrowAppContext) {
    let Some(root) = crate::extension::extensions_root()
        .map(|root| root.join(crate::extension::ExtensionKind::Composite.dir_name()))
    else {
        html_preview::clear_html_preview_transform_provider();
        cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| global.clear());
        return;
    };

    match ExtensionRuntimeCatalog::from_installed_composite_root(&root) {
        Ok(catalog) => {
            #[cfg(feature = "wasm-components")]
            let catalog = {
                let catalog = Arc::new(catalog);
                install_html_preview_transform_provider(catalog.clone());
                catalog
            };
            #[cfg(not(feature = "wasm-components"))]
            let catalog = Arc::new(catalog);
            #[cfg(feature = "wasm-components")]
            {
                let prewarm_catalog = catalog.clone();
                std::thread::spawn(move || {
                    if let Err(error) = prewarm_catalog.prewarm_document_renderers() {
                        tracing::warn!(
                            target: "extension_loader",
                            %error,
                            "预热文档渲染扩展失败"
                        );
                    }
                    if let Err(error) = prewarm_catalog.prewarm_document_exporters() {
                        tracing::warn!(
                            target: "extension_loader",
                            %error,
                            "预热文档导出扩展失败"
                        );
                    }
                });
            }
            cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
                global.replace_arc(catalog);
            });
        }
        Err(err) => {
            tracing::warn!("加载扩展运行时 catalog 失败: {err:?}");
            html_preview::clear_html_preview_transform_provider();
            cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
                global.clear();
            });
        }
    }
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
