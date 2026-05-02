use std::collections::HashMap;

use gpui::{Bounds, Pixels, Point};

use crate::{
    Edge, EdgeBuilder, EdgeId, Graph, GraphOp, Node, NodeBuilder, NodeId, Port, PortId,
    RendererRegistry, SharedState, Viewport,
    canvas::PortLayoutCache,
    plugin::{is_edge_visible, is_node_visible},
};

pub trait Command {
    fn name(&self) -> &'static str;

    /// execute command detail, e.g: move node
    fn execute(&mut self, ctx: &mut CommandContext);

    // undo command , when open sync plugin, this is diabeled.
    fn undo(&mut self, ctx: &mut CommandContext);

    /// used by sync plugin
    /// when open sync plugin, execute method is diasbeld, and using to_ops send graph intent
    fn to_ops(&self, _ctx: &mut CommandContext) -> Vec<GraphOp> {
        vec![]
    }
}

pub trait HistoryProvider {
    fn undo(&mut self, ctx: &mut CommandContext);
    fn redo(&mut self, ctx: &mut CommandContext);
    fn push(&mut self, command: Box<dyn Command>, ctx: &mut CommandContext);
    fn clear(&mut self);
}

pub struct CommandContext<'a> {
    pub graph: &'a mut Graph,
    pub port_offset_cache: &'a mut PortLayoutCache,
    viewport: &'a mut Viewport,
    pub renderers: &'a mut RendererRegistry,
    /// Shared plugin state on the [`FlowCanvas`](crate::canvas::FlowCanvas).
    pub shared_state: &'a mut SharedState,
    pub(crate) notify: &'a mut dyn FnMut(),
}
const MAX_HISTORY: usize = 100;
pub struct LocalHistory {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
}

impl LocalHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: vec![],
            redo_stack: vec![],
        }
    }
}

impl Default for LocalHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryProvider for LocalHistory {
    fn push(&mut self, mut command: Box<dyn Command>, ctx: &mut CommandContext) {
        command.execute(ctx);

        self.undo_stack.push(command);

        self.redo_stack.clear();

        if self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.remove(0);
        }
    }
    fn undo(&mut self, ctx: &mut CommandContext) {
        if let Some(mut cmd) = self.undo_stack.pop() {
            cmd.undo(ctx);
            self.redo_stack.push(cmd);
        }
    }

    fn redo(&mut self, ctx: &mut CommandContext) {
        if let Some(mut cmd) = self.redo_stack.pop() {
            cmd.execute(ctx);
            self.undo_stack.push(cmd);
        }
    }

    fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

pub struct CompositeCommand {
    commands: Vec<Box<dyn Command>>,
}

impl Default for CompositeCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CompositeCommand {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
    pub fn push(&mut self, command: impl Command + 'static) {
        self.commands.push(Box::new(command));
    }
}

impl Command for CompositeCommand {
    fn name(&self) -> &'static str {
        "composite"
    }
    fn execute(&mut self, state: &mut CommandContext) {
        for cmd in &mut self.commands {
            cmd.execute(state);
        }
    }

    fn undo(&mut self, state: &mut CommandContext) {
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(state);
        }
    }
    fn to_ops(&self, ctx: &mut CommandContext) -> Vec<GraphOp> {
        let mut list = vec![];
        for cmd in &self.commands {
            list.extend(cmd.to_ops(ctx));
        }

        vec![GraphOp::Batch(list)]
    }
}

impl<'a> CommandContext<'a> {
    pub(crate) fn new(
        graph: &'a mut Graph,
        port_offset_cache: &'a mut PortLayoutCache,
        viewport: &'a mut Viewport,
        renderers: &'a mut RendererRegistry,
        shared_state: &'a mut SharedState,
        notify: &'a mut dyn FnMut(),
    ) -> Self {
        Self {
            graph,
            port_offset_cache,
            viewport,
            renderers,
            shared_state,
            notify,
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

    pub fn remove_port(&mut self, id: &PortId) {
        self.graph.remove_port(id);
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.get_node(id)
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

    pub fn cache_all_node_port_offset(&mut self) {
        self.port_offset_cache
            .ensure_all_nodes_ports(self.graph, self.renderers);
    }

    pub fn cache_node_port_offset(&mut self, node_id: &NodeId) {
        self.port_offset_cache
            .ensure_node_ports(self.graph, self.renderers, node_id);
    }

    pub fn notify(&mut self) {
        (self.notify)();
    }
}
