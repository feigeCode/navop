use std::{cell::RefCell, rc::Rc};

use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, Element, ElementId, Entity, Focusable,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels, Point, StyleRefinement,
    Styled, Subscription, Window, anchored, deferred, div, prelude::FluentBuilder, px,
};

use crate::menu::PopupMenu;

type PopupMenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// A extension trait for adding a context menu to an element.
pub trait ContextMenuExt: InteractiveElement + ParentElement + Styled {
    /// Add a context menu to the element.
    ///
    /// This will changed the element to be `relative` positioned, and add a child `ContextMenu` element.
    /// Because the `ContextMenu` element is positioned `absolute`, it will not affect the layout of the parent element.
    fn context_menu(
        mut self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> ContextMenu<Self>
    where
        Self: Sized,
    {
        // Generate a unique ID based on the element's memory address to ensure
        // each context menu has its own state and doesn't share with others
        let id = self
            .interactivity()
            .element_id
            .clone()
            .map(|id| format!("context-menu-{:?}", id))
            .unwrap_or_else(|| format!("context-menu-{:p}", &self as *const _));
        ContextMenu::new(id, self).menu(f)
    }
}

impl<E: InteractiveElement + ParentElement + Styled> ContextMenuExt for E {}

/// A context menu that can be shown on right-click.
pub struct ContextMenu<E: ParentElement + Styled + Sized> {
    id: ElementId,
    element: Option<E>,
    menu: Option<PopupMenuBuilder>,
    // This is not in use, just for style refinement forwarding.
    _ignore_style: StyleRefinement,
    anchor: Anchor,
}

impl<E: ParentElement + Styled> ContextMenu<E> {
    /// Create a new context menu with the given ID.
    pub fn new(id: impl Into<ElementId>, element: E) -> Self {
        Self {
            id: id.into(),
            element: Some(element),
            menu: None,
            anchor: Anchor::TopLeft,
            _ignore_style: StyleRefinement::default(),
        }
    }

    /// Build the context menu using the given builder function.
    #[must_use]
    fn menu<F>(mut self, builder: F) -> Self
    where
        F: Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    {
        self.menu = Some(Rc::new(builder));
        self
    }

    fn with_element_state<R>(
        &mut self,
        id: &GlobalElementId,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&mut Self, &mut ContextMenuState, &mut Window, &mut App) -> R,
    ) -> R {
        window.with_optional_element_state::<ContextMenuState, _>(
            Some(id),
            |element_state, window| {
                let mut element_state = element_state.unwrap().unwrap_or_default();
                let result = f(self, &mut element_state, window, cx);
                (result, Some(element_state))
            },
        )
    }
}

impl<E: ParentElement + Styled> ParentElement for ContextMenu<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        if let Some(element) = &mut self.element {
            element.extend(elements);
        }
    }
}

impl<E: ParentElement + Styled> Styled for ContextMenu<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        if let Some(element) = &mut self.element {
            element.style()
        } else {
            &mut self._ignore_style
        }
    }
}

impl<E: ParentElement + Styled + IntoElement + 'static> IntoElement for ContextMenu<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct ContextMenuSharedState {
    menu_view: Option<Entity<PopupMenu>>,
    open: bool,
    position: Point<Pixels>,
    _subscription: Option<Subscription>,
}

fn should_open_context_menu(state: &ContextMenuSharedState, position: Point<Pixels>) -> bool {
    !state.open || state.position != position
}

fn mark_context_menu_open(state: &mut ContextMenuSharedState, position: Point<Pixels>) {
    state.menu_view = None;
    state._subscription = None;
    state.position = position;
    state.open = true;
}

fn open_context_menu_at(
    shared_state: Rc<RefCell<ContextMenuSharedState>>,
    builder: Option<PopupMenuBuilder>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    {
        let mut state = shared_state.borrow_mut();
        if !should_open_context_menu(&state, position) {
            return;
        }
        mark_context_menu_open(&mut state, position);
    }

    window.defer(cx, move |window, cx| {
        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
            let Some(build) = &builder else {
                return menu;
            };
            build(menu, window, cx)
        });

        let _subscription = window.subscribe(&menu, cx, {
            let shared_state = shared_state.clone();
            move |_, _: &DismissEvent, window, _cx| {
                shared_state.borrow_mut().open = false;
                window.refresh();
            }
        });

        {
            let mut state = shared_state.borrow_mut();
            state.menu_view = Some(menu.clone());
            state._subscription = Some(_subscription);
            window.refresh();
        }
    });
}

