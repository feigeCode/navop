use std::sync::{Arc, RwLock};

use gpui::BorrowAppContext;

use super::catalog::ExtensionRuntimeCatalog;

#[derive(Clone, Default)]
pub struct GlobalExtensionRuntimeCatalog {
    catalog: Arc<RwLock<Option<Arc<ExtensionRuntimeCatalog>>>>,
}

impl gpui::Global for GlobalExtensionRuntimeCatalog {}

impl GlobalExtensionRuntimeCatalog {
    pub fn get(&self) -> Option<Arc<ExtensionRuntimeCatalog>> {
        self.catalog.read().ok()?.clone()
    }

    pub fn replace(&self, catalog: ExtensionRuntimeCatalog) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = Some(Arc::new(catalog));
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = None;
        }
    }
}

pub fn refresh_global_runtime_catalog(cx: &mut impl BorrowAppContext) {
    let Some(root) = crate::extension::extensions_root()
        .map(|root| root.join(crate::extension::ExtensionKind::Composite.dir_name()))
    else {
        cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| global.clear());
        return;
    };

    match ExtensionRuntimeCatalog::from_installed_composite_root(&root) {
        Ok(catalog) => {
            cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
                global.replace(catalog);
            });
        }
        Err(err) => {
            tracing::warn!("加载扩展运行时 catalog 失败: {err:?}");
            cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
                global.clear();
            });
        }
    }
}
