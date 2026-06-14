use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use db::GlobalDbState;
use db_view::{
    extension_menu::{
        DbTreeExtensionActionContext, DbTreeExtensionActionHandler,
        GlobalDbTreeExtensionActionHandler,
    },
    extension_selector::{ExtensionSelectorOptions, load_extension_selector_options},
    extension_widget::{
        ExtensionWidgetActionHandler, ExtensionWidgetView,
        build_extension_widget_model_with_options,
    },
};
use extension_component::{PermissionSet, ViewActionEvent, ViewSpec};
use gpui::{App, AppContext, AsyncApp, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::{gpui_tokio::Tokio, popup_window::open_popup_window};

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
        let selector_options =
            load_extension_selector_options(&spec, &db_state, &permissions, &context).await;
        prepared.push(PreparedExtensionView {
            spec,
            selector_options,
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
    selector_options: ExtensionSelectorOptions,
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
            window.push_notification(Notification::info("扩展命令已执行").autohide(true), cx);
        }
        Ok(views) => open_views(views, window, cx),
        Err(err) => push_error(window, format!("扩展命令执行失败: {err}"), cx),
    }
}

fn open_views(views: Vec<PreparedExtensionView>, window: &mut Window, cx: &mut App) {
    for view in views {
        if let Err(err) = open_view(view, cx) {
            push_error(window, format!("扩展 UI 渲染失败: {err:?}"), cx);
        }
    }
}

fn open_view(view: PreparedExtensionView, cx: &mut App) -> Result<()> {
    let title = view.spec.title.clone();
    build_extension_widget_model_with_options(&view.spec, view.selector_options.clone())?;
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
                ExtensionWidgetView::new_with_options_and_handler(
                    window,
                    cx,
                    view.spec,
                    view.selector_options,
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
                Err(err) => Err(format!("扩展表单提交失败: {err}")),
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
    let catalog = catalog_for_action(input.catalog, &input.composite_root)?;
    catalog
        .handle_db_tree_component_view_action(input.context, input.db_state, input.event)
        .await?;
    Ok(())
}

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
            window.push_notification(Notification::success("扩展表单已提交").autohide(true), cx);
        }
        Err(err) => push_error(window, format!("扩展表单提交失败: {err}"), cx),
    }
}

fn push_error(window: &mut Window, message: impl Into<String>, cx: &mut App) {
    window.push_notification(Notification::error(message.into()).autohide(true), cx);
}

fn composite_root() -> Option<PathBuf> {
    extensions_root().map(|root| root.join(ExtensionKind::Composite.dir_name()))
}
