mod loader;
mod model;
#[cfg(test)]
mod model_tests;
mod renderer;

use db::GlobalDbState;
use er_flow::{
    BackgroundPlugin, EdgePlugin, FitAllGraphPlugin, FlowCanvas, Graph, NodeInteractionPlugin,
    NodePlugin, ViewportPlugin, ZoomControlsPlugin,
};
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Task, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Size, button::Button, spinner::Spinner, v_flex,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use loader::load_er_tables;
#[cfg(test)]
pub(crate) use model::infer_relationships;
pub(crate) use model::{ErColumnModel, ErForeignKeyModel, ErTableModel, build_er_graph};
use renderer::ErTableRenderer;

#[derive(Clone)]
pub(crate) struct ErDiagramConfig {
    pub connection_id: String,
    pub database_name: String,
    pub schema_name: Option<String>,
}

pub(crate) struct ErDiagramTab {
    config: ErDiagramConfig,
    canvas: Option<Entity<FlowCanvas>>,
    loading: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
}

impl ErDiagramTab {
    pub(crate) fn new(
        config: ErDiagramConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut tab = Self {
            config,
            canvas: None,
            loading: true,
            error: None,
            focus_handle: cx.focus_handle(),
        };
        tab.reload(window, cx);
        tab
    }

    fn reload(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.canvas = None;

        let global_state = cx.global::<GlobalDbState>().clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = load_graph(global_state, config, cx).await;
            let _ = cx.update(|cx| {
                if let Some(window_id) = cx.active_window() {
                    let _ = cx.update_window(window_id, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.apply_load_result(result, window, cx);
                        });
                    });
                }
            });
        })
        .detach();
    }

    fn apply_load_result(
        &mut self,
        result: anyhow::Result<Graph>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        match result {
            Ok(graph) => {
                self.canvas = Some(build_canvas(graph, window, cx));
                self.error = None;
            }
            Err(err) => {
                self.canvas = None;
                self.error = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Spinner::new().with_size(Size::Large))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("ErDiagram.loading").to_string()),
            )
    }

    fn render_error(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let message = self.error.clone().unwrap_or_default();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(div().text_sm().text_color(cx.theme().danger).child(message))
            .child(
                Button::new("reload-er-diagram")
                    .label(t!("Common.refresh").to_string())
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| this.reload(window, cx));
                    }),
            )
    }
}

async fn load_graph(
    global_state: GlobalDbState,
    config: ErDiagramConfig,
    cx: &mut AsyncApp,
) -> anyhow::Result<Graph> {
    let tables = load_er_tables(
        global_state,
        cx,
        config.connection_id,
        config.database_name,
        config.schema_name,
    )
    .await?;
    Ok(build_er_graph(tables))
}

fn build_canvas(
    graph: Graph,
    window: &mut Window,
    cx: &mut Context<ErDiagramTab>,
) -> Entity<FlowCanvas> {
    cx.new(|cx| {
        FlowCanvas::builder(graph, cx, window)
            .plugin(BackgroundPlugin::new())
            .plugin(NodeInteractionPlugin::new())
            .plugin(ViewportPlugin::with_blank_left_drag())
            .plugin(EdgePlugin::new())
            .plugin(NodePlugin::new())
            .plugin(FitAllGraphPlugin::new())
            .plugin(ZoomControlsPlugin::new())
            .node_renderer("er.table", ErTableRenderer)
            .build()
    })
}

impl Render for ErDiagramTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .when(self.loading, |this| this.child(self.render_loading(cx)))
            .when(!self.loading && self.error.is_some(), |this| {
                this.child(self.render_error(cx))
            })
            .when(!self.loading && self.error.is_none(), |this| {
                match self.canvas.as_ref() {
                    Some(canvas) => this.child(canvas.clone()),
                    None => this.child(div().size_full()),
                }
            })
    }
}

impl Focusable for ErDiagramTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for ErDiagramTab {}

impl TabContent for ErDiagramTab {
    fn content_key(&self) -> &'static str {
        "ErDiagram"
    }

    fn title(&self, _cx: &App) -> SharedString {
        let scope = self
            .config
            .schema_name
            .as_ref()
            .unwrap_or(&self.config.database_name);
        format!("{} - ER", scope).into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Network.color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        Task::ready(true)
    }
}
