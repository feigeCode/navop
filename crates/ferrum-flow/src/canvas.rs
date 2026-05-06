use futures::{StreamExt, channel::mpsc};
use gpui::*;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
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

/// Host-side callback for **outbound** [`FlowEvent`]s: invoked synchronously whenever a plugin calls
/// [`PluginContext::emit`](crate::plugin::PluginContext::emit) with the same event that is then
/// enqueued for the internal plugin pipeline ([`FlowCanvas::event_queue`]).
///
/// # Parent / shell integration
///
/// This is **not** a GPUI `subscribe` / `observe` stream: you install **one** `FnMut` on the canvas
/// ([`FlowCanvasBuilder::outbound`] or [`FlowCanvas::set_outbound`]). The closure runs on the **UI
/// thread**, **before** the event is pushed onto [`FlowCanvas::event_queue`], and receives a
/// **read-only** reference for inspection ([`FlowEvent::as_custom`]).
///
/// **Typical patterns**
///
/// - **Shared counters or queues** — [`std::sync::Arc`] + [`std::sync::atomic::AtomicUsize`] /
///   [`std::sync::Mutex`] / `mpsc` sender; the host view reads them in [`gpui::Render`] (see the
///   `outbound_host` example in this crate).
/// - **Refresh a parent `Entity`** — capture `gpui::Entity<YourShell>` (or a weak handle) in the
///   closure and call [`gpui::Entity::update`] + [`gpui::Context::notify`] after filtering with
///   `as_custom::<T>()`.
/// - **Graph edits without `emit`** — outbound does **not** run for plain [`FlowCanvas::dispatch_command`]
///   unless a plugin later `emit`s; also use [`gpui::Context::observe`] on `Entity<FlowCanvas>` for
///   those cases.
///
/// ```ignore
/// use ferrum_flow::{FlowCanvas, FlowEvent};
/// use gpui::{Context, Entity};
///
/// fn wire(canvas: &Entity<FlowCanvas>, shell: &Entity<MyShell>, cx: &mut Context<MyShell>) {
///     let shell = shell.clone();
///     canvas.update(cx, |canvas, cx| {
///         canvas.set_outbound(Some(Box::new(move |ev: &FlowEvent| {
///             if ev.as_custom::<MyPayload>().is_some() {
///                 let _ = shell.update(cx, |_, cx| cx.notify());
///             }
///         })));
///     });
/// }
/// ```
pub type FlowCanvasOutbound = Box<dyn FnMut(&FlowEvent) + Send + 'static>;

fn enqueue_plugin_emit(
    outbound: &mut Option<FlowCanvasOutbound>,
    queue: &mut Vec<FlowEvent>,
    e: FlowEvent,
) {
    if let Some(h) = outbound.as_mut() {
        h(&e);
    }
    queue.push(e);
}

pub struct FlowCanvas {
    graph: Graph,

    pub(crate) viewport: Viewport,

    pub(crate) plugins_registry: PluginRegistry,

    pub(crate) sync_plugin: Option<Box<dyn SyncPlugin + 'static>>,

    renderers: RendererRegistry,

    pub(crate) focus_handle: FocusHandle,

    pub(crate) interaction: InteractionState,

    pub history: Box<dyn HistoryProvider>,

    event_queue: Vec<FlowEvent>,
    port_offset_cache: PortLayoutCache,

    /// Visual tokens for canvas chrome; plugins adjust via [`InitPluginContext::theme`](crate::plugin::InitPluginContext::theme).
    theme: FlowTheme,

    /// Type-erased map for cross-plugin data on this canvas instance.
    shared_state: SharedState,
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    delayed_notify_tx: mpsc::UnboundedSender<()>,

