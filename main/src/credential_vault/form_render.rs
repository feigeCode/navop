use gpui::{
    ColorExt as _, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable, Size, h_flex,
    input::Input,
    scroll::ScrollableElement,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use rust_i18n::t;

use super::form::CredentialForm;

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
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production render source");
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
            "self.username_input",
            "self.password_input",
        ] {
            assert!(basic.contains(field));
        }
        assert!(basic.contains("self.render_sync_settings(cx)"));
        assert!(source.contains(".overflow_y_scrollbar()"));
        assert!(!basic.contains("self.private_key_content_input"));
        assert!(!production.contains("Popover::new(\"credential-kind-picker\")"));
        assert!(!production.contains("Checkbox::new(format!("));
        assert!(!production.contains(".dropdown_caret(true)"));

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
    fn credential_type_picker_is_removed_and_sync_stays_in_basic_info() {
        let render = include_str!("form_render.rs");
        let production = render
            .split("#[cfg(test)]")
            .next()
            .expect("production render source");

        assert!(!production.contains("render_kind_picker"));
        assert!(!production.contains(".when(self.is_editing(),"));
        assert!(production.contains("self.render_sync_settings(cx)"));
        assert!(!production.contains("CredentialForm.tab_sync"));
    }
}
