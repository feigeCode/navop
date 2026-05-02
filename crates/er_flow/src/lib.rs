mod graph;
mod model;
mod render;
mod source;
mod theme;

pub use graph::{ErGraphOptions, graph_from_diagram, graph_from_source};
pub use model::{ErDiagram, ErEntity, ErField, ErRelationship, ErRelationshipKind};
pub use render::ErEntityRenderer;
pub use source::{ErDataSource, JsonErDataSource, StaticErDataSource};
pub use theme::er_flow_theme;

use ferrum_flow::NodeRenderer;

pub const ER_ENTITY_RENDERER_KEY: &str = "er_entity";

pub fn er_node_renderers() -> Vec<(String, Box<dyn NodeRenderer>)> {
    vec![(
        ER_ENTITY_RENDERER_KEY.to_string(),
        Box::new(ErEntityRenderer),
    )]
}
