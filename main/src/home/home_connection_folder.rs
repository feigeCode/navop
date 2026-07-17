use crate::home_tab::HomePage;
use gpui::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt, h_flex,
    input::{Input, InputState},
    v_flex,
};
use one_core::storage::ConnectionType;
use rust_i18n::t;

pub(crate) fn show_folder_dialog(
    parent: Entity<HomePage>,
    folder_id: Option<i64>,
    connection_type: ConnectionType,
    parent_id: Option<i64>,
    initial_name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name_input = cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(t!("Home.folder_name_placeholder"))
            .clean_on_escape();
        if !initial_name.is_empty() {
            state.set_value(initial_name.clone(), window, cx);
        }
        state
    });

    name_input.update(cx, |state, cx| {
        state.focus(window, cx);
    });

    let input_for_render = name_input.clone();
    let input_for_ok = name_input.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let parent_for_ok = parent.clone();
        let input_for_ok = input_for_ok.clone();
        dialog
            .title(
                if folder_id.is_some() {
                    t!("Folder.edit").to_string()
                } else {
                    t!("Folder.new").to_string()
                }
                .into_any_element(),
            )
            .child(
                v_flex()
                    .gap_3()
                    .w(px(360.0))
                    .child(Input::new(&input_for_render).w_full()),
            )
            .confirm()
            .on_ok(move |_, _, cx| {
                let name = input_for_ok.read(cx).text().to_string().trim().to_string();
                if name.is_empty() {
                    return false;
                }
                let _ = parent_for_ok.update(cx, |home, cx| {
                    home.handle_save_folder(folder_id, name, connection_type, parent_id, cx);
                });
                true
            })
    });
}

#[derive(Clone)]
pub(crate) struct DragConnectionFolder {
    pub source_id: i64,
    pub name: String,
    pub connection_type: ConnectionType,
    pub parent_id: Option<i64>,
}

impl Render for DragConnectionFolder {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("drag-connection-folder")
            .cursor_grabbing()
            .w(px(220.0))
            .px_3()
            .py_2()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(Icon::new(IconName::Folder).with_size(Size::Small))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name.clone()),
            )
    }
}

#[derive(Clone)]
pub(crate) struct DragSidebarConnection {
    pub connection_id: i64,
    pub name: String,
    pub connection_type: ConnectionType,
}

impl Render for DragSidebarConnection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("drag-sidebar-connection")
            .cursor_grabbing()
            .w(px(220.0))
            .px_3()
            .py_2()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(Icon::new(self.connection_type.icon()).with_size(Size::Small))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name.clone()),
            )
    }
}

pub(crate) fn confirm_delete_folder(
    parent: Entity<HomePage>,
    folder_id: i64,
    folder_name: String,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let parent_for_ok = parent.clone();
        dialog
            .title(t!("Home.delete_folder").to_string().into_any_element())
            .child(
                div()
                    .w(px(360.0))
                    .text_sm()
                    .child(t!("Home.delete_folder_confirm", name = folder_name).to_string()),
            )
            .confirm()
            .on_ok(move |_, _, cx| {
                let _ = parent_for_ok.update(cx, |home, cx| {
                    home.handle_delete_folder(folder_id, cx);
                });
                true
            })
    });
}
