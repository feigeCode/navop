//! Child-in-parent drag bounds ([`super::policy::BoundaryDragPolicy::Clamp`]).

use gpui::{Pixels, Point, px};

use crate::{Graph, NodeId};

pub(crate) fn clamp_local_in_parent_applies(
    graph: &Graph,
    child: NodeId,
    dragged: &[NodeId],
) -> bool {
    let Some(parent) = graph.get_node(&child).and_then(|n| n.parent()) else {
        return false;
    };
    graph.get_node(&parent).is_some() && !dragged.contains(&parent)
}

pub(crate) fn clamp_local_in_parent(
    graph: &Graph,
    child: NodeId,
    local: Point<Pixels>,
) -> Option<Point<Pixels>> {
    let child_node = graph.get_node(&child)?;
    let parent_id = child_node.parent()?;
    let parent = graph.get_node(&parent_id)?;
    let child_size = *child_node.size_ref();
    let parent_size = *parent.size_ref();
    Some(Point::new(
        clamp_pixels(
            local.x,
            px(0.0),
            (parent_size.width - child_size.width).max(px(0.0)),
        ),
        clamp_pixels(
            local.y,
            px(0.0),
            (parent_size.height - child_size.height).max(px(0.0)),
        ),
    ))
}

fn clamp_pixels(value: Pixels, min: Pixels, max: Pixels) -> Pixels {
    let v = f32::from(value);
    let lo = f32::from(min);
    let hi = f32::from(max);
    px(v.clamp(lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_local_in_parent_keeps_child_inside() {
        let mut g = Graph::new();
        let p = g
            .create_node("default")
            .position(0.0, 0.0)
            .size(200.0, 100.0)
            .build();
        let c = g
            .create_node("default")
            .position(0.0, 0.0)
            .size(80.0, 40.0)
            .build();
        g.add_child(p, c).unwrap();

        let clamped = clamp_local_in_parent(&g, c, Point::new(px(500.0), px(500.0))).unwrap();
        assert_eq!(clamped, Point::new(px(120.0), px(60.0)));
    }

    #[test]
    fn clamp_local_in_parent_applies_when_parent_not_dragged() {
        let mut g = Graph::new();
        let p = g.create_node("default").build();
        let c = g.create_node("default").build();
        g.add_child(p, c).unwrap();
        assert!(clamp_local_in_parent_applies(&g, c, &[c]));
        assert!(!clamp_local_in_parent_applies(&g, c, &[p, c]));
    }
}
