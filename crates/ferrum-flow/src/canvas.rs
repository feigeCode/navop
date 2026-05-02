use futures::{StreamExt, channel::mpsc};
use gpui::*;
use std::collections::BTreeMap;
use std::panic;
use std::time::Duration;

use crate::{
    BackgroundPlugin, DeletePlugin, EdgePlugin, FlowTheme, GraphChange, HistoryPlugin,
    NodeInteractionPlugin, NodePlugin, PortInteractionPlugin, SelectionPlugin, SharedState,
    SyncPlugin, SyncPluginContext, ViewportPlugin,
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
    delayed_notify_tx: mpsc::UnboundedSender<()>,

    canvas_origin: Point<Pixels>,
    fitted_to_canvas_bounds: bool,
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
    fn init_delayed_notify_channel(&mut self, cx: &mut Context<Self>) {
        let (tx, mut rx) = mpsc::unbounded::<()>();
        self.delayed_notify_tx = tx;
        cx.spawn(async move |this, ctx| {
            while rx.next().await.is_some() {
                let _ = this.update(ctx, |_, cx| {
                    cx.notify();
                });
            }
        })
        .detach();
    }

    #[deprecated(note = "use builder instead")]
    pub fn new(graph: Graph, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let (delayed_notify_tx, _rx) = mpsc::unbounded::<()>();
        let mut canvas = Self {
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
            delayed_notify_tx,
            canvas_origin: Point::default(),
            fitted_to_canvas_bounds: false,
        };
        canvas.init_delayed_notify_channel(cx);
        canvas
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

    /// If there is an active [`Interaction`], deliver `MouseMove` / `MouseUp` only to it and return
    /// `true` so the plugin chain is skipped for this dispatch (avoids duplicate handling and keeps
    /// drag ownership consistent, including for [`Self::process_event_queue`]).
    fn dispatch_interaction_pointer(&mut self, event: &FlowEvent, cx: &mut Context<Self>) -> bool {
        let event_queue = &mut self.event_queue;
        let mut emit = |e| event_queue.push(e);
        let mut notify = || cx.notify();
        let delayed_notify_tx = self.delayed_notify_tx.clone();
        let mut schedule_after = move |delay: Duration| {
            let tx = delayed_notify_tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let _ = tx.unbounded_send(());
            });
        };
        match event {
            FlowEvent::Input(InputEvent::MouseMove(ev)) => {
                let Some(mut handler) = self.interaction.handler.take() else {
                    return false;
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
                    &mut schedule_after,
                );
                let result = handler.on_mouse_move(ev, &mut ctx);
                match result {
                    InteractionResult::Continue => self.interaction.handler = Some(handler),
                    InteractionResult::End => self.interaction.handler = None,
                    InteractionResult::Replace(h) => self.interaction.handler = Some(h),
                }
                true
            }
            FlowEvent::Input(InputEvent::MouseUp(ev)) => {
                let Some(mut handler) = self.interaction.handler.take() else {
                    return false;
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
                    &mut schedule_after,
                );
                let result = handler.on_mouse_up(ev, &mut ctx);
                match result {
                    InteractionResult::Continue => self.interaction.handler = Some(handler),
                    InteractionResult::End => self.interaction.handler = None,
                    InteractionResult::Replace(h) => self.interaction.handler = Some(h),
                }
                true
            }
            _ => false,
        }
    }

    pub fn handle_event(&mut self, event: FlowEvent, cx: &mut Context<Self>) {
        if let Some(sync_plugin) = &mut self.sync_plugin {
            let mut ctx = SyncPluginContext::new(&self.viewport);
            sync_plugin.on_event(&event, &mut ctx);
        }

        // Pointer stream is owned by the active [`Interaction`]; do not also give Move/Up to plugins.
        if self.dispatch_interaction_pointer(&event, cx) {
            return;
        }

        let event_queue = &mut self.event_queue;
        let mut emit = |e| event_queue.push(e);
        let mut notify = || cx.notify();
        let delayed_notify_tx = self.delayed_notify_tx.clone();
        let mut schedule_after = move |delay: Duration| {
            let tx = delayed_notify_tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let _ = tx.unbounded_send(());
            });
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
            &mut schedule_after,
        );

        for plugin in self.plugins_registry.iter_mut() {
            let result = plugin.on_event(&event, &mut ctx);
            match result {
                EventResult::Continue => {}
                EventResult::Stop => break,
            }
        }
    }

    fn process_event_queue(&mut self, cx: &mut Context<Self>) {
        while let Some(event) = self.event_queue.pop() {
            if let Some(sync_plugin) = &mut self.sync_plugin {
                let mut ctx = SyncPluginContext::new(&self.viewport);
                sync_plugin.on_event(&event, &mut ctx);
            }

            if self.dispatch_interaction_pointer(&event, cx) {
                continue;
            }

            let event_queue = &mut self.event_queue;
            let mut emit = |e| event_queue.push(e);
            let mut notify = || cx.notify();
            let delayed_notify_tx = self.delayed_notify_tx.clone();
            let mut schedule_after = |delay: Duration| {
                let tx = delayed_notify_tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    let _ = tx.unbounded_send(());
                });
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
                &mut schedule_after,
            );

            for plugin in self.plugins_registry.iter_mut() {
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

    fn local_point(&self, position: Point<Pixels>) -> Point<Pixels> {
        position - self.canvas_origin
    }

    fn local_mouse_down_event(&self, ev: &MouseDownEvent) -> MouseDownEvent {
        let mut ev = ev.clone();
        ev.position = self.local_point(ev.position);
        ev
    }

    fn local_mouse_move_event(&self, ev: &MouseMoveEvent) -> MouseMoveEvent {
        let mut ev = ev.clone();
        ev.position = self.local_point(ev.position);
        ev
    }

    fn local_mouse_up_event(&self, ev: &MouseUpEvent) -> MouseUpEvent {
        let mut ev = ev.clone();
        ev.position = self.local_point(ev.position);
        ev
    }

    fn local_scroll_wheel_event(&self, ev: &ScrollWheelEvent) -> ScrollWheelEvent {
        let mut ev = ev.clone();
        ev.position = self.local_point(ev.position);
        ev
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(
            FlowEvent::Input(InputEvent::MouseDown(self.local_mouse_down_event(ev))),
            cx,
        );
        self.process_event_queue(cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(
            FlowEvent::Input(InputEvent::MouseMove(self.local_mouse_move_event(ev))),
            cx,
        );
        self.process_event_queue(cx);
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(
            FlowEvent::Input(InputEvent::MouseUp(self.local_mouse_up_event(ev))),
            cx,
        );
        self.process_event_queue(cx);
    }

    fn on_scroll_wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(
            FlowEvent::Input(InputEvent::Wheel(self.local_scroll_wheel_event(ev))),
            cx,
        );
        self.process_event_queue(cx);
    }

    fn on_canvas_hover(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(FlowEvent::Input(InputEvent::Hover(*hovered)), cx);
        self.process_event_queue(cx);
    }
}

