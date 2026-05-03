mod loader;
mod scroll_pan_plugin;
mod pan_mode_plugin;

use db::GlobalDbState;
use ferrum_flow::{
    BackgroundPlugin, Command, EdgePlugin, FitAllGraphPlugin,
    FlowCanvas, Graph, Interaction,
    NodeInteractionPlugin, NodePlugin, Plugin,
    ViewportPlugin, ZoomControlsPlugin,
};

use crate::er_diagram::scroll_pan_plugin::ErDiagramScrollPanPlugin;
use er_flow::{er_flow_theme, er_node_renderers, graph_from_diagram};
use gpui::{
    div, prelude::FluentBuilder, App, AppContext as _, AsyncApp, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window
    ,
};
use gpui_component::{
    button::Button, spinner::Spinner, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, Size,
};
use loader::load_er_diagram;
use one_core::{
    popup_window::{open_popup_window, PopupWindowOptions},
    tab_container::{TabContent, TabContentEvent},
};
use rust_i18n::t;
use crate::er_diagram::pan_mode_plugin::ErDiagramPanModePlugin;

const ER_DIAGRAM_WINDOW_WIDTH: f32 = 1200.0;
const ER_DIAGRAM_WINDOW_HEIGHT: f32 = 800.0;
const ER_DIAGRAM_WINDOW_MIN_WIDTH: f32 = 800.0;
const ER_DIAGRAM_WINDOW_MIN_HEIGHT: f32 = 500.0;
const ER_DIAGRAM_TOOL_MARGIN: f32 = 16.0;
const ER_DIAGRAM_TOOL_SIZE: f32 = 36.0;
const ER_DIAGRAM_TOOL_GAP: f32 = 6.0;
const ER_DIAGRAM_PAN_LABEL: &str = "✋";

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

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.canvas = None;

        let window_handle = window.window_handle();
        let global_state = cx.global::<GlobalDbState>().clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = load_graph(global_state, config, cx).await;
            let _ = cx.update(|cx| {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.apply_load_result(result, window, cx);
                    });
                });
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

pub(crate) fn open_er_diagram_window(config: ErDiagramConfig, cx: &mut App) {
    let title = er_diagram_title(&config);
    open_popup_window(
        PopupWindowOptions::new(title)
            .size(ER_DIAGRAM_WINDOW_WIDTH, ER_DIAGRAM_WINDOW_HEIGHT)
            .min_width(ER_DIAGRAM_WINDOW_MIN_WIDTH)
            .min_height(ER_DIAGRAM_WINDOW_MIN_HEIGHT),
        move |window, cx| cx.new(|cx| ErDiagramTab::new(config, window, cx)),
        cx,
    );
}

fn er_diagram_title(config: &ErDiagramConfig) -> SharedString {
    let scope = config.schema_name.as_ref().unwrap_or(&config.database_name);
    format!("{} - ER", scope).into()
}

async fn load_graph(
    global_state: GlobalDbState,
    config: ErDiagramConfig,
    cx: &mut AsyncApp,
) -> anyhow::Result<Graph> {
    let diagram = load_er_diagram(
        global_state,
        cx,
        config.connection_id,
        config.database_name,
        config.schema_name,
    )
    .await?;
    graph_from_diagram(&diagram)
}





fn build_canvas(
    graph: Graph,
    window: &mut Window,
    cx: &mut Context<ErDiagramTab>,
) -> Entity<FlowCanvas> {
    cx.new(|cx| {
        FlowCanvas::builder(graph, cx, window)
            .theme(er_flow_theme())
            .plugin(BackgroundPlugin::new())
            .plugin(ErDiagramScrollPanPlugin::new())
            .plugin(ErDiagramPanModePlugin::new())
            .plugin(ViewportPlugin::new())
            .plugin(EdgePlugin::new())
            .plugin(NodePlugin::new())
            .plugin(NodeInteractionPlugin::new())
            .plugin(FitAllGraphPlugin::new())
            .plugin(ZoomControlsPlugin::new())
            .node_renderers(er_node_renderers())
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
        er_diagram_title(&self.config)
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
