use gpui::{AnyElement, App, IntoElement, Window};
use gpui_component::{
    IconName, Sizable, Size,
    button::{Button, ButtonVariants},
};

#[derive(Clone, Debug)]
pub enum SendButtonEvent {
    Submit,
    Cancel,
}

#[derive(Clone, Debug, Default)]
pub struct SendButtonState {
    pub is_loading: bool,
    pub send_label: String,
    pub cancel_label: String,
}

impl SendButtonState {
    pub fn new() -> Self {
        Self {
            is_loading: false,
            send_label: "发送".to_string(),
            cancel_label: "取消".to_string(),
        }
    }

    pub fn with_loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    pub fn with_send_label(mut self, label: impl Into<String>) -> Self {
        self.send_label = label.into();
        self
    }

    pub fn with_cancel_label(mut self, label: impl Into<String>) -> Self {
        self.cancel_label = label.into();
        self
    }

    pub fn set_loading(&mut self, is_loading: bool) {
        self.is_loading = is_loading;
    }
}

pub struct SendButton;

impl SendButton {
    pub fn render<F, G>(state: &SendButtonState, on_submit: F, on_cancel: G) -> AnyElement
    where
        F: Fn(&mut Window, &mut App) + 'static,
        G: Fn(&mut Window, &mut App) + 'static,
    {
        if state.is_loading {
            Button::new("send-cancel")
                .with_size(Size::Small)
                .danger()
                .icon(IconName::CircleX)
                .label(state.cancel_label.clone())
                .on_click(move |_, window, cx| on_cancel(window, cx))
                .into_any_element()
        } else {
            Button::new("send-submit")
                .with_size(Size::Small)
                .primary()
                .icon(IconName::ArrowRight)
                .label(state.send_label.clone())
                .on_click(move |_, window, cx| on_submit(window, cx))
                .into_any_element()
        }
    }

    pub fn render_with_id<F, G>(
        id: impl Into<gpui::ElementId>,
        state: &SendButtonState,
        on_submit: F,
        on_cancel: G,
    ) -> AnyElement
    where
        F: Fn(&mut Window, &mut App) + 'static,
        G: Fn(&mut Window, &mut App) + 'static,
    {
        let id = id.into();
        if state.is_loading {
            Button::new(id)
                .with_size(Size::Small)
                .danger()
                .icon(IconName::CircleX)
                .label(state.cancel_label.clone())
                .on_click(move |_, window, cx| on_cancel(window, cx))
                .into_any_element()
        } else {
            Button::new(id)
                .with_size(Size::Small)
                .primary()
                .icon(IconName::ArrowRight)
                .label(state.send_label.clone())
                .on_click(move |_, window, cx| on_submit(window, cx))
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_button_state_defaults_to_not_loading() {
        let state = SendButtonState::new();

        assert!(!state.is_loading);
        assert_eq!(state.send_label, "发送");
        assert_eq!(state.cancel_label, "取消");
    }

    #[test]
    fn send_button_state_builders_update_labels_and_loading() {
        let state = SendButtonState::new()
            .with_loading(true)
            .with_send_label("Run")
            .with_cancel_label("Stop");

        assert!(state.is_loading);
        assert_eq!(state.send_label, "Run");
        assert_eq!(state.cancel_label, "Stop");
    }
}
