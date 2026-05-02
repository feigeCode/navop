use std::{any::Any, collections::HashMap, time::Duration};

use gpui::{
    AnyElement, Bounds, Context, Div, ElementId, InteractiveElement as _, KeyDownEvent, KeyUpEvent,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, Size, Stateful,
    Styled, Window, div, rgb,
};

use crate::{
    Edge, EdgeBuilder, EdgeId, FlowCanvas, FlowTheme, Graph, GraphOp, Node, NodeBuilder, NodeId,
    NodeRenderer, Port, PortId, PortPosition, RendererRegistry, SharedState, Viewport,
    canvas::{
        Command, CommandContext, HistoryProvider, Interaction, InteractionState, PortLayoutCache,
    },
    port_screen::PortScreenFrame,
};

mod sync;
mod utils;

pub use sync::{SyncPlugin, SyncPluginContext};

pub use utils::{
    invalidate_port_layout_cache_for_graph_change, is_edge_visible, is_node_visible,
    primary_platform_modifier,
};

/// Chrome for [`RenderContext::node_card_shell`]. [`NodeCardVariant::Default`] and
/// [`NodeCardVariant::UndefinedType`] read colors from [`RenderContext::theme`]; plugins may change
/// them via [`InitPluginContext::theme`] / [`PluginContext::theme`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCardVariant {
    /// Card from [`FlowTheme::node_card_background`] and border from [`FlowTheme::node_card_border`]
    /// / [`FlowTheme::node_card_border_selected`] when `selected`.
    Default,
    /// Card from [`FlowTheme::undefined_node_background`] and [`FlowTheme::undefined_node_border`]
    /// (no selection styling).
    UndefinedType,

    /// Geometry and border width only; set `.bg` / `.border_color` yourself.
    Custom,
}

pub trait Plugin {
    fn name(&self) -> &'static str;

    fn setup(&mut self, _ctx: &mut InitPluginContext) {}

    fn on_event(&mut self, _event: &FlowEvent, _ctx: &mut PluginContext) -> EventResult {
        EventResult::Continue
    }

    fn render(&mut self, _ctx: &mut RenderContext) -> Option<AnyElement> {
        None
    }

    fn priority(&self) -> i32 {
        0
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }
}

pub struct InitPluginContext<'a, 'b> {
    graph: &'a mut Graph,
    port_offset_cache: &'a mut PortLayoutCache,
    viewport: &'a mut Viewport,
    renderers: &'a mut RendererRegistry,
    pub gpui_ctx: &'a Context<'b, FlowCanvas>,
    /// Drawable size from the `window` passed to [`FlowCanvas::builder`] (`Window::viewport_size` when `build()` runs).
    pub drawable_size: Size<Pixels>,
    /// Canvas colors and strokes; mutate in [`Plugin::setup`](Plugin::setup) to customize chrome.
    pub theme: &'a mut FlowTheme,
    /// Plugin-local shared state on the [`FlowCanvas`](FlowCanvas).
    pub shared_state: &'a mut SharedState,
    // pub notify: &'a mut dyn FnMut(),
}

