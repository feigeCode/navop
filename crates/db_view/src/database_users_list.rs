use crate::database_users_tab::DatabaseUsersTab;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ListSizingBehavior, MouseButton,
    MouseDownEvent, ParentElement, Styled, div, uniform_list,
};
use std::ops::Range;

pub(super) fn users_list(row_count: usize, cx: &mut Context<DatabaseUsersTab>) -> AnyElement {
    div()
        .flex_1()
        .overflow_hidden()
        .child(
            uniform_list(
                "database-users-list",
                row_count,
                cx.processor(
                    move |state: &mut DatabaseUsersTab, range: Range<usize>, _, cx| {
                        range
                            .map(|row_ix| user_row_entry(state, row_ix, cx))
                            .collect()
                    },
                ),
            )
            .with_sizing_behavior(ListSizingBehavior::Infer),
        )
        .into_any_element()
}

fn user_row_entry(
    state: &mut DatabaseUsersTab,
    row_ix: usize,
    cx: &mut Context<DatabaseUsersTab>,
) -> AnyElement {
    div()
        .id(row_ix)
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                this.select_row(row_ix, cx);
            }),
        )
        .child(state.render_row(row_ix, cx))
        .into_any_element()
}
