use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, Element, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, Size,
    Styled, div, px, rgb, rgba,
};

use crate::{
    FlowTheme, Graph, Node, NodeId,
    canvas::{Interaction, InteractionResult},
    plugin::{
        EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
    },
    plugins::current_tool_mode,
};

const DRAG_THRESHOLD: Pixels = px(2.0);
const DRAG_COMMAND_INTERVAL: Duration = Duration::from_millis(50);

pub struct SelectionPlugin {
    selected: Option<Selected>,
}

struct Selected {
    bounds: Bounds<Pixels>,
    nodes: HashMap<NodeId, Point<Pixels>>,
}

impl SelectionPlugin {
    pub fn new() -> Self {
        Self { selected: None }
    }
}

impl Plugin for SelectionPlugin {
    fn name(&self) -> &'static str {
        "selection"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button != MouseButton::Left {
                return EventResult::Continue;
            }
            if current_tool_mode(ctx.shared_state).is_pan() {
                return EventResult::Continue;
            }
            if !ev.modifiers.shift {
                let start = ctx.viewport.screen_to_world(ev.position);
                if let Some(Selected { bounds, nodes }) = self.selected.take() {
                    if bounds.contains(&start) {
                        ctx.start_interaction(SelectionInteraction::start_move(
                            start, bounds, nodes,
                        ));

                        return EventResult::Stop;
                    }
                }

                ctx.start_interaction(SelectionInteraction::new(start));
                return EventResult::Stop;
            }
        } else if let Some(SelectedEvent { bounds, nodes }) = event.as_custom() {
            self.selected = Some(Selected {
                bounds: *bounds,
                nodes: nodes.clone(),
            });
            return EventResult::Stop;
        }

        EventResult::Continue
    }
    fn priority(&self) -> i32 {
        100
    }
    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Selection
    }
    fn render(&mut self, ctx: &mut RenderContext) -> Option<AnyElement> {
        self.selected.as_ref().map(|Selected { bounds, .. }| {
            let top_left = ctx.world_to_screen(bounds.origin);

            let size = Size::new(
                bounds.size.width * ctx.viewport.zoom,
                bounds.size.height * ctx.viewport.zoom,
            );
            render_rect(Bounds::new(top_left, size), ctx.theme)
        })
    }
}

pub struct SelectionInteraction {
    state: SelectionState,
    last_drag_command_at: Option<Instant>,
}

enum SelectionState {
    Pending {
        start: Point<Pixels>,
    },
    Selecting {
        start: Point<Pixels>,
        end: Point<Pixels>,
    },
    Moving {
        start_mouse: Point<Pixels>,
        start_bounds: Bounds<Pixels>,
        bounds: Bounds<Pixels>,
        nodes: HashMap<NodeId, Point<Pixels>>,
    },
}
struct SelectedEvent {
    bounds: Bounds<Pixels>,
    nodes: HashMap<NodeId, Point<Pixels>>,
}

impl SelectionInteraction {
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            state: SelectionState::Pending { start },
            last_drag_command_at: None,
        }
    }
    pub fn start_move(
        mouse: Point<Pixels>,
        bounds: Bounds<Pixels>,
        nodes: HashMap<NodeId, Point<Pixels>>,
    ) -> Self {
        Self {
            state: SelectionState::Moving {
                start_mouse: mouse,
                start_bounds: bounds.clone(),
                bounds: bounds,
                nodes,
            },
            last_drag_command_at: None,
        }
    }
}