pub struct ContextMenuState {
    element: Option<AnyElement>,
    shared_state: Rc<RefCell<ContextMenuSharedState>>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            element: None,
            shared_state: Rc::new(RefCell::new(ContextMenuSharedState {
                menu_view: None,
                open: false,
                position: Default::default(),
                _subscription: None,
            })),
        }
    }
}

impl<E: ParentElement + Styled + IntoElement + 'static> Element for ContextMenu<E> {
    type RequestLayoutState = ContextMenuState;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let anchor = self.anchor;

        self.with_element_state(
            id.unwrap(),
            window,
            cx,
            |this, state: &mut ContextMenuState, window, cx| {
                let (position, open) = {
                    let shared_state = state.shared_state.borrow();
                    (shared_state.position, shared_state.open)
                };
                let menu_view = state.shared_state.borrow().menu_view.clone();
                let mut menu_element = None;
                if open {
                    let has_menu_item = menu_view
                        .as_ref()
                        .map(|menu| !menu.read(cx).is_empty())
                        .unwrap_or(false);

                    if has_menu_item {
                        menu_element = Some(
                            deferred(
                                anchored().child(
                                    div()
                                        .w(window.bounds().size.width)
                                        .h(window.bounds().size.height)
                                        .on_scroll_wheel(|_, _, cx| {
                                            cx.stop_propagation();
                                        })
                                        .child(
                                            anchored()
                                                .position(position)
                                                .snap_to_window_with_margin(px(8.))
                                                .anchor(anchor)
                                                .when_some(menu_view, |this, menu| {
                                                    // Focus the menu, so that can be handle the action.
                                                    if !menu
                                                        .focus_handle(cx)
                                                        .contains_focused(window, cx)
                                                    {
                                                        menu.focus_handle(cx).focus(window, cx);
                                                    }

                                                    this.child(menu.clone())
                                                }),
                                        ),
                                ),
                            )
                            .with_priority(1)
                            .into_any(),
                        );
                    }
                }

                let mut element = this
                    .element
                    .take()
                    .expect("Element should exists.")
                    .children(menu_element)
                    .into_any_element();

                let layout_id = element.request_layout(window, cx);

                (
                    layout_id,
                    ContextMenuState {
                        element: Some(element),
                        ..Default::default()
                    },
                )
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(element) = &mut request_layout.element {
            element.prepaint(window, cx);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = &mut request_layout.element {
            element.paint(window, cx);
        }

        // Take the builder before setting up element state to avoid borrow issues
        let builder = self.menu.clone();

        self.with_element_state(
            id.unwrap(),
            window,
            cx,
            |_view, state: &mut ContextMenuState, window, _| {
                let shared_state = state.shared_state.clone();

                let hitbox_for_down = hitbox.clone();
                let state_for_down = shared_state.clone();
                let builder_for_down = builder.clone();
                // Build the context menu from mouse down on platforms that emit it.
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase.bubble()
                        && event.button == MouseButton::Right
                        && hitbox_for_down.is_hovered(window)
                    {
                        open_context_menu_at(
                            state_for_down.clone(),
                            builder_for_down.clone(),
                            event.position,
                            window,
                            cx,
                        );
                    }
                });

                let hitbox_for_up = hitbox.clone();
                let state_for_up = shared_state.clone();
                let builder_for_up = builder.clone();
                // Some platforms, including the OHOS backend, surface context-menu
                // clicks on button release instead of button press.
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase.bubble()
                        && event.button == MouseButton::Right
                        && hitbox_for_up.is_hovered(window)
                    {
                        open_context_menu_at(
                            state_for_up.clone(),
                            builder_for_up.clone(),
                            event.position,
                            window,
                            cx,
                        );
                    }
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn test_state(position: Point<Pixels>, open: bool) -> ContextMenuSharedState {
        ContextMenuSharedState {
            menu_view: None,
            open,
            position,
            _subscription: None,
        }
    }

    fn point(x: f32, y: f32) -> Point<Pixels> {
        Point { x: px(x), y: px(y) }
    }

    #[test]
    fn context_menu_suppresses_same_click_release_after_press() {
        let position = point(10.0, 20.0);
        let mut state = test_state(position, false);

        assert!(should_open_context_menu(&state, position));
        mark_context_menu_open(&mut state, position);

        assert!(!should_open_context_menu(&state, position));
    }

    #[test]
    fn context_menu_allows_release_only_or_new_position() {
        let position = point(10.0, 20.0);
        let new_position = point(20.0, 20.0);
        let open_state = test_state(position, true);
        let closed_state = test_state(position, false);

        assert!(should_open_context_menu(&closed_state, position));
        assert!(should_open_context_menu(&open_state, new_position));
    }
}
