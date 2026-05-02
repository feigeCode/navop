use gpui::*;
use std::collections::HashMap;

use crate::node::Node;
use crate::plugin::{NodeCardVariant, RenderContext};
use crate::{Graph, Port, PortId, PortPosition};

pub trait NodeRenderer: Send + Sync {
    /// render node inner UI
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement;

    // custom render port UI
    fn port_render(&self, node: &Node, port: &Port, ctx: &mut RenderContext) -> Option<AnyElement> {
        let frame = ctx.port_screen_frame(node, port)?;
        Some(
            frame
                .anchor_div()
                .rounded_full()
                .bg(rgb(ctx.theme.default_port_fill))
                .into_any(),
        )
    }

    /// computing the position of port relative to node
    /// built-in Node Plugin is cached this.
    fn port_offset(&self, node: &Node, port: &Port, graph: &Graph) -> Point<Pixels> {
        let total = graph
            .ports_values()
            .filter(|p| {
                p.node_id() == node.id()
                    && p.kind() == port.kind()
                    && p.position() == port.position()
            })
            .count() as f32;
        let index = port.index() as f32;
        let size = *node.size_ref();

        match port.position() {
            PortPosition::Left => {
                let spacing = size.height / (total + 1.0);
                Point::new(px(0.0), spacing * (index + 1.0))
            }
            PortPosition::Right => {
                let spacing = size.height / (total + 1.0);
                Point::new(size.width, spacing * (index + 1.0))
            }
            PortPosition::Top => {
                let spacing = size.width / (total + 1.0);
                Point::new(spacing * (index + 1.0), px(0.0))
            }
            PortPosition::Bottom => {
                let spacing = size.width / (total + 1.0);
                Point::new(spacing * (index + 1.0), size.height)
            }
        }
    }
}

pub struct RendererRegistry {
    map: HashMap<String, Box<dyn NodeRenderer>>,
    default: Box<dyn NodeRenderer>,
    undefined: Box<dyn NodeRenderer>,
}

impl Default for RendererRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            default: Box::new(DefaultNodeRenderer {}),
            undefined: Box::new(UndefinedNodeRenderer {}),
        }
    }

    pub fn register<R>(&mut self, name: impl Into<String>, renderer: R)
    where
        R: NodeRenderer + 'static,
    {
        self.map.insert(name.into(), Box::new(renderer));
    }

    pub fn register_boxed(&mut self, name: impl Into<String>, renderer: Box<dyn NodeRenderer>) {
        self.map.insert(name.into(), renderer);
    }

    pub fn get(&self, name: &str) -> &dyn NodeRenderer {
        if name.is_empty() || name == "default" {
            return self.default.as_ref();
        }

        self.map
            .get(name)
            .map(|r| r.as_ref())
            .unwrap_or(self.undefined.as_ref())
    }
}

struct DefaultNodeRenderer;

impl NodeRenderer for DefaultNodeRenderer {
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement {
        let node_id = node.id();
        let selected = ctx.graph.selected_node().iter().any(|id| *id == node_id);

        ctx.node_card_shell(node, selected, NodeCardVariant::Default)
            .rounded(px(6.0))
            .border(px(1.5))
            .child(
                div()
                    .id(ElementId::Uuid(*node_id.as_uuid()))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .px_2()
                    .child(default_node_caption(node))
                    .text_color(rgb(ctx.theme.node_caption_text)),
            )
            .into_any()
    }
}

struct UndefinedNodeRenderer;

impl NodeRenderer for UndefinedNodeRenderer {
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement {
        ctx.node_card_shell(node, false, NodeCardVariant::UndefinedType)
            .rounded(px(6.0))
            .border(px(1.5))
            .child(
                div()
                    .id(ElementId::Uuid(*node.id().as_uuid()))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .px_2()
                    .child(undefined_node_caption(node))
                    .text_color(rgb(ctx.theme.undefined_node_caption_text)),
            )
            .into_any()
    }
}

#[deprecated(note = "use `ctx.port_screen_center(node, port_id)`")]
pub fn port_screen_position(
    node: &Node,
    port_id: PortId,
    ctx: &RenderContext,
) -> Option<Point<Pixels>> {
    ctx.port_screen_center(node, port_id)
}

fn data_title(data: &serde_json::Value) -> Option<String> {
    if let Some(s) = data.get("label").and_then(|v| v.as_str()) {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    None
}

/// Label for [`DefaultNodeRenderer`]: user-facing title from `data`, else `node_type`, else a generic word.
/// UUID stays off-canvas; use debug/inspector/tooltip if operators need the id.
pub fn default_node_caption(node: &Node) -> String {
    if let Some(s) = data_title(node.data_ref()) {
        return s;
    }
    if !node.renderer_key().is_empty() {
        return node.renderer_key().to_string();
    }
    "Node".to_string()
}

fn undefined_node_caption(node: &Node) -> String {
    if !node.renderer_key().is_empty() {
        return format!("Unknown type: {}", node.renderer_key());
    }
    "Unknown node type".to_string()
}
