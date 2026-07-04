use crate::database_users_tab::DatabaseUsersTab;
use db::plugin_manifest::DatabaseCapabilities;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{ActiveTheme, IconName, Sizable, Size, button::Button};

#[derive(Clone, Copy)]
pub(super) enum DatabaseUsersToolbarAction {
    Add,
    Edit,
    Delete,
    Privileges,
    Refresh,
}

pub(super) fn render_users_toolbar(
    connection_name: String,
    capabilities: DatabaseCapabilities,
    window: &mut Window,
    cx: &mut Context<DatabaseUsersTab>,
) -> AnyElement {
    let mut actions = Vec::new();
    if capabilities.supports_user_create {
        actions.push((
            "users-add",
            IconName::Plus,
            "新增",
            DatabaseUsersToolbarAction::Add,
        ));
    }
    if capabilities.supports_user_edit {
        actions.push((
            "users-edit",
            IconName::Edit,
            "编辑",
            DatabaseUsersToolbarAction::Edit,
        ));
    }
    if capabilities.supports_user_delete {
        actions.push((
            "users-delete",
            IconName::Remove,
            "删除",
            DatabaseUsersToolbarAction::Delete,
        ));
    }
    if capabilities.supports_user_privileges {
        actions.push((
            "users-lock",
            IconName::GoldKey,
            "权限",
            DatabaseUsersToolbarAction::Privileges,
        ));
    }
    actions.push((
        "users-refresh",
        IconName::Refresh,
        "刷新",
        DatabaseUsersToolbarAction::Refresh,
    ));

    actions
        .into_iter()
        .fold(toolbar_base(), |toolbar, (id, icon, label, action)| {
            toolbar.child(toolbar_button(id, icon, label, action, window, cx))
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
    action: DatabaseUsersToolbarAction,
    window: &mut Window,
    cx: &mut Context<DatabaseUsersTab>,
) -> AnyElement {
    Button::new(id)
        .with_size(Size::Medium)
        .icon(icon)
        .tooltip(label)
        .on_click(
            window.listener_for(&cx.entity(), move |this, _, window, cx| {
                this.handle_toolbar_action(action, window, cx);
            }),
        )
        .into_any_element()
}
