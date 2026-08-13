use std::{collections::HashMap, rc::Rc};

use gpui::{AnyWindowHandle, App, Global, Subscription, WindowId};

type WindowCloseHandler = Rc<dyn Fn(AnyWindowHandle, &mut App) + 'static>;

#[derive(Default)]
struct WindowCloseState {
    handlers: HashMap<WindowId, Option<WindowCloseHandler>>,
}

impl WindowCloseState {
    fn register(&mut self, window_id: WindowId) {
        self.handlers.entry(window_id).or_default();
    }

    fn set_handler(&mut self, window_id: WindowId, handler: WindowCloseHandler) {
        self.handlers.insert(window_id, Some(handler));
    }

    fn handler(&self, window_id: WindowId) -> Option<WindowCloseHandler> {
        self.handlers.get(&window_id).and_then(Clone::clone)
    }

    fn remove(&mut self, window_id: WindowId) {
        self.handlers.remove(&window_id);
    }

    #[cfg(test)]
    fn contains(&self, window_id: WindowId) -> bool {
        self.handlers.contains_key(&window_id)
    }
}

struct WindowCloseRegistry {
    state: WindowCloseState,
    _window_closed_subscription: Subscription,
}

impl Global for WindowCloseRegistry {}

pub fn init(cx: &mut App) {
    if cx.has_global::<WindowCloseRegistry>() {
        return;
    }

    let subscription = cx.on_window_closed(|cx, window_id| {
        if cx.has_global::<WindowCloseRegistry>() {
            cx.global_mut::<WindowCloseRegistry>()
                .state
                .remove(window_id);
        }
    });
    cx.set_global(WindowCloseRegistry {
        state: WindowCloseState::default(),
        _window_closed_subscription: subscription,
    });
}

pub fn register_window(window_handle: AnyWindowHandle, cx: &mut App) {
    init(cx);
    cx.global_mut::<WindowCloseRegistry>()
        .state
        .register(window_handle.window_id());
}

pub fn set_window_close_handler(
    window_handle: AnyWindowHandle,
    handler: impl Fn(AnyWindowHandle, &mut App) + 'static,
    cx: &mut App,
) {
    init(cx);
    cx.global_mut::<WindowCloseRegistry>()
        .state
        .set_handler(window_handle.window_id(), Rc::new(handler));
}

pub fn request_close_window(window_handle: AnyWindowHandle, cx: &mut App) {
    let handler = cx
        .try_global::<WindowCloseRegistry>()
        .and_then(|registry| registry.state.handler(window_handle.window_id()));

    if let Some(handler) = handler {
        handler(window_handle, cx);
        return;
    }

    cx.defer(move |cx| {
        let _ = window_handle.update(cx, |_, window, _| window.remove_window());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_registration_defaults_to_direct_close() {
        let window_id = WindowId::from(1);
        let mut state = WindowCloseState::default();

        state.register(window_id);

        assert!(state.contains(window_id));
        assert!(state.handler(window_id).is_none());
    }

    #[test]
    fn custom_handler_replaces_the_default_close_route() {
        let window_id = WindowId::from(2);
        let mut state = WindowCloseState::default();
        let handler: WindowCloseHandler = Rc::new(|_, _| {});

        state.register(window_id);
        state.set_handler(window_id, handler);

        assert!(state.handler(window_id).is_some());
    }

    #[test]
    fn closed_window_is_removed_from_the_registry() {
        let window_id = WindowId::from(3);
        let mut state = WindowCloseState::default();
        state.register(window_id);

        state.remove(window_id);

        assert!(!state.contains(window_id));
    }
}
