use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Axis, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div,
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