impl<'a, 'b> InitPluginContext<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        graph: &'a mut Graph,
        port_offset_cache: &'a mut PortLayoutCache,
        viewport: &'a mut Viewport,
        renderers: &'a mut RendererRegistry,
        gpui_ctx: &'a Context<'b, FlowCanvas>,
        drawable_size: Size<Pixels>,
        theme: &'a mut FlowTheme,
        shared_state: &'a mut SharedState,
    ) -> Self {
        Self {
            graph,
            port_offset_cache,
            viewport,
            renderers,
            gpui_ctx,
            drawable_size,
            theme,
            shared_state,
        }
    }
    pub fn create_node(&mut self, node_type: &str) -> NodeBuilder<'_> {
        self.graph.create_node(node_type)
    }

    pub fn create_edge(&mut self) -> EdgeBuilder<'_> {
        self.graph.create_edge()
    }

    pub fn next_node_id(&self) -> NodeId {
        self.graph.next_node_id()
    }

    pub fn next_port_id(&self) -> PortId {
        self.graph.next_port_id()
    }

    pub fn next_edge_id(&self) -> EdgeId {
        self.graph.next_edge_id()
    }

    pub fn add_node(&mut self, node: Node) {
        self.graph.add_node(node);
    }

    pub fn add_port(&mut self, port: Port) {
        self.graph.add_port(port);
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.get_node(id)
    }

    pub fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.graph.get_node_mut(id)
    }
    pub fn remove_node(&mut self, id: &NodeId) {
        self.graph.remove_node(id);
    }
    pub fn nodes(&self) -> &HashMap<NodeId, Node> {
        self.graph.nodes()
    }
    pub fn node_order(&self) -> &Vec<NodeId> {
        self.graph.node_order()
    }

    pub fn new_edge(&self) -> Edge {
        self.graph.new_edge()
    }

    pub fn add_edge(&mut self, edge: Edge) {
        self.graph.add_edge(edge);
    }

    pub fn remove_edge(&mut self, edge_id: &EdgeId) {
        self.graph.remove_edge(edge_id);
    }

    pub fn add_selected_node(&mut self, id: NodeId, shift: bool) {
        self.graph.add_selected_node(id, shift);
    }
    pub fn clear_selected_node(&mut self) {
        self.graph.clear_selected_node();
    }
    pub fn remove_selected_node(&mut self) -> bool {
        self.graph.remove_selected_node()
    }

    pub fn add_selected_edge(&mut self, id: EdgeId, shift: bool) {
        self.graph.add_selected_edge(id, shift);
    }
    pub fn clear_selected_edge(&mut self) {
        self.graph.clear_selected_edge();
    }
    pub fn remove_selected_edge(&mut self) -> bool {
        self.graph.remove_selected_edge()
    }

    pub fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        self.graph.selection_bounds()
    }

    pub fn selected_nodes_with_positions(&self) -> HashMap<NodeId, Point<Pixels>> {
        self.graph.selected_nodes_with_positions()
    }

    pub fn hit_node(&self, mouse: Point<Pixels>) -> Option<NodeId> {
        self.graph.hit_node(mouse, self.viewport)
    }

    pub fn bring_node_to_front(&mut self, node_id: NodeId) {
        self.graph.bring_node_to_front(node_id);
    }

    // ---- Viewport shortcuts ----
    pub fn zoom(&self) -> f32 {
        self.viewport.zoom()
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.viewport.set_zoom(zoom);
    }

    pub fn zoom_scaled_by(&self, factor: f32) -> f32 {
        self.viewport.zoom_scaled_by(factor)
    }

    pub fn offset(&self) -> Point<Pixels> {
        self.viewport.offset()
    }

    pub fn set_offset(&mut self, offset: Point<Pixels>) {
        self.viewport.set_offset(offset);
    }

    pub fn set_offset_xy(&mut self, x: Pixels, y: Pixels) {
        self.viewport.set_offset_xy(x, y);
    }

    pub fn translate_offset(&mut self, dx: Pixels, dy: Pixels) {
        self.viewport.translate_offset(dx, dy);
    }

    pub fn window_bounds(&self) -> Option<Bounds<Pixels>> {
        self.viewport.window_bounds()
    }

    pub fn set_window_bounds(&mut self, bounds: Option<Bounds<Pixels>>) {
        self.viewport.set_window_bounds(bounds);
    }

    pub fn world_scalar_to_screen(&self, value: f32) -> f32 {
        self.viewport.world_scalar_to_screen(value)
    }

    pub fn screen_scalar_to_world(&self, value: f32) -> f32 {
        self.viewport.screen_scalar_to_world(value)
    }

    pub fn world_length_to_screen(&self, value: Pixels) -> Pixels {
        self.viewport.world_length_to_screen(value)
    }

    pub fn screen_length_to_world(&self, value: Pixels) -> Pixels {
        self.viewport.screen_length_to_world(value)
    }

    pub fn world_to_screen(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.world_to_screen(p)
    }

    pub fn screen_to_world(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.screen_to_world(p)
    }

    pub fn edge_control_point(
        &self,
        source: Point<Pixels>,
        position: PortPosition,
    ) -> Point<Pixels> {
        self.viewport.edge_control_point(source, position)
    }

    pub fn is_node_visible(&self, node_id: &NodeId) -> bool {
        is_node_visible(self.graph, self.viewport, node_id)
    }
    pub fn is_node_visible_node(&self, node: &Node) -> bool {
        self.viewport.is_node_visible(node)
    }

    pub fn is_edge_visible(&self, edge: &Edge) -> bool {
        is_edge_visible(self.graph, self.viewport, edge)
    }

    pub fn port_offset_cached(&self, node_id: &NodeId, port_id: &PortId) -> Option<Point<Pixels>> {
        self.port_offset_cache.get_offset(node_id, port_id)
    }

    /// Port center in screen pixels when you already have the owning [`Node`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center(&self, node: &Node, port_id: PortId) -> Option<Point<Pixels>> {
        let node_pos = node.point();
        let offset = self.port_offset_cached(&node.id(), &port_id)?;
        Some(self.viewport.world_to_screen(node_pos + offset))
    }

    /// Like [`Self::port_screen_center`], resolving the port from [`Graph::ports`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center_by_port_id(&self, port_id: PortId) -> Option<Point<Pixels>> {
        let port = self.graph.get_port(&port_id)?;
        let node = self.get_node(&port.node_id())?;
        self.port_screen_center(node, port_id)
    }

    /// Full port layout for custom [`NodeRenderer::port_render`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_frame(&self, node: &Node, port: &Port) -> Option<PortScreenFrame> {
        Some(PortScreenFrame {
            center: self.port_screen_center(node, port.id())?,
            size: *port.size_ref(),
            zoom: self.viewport.zoom(),
            port_id: port.id(),
        })
    }

    pub fn cache_port_offset_with_node(&mut self, node_ids: &Vec<NodeId>) {
        for node_id in node_ids {
            self.cache_node_port_offset(node_id);
        }
    }

    pub fn cache_port_offset_with_edge(&mut self, edge_id: &EdgeId) {
        self.port_offset_cache
            .ensure_edge_ports(self.graph, self.renderers, edge_id);
    }

    pub fn cache_port_offset_with_port(&mut self, port_id: &PortId) {
        self.port_offset_cache
            .ensure_node_ports_for_port(self.graph, self.renderers, port_id);
    }

    fn cache_node_port_offset(&mut self, node_id: &NodeId) {
        self.port_offset_cache
            .ensure_node_ports(self.graph, self.renderers, node_id);
    }
}

