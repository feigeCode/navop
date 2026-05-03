use gpui::prelude::FluentBuilder;
use gpui::{
    AnyView, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, InteractiveElementExt, Sizable, Size, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

use crate::home_tab::HomePage;
use crate::new_connection::connection_kind::{NewConnectionCategory, NewConnectionKind};
use crate::new_connection::form_page::{NewConnectionFormPage, NewConnectionFormResult};

pub(crate) struct NewConnectionWindow {
    parent: Entity<HomePage>,
    parent_window: AnyWindowHandle,
    focus_handle: FocusHandle,
    selected_category: NewConnectionCategory,
    selected_kind: Option<NewConnectionKind>,
    form: Option<AnyView>,
}

impl NewConnectionWindow {
    pub(crate) fn new(
        parent: Entity<HomePage>,
        parent_window: AnyWindowHandle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            parent,
            parent_window,
            focus_handle: cx.focus_handle(),
            selected_category: NewConnectionCategory::All,
            selected_kind: None,
            form: None,
        }
    }

    fn visible_items(&self) -> Vec<NewConnectionKind> {
        NewConnectionKind::all()
            .into_iter()
            .filter(|kind| {
                self.selected_category == NewConnectionCategory::All
                    || kind.category() == self.selected_category
            })
            .collect()
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.selected_kind.clone() else {
            return;
        };

        match kind.build_form_view(self.parent.clone(), self.parent_window, window, cx) {
            NewConnectionFormResult::Form(form) => {
                self.form = Some(form);
                cx.notify();
            }
            NewConnectionFormResult::Done => {
                window.remove_window();
            }
            NewConnectionFormResult::Blocked => {
                cx.notify();
            }
        }
    }

    fn go_back_to_selection(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_1()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().foreground)
                .child(t!("Home.new_connection").to_string()),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(180.0))
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .p_2()
            .gap_2()
            .children(NewConnectionCategory::all().into_iter().map(|category| {
                let is_selected = self.selected_category == category;
                div()
                    .id(SharedString::from(format!(
                        "new-connection-category-{}",
                        category.label()
                    )))
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .overflow_hidden()
                    .when(is_selected, |this| {
                        this.bg(cx.theme().list_active)
                            .border_l_3()
                            .border_color(cx.theme().list_active_border)
                    })
                    .when(!is_selected, |this| {
                        this.bg(cx.theme().sidebar)
                            .hover(|style| style.bg(cx.theme().sidebar_accent))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_category = category;
                        this.selected_kind = None;
                        cx.notify();
                    }))
                    .child(Icon::new(category.icon()).color().with_size(Size::Medium))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .when(is_selected, |this| this.font_weight(FontWeight::MEDIUM))
                            .child(category.label()),
                    )
            }))
    }

    fn render_card_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().flex().flex_wrap().w_full().gap_3();
        for kind in self.visible_items() {
            grid = grid.child(
                div()
                    .w(px(280.0))
                    .flex_shrink_0()
                    .child(self.render_connection_type_card(kind, cx)),
            );
        }

        v_flex()
            .flex_1()
            .h_full()
            .overflow_y_scrollbar()
            .bg(cx.theme().muted)
            .p_6()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child("选择连接类型"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("单击选择后点击下一步，或双击直接进入连接表单。"),
                    ),
            )
            .child(grid)
    }

    fn render_connection_type_card(
        &self,
        kind: NewConnectionKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_kind.as_ref() == Some(&kind);
        let click_kind = kind.clone();
        let double_click_kind = kind.clone();
        let label = kind.label();
        let description = kind.description();

        v_flex()
            .id(SharedString::from(format!("new-connection-kind-{}", label)))
            .justify_center()
            .w_full()
            .h(px(112.0))
            .rounded_lg()
            .bg(cx.theme().background)
            .p_3()
            .border_1()
            .relative()
            .overflow_hidden()
            .shadow_sm()
            .cursor_pointer()
            .when(is_selected, |this| {
                this.border_color(cx.theme().list_active_border)
                    .shadow_lg()
                    .border_l_3()
            })
            .when(!is_selected, |this| this.border_color(cx.theme().border))
            .hover(|style| {
                style
                    .shadow_lg()
                    .border_color(cx.theme().list_active_border)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_kind = Some(click_kind.clone());
                cx.notify();
            }))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.selected_kind = Some(double_click_kind.clone());
                this.open_selected(window, cx);
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .w(px(48.0))
                            .h(px(48.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(kind.icon()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(description),
                            ),
                    ),
            )
    }

    fn render_selection_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .justify_end()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("cancel-new-connection")
                    .small()
                    .label(t!("Common.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.remove_window();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("next-new-connection")
                    .small()
                    .primary()
                    .label(t!("Common.next").to_string())
                    .disabled(self.selected_kind.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_selected(window, cx);
                    })),
            )
    }

    fn render_form_page(&self, form: AnyView, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().relative().child(form).child(
            div().absolute().left(px(16.0)).bottom(px(16.0)).child(
                Button::new("back-to-new-connection-kind")
                    .small()
                    .outline()
                    .label("上一步")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_back_to_selection(cx);
                    })),
            ),
        )
    }
}

impl Focusable for NewConnectionWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NewConnectionWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(form) = self.form.clone() {
            return self.render_form_page(form, cx).into_any_element();
        }

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(self.render_card_area(cx)),
            )
            .child(self.render_selection_footer(cx))
            .into_any_element()
    }
}