impl FlowCanvas {
    fn set_canvas_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.canvas_origin = bounds.origin;
        self.viewport
            .set_window_bounds(Some(Bounds::new(Point::default(), bounds.size)));
        if !self.fitted_to_canvas_bounds {
            self.fit_to_canvas_bounds(bounds.size);
            self.fitted_to_canvas_bounds = true;
        }
    }

    fn fit_to_canvas_bounds(&mut self, size: Size<Pixels>) {
        let Some((x, y, width, height)) = self.graph_world_bounds() else {
            return;
        };
        let canvas_width: f32 = size.width.into();
        let canvas_height: f32 = size.height.into();
        if canvas_width <= 0.0 || canvas_height <= 0.0 {
            return;
        }

        let available_width = canvas_width * 0.84;
        let available_height = canvas_height * 0.84;
        let zoom = (available_width / width.max(1.0))
            .min(available_height / height.max(1.0))
            .clamp(0.7, 3.0);
        let center_x = x + width / 2.0;
        let center_y = y + height / 2.0;
        self.viewport.set_zoom(zoom);
        self.viewport.set_offset(Point::new(
            px(canvas_width / 2.0 - center_x * zoom),
            px(canvas_height / 2.0 - center_y * zoom),
        ));
    }

    fn graph_world_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut has_node = false;

        for node in self.graph.nodes().values() {
            let (x, y) = node.position();
            let size = node.size_ref();
            let x: f32 = x.into();
            let y: f32 = y.into();
            let width: f32 = size.width.into();
            let height: f32 = size.height.into();
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + width);
            max_y = max_y.max(y + height);
            has_node = true;
        }

        has_node.then_some((min_x, min_y, max_x - min_x, max_y - min_y))
    }
}

