use gpui::Axis;
use gpui_component::Placement;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerminalPaneId(u64);

impl TerminalPaneId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerminalSplitId(u64);

impl TerminalSplitId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSplitNode {
    Pane {
        pane_id: TerminalPaneId,
    },
    Group {
        split_id: TerminalSplitId,
        axis: Axis,
        children: Vec<TerminalSplitNode>,
    },
}

impl TerminalSplitNode {
    fn contains(&self, pane_id: TerminalPaneId) -> bool {
        match self {
            Self::Pane { pane_id: current } => *current == pane_id,
            Self::Group { children, .. } => children.iter().any(|child| child.contains(pane_id)),
        }
    }

    fn collect_panes(&self, panes: &mut Vec<TerminalPaneId>) {
        match self {
            Self::Pane { pane_id } => panes.push(*pane_id),
            Self::Group { children, .. } => {
                for child in children {
                    child.collect_panes(panes);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct TerminalSplitTree {
    root: TerminalSplitNode,
    next_split_id: u64,
}

impl TerminalSplitTree {
    pub fn new(initial_pane_id: TerminalPaneId) -> Self {
        Self {
            root: TerminalSplitNode::Pane {
                pane_id: initial_pane_id,
            },
            next_split_id: 1,
        }
    }

    pub fn root(&self) -> &TerminalSplitNode {
        &self.root
    }

    pub fn contains(&self, pane_id: TerminalPaneId) -> bool {
        self.root.contains(pane_id)
    }

    pub fn panes(&self) -> Vec<TerminalPaneId> {
        let mut panes = Vec::new();
        self.root.collect_panes(&mut panes);
        panes
    }

    pub fn transferable_pane(&self) -> Option<TerminalPaneId> {
        match self.root {
            TerminalSplitNode::Pane { pane_id } => Some(pane_id),
            TerminalSplitNode::Group { .. } => None,
        }
    }

    pub fn split(
        &mut self,
        target: TerminalPaneId,
        new_pane: TerminalPaneId,
        placement: Placement,
    ) -> bool {
        if self.contains(new_pane) {
            return false;
        }
        Self::insert_split(
            &mut self.root,
            target,
            new_pane,
            placement,
            &mut self.next_split_id,
        )
    }

    pub fn remove(&mut self, pane_id: TerminalPaneId) -> Option<TerminalPaneId> {
        if !self.contains(pane_id) {
            return None;
        }
        let panes = self.panes();
        if panes.len() <= 1 {
            return None;
        }
        let index = panes.iter().position(|current| *current == pane_id)?;
        let neighbor = panes
            .get(index + 1)
            .or_else(|| index.checked_sub(1).and_then(|index| panes.get(index)))
            .copied();
        self.root = Self::remove_from(self.root.clone(), pane_id)?;
        neighbor
    }

    fn next_split_id(next_split_id: &mut u64) -> TerminalSplitId {
        let split_id = TerminalSplitId(*next_split_id);
        *next_split_id += 1;
        split_id
    }

    fn insert_split(
        node: &mut TerminalSplitNode,
        target: TerminalPaneId,
        new_pane: TerminalPaneId,
        placement: Placement,
        next_split_id: &mut u64,
    ) -> bool {
        match node {
            TerminalSplitNode::Pane { pane_id } if *pane_id == target => {
                let target_node = TerminalSplitNode::Pane { pane_id: target };
                let new_node = TerminalSplitNode::Pane { pane_id: new_pane };
                let children = ordered_children(target_node, new_node, placement);
                *node = TerminalSplitNode::Group {
                    split_id: Self::next_split_id(next_split_id),
                    axis: placement.axis(),
                    children,
                };
                true
            }
            TerminalSplitNode::Pane { .. } => false,
            TerminalSplitNode::Group { axis, children, .. } => {
                let Some(index) = children.iter().position(|child| child.contains(target)) else {
                    return false;
                };
                if matches!(children[index], TerminalSplitNode::Pane { pane_id } if pane_id == target)
                    && *axis == placement.axis()
                {
                    let insert_at = match placement {
                        Placement::Right | Placement::Bottom => index + 1,
                        Placement::Left | Placement::Top => index,
                    };
                    children.insert(insert_at, TerminalSplitNode::Pane { pane_id: new_pane });
                    true
                } else {
                    Self::insert_split(
                        &mut children[index],
                        target,
                        new_pane,
                        placement,
                        next_split_id,
                    )
                }
            }
        }
    }

    fn remove_from(node: TerminalSplitNode, pane_id: TerminalPaneId) -> Option<TerminalSplitNode> {
        match node {
            TerminalSplitNode::Pane { pane_id: current } => {
                (current != pane_id).then_some(TerminalSplitNode::Pane { pane_id: current })
            }
            TerminalSplitNode::Group {
                split_id,
                axis,
                children,
            } => {
                let mut children = children
                    .into_iter()
                    .filter_map(|child| Self::remove_from(child, pane_id))
                    .collect::<Vec<_>>();
                match children.len() {
                    0 => None,
                    1 => children.pop(),
                    _ => Some(TerminalSplitNode::Group {
                        split_id,
                        axis,
                        children,
                    }),
                }
            }
        }
    }
}

fn ordered_children(
    target: TerminalSplitNode,
    new_pane: TerminalSplitNode,
    placement: Placement,
) -> Vec<TerminalSplitNode> {
    match placement {
        Placement::Right | Placement::Bottom => vec![target, new_pane],
        Placement::Left | Placement::Top => vec![new_pane, target],
    }
}
