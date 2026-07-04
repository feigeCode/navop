use crate::database_users_tab::DatabaseUsersTab;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, IconName, Sizable, Size, WindowExt, button::Button, notification::Notification,
};

pub(super) fn render_users_toolbar(
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<DatabaseUsersTab>,
) -> AnyElement {
    let actions = [
        ("users-add", IconName::Plus, "新增"),
        ("users-edit", IconName::Edit, "编辑"),
        ("users-delete", IconName::Remove, "删除"),
        ("users-lock", IconName::GoldKey, "权限"),
        ("users-refresh", IconName::Refresh, "刷新"),
    ];

    actions
        .into_iter()
        .fold(toolbar_base(), |toolbar, (id, icon, label)| {
            toolbar.child(toolbar_button(id, icon, label, window, cx))
        })
        .child(div().flex_1())
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(connection_name),
        )
        .into_any_element()
}

fn toolbar_base() -> gpui::Div {
    gpui_component::h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_1()
}

fn toolbar_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    window: &mut Window,
    cx: &mut Context<DatabaseUsersTab>,
) -> AnyElement {
    Button::new(id)
        .with_size(Size::Medium)
        .icon(icon)
        .on_click(
            window.listener_for(&cx.entity(), move |this, _, window, cx| {
                if label == "刷新" {
                    this.reload(cx);
                } else {
                    notify_unimplemented(label, window, cx);
                }
            }),
        )
        .into_any_element()
}

fn notify_unimplemented(label: &str, window: &mut Window, cx: &mut Context<DatabaseUsersTab>) {
    window.push_notification(
        Notification::info(format!("{label}用户功能暂未实现。")).autohide(true),
        cx,
    );
}
