use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Axis, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, Window, div,
};
use gpui_component::{
    Placement,
    resizable::{h_resizable, resizable_panel, v_resizable},
};

use crate::tab_container::{TabContainer, TabContainerEvent};

pub type TabPaneFactory = Rc<dyn Fn(&mut Window, &mut Context<TabContainer>, bool) -> TabContainer>;

pub(crate) fn split_tree_visible_for_layout(
    _primary_active_tab_can_split: bool,
    split_tree_exists: bool,
    _primary_regular_tabs_empty: bool,
    _has_non_primary_tabs: bool,
) -> bool {
    split_tree_exists
}

pub enum SplitTabContainerEvent {
    ActivePaneChanged,
    LayoutChanged,
}

#[derive(Clone)]
pub enum SplitNode {
    Leaf(Entity<TabContainer>),
    Split {
        axis: Axis,
        children: Vec<SplitNode>,
    },
}

impl SplitNode {
    fn is_split(&self) -> bool {
        matches!(self, Self::Split { .. })
    }

    fn contains(&self, pane: &Entity<TabContainer>) -> bool {
        match self {
            Self::Leaf(leaf) => leaf == pane,
            Self::Split { children, .. } => children.iter().any(|child| child.contains(pane)),
        }
    }

    fn collect_close_panes(&self) -> Vec<Entity<TabContainer>> {
        let mut panes = Vec::new();
        self.collect_close_panes_into(&mut panes);
        panes
    }

    fn collect_close_panes_into(&self, panes: &mut Vec<Entity<TabContainer>>) {
        match self {
            Self::Leaf(pane) => panes.push(pane.clone()),
            Self::Split { children, .. } => {
                for child in children {
                    child.collect_close_panes_into(panes);
                }
            }
        }
    }

    fn insert_split(
        &mut self,
        target: &Entity<TabContainer>,
        new_pane: Entity<TabContainer>,
        placement: Placement,
    ) -> bool {
        match self {
            Self::Leaf(pane) if pane == target => {
                *self = Self::split_leaf(pane.clone(), new_pane, placement);
                true
            }
            Self::Leaf(_) => false,
            Self::Split { axis, children } => {
                let Some(index) = children.iter().position(|child| child.contains(target)) else {
                    return false;
                };

                if matches!(&children[index], Self::Leaf(pane) if pane == target)
                    && *axis == placement.axis()
                {
                    let insert_at = match placement {
                        Placement::Right | Placement::Bottom => index + 1,
                        Placement::Left | Placement::Top => index,
                    };
                    children.insert(insert_at, Self::Leaf(new_pane));
                    true
                } else {
                    children[index].insert_split(target, new_pane, placement)
                }
            }
        }
    }

    fn split_leaf(
        target: Entity<TabContainer>,
        new_pane: Entity<TabContainer>,
        placement: Placement,
    ) -> Self {
        let children = match placement {
            Placement::Right | Placement::Bottom => {
                vec![Self::Leaf(target), Self::Leaf(new_pane)]
            }
            Placement::Left | Placement::Top => vec![Self::Leaf(new_pane), Self::Leaf(target)],
        };

        Self::Split {
            axis: placement.axis(),
            children,
        }
    }
}

pub struct SplitTabContainer {
    focus_handle: FocusHandle,
    root: SplitNode,
    primary_pane: Entity<TabContainer>,
    active_pane: Entity<TabContainer>,
    pane_factory: TabPaneFactory,
    suppress_cleanup: bool,
    _subscriptions: Vec<Subscription>,
}

