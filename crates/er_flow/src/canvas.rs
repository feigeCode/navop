use futures::{StreamExt, channel::mpsc};
use gpui::*;

use crate::{
    BackgroundPlugin, DeletePlugin, EdgePlugin, FlowTheme, GraphChange, HistoryPlugin,
    NodeInteractionPlugin, NodePlugin, PortInteractionPlugin, SelectionPlugin, SharedState,
    SyncPlugin, ViewportPlugin,
    graph::Graph,
    plugin::{
        EventResult, FlowEvent, InitPluginContext, InputEvent, Plugin, PluginContext,
        PluginRegistry, RenderContext, RenderLayer, invalidate_port_layout_cache_for_graph_change,
    },
    viewport::Viewport,
};

mod node_renderer;
mod port_cache;
mod types;
mod undo;

pub use port_cache::PortLayoutCache;

pub use undo::{Command, CommandContext, CompositeCommand, HistoryProvider, LocalHistory};

pub use types::{Interaction, InteractionResult, InteractionState};

#[allow(deprecated)]
pub use node_renderer::port_screen_position;
pub use node_renderer::{NodeRenderer, RendererRegistry, default_node_caption};

pub struct FlowCanvas {
    pub graph: Graph,

    pub(crate) viewport: Viewport,

    pub(crate) plugins_registry: PluginRegistry,

    pub(crate) sync_plugin: Option<Box<dyn SyncPlugin + 'static>>,

    renderers: RendererRegistry,

    pub(crate) focus_handle: FocusHandle,

    pub(crate) interaction: InteractionState,

    pub history: Box<dyn HistoryProvider>,

    pub event_queue: Vec<FlowEvent>,
    pub port_offset_cache: PortLayoutCache,

    /// Visual tokens for canvas chrome; plugins adjust via [`InitPluginContext::theme`](crate::plugin::InitPluginContext::theme).
    pub theme: FlowTheme,

    /// Type-erased map for cross-plugin data on this canvas instance.
    pub shared_state: SharedState,
}

// // TODO
// impl Clone for FlowCanvas {
//     fn clone(&self) -> Self {
//         Self {
//             graph: self.graph.clone(),
//             viewport: self.viewport.clone(),
//             plugins_registry: PluginRegistry::new(),
//             focus_handle: self.focus_handle.clone(),
//             interaction: InteractionState::new(),
//             event_queue: vec![],
//         }
//     }
// }

