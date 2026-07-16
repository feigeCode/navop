use gpui::Axis;
use gpui_component::Placement;
use terminal_view::workspace::{TerminalPaneId, TerminalSplitNode, TerminalSplitTree};

#[test]
fn same_axis_splits_flatten_into_one_group() {
    let main = TerminalPaneId::new(1);
    let right = TerminalPaneId::new(2);
    let far_right = TerminalPaneId::new(3);
    let mut tree = TerminalSplitTree::new(main);

    assert!(tree.split(main, right, Placement::Right));
    assert!(tree.split(right, far_right, Placement::Right));

    let TerminalSplitNode::Group { axis, children, .. } = tree.root() else {
        panic!("horizontal splits should create one group");
    };
    assert_eq!(Axis::Horizontal, *axis);
    assert_eq!(3, children.len());
    assert_eq!(vec![main, right, far_right], tree.panes());
}

#[test]
fn different_axis_split_nests_only_the_target_branch() {
    let main = TerminalPaneId::new(1);
    let right = TerminalPaneId::new(2);
    let bottom_right = TerminalPaneId::new(3);
    let mut tree = TerminalSplitTree::new(main);

    assert!(tree.split(main, right, Placement::Right));
    assert!(tree.split(right, bottom_right, Placement::Bottom));

    let TerminalSplitNode::Group { axis, children, .. } = tree.root() else {
        panic!("root should stay horizontal");
    };
    assert_eq!(Axis::Horizontal, *axis);
    assert_eq!(2, children.len());
    assert!(matches!(
        &children[1],
        TerminalSplitNode::Group {
            axis: Axis::Vertical,
            children,
            ..
        } if children.len() == 2
    ));
}

#[test]
fn removing_a_pane_prunes_single_child_groups_and_returns_neighbor() {
    let main = TerminalPaneId::new(1);
    let right = TerminalPaneId::new(2);
    let bottom_right = TerminalPaneId::new(3);
    let mut tree = TerminalSplitTree::new(main);
    tree.split(main, right, Placement::Right);
    tree.split(right, bottom_right, Placement::Bottom);

    assert_eq!(Some(bottom_right), tree.remove(right));
    assert_eq!(vec![main, bottom_right], tree.panes());
    assert!(!tree.contains(right));

    assert_eq!(Some(main), tree.remove(bottom_right));
    assert_eq!(&TerminalSplitNode::Pane { pane_id: main }, tree.root());
}

#[test]
fn any_pane_can_be_removed_while_another_pane_remains() {
    let first = TerminalPaneId::new(1);
    let second = TerminalPaneId::new(2);
    let mut tree = TerminalSplitTree::new(first);
    tree.split(first, second, Placement::Right);

    assert_eq!(Some(second), tree.remove(first));
    assert_eq!(vec![second], tree.panes());
    assert_eq!(None, tree.remove(second));
}

#[test]
fn only_a_single_pane_workspace_can_transfer_into_another_workspace() {
    let main = TerminalPaneId::new(1);
    let child = TerminalPaneId::new(2);
    let mut tree = TerminalSplitTree::new(main);

    assert_eq!(Some(main), tree.transferable_pane());

    tree.split(main, child, Placement::Right);

    assert_eq!(None, tree.transferable_pane());
}
