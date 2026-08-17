use gpui::prelude::FluentBuilder;
use gpui::{
    ColorExt as _, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    popover::Popover,
    scroll::ScrollableElement,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use rust_i18n::t;

use super::form::{
    CREDENTIAL_KIND_OPTIONS, CredentialForm, credential_kind_description, credential_kind_label,
    ordered_credential_kinds,
};

impl Render for CredentialForm {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let active_tab = self.active_tab;

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .justify_center()
                    .px_4()
                    .pt_3()
                    .child(
                        TabBar::new("credential-form-tabs")
                            .with_size(Size::Small)
                            .underline()
                            .selected_index(active_tab)
                            .on_click(cx.listener(|form, index: &usize, _, cx| {
                                form.active_tab = *index;
                                cx.notify();
                            }))
                            .child(Tab::new().label(t!("CredentialForm.tab_basic").to_string()))
                            .child(Tab::new().label(t!("CredentialForm.tab_ssh_key").to_string()))
                            .child(
                                Tab::new().label(t!("CredentialForm.tab_auto_login").to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .id("credential-form-content")
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(div().size_full().p_4().overflow_y_scrollbar().child(
                        match active_tab {
                            0 => self.render_basic_tab(cx).into_any_element(),
                            1 => self.render_ssh_key_tab(cx).into_any_element(),
                            2 => self.render_account_expect_tab(cx).into_any_element(),
                            _ => div().into_any_element(),
                        },
                    )),
            )
    }
}

impl CredentialForm {
    fn render_basic_tab(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .child(form_field(
                t!("CredentialForm.name").to_string(),
                Input::new(&self.name_input).w_full(),
            ))
            .when(self.is_editing(), |this| {
                this.child(form_field(
                    t!("CredentialForm.applicable_types").to_string(),
                    self.render_kind_picker(cx),
                ))
            })
            .child(form_field(
                t!("CredentialForm.username").to_string(),
                Input::new(&self.username_input).w_full(),
            ))
            .child(form_field(
                t!("CredentialForm.password").to_string(),
                Input::new(&self.password_input).w_full().mask_toggle(),
            ))
            .child(self.render_sync_settings(cx))
    }

    fn render_kind_picker(&self, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let selected_kinds = self.selected_kinds.clone();
        let ordered_selected = ordered_credential_kinds(&selected_kinds);
        let trigger_label = match ordered_selected.as_slice() {
            [] => t!("CredentialForm.select_types").to_string(),
            [kind] => credential_kind_label(kind),
            [first, second] => t!(
                "CredentialForm.selected_two_types",
                first = credential_kind_label(first),
                second = credential_kind_label(second)
            )
            .to_string(),
            [first, second, ..] => t!(
                "CredentialForm.selected_many_types",
                first = credential_kind_label(first),
                second = credential_kind_label(second),
                count = ordered_selected.len()
            )
            .to_string(),
        };
        let mut options = CREDENTIAL_KIND_OPTIONS
            .iter()
            .map(|kind| {
                (
                    (*kind).to_string(),
                    credential_kind_label(kind),
                    credential_kind_description(kind),
                )
            })
            .collect::<Vec<_>>();
        options.extend(
            ordered_selected
                .iter()
                .filter(|kind| {
                    !CREDENTIAL_KIND_OPTIONS
                        .iter()
                        .any(|option| *option == kind.as_str())
                })
                .map(|kind| {
                    (
                        kind.clone(),
                        kind.clone(),
                        credential_kind_description(kind),
                    )
                }),
        );

        let form = cx.entity();
        Popover::new("credential-kind-picker")
            .open(self.kind_picker_open)
            .on_open_change(cx.listener(|form, open, _, cx| {
                form.kind_picker_open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("credential-kind-picker-trigger")
                    .label(trigger_label)
                    .dropdown_caret(true)
                    .w_full(),
            )
            .content(move |_, window, cx| {
                let selected_count = selected_kinds.len();
                let content_width =
                    px((window.viewport_size().width.as_f32() - 32.0).clamp(0.0, 360.0));
                v_flex()
                    .w(content_width)
                    .max_h(px(440.0))
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(
                                                t!("CredentialForm.select_applicable_types")
                                                    .to_string(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                t!(
                                                    "CredentialForm.selected_type_count",
                                                    count = selected_count
                                                )
                                                .to_string(),
                                            ),
                                    ),
                            )
                            .child({
                                let form = form.clone();
                                Button::new("credential-kind-clear")
                                    .small()
                                    .ghost()
                                    .label(t!("CredentialForm.clear").to_string())
                                    .on_click(move |_, _, cx| {
                                        form.update(cx, |form, cx| {
                                            form.selected_kinds.clear();
                                            cx.notify();
                                        });
                                    })
                            }),
                    )
                    .child(div().border_t_1().border_color(cx.theme().border))
                    .child(
                        div()
                            .w_full()
                            .max_h(px(340.0))
                            .overflow_y_scrollbar()
                            .child(v_flex().w_full().gap_1().children(options.iter().map(
                                |(kind, label, description)| {
                                    let checked = selected_kinds.contains(kind);
                                    let kind_for_click = kind.clone();
                                    let form = form.clone();
                                    v_flex()
                                        .w_full()
                                        .gap_0p5()
                                        .rounded_md()
                                        .px_2()
                                        .py_1p5()
                                        .hover(|this| this.bg(cx.theme().muted.opacity(0.4)))
                                        .child(
                                            Checkbox::new(format!("credential-kind-option-{kind}"))
                                                .checked(checked)
                                                .label(label.clone())
                                                .on_click(move |checked, _, cx| {
                                                    form.update(cx, |form, cx| {
                                                        if *checked {
                                                            form.selected_kinds
                                                                .insert(kind_for_click.clone());
                                                        } else {
                                                            form.selected_kinds
                                                                .remove(&kind_for_click);
                                                        }
                                                        cx.notify();
                                                    });
                                                }),
                                        )
                                        .child(
                                            div()
                                                .ml_6()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(description.clone()),
                                        )
                                },
                            ))),
                    )
            })
            .into_any_element()
    }

    fn render_ssh_key_tab(&self, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .child(info_panel(
                t!("CredentialForm.ssh_key_title").to_string(),
                t!("CredentialForm.ssh_key_description").to_string(),
                cx,
            ))
            .child(form_field(
                t!("CredentialForm.private_key_path").to_string(),
                Input::new(&self.private_key_path_input).w_full(),
            ))
            .child(form_field(
                t!("CredentialForm.private_key_content").to_string(),
                div().w_full().h(px(150.0)).child(
                    Input::new(&self.private_key_content_input)
                        .w_full()
                        .h_full(),
                ),
            ))
            .child(form_field(
                t!("CredentialForm.passphrase").to_string(),
                Input::new(&self.passphrase_input).w_full().mask_toggle(),
            ))
    }

    fn render_account_expect_tab(&self, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .child(info_panel(
                t!("CredentialForm.auto_login_title").to_string(),
                t!("CredentialForm.auto_login_description").to_string(),
                cx,
            ))
            .child(form_field(
                t!("CredentialForm.username_expect").to_string(),
                Input::new(&self.username_expect_input).w_full(),
            ))
            .child(form_field(
                t!("CredentialForm.username_send").to_string(),
                Input::new(&self.username_send_input).w_full(),
            ))
            .child(form_field(
                t!("CredentialForm.password_expect").to_string(),
                Input::new(&self.password_expect_input).w_full(),
            ))
            .child(form_field(
                t!("CredentialForm.password_send").to_string(),
                Input::new(&self.password_send_input).w_full().mask_toggle(),
            ))
    }

    fn render_sync_settings(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_4()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .p_3()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("CredentialForm.allow_sync").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("CredentialForm.allow_sync_description").to_string()),
                    ),
            )
            .child(
                Switch::new("credential-sync-enabled")
                    .checked(self.sync_enabled)
                    .on_click(cx.listener(|form, checked, _, cx| {
                        form.sync_enabled = *checked;
                        cx.notify();
                    })),
            )
    }
}