pub struct FlowCanvasElement {
    canvas: Entity<FlowCanvas>,
}

impl FlowCanvasElement {
    pub fn new(canvas: Entity<FlowCanvas>) -> Self {
        Self { canvas }
    }
}

impl IntoElement for FlowCanvasElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FlowCanvasElement {
    type RequestLayoutState = (LayoutId, AnyElement);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("ferrum_flow_canvas_element".into())
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.canvas.clone().into_any_element();
        let layout_id = child.request_layout(window, cx);
        (layout_id, (layout_id, child))
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.canvas.update(cx, |canvas, _| {
            canvas.set_canvas_bounds(bounds);
        });
        let (_, child) = request_layout;
        child.prepaint_at(bounds.origin, window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _child: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (_, child) = _request_layout;
        child.paint(window, cx);
    }
}

impl Render for FlowCanvas {
    fn render(&mut self, window: &mut Window, this_cx: &mut Context<Self>) -> impl IntoElement {
        let entity = this_cx.entity();

        let graph = &mut self.graph;
        let viewport = &self.viewport;
        let renderers = &self.renderers;
        let port_offset_cache = &mut self.port_offset_cache;
        let theme = &self.theme;
        let shared_state = &self.shared_state;

        let mut layers: Vec<Vec<AnyElement>> =
            (0..RenderLayer::ALL.len()).map(|_| Vec::new()).collect();

        for plugin in self.plugins_registry.iter_mut() {
            let mut ctx = RenderContext::new(
                graph,
                port_offset_cache,
                viewport,
                renderers,
                window,
                theme,
                shared_state,
            );

            if let Some(el) = plugin.render(&mut ctx) {
                layers[plugin.render_layer().index()].push(el);
            }
        }

        if let Some(i) = self.interaction.handler.as_ref() {
            let mut ctx = RenderContext::new(
                graph,
                port_offset_cache,
                viewport,
                renderers,
                window,
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
                renderers,
                window,
                theme,
                shared_state,
            );
            let els = sync_plugin.render(&mut ctx);
            for el in els {
                layers[RenderLayer::Overlay.index()].push(el);
            }
        }

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
            .children(RenderLayer::ALL.iter().map(|layer| {
                div()
                    .id(ElementId::Integer(layer.index() as u64))
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
        self.plugins.extend_boxed(plugins);
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
    pub fn default_plugins(mut self) -> Self {
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
        let mut duplicate_plugins: BTreeMap<&'static str, usize> = BTreeMap::new();
        for plugin in self.plugins.iter() {
            *duplicate_plugins.entry(plugin.name()).or_insert(0) += 1;
        }
        for (name, count) in duplicate_plugins
            .into_iter()
            .filter(|(_, count)| *count > 1)
        {
            eprintln!(
                "warning: plugin '{name}' is registered {count} times; this can cause duplicated event handling"
            );
        }

        let focus_handle = self.ctx.focus_handle();
        let drawable_size = self.window.viewport_size();
        let (delayed_notify_tx, _rx) = mpsc::unbounded::<()>();

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
            delayed_notify_tx,
            canvas_origin: Point::default(),
            fitted_to_canvas_bounds: false,
        };
        canvas.init_delayed_notify_channel(self.ctx);

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

        canvas.plugins_registry.sort_by_priority_desc();

        {
            let mut ctx = InitPluginContext::new(
                &mut canvas.graph,
                &mut canvas.port_offset_cache,
                &mut canvas.viewport,
                &mut canvas.renderers,
                self.ctx,
                drawable_size,
                &mut canvas.theme,
                &mut canvas.shared_state,
            );

            for plugin in canvas.plugins_registry.iter_mut() {
                plugin.setup(&mut ctx);
            }
        }

        canvas
    }
}