impl SplitTabContainer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, pane_factory: TabPaneFactory) -> Self {
        let primary_pane = cx.new(|cx| pane_factory(window, cx, true));
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            root: SplitNode::Leaf(primary_pane.clone()),
            primary_pane: primary_pane.clone(),
            active_pane: primary_pane.clone(),
            pane_factory,
            suppress_cleanup: false,
            _subscriptions: Vec::new(),
        };
        this.subscribe_to_pane(&primary_pane, window, cx);
        this
    }

    pub fn primary_pane(&self) -> Entity<TabContainer> {
        self.primary_pane.clone()
    }

    pub fn active_pane(&self) -> Entity<TabContainer> {
        self.active_pane.clone()
    }

    pub fn close_all_tabs(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let panes = self.root.collect_close_panes();
        let Some(window_id) = cx.active_window() else {
            return Task::ready(false);
        };

        cx.spawn(async move |_handle, cx| {
            for pane in panes {
                let close_task = cx.update_window(window_id, |_, window, cx| {
                    pane.update(cx, |pane, cx| pane.close_all_tabs(window, cx))
                });

                match close_task {
                    Ok(task) => {
                        if !task.await {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
            true
        })
    }

    fn create_secondary_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TabContainer> {
        let factory = self.pane_factory.clone();
        cx.new(|cx| factory(window, cx, false))
    }

    fn subscribe_to_pane(
        &mut self,
        pane: &Entity<TabContainer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._subscriptions.push(cx.subscribe_in(
            pane,
            window,
            |this, pane, event: &TabContainerEvent, window, cx| {
                this.handle_pane_event(pane.clone(), event, window, cx);
            },
        ));
    }

    fn handle_pane_event(
        &mut self,
        pane: Entity<TabContainer>,
        event: &TabContainerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TabContainerEvent::TabActivated { .. } => {
                self.active_pane = pane;
                cx.emit(SplitTabContainerEvent::ActivePaneChanged);
                cx.notify();
            }
            TabContainerEvent::LayoutChanged | TabContainerEvent::TabClosed { .. } => {
                if !self.suppress_cleanup {
                    self.cleanup_empty_panes(cx);
                }
                cx.emit(SplitTabContainerEvent::LayoutChanged);
            }
            TabContainerEvent::SplitRequested {
                placement,
                source,
                tab_index,
            } => {
                self.split_tab(source.clone(), *tab_index, *placement, window, cx);
            }
            TabContainerEvent::MoveToPrimaryRequested { source, tab_index } => {
                self.move_tab_to_primary(source.clone(), *tab_index, window, cx);
            }
        }
    }

    fn split_tab(
        &mut self,
        source: Entity<TabContainer>,
        tab_index: usize,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(placement, Placement::Right | Placement::Bottom) {
            return;
        }

        self.suppress_cleanup = true;
        let moved_tab = source.update(cx, |source, cx| source.take_tab(tab_index, window, cx));
        self.suppress_cleanup = false;
        let Some(tab) = moved_tab else {
            return;
        };

        let new_pane = self.create_secondary_pane(window, cx);
        if self.root.insert_split(&source, new_pane.clone(), placement) {
            self.subscribe_to_pane(&new_pane, window, cx);
            new_pane.update(cx, |pane, cx| {
                pane.insert_tab_at_end_and_activate(tab, window, cx);
            });
            self.active_pane = new_pane;
        } else {
            source.update(cx, |pane, cx| {
                pane.insert_tab_at_end_and_activate(tab, window, cx);
            });
        }
        self.cleanup_empty_panes(cx);
        cx.emit(SplitTabContainerEvent::LayoutChanged);
        cx.emit(SplitTabContainerEvent::ActivePaneChanged);
        cx.notify();
    }

    fn move_tab_to_primary(
        &mut self,
        source: Entity<TabContainer>,
        tab_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if source == self.primary_pane {
            return;
        }

        self.suppress_cleanup = true;
        let moved_tab = source.update(cx, |source, cx| source.take_tab(tab_index, window, cx));
        self.suppress_cleanup = false;
        let Some(tab) = moved_tab else {
            return;
        };

        self.primary_pane.update(cx, |pane, cx| {
            pane.insert_tab_at_end_and_activate(tab, window, cx);
        });
        self.active_pane = self.primary_pane.clone();
        self.cleanup_empty_panes(cx);
        cx.emit(SplitTabContainerEvent::LayoutChanged);
        cx.emit(SplitTabContainerEvent::ActivePaneChanged);
        cx.notify();
    }

    fn cleanup_empty_panes(&mut self, cx: &mut Context<Self>) {
        self.root = Self::prune_node(self.root.clone(), &self.primary_pane, cx)
            .unwrap_or_else(|| SplitNode::Leaf(self.primary_pane.clone()));
        if !self.root.contains(&self.active_pane) {
            self.active_pane = self.primary_pane.clone();
        }
        cx.notify();
    }

    fn prune_node(
        node: SplitNode,
        primary_pane: &Entity<TabContainer>,
        cx: &App,
    ) -> Option<SplitNode> {
        match node {
            SplitNode::Leaf(pane) => {
                if &pane == primary_pane || !pane.read(cx).is_empty() {
                    Some(SplitNode::Leaf(pane))
                } else {
                    None
                }
            }
            SplitNode::Split { axis, children } => {
                let children: Vec<_> = children
                    .into_iter()
                    .filter_map(|child| Self::prune_node(child, primary_pane, cx))
                    .collect();
                match children.len() {
                    0 => None,
                    1 => children.into_iter().next(),
                    _ => Some(SplitNode::Split { axis, children }),
                }
            }
        }
    }

    fn render_node(&self, node: &SplitNode, path: &str) -> AnyElement {
        match node {
            SplitNode::Leaf(pane) => div()
                .id(SharedString::from(format!("split-pane-{path}")))
                .size_full()
                .overflow_hidden()
                .child(pane.clone())
                .into_any_element(),
            SplitNode::Split { axis, children } => {
                let id = SharedString::from(format!("split-group-{path}"));
                let mut group = if *axis == Axis::Horizontal {
                    h_resizable(id)
                } else {
                    v_resizable(id)
                };

                for (index, child) in children.iter().enumerate() {
                    let child_path = format!("{path}-{index}");
                    group =
                        group.child(resizable_panel().child(self.render_node(child, &child_path)));
                }

                group.into_any_element()
            }
        }
    }

    fn should_render_split_tree(&self, _cx: &App) -> bool {
        split_tree_visible_for_layout(false, self.root.is_split(), false, false)
    }

    fn render_primary_pane(&self) -> AnyElement {
        div()
            .id("split-pane-primary-only")
            .size_full()
            .overflow_hidden()
            .child(self.primary_pane.clone())
            .into_any_element()
    }
}

impl EventEmitter<SplitTabContainerEvent> for SplitTabContainer {}

impl Focusable for SplitTabContainer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.should_render_split_tree(cx) && self.root.contains(&self.active_pane) {
            self.active_pane.read(cx).focus_handle(cx)
        } else {
            self.primary_pane.read(cx).focus_handle(cx)
        }
    }
}

