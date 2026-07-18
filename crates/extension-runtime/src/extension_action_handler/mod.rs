use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use db::GlobalDbState;
#[cfg(feature = "wasm-components")]
use db_view::extension_selector::load_extension_selector_data;
use db_view::{
    extension_menu::{
        DbTreeExtensionActionContext, DbTreeExtensionActionHandler,
        GlobalDbTreeExtensionActionHandler,
    },
    extension_selector::ExtensionSelectorData,
    extension_widget::{
        ExtensionWidgetActionHandler, ExtensionWidgetView,
        build_extension_widget_model_with_selector_data,
    },
};
#[cfg(feature = "wasm-components")]
use extension_component::PermissionSet;
use extension_component::{ViewActionEvent, ViewSpec};
use gpui::{App, AppContext, AsyncApp, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::{gpui_tokio::Tokio, popup_window::open_popup_window};
use rust_i18n::t;

use crate::{
    ExtensionRuntimeCatalog, GlobalExtensionRuntimeCatalog,
    extension::{ExtensionKind, extensions_root},
};

mod popup;

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
            Ok(Ok(views)) => Ok(views),
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
) -> Result<Vec<PreparedExtensionView>> {
    #[cfg(not(feature = "wasm-components"))]
    {
        let _ = (composite_root, catalog, context, db_state);
        return Err(anyhow::anyhow!("wasm component runtime is disabled"));
    }

    #[cfg(feature = "wasm-components")]
    {
        let catalog = catalog_for_action(catalog, &composite_root)?;
        let permissions = PermissionSet::new(
            catalog
                .component_permissions_for_command(&context.command_id)?
                .iter(),
        );
        let views = catalog
            .run_db_tree_component_action(context.clone(), db_state.clone())
            .await?;
        prepare_views(
            views,
            composite_root,
            catalog,
            context,
            db_state,
            permissions,
        )
        .await
    }
}

#[cfg(feature = "wasm-components")]
async fn prepare_views(
    views: Vec<ViewSpec>,
    composite_root: PathBuf,
    catalog: Arc<ExtensionRuntimeCatalog>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
    permissions: PermissionSet,
) -> Result<Vec<PreparedExtensionView>> {
    let mut prepared = Vec::new();
    for spec in views {
        let selector_data =
            load_extension_selector_data(&spec, &db_state, &permissions, &context).await;
        prepared.push(PreparedExtensionView {
            spec,
            selector_data,
            composite_root: composite_root.clone(),
            catalog: Some(catalog.clone()),
            context: context.clone(),
            db_state: db_state.clone(),
        });
    }
    Ok(prepared)
}

struct PreparedExtensionView {
    spec: ViewSpec,
    selector_data: ExtensionSelectorData,
    composite_root: PathBuf,
    catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
}

fn apply_action_outcome(
    outcome: std::result::Result<Vec<PreparedExtensionView>, String>,
    window: &mut Window,
    cx: &mut App,
) {
    match outcome {
        Ok(views) if views.is_empty() => {
            window.push_notification(
                Notification::info(t!("ExtensionAction.executed").to_string()).autohide(true),
                cx,
            );
        }
        Ok(views) => open_views(views, window, cx),
        Err(err) => push_error(
            window,
            t!("ExtensionAction.failed", error = err).to_string(),
            cx,
        ),
    }
}

fn open_views(views: Vec<PreparedExtensionView>, window: &mut Window, cx: &mut App) {
    for view in views {
        if let Err(err) = open_view(view, cx) {
            push_error(
                window,
                t!("ExtensionAction.render_failed", error = format!("{err:?}")).to_string(),
                cx,
            );
        }
    }
}

fn open_view(view: PreparedExtensionView, cx: &mut App) -> Result<()> {
    let title = view.spec.title.clone();
    build_extension_widget_model_with_selector_data(
        &view.spec,
        view.selector_data.options.clone(),
        view.selector_data.policies.clone(),
    )?;
    let popup_options = popup::popup_options_for_view(title, view.spec.window.as_ref());
    let action_handler = view_action_handler(
        view.composite_root,
        view.catalog.clone(),
        view.context,
        view.db_state,
    );
    open_popup_window(
        popup_options,
        move |window, cx| {
            cx.new(|cx| {
                ExtensionWidgetView::new_with_selector_data_and_handler(
                    window,
                    cx,
                    view.spec,
                    view.selector_data.options,
                    view.selector_data.policies,
                    Some(action_handler),
                )
                .expect("validated extension view spec")
            })
        },
        cx,
    );
    Ok(())
}

fn view_action_handler(
    composite_root: PathBuf,
    catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
) -> ExtensionWidgetActionHandler {
    Arc::new(move |event, window, cx| {
        let window_handle = window.window_handle();
        let input = SubmitInput {
            composite_root: composite_root.clone(),
            catalog: catalog.clone(),
            context: context.clone(),
            db_state: db_state.clone(),
            event,
        };
        cx.spawn(async move |cx: &mut AsyncApp| {
            let task = Tokio::spawn(cx, submit_view_action(input));
            let outcome = match task.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => Err(format!("{err:?}")),
                Err(err) => Err(t!("ExtensionAction.submit_failed", error = err).to_string()),
            };
            let _ = cx.update(|cx| {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    apply_submit_outcome(outcome, window, cx);
                });
            });
        })
        .detach();
    })
}

struct SubmitInput {
    composite_root: PathBuf,
    catalog: Option<Arc<ExtensionRuntimeCatalog>>,
    context: DbTreeExtensionActionContext,
    db_state: GlobalDbState,
    event: ViewActionEvent,
}

async fn submit_view_action(input: SubmitInput) -> Result<()> {
    #[cfg(not(feature = "wasm-components"))]
    {
        let SubmitInput {
            composite_root,
            catalog,
            context,
            db_state,
            event,
        } = input;
        let _ = (composite_root, catalog, context, db_state, event);
        return Err(anyhow::anyhow!("wasm component runtime is disabled"));
    }

    #[cfg(feature = "wasm-components")]
    {
        let catalog = catalog_for_action(input.catalog, &input.composite_root)?;
        catalog
            .handle_db_tree_component_view_action(input.context, input.db_state, input.event)
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

fn apply_submit_outcome(
    outcome: std::result::Result<(), String>,
    window: &mut Window,
    cx: &mut App,
) {
    match outcome {
        Ok(()) => {
            window.push_notification(
                Notification::success(t!("ExtensionAction.submitted").to_string()).autohide(true),
                cx,
            );
        }
        Err(err) => push_error(
            window,
            t!("ExtensionAction.submit_failed", error = err).to_string(),
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