fn info_panel(title: String, description: String, cx: &gpui::App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted.opacity(0.35))
        .p_3()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
}

fn form_field(label: String, input: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        )
        .child(input)
}

#[cfg(test)]
mod tests {
    #[test]
    fn credential_form_uses_grouped_tabs_with_a_bounded_scroll_region() {
        let render = include_str!("form_render.rs");
        let window = include_str!("form_window.rs");
        let production = render
            .split("#[cfg(test)]")
            .next()
            .expect("production render source");

        assert!(render.contains("TabBar::new(\"credential-form-tabs\")"));
        assert!(render.contains("CredentialForm.tab_basic"));
        assert!(render.contains("CredentialForm.tab_ssh_key"));
        assert!(render.contains("CredentialForm.tab_auto_login"));
        assert!(!production.contains("CredentialForm.tab_sync"));
        assert!(render.contains(".id(\"credential-form-content\")"));
        assert!(render.contains(".min_h_0()"));
        assert!(render.contains(".overflow_hidden()"));
        assert!(render.contains(".overflow_y_scrollbar()"));
        assert!(window.contains(".size_full()"));
        assert!(window.contains(".min_h_0()"));
        assert!(window.contains(".overflow_hidden()"));
    }

    #[test]
    fn credential_fields_are_grouped_across_basic_and_ssh_tabs() {
        let source = include_str!("form_render.rs");
        let basic = source
            .split("fn render_basic_tab")
            .nth(1)
            .and_then(|source| source.split("fn render_ssh_key_tab").next())
            .expect("basic tab");
        let ssh_key = source
            .split("fn render_ssh_key_tab")
            .nth(1)
            .and_then(|source| source.split("fn render_account_expect_tab").next())
            .expect("SSH key tab");
        let expect = source
            .split("fn render_account_expect_tab")
            .nth(1)
            .and_then(|source| source.split("fn render_sync_settings").next())
            .expect("expect tab");
        let sync = source
            .split("fn render_sync_settings")
            .nth(1)
            .and_then(|source| source.split("fn info_panel").next())
            .expect("sync settings");

        for field in [
            "self.name_input",
            "self.render_kind_picker",
            "self.username_input",
            "self.password_input",
        ] {
            assert!(basic.contains(field));
        }
        assert!(basic.contains(".when(self.is_editing(),"));
        assert!(basic.contains("self.render_sync_settings(cx)"));
        assert!(source.contains("Popover::new(\"credential-kind-picker\")"));
        assert!(source.contains("Checkbox::new(format!("));
        assert!(source.contains(".dropdown_caret(true)"));
        assert!(source.contains("window.viewport_size().width.as_f32()"));
        assert!(source.contains(".overflow_y_scrollbar()"));
        assert!(!basic.contains("self.private_key_content_input"));

        for field in [
            "self.private_key_path_input",
            "self.private_key_content_input",
            "self.passphrase_input",
        ] {
            assert!(ssh_key.contains(field));
        }
        for field in [
            "self.username_expect_input",
            "self.username_send_input",
            "self.password_expect_input",
            "self.password_send_input",
        ] {
            assert!(expect.contains(field));
        }
        assert!(!basic.contains("self.username_expect_input"));
        assert!(!ssh_key.contains("self.username_expect_input"));
        assert!(sync.contains("credential-sync-enabled"));
    }

    #[test]
    fn new_credentials_hide_applicable_type_and_keep_sync_in_basic_info() {
        let render = include_str!("form_render.rs");
        let production = render
            .split("#[cfg(test)]")
            .next()
            .expect("production render source");

        assert!(render.contains(".when(self.is_editing(),"));
        assert!(render.contains("self.render_sync_settings(cx)"));
        assert!(!production.contains("CredentialForm.tab_sync"));
    }
}
