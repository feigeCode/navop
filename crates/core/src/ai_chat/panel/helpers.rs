use gpui::{App, ElementId, SharedString, Styled};
use gpui_component::{
    IconName, Selectable, Sizable, Size,
    button::{Button, ButtonVariants as _},
};

pub(crate) fn plan_backend_option_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: IconName,
    selected: bool,
    on_select: impl Fn(&mut App) + 'static,
) -> Button {
    Button::new(id)
        .icon(icon)
        .label(label)
        .ghost()
        .with_size(Size::Small)
        .selected(selected)
        .w_full()
        .on_click(move |_, _window, cx| on_select(cx))
}