pub struct PluginContext<'a> {
    pub graph: &'a mut Graph,
    port_offset_cache: &'a mut PortLayoutCache,
    viewport: &'a mut Viewport,
    pub(crate) interaction: &'a mut InteractionState,
    renderers: &'a mut RendererRegistry,

    sync_plugin: &'a mut Option<Box<dyn SyncPlugin + 'static>>,

    history: &'a mut dyn HistoryProvider,
    /// Canvas theme; change during event handling and call [`PluginContext::notify`] to redraw.
    pub theme: &'a mut FlowTheme,
    /// Plugin-local shared state on the [`FlowCanvas`](FlowCanvas).
    pub shared_state: &'a mut SharedState,
    emit: &'a mut dyn FnMut(FlowEvent),
    notify: &'a mut dyn FnMut(),
    schedule_after: &'a mut dyn FnMut(Duration),
}

pub enum EventResult {
    Continue,
    Stop,
}

impl<'a> PluginContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        graph: &'a mut Graph,
        port_offset_cache: &'a mut PortLayoutCache,
        viewport: &'a mut Viewport,
        interaction: &'a mut InteractionState,
        renderers: &'a mut RendererRegistry,
        sync_plugin: &'a mut Option<Box<dyn SyncPlugin + 'static>>,
        history: &'a mut dyn HistoryProvider,
        theme: &'a mut FlowTheme,
        shared_state: &'a mut SharedState,
        emit: &'a mut dyn FnMut(FlowEvent),
        notify: &'a mut dyn FnMut(),
        schedule_after: &'a mut dyn FnMut(Duration),
    ) -> Self {
        Self {
            graph,
            port_offset_cache,
            viewport,
            interaction,
            renderers,
            sync_plugin,
            history,
            theme,
            shared_state,
            emit,
            notify,
            schedule_after,
        }
    }

    pub fn start_interaction(&mut self, handler: impl Interaction + 'static) {
        self.interaction.handler = Some(Box::new(handler));
    }

    pub fn cancel_interaction(&mut self) {
        self.interaction.handler = None;
    }

    pub fn has_interaction(&self) -> bool {
        self.interaction.handler.is_some()
    }

    /// Tell GPUI that this entity has changed and observers of it should be notified.
    pub fn notify(&mut self) {
        (self.notify)();
    }

    /// Schedule a future canvas refresh after a delay.
    pub fn schedule_after(&mut self, delay: Duration) {
        (self.schedule_after)(delay);
    }

    pub fn emit(&mut self, event: FlowEvent) {
        (self.emit)(event);
        self.notify();
    }

    pub fn has_sync_plugin(&self) -> bool {
        self.sync_plugin.is_some()
    }

    pub fn execute_command(&mut self, command: impl Command + 'static) {
        let mut ctx = CommandContext::new(
            self.graph,
            self.port_offset_cache,
            self.viewport,
            self.renderers,
            self.shared_state,
            self.notify,
        );
        if let Some(sync) = &mut self.sync_plugin {
            sync.process_intent(GraphOp::Batch(command.to_ops(&mut ctx)));

            self.notify();
        } else {
            self.history.push(Box::new(command), &mut ctx);

            self.notify();
        }
    }

    pub fn undo(&mut self) {
        if let Some(sync) = &mut self.sync_plugin {
            sync.undo();
        } else {
            let mut ctx = CommandContext::new(
                self.graph,
                self.port_offset_cache,
                self.viewport,
                self.renderers,
                self.shared_state,
                self.notify,
            );

            self.history.undo(&mut ctx);

            self.notify();
        }
    }

    pub fn redo(&mut self) {
        if let Some(sync) = &mut self.sync_plugin {
            sync.redo();
        } else {
            let mut ctx = CommandContext::new(
                self.graph,
                self.port_offset_cache,
                self.viewport,
                self.renderers,
                self.shared_state,
                self.notify,
            );

            self.history.redo(&mut ctx);

            self.notify();
        }
    }

    pub fn history_clear(&mut self) {
        self.history.clear();
    }

    pub fn create_node(&mut self, node_type: &str) -> NodeBuilder<'_> {
        self.graph.create_node(node_type)
    }

    pub fn create_edge(&mut self) -> EdgeBuilder<'_> {
        self.graph.create_edge()
    }

    pub fn next_node_id(&self) -> NodeId {
        self.graph.next_node_id()
    }

    pub fn next_port_id(&self) -> PortId {
        self.graph.next_port_id()
    }

    pub fn next_edge_id(&self) -> EdgeId {
        self.graph.next_edge_id()
    }

    pub fn add_node(&mut self, node: Node) {
        self.graph.add_node(node);
    }

    pub fn add_port(&mut self, port: Port) {
        self.graph.add_port(port);
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.get_node(id)
    }

    pub fn get_node_render(&self, id: &NodeId) -> Option<&dyn NodeRenderer> {
        let node = self.get_node(id)?;

        Some(self.renderers.get(node.renderer_key()))
    }

    pub fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.graph.get_node_mut(id)
    }
    pub fn remove_node(&mut self, id: &NodeId) {
        self.graph.remove_node(id);
        self.port_offset_cache.clear_node(id);
    }
    pub fn nodes(&self) -> &HashMap<NodeId, Node> {
        self.graph.nodes()
    }
    pub fn node_order(&self) -> &Vec<NodeId> {
        self.graph.node_order()
    }

    pub fn new_edge(&self) -> Edge {
        self.graph.new_edge()
    }

    pub fn add_edge(&mut self, edge: Edge) {
        self.graph.add_edge(edge);
    }

    pub fn remove_edge(&mut self, edge_id: &EdgeId) {
        self.graph.remove_edge(edge_id);
    }

    pub fn add_selected_node(&mut self, id: NodeId, shift: bool) {
        self.graph.add_selected_node(id, shift);
    }
    pub fn clear_selected_node(&mut self) {
        self.graph.clear_selected_node();
    }
    pub fn remove_selected_node(&mut self) -> bool {
        self.graph.remove_selected_node()
    }

    pub fn add_selected_edge(&mut self, id: EdgeId, shift: bool) {
        self.graph.add_selected_edge(id, shift);
    }
    pub fn clear_selected_edge(&mut self) {
        self.graph.clear_selected_edge();
    }
    pub fn remove_selected_edge(&mut self) -> bool {
        self.graph.remove_selected_edge()
    }

    pub fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        self.graph.selection_bounds()
    }

    pub fn selected_nodes_with_positions(&self) -> HashMap<NodeId, Point<Pixels>> {
        self.graph.selected_nodes_with_positions()
    }

    pub fn hit_node(&self, mouse: Point<Pixels>) -> Option<NodeId> {
        self.graph.hit_node(mouse, self.viewport)
    }

    pub fn bring_node_to_front(&mut self, node_id: NodeId) {
        self.graph.bring_node_to_front(node_id);
    }

    // ---- Viewport shortcuts ----
    pub fn zoom(&self) -> f32 {
        self.viewport.zoom()
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.viewport.set_zoom(zoom);
    }

    pub fn zoom_scaled_by(&self, factor: f32) -> f32 {
        self.viewport.zoom_scaled_by(factor)
    }

    pub fn offset(&self) -> Point<Pixels> {
        self.viewport.offset()
    }

    pub fn set_offset(&mut self, offset: Point<Pixels>) {
        self.viewport.set_offset(offset);
    }

    pub fn set_offset_xy(&mut self, x: Pixels, y: Pixels) {
        self.viewport.set_offset_xy(x, y);
    }

    pub fn translate_offset(&mut self, dx: Pixels, dy: Pixels) {
        self.viewport.translate_offset(dx, dy);
    }

    pub fn window_bounds(&self) -> Option<Bounds<Pixels>> {
        self.viewport.window_bounds()
    }

    pub fn set_window_bounds(&mut self, bounds: Option<Bounds<Pixels>>) {
        self.viewport.set_window_bounds(bounds);
    }

    pub fn world_scalar_to_screen(&self, value: f32) -> f32 {
        self.viewport.world_scalar_to_screen(value)
    }

    pub fn screen_scalar_to_world(&self, value: f32) -> f32 {
        self.viewport.screen_scalar_to_world(value)
    }

    pub fn world_length_to_screen(&self, value: Pixels) -> Pixels {
        self.viewport.world_length_to_screen(value)
    }

    pub fn screen_length_to_world(&self, value: Pixels) -> Pixels {
        self.viewport.screen_length_to_world(value)
    }

    pub fn world_to_screen(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.world_to_screen(p)
    }

    pub fn screen_to_world(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.screen_to_world(p)
    }

    pub fn edge_control_point(
        &self,
        source: Point<Pixels>,
        position: PortPosition,
    ) -> Point<Pixels> {
        self.viewport.edge_control_point(source, position)
    }

    pub fn is_node_visible(&self, node_id: &NodeId) -> bool {
        is_node_visible(self.graph, self.viewport, node_id)
    }
    pub fn is_node_visible_node(&self, node: &Node) -> bool {
        self.viewport.is_node_visible(node)
    }

    pub fn is_edge_visible(&self, edge: &Edge) -> bool {
        is_edge_visible(self.graph, self.viewport, edge)
    }

    pub fn port_offset_cached(&self, node_id: &NodeId, port_id: &PortId) -> Option<Point<Pixels>> {
        self.port_offset_cache.get_offset(node_id, port_id)
    }

    pub fn port_offset_cache_clear_all(&mut self) {
        self.port_offset_cache.clear_all();
    }

    /// Port center in screen pixels when you already have the owning [`Node`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center(&self, node: &Node, port_id: PortId) -> Option<Point<Pixels>> {
        let node_pos = node.point();
        let offset = self.port_offset_cached(&node.id(), &port_id)?;
        Some(self.viewport.world_to_screen(node_pos + offset))
    }

    /// Like [`Self::port_screen_center`], resolving the port from [`Graph::ports`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center_by_port_id(&self, port_id: PortId) -> Option<Point<Pixels>> {
        let port = self.graph.get_port(&port_id)?;
        let node = self.get_node(&port.node_id())?;
        self.port_screen_center(node, port_id)
    }

    /// Full port layout for custom [`NodeRenderer::port_render`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_frame(&self, node: &Node, port: &Port) -> Option<PortScreenFrame> {
        Some(PortScreenFrame {
            center: self.port_screen_center(node, port.id())?,
            size: *port.size_ref(),
            zoom: self.viewport.zoom(),
            port_id: port.id(),
        })
    }

    pub fn cache_all_node_port_offset(&mut self) {
        self.port_offset_cache
            .ensure_all_nodes_ports(self.graph, self.renderers);
    }

    pub fn cache_port_offset_with_node(&mut self, node_ids: &Vec<NodeId>) {
        for node_id in node_ids {
            self.cache_node_port_offset(node_id);
        }
    }

    pub fn cache_port_offset_with_edge(&mut self, edge_id: &EdgeId) {
        self.port_offset_cache
            .ensure_edge_ports(self.graph, self.renderers, edge_id);
    }

    pub fn cache_port_offset_with_port(&mut self, port_id: &PortId) {
        self.port_offset_cache
            .ensure_node_ports_for_port(self.graph, self.renderers, port_id);
    }

    fn cache_node_port_offset(&mut self, node_id: &NodeId) {
        self.port_offset_cache
            .ensure_node_ports(self.graph, self.renderers, node_id);
    }
}