    /// Optional host hook for every plugin [`PluginContext::emit`](crate::plugin::PluginContext::emit).
    outbound: Option<FlowCanvasOutbound>,
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
            canvas_bounds: Rc::new(Cell::new(None)),
            delayed_notify_tx,
            outbound: None,
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
            outbound: None,
        }
    }

    /// If there is an active [`Interaction`], deliver `MouseMove` / `MouseUp` only to it and return
    /// `true` so the plugin chain is skipped for this dispatch (avoids duplicate handling and keeps
    /// drag ownership consistent, including for [`Self::process_event_queue`]).
    fn dispatch_interaction_pointer(&mut self, event: &FlowEvent, cx: &mut Context<Self>) -> bool {
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
                let outbound = &mut self.outbound;
                let event_queue = &mut self.event_queue;
                let mut emit = |e| enqueue_plugin_emit(outbound, event_queue, e);
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
                let outbound = &mut self.outbound;
                let event_queue = &mut self.event_queue;
                let mut emit = |e| enqueue_plugin_emit(outbound, event_queue, e);
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

    fn handle_event(&mut self, event: FlowEvent, cx: &mut Context<Self>) {
        if let Some(sync_plugin) = &mut self.sync_plugin {
            let mut ctx = SyncPluginContext::new(&self.viewport);
            sync_plugin.on_event(&event, &mut ctx);
        }

        // Pointer stream is owned by the active [`Interaction`]; do not also give Move/Up to plugins.
        if self.dispatch_interaction_pointer(&event, cx) {
            return;
        }

        let outbound = &mut self.outbound;
        let event_queue = &mut self.event_queue;
        let mut emit = |e| enqueue_plugin_emit(outbound, event_queue, e);
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

    /// Same [`PluginContext`] wiring as input dispatch, for **inbound** control from other GPUI
    /// entities (toolbar, palette, automation) without touching `graph` directly.
    ///
    /// Use from `Entity::update`:
    ///
    /// ```ignore
    /// canvas_entity.update(cx, |canvas, cx| {
    ///     canvas.dispatch_command(CreateNode::new(node), cx);
    /// });
    /// ```
    fn with_plugin_context_for_dispatch(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut PluginContext<'_>),
    ) {
        let outbound = &mut self.outbound;
        let event_queue = &mut self.event_queue;
        let mut emit = |e| enqueue_plugin_emit(outbound, event_queue, e);
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
        f(&mut ctx);
    }

    /// Run a [`Command`] through the same path as plugins: local [`HistoryProvider`] or
    /// [`SyncPlugin::process_intent`], then redraw.
    ///
    /// Prefer this for graph edits so undo/redo and sync stay consistent.
    pub fn dispatch_command(&mut self, command: impl Command + 'static, cx: &mut Context<Self>) {
        self.with_plugin_context_for_dispatch(cx, |ctx| {
            ctx.execute_command(command);
        });
    }

    /// Undo the last command (same as plugin [`PluginContext::undo`]).
    pub fn dispatch_undo(&mut self, cx: &mut Context<Self>) {
        self.with_plugin_context_for_dispatch(cx, |ctx| {
            ctx.undo();
        });
    }

    /// Redo (same as plugin [`PluginContext::redo`]).
    pub fn dispatch_redo(&mut self, cx: &mut Context<Self>) {
        self.with_plugin_context_for_dispatch(cx, |ctx| {
            ctx.redo();
        });
    }

    /// Replace or clear the outbound hook ([`FlowCanvasOutbound`]). Prefer calling from
    /// `Entity<FlowCanvas>::update` once the canvas exists; see [`FlowCanvasOutbound`] for parent
    /// wiring and the `outbound_host` example in this crate.
    ///
    /// The hook runs on the same thread as input dispatch, **before** the event is pushed onto
    /// [`Self::event_queue`]. Graph changes that do not go through
    /// [`PluginContext::emit`](crate::plugin::PluginContext::emit) (for example plain
    /// [`Self::dispatch_command`] with no follow-up emit) are **not** reported here; use
    /// [`gpui::Context::observe`] on the canvas entity if you need those as well.
    pub fn set_outbound(&mut self, hook: Option<FlowCanvasOutbound>) {
        self.outbound = hook;
    }

    /// Read-only view of the document graph (nodes, edges, selection).
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Clone the graph for use outside the current `update` closure (e.g. async snapshots).
    pub fn graph_snapshot(&self) -> Graph {
        self.graph.clone()
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

            let outbound = &mut self.outbound;
            let event_queue = &mut self.event_queue;
            let mut emit = |e| enqueue_plugin_emit(outbound, event_queue, e);
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

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_viewport_to_canvas_bounds();
        let ev = self.mouse_down_event_in_canvas(ev);
        self.handle_event(FlowEvent::Input(InputEvent::MouseDown(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_viewport_to_canvas_bounds();
        let ev = self.mouse_move_event_in_canvas(ev);
        self.handle_event(FlowEvent::Input(InputEvent::MouseMove(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_viewport_to_canvas_bounds();
        let ev = self.mouse_up_event_in_canvas(ev);
        self.handle_event(FlowEvent::Input(InputEvent::MouseUp(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_scroll_wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_viewport_to_canvas_bounds();
        let ev = self.scroll_wheel_event_in_canvas(ev);
        self.handle_event(FlowEvent::Input(InputEvent::Wheel(ev)), cx);
        self.process_event_queue(cx);
    }

    fn on_canvas_hover(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_event(FlowEvent::Input(InputEvent::Hover(*hovered)), cx);
        self.process_event_queue(cx);
    }

    fn canvas_origin(&self) -> Point<Pixels> {
        self.canvas_bounds
            .get()
            .map(|bounds| bounds.origin)
            .unwrap_or(Point::new(px(0.0), px(0.0)))
    }

    fn sync_viewport_to_canvas_bounds(&mut self) {
        if let Some(bounds) = self.canvas_bounds.get() {
            self.viewport.sync_canvas_bounds(bounds);
        }
    }

    fn mouse_down_event_in_canvas(&self, ev: &MouseDownEvent) -> MouseDownEvent {
        let mut ev = ev.clone();
        ev.position = window_point_to_canvas_point(ev.position, self.canvas_origin());
        ev
    }

    fn mouse_move_event_in_canvas(&self, ev: &MouseMoveEvent) -> MouseMoveEvent {
        let mut ev = ev.clone();
        ev.position = window_point_to_canvas_point(ev.position, self.canvas_origin());
        ev
    }

    fn mouse_up_event_in_canvas(&self, ev: &MouseUpEvent) -> MouseUpEvent {
        let mut ev = ev.clone();
        ev.position = window_point_to_canvas_point(ev.position, self.canvas_origin());
        ev
    }

    fn scroll_wheel_event_in_canvas(&self, ev: &ScrollWheelEvent) -> ScrollWheelEvent {
        let mut ev = ev.clone();
        ev.position = window_point_to_canvas_point(ev.position, self.canvas_origin());
        ev
    }
}

impl Render for FlowCanvas {
    fn render(&mut self, window: &mut Window, this_cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(bounds) = self.canvas_bounds.get() {
            self.viewport.sync_canvas_bounds(bounds);
        } else {
            self.viewport.sync_drawable_bounds(window);
        }

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

        let root = div()
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
            }));

        CanvasRootElement::new(root, Rc::clone(&self.canvas_bounds))
    }
}

fn window_point_to_canvas_point(
    window_point: Point<Pixels>,
    canvas_origin: Point<Pixels>,
) -> Point<Pixels> {
    window_point - canvas_origin
}

struct CanvasRootElement<E> {
    element: E,
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl<E> CanvasRootElement<E> {
    fn new(element: E, canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>) -> Self {
        Self {
            element,
            canvas_bounds,
        }
    }
}

impl<E> IntoElement for CanvasRootElement<E>
where
    E: Element,
{
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E> Element for CanvasRootElement<E>
where
    E: Element,
{
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.element.id()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.element.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.canvas_bounds.set(Some(bounds));
        self.element
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Point, px};

    use super::window_point_to_canvas_point;

    #[test]
    fn window_point_to_canvas_point_subtracts_canvas_origin() {
        let window_point = Point::new(px(360.0), px(140.0));
        let canvas_origin = Point::new(px(320.0), px(96.0));

        let canvas_point = window_point_to_canvas_point(window_point, canvas_origin);

        assert_eq!(canvas_point, Point::new(px(40.0), px(44.0)));
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
    outbound: Option<FlowCanvasOutbound>,
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

    /// Register an outbound hook: invoked for every [`PluginContext::emit`](crate::plugin::PluginContext::emit)
    /// on this canvas (same as [`FlowCanvas::set_outbound`]). See [`FlowCanvasOutbound`] and the
    /// `outbound_host` example for how a parent view can react (e.g. `Arc<AtomicUsize>` or
    /// `Entity::update` on a shell).
    pub fn outbound(mut self, hook: impl FnMut(&FlowEvent) + Send + 'static) -> Self {
        self.outbound = Some(Box::new(hook));
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
            canvas_bounds: Rc::new(Cell::new(None)),
            delayed_notify_tx,
            outbound: self.outbound,
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
