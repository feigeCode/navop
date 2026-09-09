pub mod datetime_picker;
pub mod time_picker;

use gpui::{Action, actions};
use gpui::{App, Styled as _};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use serde::Deserialize;

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = one_ui_time, no_json)]
struct Confirm {
    secondary: bool,
}

actions!(one_ui_time, [Cancel]);

fn clear_button(cx: &App) -> Button {
    Button::new("clean")
        .icon(Icon::new(IconName::CircleX))
        .ghost()
        .xsmall()
        .tab_stop(false)
        .text_color(cx.theme().muted_foreground)
}

pub(crate) fn init(cx: &mut App) {
    datetime_picker::init(cx);
    time_picker::init(cx);
}
