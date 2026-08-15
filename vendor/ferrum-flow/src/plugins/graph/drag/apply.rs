use gpui::{Pixels, Point};

use crate::{NodeId, plugin::PluginContext, plugins::node::ApplyNodeDragDelta};

use super::{boundary, policy::BoundaryDragPolicy};

pub(crate) struct HierarchyDragDelta {
    pub policy: BoundaryDragPolicy,
}

impl ApplyNodeDragDelta for HierarchyDragDelta {
    fn apply(
        &self,
        ctx: &mut PluginContext,
        id: NodeId,
        start_local: Point<Pixels>,
        start_world: Point<Pixels>,
        world_delta: Point<Pixels>,
        dragged: &[NodeId],
    ) {
        // Parent also moving: keep local offset so the child stays fixed inside the parent.
        if let Some(parent_id) = ctx.graph.get_node(&id).and_then(|n| n.parent())
            && dragged.contains(&parent_id)
        {
            if let Some(node) = ctx.get_node_mut(&id) {
                node.set_position_with_point(start_local);
            }
            return;
        }

        let parent = ctx.graph.get_node(&id).and_then(|n| n.parent());
        let target_world = Point::new(start_world.x + world_delta.x, start_world.y + world_delta.y);
        let Ok(mut local) = ctx.graph.local_point_from_world(target_world, parent) else {
            return;
        };

        if self.policy == BoundaryDragPolicy::Clamp
            && boundary::clamp_local_in_parent_applies(ctx.graph, id, dragged)
            && let Some(clamped) = boundary::clamp_local_in_parent(ctx.graph, id, local)
        {
            local = clamped;
        }

        if let Some(node) = ctx.get_node_mut(&id) {
            node.set_position_with_point(local);
        }
    }
}
