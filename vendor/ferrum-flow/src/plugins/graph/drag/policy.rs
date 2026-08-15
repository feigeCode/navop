/// How dragging a **child** node behaves relative to its parent's bounds ([`super::NestedNodeDragPlugin`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundaryDragPolicy {
    /// Local position is clamped so the node stays inside the parent's size.
    #[default]
    Clamp,
    /// Dragging past the parent edge reparents the node to the parent's parent (or root), preserving world position.
    Promote,
    /// On release, reparent under another node whose world bounds contain the drop point, or promote to root.
    Reparent,
}