impl FlowCanvas {
    pub fn new(graph: Graph, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            graph,
            viewport: Viewport::new(),
            plugins_registry: PluginRegistry::new(),
            sync_plugin: None,
            renderers: RendererRegistry::new(),
            focus_handle,
            interaction: InteractionState::new(),
            history: Box::new(LocalHistory::new()),
            event_queue: vec![],
            port_offset_cache: PortLayoutCache::new(),
            theme: FlowTheme::default(),
            shared_state: SharedState::new(),
        }
    }

    pub fn builder<'a, 'b>(
        graph: Graph,
        ctx: &'a mut Context<'b, Self>,
        window: &'a Window,
    ) -> FlowCanvasBuilder<'a, 'b> {
        FlowCanvasBuilder {
            graph,
            ctx,
            window,
            plugins: PluginRegistry::new(),
            sync_plugin: None,
            renderers: RendererRegistry::new(),
            theme: FlowTheme::default(),
        }
    }

    pub fn handle_event(&mut self, event: FlowEvent, cx: &mut Context<Self>) {
        let event_queue = &mut self.event_queue;

        let mut emit = |event: FlowEvent| {
            event_queue.push(event);
        };

        let mut notify = || {
            cx.notify();
        };

        // if has interaction
        if let Some(mut handler) = self.interaction.handler.take() {
            let mut ctx = PluginContext::new(
                &mut self.graph,
                &mut self.port_offset_cache,
                &mut self.viewport,
                &mut self.interaction,
                &mut self.renderers,
                &mut self.sync_plugin,
                self.history.as_mut(),
                &mut self.theme,
                &mut self.shared_state,
                &mut emit,
                &mut notify,
            );
            let mut fast_return = false;
            let result = match &event {
                FlowEvent::Input(InputEvent::MouseMove(ev)) => {
                    fast_return = true;
                    handler.on_mouse_move(ev, &mut ctx)
                }

                FlowEvent::Input(InputEvent::MouseUp(ev)) => {
                    fast_return = true;
                    handler.on_mouse_up(ev, &mut ctx)
                }

                _ => InteractionResult::Continue,
            };

            if fast_return {
                match result {
                    InteractionResult::Continue => self.interaction.handler = Some(handler),

                    InteractionResult::End => {
                        self.interaction.handler = None;
                    }

                    InteractionResult::Replace(h) => {
                        self.interaction.handler = Some(h);
                    }
                }
                return;
            }
        }

        let mut ctx = PluginContext::new(
            &mut self.graph,
            &mut self.port_offset_cache,
            &mut self.viewport,
            &mut self.interaction,
            &mut self.renderers,
            &mut self.sync_plugin,
            self.history.as_mut(),
            &mut self.theme,
            &mut self.shared_state,
            &mut emit,
            &mut notify,
        );

        for plugin in &mut self.plugins_registry.plugins {
            let result = plugin.on_event(&event, &mut ctx);
            match result {
                EventResult::Continue => {}
                EventResult::Stop => break,
            }
        }
    }

    fn process_event_queue(&mut self, cx: &mut Context<Self>) {
        while let Some(event) = self.event_queue.pop() {
            let mut emit = |e| self.event_queue.push(e);

            let mut notify = || {
                cx.notify();
            };

            let mut ctx = PluginContext::new(
                &mut self.graph,
                &mut self.port_offset_cache,
                &mut self.viewport,
                &mut self.interaction,
                &mut self.renderers,
                &mut self.sync_plugin,
                self.history.as_mut(),
                &mut self.theme,
                &mut self.shared_state,
                &mut emit,
                &mut notify,
            );

            for plugin in &mut self.plugins_registry.plugins {
                let result = plugin.on_event(&event, &mut ctx);
                match result {
                    EventResult::Continue => {}
                    EventResult::Stop => break,
                }
            }
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(FlowEvent::Input(InputEvent::KeyDown(ev.clone())), cx);
        self.process_event_queue(cx);
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(FlowEvent::Input(InputEvent::KeyUp(ev.clone())), cx);
        self.process_event_queue(cx);
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut ev = ev.clone();
        ev.position = self.viewport.window_to_local(ev.position);
        self.handle_event(FlowEvent::Input(InputEvent::MouseDown(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut ev = ev.clone();
        ev.position = self.viewport.window_to_local(ev.position);
        self.handle_event(FlowEvent::Input(InputEvent::MouseMove(ev.clone())), cx);
        if let Some(sync_plugin) = &mut self.sync_plugin {
            let world = self.viewport.screen_to_world(ev.position);
            sync_plugin.on_mouse_move(&ev, world);
        }
        self.process_event_queue(cx);
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut ev = ev.clone();
        ev.position = self.viewport.window_to_local(ev.position);
        self.handle_event(FlowEvent::Input(InputEvent::MouseUp(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_scroll_wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut ev = ev.clone();
        ev.position = self.viewport.window_to_local(ev.position);
        self.handle_event(FlowEvent::Input(InputEvent::Wheel(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_canvas_hover(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if !*hovered {
            if let Some(sync_plugin) = &mut self.sync_plugin {
                sync_plugin.on_mouse_leave();
            }
            cx.notify();
        }
    }
}

impl Render for FlowCanvas {
    fn render(&mut self, window: &mut Window, this_cx: &mut Context<Self>) -> impl IntoElement {
        self.viewport.sync_drawable_bounds(window);

        let entity = this_cx.entity();

        let graph = &mut self.graph;
        let viewport = &self.viewport;
        let renderder = &self.renderers;
        let port_offset_cache = &mut self.port_offset_cache;
        let theme = &self.theme;
        let shared_state = &self.shared_state;

        let mut layers: Vec<Vec<AnyElement>> =
            (0..RenderLayer::ALL.len()).map(|_| Vec::new()).collect();

        for plugin in self.plugins_registry.plugins.iter_mut() {
            let layer = plugin.render_layer();

            let mut ctx = RenderContext::new(
                graph,
                port_offset_cache,
                viewport,
                renderder,
                entity.clone(),
                window,
                layer,
                theme,
                shared_state,
            );

            if let Some(el) = plugin.render(&mut ctx) {
                layers[layer.index()].push(el);
            }
        }

        if let Some(i) = self.interaction.handler.as_ref() {
            let mut ctx = RenderContext::new(
                graph,
                port_offset_cache,
                viewport,
                renderder,
                entity.clone(),
                window,
                RenderLayer::Interaction,
                theme,
                shared_state,
            );

            if let Some(el) = i.render(&mut ctx) {
                layers[RenderLayer::Interaction.index()].push(el);
            }
        }

        if let Some(sync_plugin) = &mut self.sync_plugin {
            let mut ctx = RenderContext::new(
                graph,
                port_offset_cache,
                viewport,
                renderder,
                entity.clone(),
                window,
                RenderLayer::Overlay,
                theme,
                shared_state,
            );
            let els = sync_plugin.render(&mut ctx);
            for el in els {
                layers[RenderLayer::Overlay.index()].push(el);
            }
        }

        let bounds_entity = entity.clone();
        let bounds_probe = canvas(
            move |bounds, _, cx| {
                bounds_entity.update(cx, |this, _| {
                    this.viewport.sync_canvas_bounds(bounds);
                });
            },
            move |_, _, _, _| {},
        )
        .size_full();
        div()
            .id("ferrum_flow_canvas")
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(window.listener_for(&entity, Self::on_key_down))
            .on_key_up(window.listener_for(&entity, Self::on_key_up))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&entity, Self::on_mouse_down),
            )
            .on_mouse_down(
                MouseButton::Right,
                window.listener_for(&entity, Self::on_mouse_down),
            )
            .on_mouse_move(window.listener_for(&entity, Self::on_mouse_move))
            .on_hover(window.listener_for(&entity, Self::on_canvas_hover))
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&entity, Self::on_mouse_up),
            )
            .on_scroll_wheel(window.listener_for(&entity, Self::on_scroll_wheel))
            .child(div().absolute().size_full().child(bounds_probe))
            .children(RenderLayer::ALL.iter().map(|layer| {
                div()
                    .absolute()
                    .size_full()
                    .children(layers[layer.index()].drain(..))
            }))
    }
}

pub struct FlowCanvasBuilder<'a, 'b> {
    graph: Graph,
    ctx: &'a mut Context<'b, FlowCanvas>,
    window: &'a Window,

    plugins: PluginRegistry,
    renderers: RendererRegistry,
    sync_plugin: Option<Box<dyn SyncPlugin + 'static>>,
    theme: FlowTheme,
}

impl<'a, 'b> FlowCanvasBuilder<'a, 'b> {
    /// register plugin
    pub fn plugin(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins = self.plugins.add(plugin);
        self
    }

    /// Registers several plugins in one call (each item is a `Box<dyn Plugin>`).
    ///
    /// Order is only relevant before [`Self::build`], which sorts by [`Plugin::priority`]. Prefer
    /// [`.plugin`](Self::plugin) for single plugins so the compiler boxes them for you.
    ///
    /// When building a list of heterogeneous plugin types, use an explicitly typed
    /// `Vec<Box<dyn Plugin>>` so each `Box::new(concrete)` coerces to the trait object.
    pub fn plugins(mut self, plugins: impl IntoIterator<Item = Box<dyn Plugin>>) -> Self {
        self.plugins.plugins.extend(plugins);
        self
    }

    /// Registers the **core** plugin set for editing a node graph on the canvas: background,
    /// selection, node drag, pan/zoom, node/edge rendering, port wiring, delete, and undo/redo
    /// ([`BackgroundPlugin`], [`SelectionPlugin`], [`NodeInteractionPlugin`], [`ViewportPlugin`],
    /// [`NodePlugin`], [`PortInteractionPlugin`], [`EdgePlugin`], [`DeletePlugin`], [`HistoryPlugin`]).
    ///
    /// Event order is determined by each plugin’s [`Plugin::priority`] when [`FlowCanvas::build`]
    /// runs (not by the order of calls to [`.plugin`](Self::plugin)). Add minimap, clipboard,
    /// context menu, etc. with [`.plugin`](Self::plugin) before or after this call.
    pub fn plugins_core(mut self) -> Self {
        self.plugins = self
            .plugins
            .add(BackgroundPlugin::new())
            .add(SelectionPlugin::new())
            .add(NodeInteractionPlugin::new())
            .add(ViewportPlugin::new())
            .add(NodePlugin::new())
            .add(PortInteractionPlugin::new())
            .add(EdgePlugin::new())
            .add(DeletePlugin::new())
            .add(HistoryPlugin::new());
        self
    }

    pub fn sync_plugin(mut self, plugin: impl SyncPlugin + 'static) -> Self {
        self.sync_plugin = Some(Box::new(plugin));
        self
    }

    /// register node renderer
    pub fn node_renderer<R>(mut self, name: impl Into<String>, renderer: R) -> Self
    where
        R: node_renderer::NodeRenderer + 'static,
    {
        self.renderers.register(name, renderer);
        self
    }

    /// Registers several [`NodeRenderer`](node_renderer::NodeRenderer) entries (each `Box<dyn …>`), same idea as [`Self::plugins`].
    pub fn node_renderers<S: Into<String>>(
        mut self,
        items: impl IntoIterator<Item = (S, Box<dyn node_renderer::NodeRenderer>)>,
    ) -> Self {
        for (name, renderer) in items {
            self.renderers.register_boxed(name, renderer);
        }
        self
    }

    /// Replace the default [`FlowTheme`] before plugins run [`Plugin::setup`](crate::plugin::Plugin::setup).
    pub fn theme(mut self, theme: FlowTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn build(self) -> FlowCanvas {
        let focus_handle = self.ctx.focus_handle();
        let drawable_size = self.window.viewport_size();

        let mut canvas = FlowCanvas {
            graph: self.graph,
            viewport: Viewport::new(),
            plugins_registry: self.plugins,
            sync_plugin: self.sync_plugin,
            renderers: self.renderers,
            focus_handle,
            interaction: InteractionState::new(),
            history: Box::new(LocalHistory::new()),
            event_queue: vec![],
            port_offset_cache: PortLayoutCache::new(),
            theme: self.theme,
            shared_state: SharedState::new(),
        };

        if let Some(sync_plugin) = &mut canvas.sync_plugin {
            let (change_sender, mut change_receiver) = mpsc::unbounded::<GraphChange>();

            self.ctx
                .spawn(async move |this, ctx| {
                    while let Some(change) = change_receiver.next().await {
                        let _ = this.update(ctx, |this, cx| {
                            invalidate_port_layout_cache_for_graph_change(
                                &mut this.port_offset_cache,
                                &this.graph,
                                &change.kind,
                            );
                            this.graph.apply(change.kind);
                            cx.notify();
                        });
                    }
                })
                .detach();
            sync_plugin.setup(change_sender);
        }

        canvas
            .plugins_registry
            .plugins
            .sort_by_key(|p| -p.priority());

        {
            let mut ctx = InitPluginContext {
                graph: &mut canvas.graph,
                port_offset_cache: &mut canvas.port_offset_cache,
                viewport: &mut canvas.viewport,
                renderers: &mut canvas.renderers,
                gpui_ctx: &self.ctx,
                drawable_size,
                theme: &mut canvas.theme,
                shared_state: &mut canvas.shared_state,
                //notify: &mut notify,
            };

            for plugin in &mut canvas.plugins_registry.plugins {
                plugin.setup(&mut ctx);
            }
        }

        canvas
    }
}
