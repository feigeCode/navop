mod composite_provider;
mod database_driver_provider;
mod kind;
mod language_provider;
pub mod manifest;
mod provider;
mod remote_desktop_provider;
mod summary;

pub use composite_provider::CompositeExtensionProvider;
pub use database_driver_provider::DatabaseDriverExtensionProvider;
pub use kind::ExtensionKind;
pub use language_provider::LanguageExtensionProvider;
pub use provider::{ExtensionProvider, ExtensionRegistry, init_global};
pub use remote_desktop_provider::RemoteDesktopProviderExtensionProvider;
pub use summary::ExtensionSummary;

use std::{path::PathBuf, sync::Arc};

use db_view::extension_menu::DbTreeExtensionMenuRegistry;
use gpui::{App, BorrowAppContext};
use gpui_component::highlighter::{LanguageRegistry, LoadReport, load_extensions_dir};

pub fn init(cx: &mut App) {
    let Some(root) = extensions_root() else {
        tracing::warn!("无法解析扩展根目录,跳过 ExtensionRegistry 初始化");
        return;
    };
    let registry = builtin_registry(root.clone());
    init_global(registry);
    load_language_extensions(&root);
    crate::refresh_global_runtime_catalog(cx);
    refresh_runtime_contributions(cx);
    crate::extension_action_handler::register_db_tree_extension_action_handler(cx);
}

pub fn builtin_registry(extensions_root: PathBuf) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new(extensions_root);
    registry.register_provider(Arc::new(LanguageExtensionProvider));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));
    registry.register_provider(Arc::new(RemoteDesktopProviderExtensionProvider));
    registry.register_provider(Arc::new(CompositeExtensionProvider));
    registry
}

pub fn extensions_root() -> Option<PathBuf> {
    let base = one_core::storage::manager::get_config_dir().ok()?;
    Some(base.join("extensions"))
}

pub fn load_language_extensions_from_root(root: &std::path::Path) -> anyhow::Result<LoadReport> {
    load_extensions_dir(
        &root.join(ExtensionKind::Language.dir_name()),
        LanguageRegistry::singleton(),
    )
}

fn load_language_extensions(root: &std::path::Path) {
    match load_language_extensions_from_root(root) {
        Ok(report) => {
            if !report.loaded.is_empty() {
                tracing::info!(
                    "已加载 {} 个语言扩展: {:?}",
                    report.loaded.len(),
                    report.loaded
                );
            }
            if !report.failed.is_empty() {
                tracing::warn!(
                    "有 {} 个语言扩展加载失败: {:?}",
                    report.failed.len(),
                    report.failed
                );
            }
        }
        Err(err) => {
            tracing::warn!("加载语言扩展失败: {err:?}");
        }
    }
}

pub fn refresh_runtime_contributions(cx: &mut impl BorrowAppContext) {
    let registry = load_db_tree_extension_menu_registry(cx);
    cx.update_default_global::<DbTreeExtensionMenuRegistry, _>(|global, _| {
        *global = registry;
    });
}

fn load_db_tree_extension_menu_registry(
    cx: &mut impl BorrowAppContext,
) -> DbTreeExtensionMenuRegistry {
    let cached = cx
        .update_default_global::<crate::GlobalExtensionRuntimeCatalog, _>(|global, _| global.get());
    if let Some(catalog) = cached {
        return catalog.db_tree_menu_registry();
    }

    let Some(root) = extensions_root() else {
        return DbTreeExtensionMenuRegistry::default();
    };
    let composite_root = root.join(ExtensionKind::Composite.dir_name());
    match crate::ExtensionRuntimeCatalog::from_installed_composite_root(&composite_root) {
        Ok(catalog) => catalog.db_tree_menu_registry(),
        Err(err) => {
            tracing::warn!("加载扩展运行时贡献点失败: {err:?}");
            DbTreeExtensionMenuRegistry::default()
        }
    }
}

#[cfg(test)]
mod composite_provider_tests;
#[cfg(test)]
mod provider_tests;