impl Interaction for SelectionInteraction {
    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, ctx: &mut PluginContext) -> InteractionResult {
        let mouse_world = ctx.screen_to_world(ev.position);
        match &mut self.state {
            SelectionState::Pending { start } => {
                let delta = mouse_world - *start;

                if delta.x.abs() > DRAG_THRESHOLD && delta.y.abs() > DRAG_THRESHOLD {
                    self.state = SelectionState::Selecting {
                        start: *start,
                        end: mouse_world,
                    };

                    ctx.notify();
                }
            }

            SelectionState::Selecting { end, .. } => {
                *end = mouse_world;
                ctx.notify();
            }

            SelectionState::Moving {
                start_mouse,
                start_bounds,
                bounds,
                nodes,
            } => {
                let delta = mouse_world - *start_mouse;

                for (id, start_pos) in nodes.iter_mut() {
                    if let Some(node) = ctx.get_node_mut(id) {
                        node.x = start_pos.x + delta.x;
                        node.y = start_pos.y + delta.y;
                    }
                }
                *bounds = Bounds::new(start_bounds.origin + delta, start_bounds.size);

                if ctx.has_sync_plugin() {
                    let now = Instant::now();
                    let should_command = self
                        .last_drag_command_at
                        .map(|t| now.duration_since(t) >= DRAG_COMMAND_INTERVAL)
                        .unwrap_or(true);
                    if should_command {
                        let start_position: Vec<_> = nodes
                            .iter()
                            .map(|(id, point)| (id.clone(), point.clone()))
                            .collect();
                        ctx.execute_command(super::node::DragNodesCommand::new(
                            &start_position,
                            &ctx,
                        ));
                        self.last_drag_command_at = Some(now);
                    }
                }

                ctx.notify();
            }
        }

        InteractionResult::Continue
    }
    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, ctx: &mut PluginContext) -> InteractionResult {
        match &mut self.state {
            SelectionState::Pending { .. } => {
                return InteractionResult::End;
            }

            SelectionState::Selecting { start, end } => {
                let rect = normalize_rect(*start, *end);

                let mut selected = HashMap::new();

                ctx.clear_selected_node();

                for node in ctx.graph.nodes().values() {
                    let bounds = node_world_bounds(node);

                    if rect.intersects(&bounds) {
                        selected.insert(node.id, node.point());
                    }
                }

                for (id, _) in selected.iter() {
                    ctx.add_selected_node(*id, true);
                }

                let bounds = compute_nodes_bounds(&selected, ctx.graph);

                ctx.cancel_interaction();
                ctx.emit(FlowEvent::custom(SelectedEvent {
                    bounds,
                    nodes: selected,
                }));

                return InteractionResult::End;
            }

            SelectionState::Moving { bounds, nodes, .. } => {
                let bounds = *bounds;

                let mut new_nodes = HashMap::new();
                for (id, _) in nodes.iter() {
                    ctx.add_selected_node(*id, true);
                    if let Some(node) = ctx.graph.nodes().get(id) {
                        new_nodes.insert(id.clone(), node.point());
                    }
                }

                let start_position: Vec<_> = nodes
                    .iter()
                    .map(|(id, point)| (id.clone(), point.clone()))
                    .collect();

                ctx.execute_command(super::node::DragNodesCommand::new(&start_position, ctx));

                ctx.emit(FlowEvent::custom(SelectedEvent {
                    bounds,
                    nodes: new_nodes,
                }));

                return InteractionResult::End;
            }
        }
    }
    fn render(&self, ctx: &mut RenderContext) -> Option<AnyElement> {
        match &self.state {
            SelectionState::Selecting { start, end } => {
                let rect = normalize_rect(*start, *end);

                let top_left = ctx.world_to_screen(rect.origin);

                let size = Size::new(
                    rect.size.width * ctx.viewport.zoom,
                    rect.size.height * ctx.viewport.zoom,
                );

                Some(render_rect(Bounds::new(top_left, size), ctx.theme))
            }

            SelectionState::Moving { bounds, .. } => {
                let top_left = ctx.world_to_screen(bounds.origin);

                let size = Size::new(
                    bounds.size.width * ctx.viewport.zoom,
                    bounds.size.height * ctx.viewport.zoom,
                );
                Some(render_rect(Bounds::new(top_left, size), ctx.theme))
            }

            _ => None,
        }
    }
}

fn normalize_rect(start: Point<Pixels>, end: Point<Pixels>) -> Bounds<Pixels> {
    let x = start.x.min(end.x);
    let y = start.y.min(end.y);

    let w = (end.x - start.x).abs();
    let h = (end.y - start.y).abs();

    Bounds::new(Point::new(x, y), Size::new(w, h))
}

fn render_rect(bounds: Bounds<Pixels>, theme: &FlowTheme) -> AnyElement {
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .border(px(1.0))
        .border_color(rgb(theme.selection_rect_border))
        .bg(rgba(theme.selection_rect_fill_rgba))
        .into_any()
}

fn node_world_bounds(node: &Node) -> Bounds<Pixels> {
    Bounds::new(Point::new(node.x, node.y), node.size)
}

fn compute_nodes_bounds(nodes: &HashMap<NodeId, Point<Pixels>>, graph: &Graph) -> Bounds<Pixels> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for id in nodes.keys() {
        let Some(node) = &graph.nodes().get(id) else {
            continue;
        };

        min_x = min_x.min(node.x.into());
        min_y = min_y.min(node.y.into());

        max_x = max_x.max((node.x + node.size.width).into());
        max_y = max_y.max((node.y + node.size.height).into());
    }

    Bounds::new(
        Point::new(px(min_x), px(min_y)),
        Size::new(px(max_x - min_x), px(max_y - min_y)),
    )
}
