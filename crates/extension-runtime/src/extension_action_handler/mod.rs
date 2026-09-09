use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use db::GlobalDbState;
use gpui::{App, AppContext, AsyncApp, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;

use crate::{
    ExtensionRuntimeCatalog, GlobalExtensionRuntimeCatalog,
    extension::{ExtensionKind, extensions_root},
};

pub fn register_db_tree_extension_action_handler(cx: &mut App) {
    cx.set_global(GlobalDbTreeExtensionActionHandler::new(Arc::new(
        MainDbTreeExtensionActionHandler,
    )));
}

struct MainDbTreeExtensionActionHandler;

impl DbTreeExtensionActionHandler for MainDbTreeExtensionActionHandler {
    fn run(&self, context: DbTreeExtensionActionContext, window: &mut Window, cx: &mut App) {
        let Some(db_state) = cx.try_global::<GlobalDbState>().cloned() else {
            push_error(window, "数据库状态未初始化", cx);
            return;
        };
        let Some(composite_root) = composite_root() else {
            push_error(window, "扩展目录不可用", cx);
            return;
        };
        let cached_catalog = cx
            .try_global::<GlobalExtensionRuntimeCatalog>()
            .and_then(|global| global.get());
        spawn_action_task(
            composite_root,
            cached_catalog,
            context,
            db_state,
            window,
            cx,
        );
    }
}

fn spawn_action_task(
    composite_root: PathBuf,
    cached_catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
    window: &mut Window,
    cx: &mut App,
) {
    let window_handle = window.window_handle();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let task = Tokio::spawn(
            cx,
            run_action(composite_root, cached_catalog, context, db_state),
        );
        let outcome = match task.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(format!("{err:?}")),
            Err(err) => Err(format!("扩展任务执行失败: {err}")),
        };
        let _ = cx.update(|cx| {
            let _ = cx.update_window(window_handle, |_, window, cx| {
                apply_action_outcome(outcome, window, cx);
            });
        });
    })
    .detach();
}

async fn run_action(
    composite_root: PathBuf,
    catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
) -> Result<()> {
    #[cfg(not(feature = "wasm-components"))]
    {
        let _ = (composite_root, catalog, context, db_state);
        return Err(anyhow::anyhow!("wasm component runtime is disabled"));
    }

    #[cfg(feature = "wasm-components")]
    {
        let catalog = catalog_for_action(catalog, &composite_root)?;
        catalog
            .run_db_tree_component_action(context, db_state)
            .await?;
        Ok(())
    }
}

#[cfg(feature = "wasm-components")]
fn catalog_for_action(
    catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    composite_root: &PathBuf,
) -> Result<Arc<ExtensionRuntimeCatalog>> {
    match catalog {
        Some(catalog) => Ok(catalog),
        None => Ok(Arc::new(
            ExtensionRuntimeCatalog::from_installed_composite_root(composite_root)?,
        )),
    }
}

fn apply_action_outcome(
    outcome: std::result::Result<(), String>,
    window: &mut Window,
    cx: &mut App,
) {
    match outcome {
        Ok(()) => {
            window.push_notification(
                Notification::info(t!("ExtensionAction.executed").to_string()).autohide(true),
                cx,
            );
        }
        Err(err) => push_error(
            window,
            t!("ExtensionAction.failed", error = err).to_string(),
            cx,
        ),
    }
}

fn push_error(window: &mut Window, message: impl Into<String>, cx: &mut App) {
    window.push_notification(Notification::error(message.into()).autohide(true), cx);
}

fn composite_root() -> Option<PathBuf> {
    extensions_root().map(|root| root.join(ExtensionKind::Composite.dir_name()))
}

use db_view::extension_menu::{
    DbTreeExtensionActionContext, DbTreeExtensionActionHandler, GlobalDbTreeExtensionActionHandler,
};