impl Render for SplitTabContainer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.should_render_split_tree(cx) {
            self.render_node(&self.root, "root")
        } else {
            self.render_primary_pane()
        };

        div()
            .id("split-tab-container")
            .size_full()
            .track_focus(&self.focus_handle)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, WindowOptions};
    use gpui_component::Theme;

    #[gpui::test]
    fn split_close_collects_leaf_panes_in_tree_order(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::default());
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let primary = cx.new(|cx| TabContainer::new(window, cx));
                    let right_top = cx.new(|cx| TabContainer::new(window, cx));
                    let right_bottom = cx.new(|cx| TabContainer::new(window, cx));
                    let tree = SplitNode::Split {
                        axis: Axis::Horizontal,
                        children: vec![
                            SplitNode::Leaf(primary.clone()),
                            SplitNode::Split {
                                axis: Axis::Vertical,
                                children: vec![
                                    SplitNode::Leaf(right_top.clone()),
                                    SplitNode::Leaf(right_bottom.clone()),
                                ],
                            },
                        ],
                    };

                    let panes = tree.collect_close_panes();

                    assert_eq!(vec![primary, right_top, right_bottom], panes);
                    panes[0].clone()
                })
                .expect("window opens");
            drop(window);
        });
    }

    #[test]
    fn split_container_exposes_close_all_tabs_contract() {
        let _close_all_tabs: fn(
            &mut SplitTabContainer,
            &mut Window,
            &mut Context<SplitTabContainer>,
        ) -> gpui::Task<bool> = SplitTabContainer::close_all_tabs;
    }
}