pub enum FlowEvent {
    Input(InputEvent),
    Custom(Box<dyn std::any::Any + Send>),
}

impl FlowEvent {
    pub fn custom<T: 'static + Send>(event: T) -> Self {
        FlowEvent::Custom(Box::new(event))
    }
    pub fn as_custom<T: 'static>(&self) -> Option<&T> {
        match self {
            FlowEvent::Custom(e) => e.downcast_ref::<T>(),
            _ => None,
        }
    }
}

pub enum InputEvent {
    KeyDown(KeyDownEvent),
    KeyUp(KeyUpEvent),

    MouseDown(MouseDownEvent),
    MouseMove(MouseMoveEvent),
    MouseUp(MouseUpEvent),

    Wheel(ScrollWheelEvent),

    Hover(bool),
}

pub struct RenderContext<'a> {
    pub graph: &'a Graph,
    port_offset_cache: &'a mut PortLayoutCache,
    viewport: &'a Viewport,
    pub renderers: &'a RendererRegistry,

    pub window: &'a Window,
    /// Active canvas theme (from [`FlowCanvas::theme`](crate::canvas::FlowCanvas::theme)).
    pub theme: &'a FlowTheme,
    /// Read-only shared plugin state on the [`FlowCanvas`](FlowCanvas).
    shared_state: &'a SharedState,
}

impl<'a> RenderContext<'a> {
    pub(crate) fn new(
        graph: &'a mut Graph,
        port_offset_cache: &'a mut PortLayoutCache,
        viewport: &'a Viewport,
        renderers: &'a RendererRegistry,
        window: &'a Window,
        theme: &'a FlowTheme,
        shared_state: &'a SharedState,
    ) -> Self {
        Self {
            graph,
            port_offset_cache,
            viewport,
            renderers,
            window,
            theme,
            shared_state,
        }
    }

    /// Detached builder (no graph); use [`PluginContext::create_node`] or [`Graph::create_node`] to commit.
    pub fn create_node(&self, renderer_key: &str) -> NodeBuilder<'_> {
        NodeBuilder::new(renderer_key)
    }

    pub fn next_node_id(&self) -> NodeId {
        self.graph.next_node_id()
    }

    pub fn next_port_id(&self) -> PortId {
        self.graph.next_port_id()
    }

    pub fn next_edge_id(&self) -> EdgeId {
        self.graph.next_edge_id()
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.get_node(id)
    }

    pub fn get_node_render(&self, id: &NodeId) -> Option<&dyn NodeRenderer> {
        let node = self.get_node(id)?;

        Some(self.renderers.get(node.renderer_key()))
    }

    pub fn nodes(&self) -> &HashMap<NodeId, Node> {
        self.graph.nodes()
    }
    pub fn node_order(&self) -> &Vec<NodeId> {
        self.graph.node_order()
    }

    pub fn new_edge(&self) -> Edge {
        self.graph.new_edge()
    }

    pub fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        self.graph.selection_bounds()
    }

    pub fn selected_nodes_with_positions(&self) -> HashMap<NodeId, Point<Pixels>> {
        self.graph.selected_nodes_with_positions()
    }

    pub fn hit_node(&self, mouse: Point<Pixels>) -> Option<NodeId> {
        self.graph.hit_node(mouse, self.viewport)
    }

    // ---- Viewport shortcuts ----

    pub fn viewport(&self) -> &Viewport {
        self.viewport
    }

    pub fn zoom(&self) -> f32 {
        self.viewport.zoom()
    }

    pub fn zoom_scaled_by(&self, factor: f32) -> f32 {
        self.viewport.zoom_scaled_by(factor)
    }

    pub fn offset(&self) -> Point<Pixels> {
        self.viewport.offset()
    }

    pub fn window_bounds(&self) -> Option<Bounds<Pixels>> {
        self.viewport.window_bounds()
    }

    pub fn world_scalar_to_screen(&self, value: f32) -> f32 {
        self.viewport.world_scalar_to_screen(value)
    }

    pub fn screen_scalar_to_world(&self, value: f32) -> f32 {
        self.viewport.screen_scalar_to_world(value)
    }

    pub fn world_length_to_screen(&self, value: Pixels) -> Pixels {
        self.viewport.world_length_to_screen(value)
    }

    pub fn screen_length_to_world(&self, value: Pixels) -> Pixels {
        self.viewport.screen_length_to_world(value)
    }

    pub fn world_to_screen(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.world_to_screen(p)
    }

    /// Absolute-positioned node card shell: screen origin, zoom-scaled size.
    ///
    /// Chain `.child(...)` for the inner body, then `.into_any()` (see [`gpui::Element`]).
    pub fn node_card_shell(
        &self,
        node: &Node,
        selected: bool,
        variant: NodeCardVariant,
    ) -> Stateful<Div> {
        let screen = self.world_to_screen(node.point());
        let z = self.viewport.zoom();
        let base = div()
            .id(ElementId::Uuid(*node.id().as_uuid()))
            .absolute()
            .left(screen.x)
            .top(screen.y)
            .w(node.size_ref().width * z)
            .h(node.size_ref().height * z);
        let t = self.theme;
        match variant {
            NodeCardVariant::Default => {
                base.bg(rgb(t.node_card_background))
                    .border_color(rgb(if selected {
                        t.node_card_border_selected
                    } else {
                        t.node_card_border
                    }))
            }
            NodeCardVariant::UndefinedType => base
                .bg(rgb(t.undefined_node_background))
                .border_color(rgb(t.undefined_node_border)),
            NodeCardVariant::Custom => base,
        }
    }

    pub fn screen_to_world(&self, p: Point<Pixels>) -> Point<Pixels> {
        self.viewport.screen_to_world(p)
    }

    pub fn edge_control_point(
        &self,
        source: Point<Pixels>,
        position: PortPosition,
    ) -> Point<Pixels> {
        self.viewport.edge_control_point(source, position)
    }

    pub fn is_node_visible(&self, node_id: &NodeId) -> bool {
        is_node_visible(self.graph, self.viewport, node_id)
    }
    pub fn is_node_visible_node(&self, node: &Node) -> bool {
        self.viewport.is_node_visible(node)
    }

    pub fn is_edge_visible(&self, edge: &Edge) -> bool {
        is_edge_visible(self.graph, self.viewport, edge)
    }

    pub fn port_offset_cached(&self, node_id: &NodeId, port_id: &PortId) -> Option<Point<Pixels>> {
        self.port_offset_cache.get_offset(node_id, port_id)
    }

    /// Port ids with layout cached for this node (see [`PortLayoutCache::cached_port_ids_for_node`]).
    ///
    /// Call [`Self::cache_port_offset_with_nodes`] (or other `cache_port_offset_*` helpers) first
    /// so the list is complete for rendering.
    pub fn cached_port_ids_for_node(&self, node_id: &NodeId) -> impl Iterator<Item = PortId> + '_ {
        self.port_offset_cache.cached_port_ids_for_node(node_id)
    }

    /// Port center in screen pixels when you already have the owning [`Node`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center(&self, node: &Node, port_id: PortId) -> Option<Point<Pixels>> {
        let node_pos = node.point();
        let offset = self.port_offset_cached(&node.id(), &port_id)?;
        Some(self.viewport.world_to_screen(node_pos + offset))
    }

    /// Like [`Self::port_screen_center`], resolving the port from [`Graph::ports`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_center_by_port_id(&self, port_id: PortId) -> Option<Point<Pixels>> {
        let port = self.graph.get_port(&port_id)?;
        let node = self.get_node(&port.node_id())?;
        self.port_screen_center(node, port_id)
    }

    /// Full port layout for custom [`NodeRenderer::port_render`].
    /// *warning*: this is using port offset cache, so it will not be accurate if the port offset is not cached.
    pub fn port_screen_frame(&self, node: &Node, port: &Port) -> Option<PortScreenFrame> {
        Some(PortScreenFrame {
            center: self.port_screen_center(node, port.id())?,
            size: *port.size_ref(),
            zoom: self.viewport.zoom(),
            port_id: port.id(),
        })
    }

    pub fn cache_all_node_port_offset(&mut self) {
        self.port_offset_cache
            .ensure_all_nodes_ports(self.graph, self.renderers);
    }

    pub fn cache_port_offset_with_nodes(&mut self, node_ids: &[NodeId]) {
        for node_id in node_ids {
            self.cache_node_port_offset(node_id);
        }
    }

    pub fn cache_port_offset_with_edge(&mut self, edge_id: &EdgeId) {
        self.port_offset_cache
            .ensure_edge_ports(self.graph, self.renderers, edge_id);
    }

    pub fn cache_port_offset_with_port(&mut self, port_id: &PortId) {
        self.port_offset_cache
            .ensure_node_ports_for_port(self.graph, self.renderers, port_id);
    }

    fn cache_node_port_offset(&mut self, node_id: &NodeId) {
        self.port_offset_cache
            .ensure_node_ports(self.graph, self.renderers, node_id);
    }

    pub fn get_shared_state<T: Any + Send + 'static>(&self) -> Option<&T> {
        self.shared_state.get::<T>()
    }

    pub fn contains_shared_state<T: Any + Send + 'static>(&self) -> bool {
        self.shared_state.contains::<T>()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RenderLayer {
    Background,
    Edges,
    Nodes,
    Selection,
    Interaction,
    Overlay,
}

impl RenderLayer {
    pub const ALL: [RenderLayer; 6] = [
        RenderLayer::Background,
        RenderLayer::Edges,
        RenderLayer::Nodes,
        RenderLayer::Selection,
        RenderLayer::Interaction,
        RenderLayer::Overlay,
    ];
    pub fn index(self) -> usize {
        match self {
            RenderLayer::Background => 0,
            RenderLayer::Edges => 1,
            RenderLayer::Nodes => 2,
            RenderLayer::Selection => 3,
            RenderLayer::Interaction => 4,
            RenderLayer::Overlay => 5,
        }
    }
}

pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self { plugins: vec![] }
    }

    pub fn add(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub fn extend_boxed(&mut self, plugins: impl IntoIterator<Item = Box<dyn Plugin>>) {
        self.plugins.extend(plugins);
    }

    pub fn sort_by_priority_desc(&mut self) {
        self.plugins.sort_by_key(|p| -p.priority());
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Box<dyn Plugin>> {
        self.plugins.iter_mut()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Box<dyn Plugin>> {
        self.plugins.iter()
    }
}
