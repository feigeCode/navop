use gpui::prelude::FluentBuilder;
use gpui::{
    ColorExt as _, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    v_flex,
};
use one_core::storage::CredentialSummary;
use rust_i18n::t;

use super::form::{credential_kind_label, credential_kind_values};
use super::{CredentialVaultView, button_id, vault_unlocked};

impl Render for CredentialVaultView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let unlocked = vault_unlocked();
        let summaries = self.filtered_summaries(cx);
        let has_search_query = !self.search_input.read(cx).value().trim().is_empty();
        let total = self.summaries.len();
        let filtered = summaries.len();
        let content = if let Some(error) = self.load_error.clone() {
            load_error_state(error, cx).into_any_element()
        } else if summaries.is_empty() {
            empty_state(has_search_query, cx).into_any_element()
        } else {
            credential_list(summaries, total, filtered, cx).into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("credential-vault-add")
                                    .icon(IconName::Plus)
                                    .label(t!("CredentialVault.create").to_string())
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.open_create(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("credential-vault-refresh")
                                    .icon(IconName::Refresh)
                                    .label(t!("CredentialVault.refresh").to_string())
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
                            ),
                    )
                    .child(
                        div().min_w(px(220.0)).max_w(px(420.0)).flex_1().child(
                            Input::new(&self.search_input)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .cleanable(true)
                                .small()
                                .w_full(),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(header(unlocked, total, cx))
                    .child(
                        div()
                            .id("credential-vault-content")
                            .w_full()
                            .min_h_0()
                            .flex_1()
                            .overflow_hidden()
                            .child(content),
                    ),
            )
    }
}

fn header(unlocked: bool, total: usize, cx: &gpui::App) -> impl IntoElement {
    let (status, color) = if unlocked {
        (
            t!("CredentialVault.unlocked").to_string(),
            cx.theme().success,
        )
    } else {
        (t!("CredentialVault.locked").to_string(), cx.theme().warning)
    };
    let security_message = if unlocked {
        t!("CredentialVault.security_unlocked").to_string()
    } else {
        t!("CredentialVault.security_locked").to_string()
    };

    v_flex()
        .w_full()
        .flex_shrink_0()
        .gap_1()
        .px_4()
        .pt_4()
        .pb_3()
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(t!("CredentialVault.title").to_string()),
                )
                .child(
                    div()
                        .rounded_full()
                        .border_1()
                        .border_color(color)
                        .px_2()
                        .py_0p5()
                        .text_xs()
                        .text_color(color)
                        .child(status),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("CredentialVault.total_count", count = total).to_string()),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(security_message),
        )
}

fn credential_list(
    summaries: Vec<CredentialSummary>,
    total: usize,
    filtered: usize,
    cx: &gpui::Context<CredentialVaultView>,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .min_h_0()
        .flex_1()
        .overflow_hidden()
        .child(
            div()
                .flex_shrink_0()
                .px_4()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if total == filtered {
                    t!("CredentialVault.sorted_count", count = total).to_string()
                } else {
                    t!(
                        "CredentialVault.filtered_count",
                        filtered = filtered,
                        total = total
                    )
                    .to_string()
                }),
        )
        .child(
            div()
                .id("credential-vault-list")
                .w_full()
                .min_h_0()
                .flex_1()
                .overflow_y_scrollbar()
                .px_4()
                .pb_4()
                .child(
                    v_flex().gap_2().children(
                        summaries
                            .into_iter()
                            .map(|summary| credential_row(summary, cx)),
                    ),
                ),
        )
}

fn empty_state(
    has_search_query: bool,
    cx: &gpui::Context<CredentialVaultView>,
) -> impl IntoElement {
    let (icon, title, description) = if has_search_query {
        (
            IconName::Search,
            t!("CredentialVault.no_matches").to_string(),
            t!("CredentialVault.no_matches_description").to_string(),
        )
    } else {
        (
            IconName::Key,
            t!("CredentialVault.empty_title").to_string(),
            t!("CredentialVault.empty_description").to_string(),
        )
    };

    v_flex()
        .w_full()
        .min_h_0()
        .flex_1()
        .items_center()
        .justify_center()
        .text_center()
        .gap_3()
        .p_6()
        .child(
            div()
                .w(px(72.0))
                .h(px(72.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(20.0))
                .bg(cx.theme().muted)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(icon).size_8()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(px(560.0))
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
        .when(!has_search_query, |this| {
            this.child(
                Button::new("credential-vault-empty-add")
                    .icon(IconName::Plus)
                    .label(t!("CredentialVault.create").to_string())
                    .primary()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_create(window, cx);
                    })),
            )
        })
}

fn load_error_state(error: String, cx: &gpui::Context<CredentialVaultView>) -> impl IntoElement {
    v_flex()
        .w_full()
        .min_h_0()
        .flex_1()
        .items_center()
        .justify_center()
        .text_center()
        .gap_3()
        .p_6()
        .child(
            div()
                .w(px(72.0))
                .h(px(72.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(20.0))
                .bg(cx.theme().danger.opacity(0.1))
                .text_color(cx.theme().danger)
                .child(Icon::new(IconName::Refresh).size_8()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .child(t!("CredentialVault.load_failed_title").to_string()),
        )
        .child(
            div()
                .max_w(px(560.0))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(error),
        )
        .child(
            Button::new("credential-vault-retry")
                .icon(IconName::Refresh)
                .label(t!("CredentialVault.retry").to_string())
                .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
        )
}

fn credential_row(
    summary: CredentialSummary,
    cx: &gpui::Context<CredentialVaultView>,
) -> impl IntoElement {
    let id = summary.id;
    let edit_id = button_id("credential-edit", id);
    let delete_id = button_id("credential-delete", id);
    let name = summary.name.clone();
    let username = summary
        .username
        .clone()
        .unwrap_or_else(|| t!("CredentialVault.username_not_set").to_string());
    let kind_chips = credential_kind_values(&summary.kind)
        .into_iter()
        .map(|kind| chip(credential_kind_label(&kind), cx).into_any_element())
        .collect::<Vec<_>>();
    let chips = capability_chips(&summary);
    v_flex()
        .w_full()
        .gap_3()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .p_3()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap_1()
                        .child(
                            h_flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(summary.name),
                                )
                                .children(kind_chips)
                                .child(chip(
                                    if summary.sync_enabled {
                                        t!("CredentialVault.sync_enabled").to_string()
                                    } else {
                                        t!("CredentialVault.local_only").to_string()
                                    },
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(username),
                        ),
                )
                .child(
                    Button::new(edit_id)
                        .icon(IconName::Edit)
                        .ghost()
                        .small()
                        .tooltip(t!("CredentialVault.edit").to_string())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_edit(id, window, cx);
                        })),
                )
                .child(
                    Button::new(delete_id)
                        .icon(IconName::Remove)
                        .ghost()
                        .small()
                        .tooltip(t!("CredentialVault.delete").to_string())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.confirm_delete(id, name.clone(), window, cx);
                        })),
                ),
        )
        .when(!chips.is_empty(), |this| {
            this.child(h_flex().flex_wrap().gap_2().children(chips))
        })
}

fn capability_chips(summary: &CredentialSummary) -> Vec<gpui::AnyElement> {
    [
        (
            summary.has_password,
            t!("CredentialVault.capability_password").to_string(),
        ),
        (
            summary.has_private_key_path,
            t!("CredentialVault.capability_private_key_path").to_string(),
        ),
        (
            summary.has_private_key_content,
            t!("CredentialVault.capability_private_key_content").to_string(),
        ),
        (
            summary.has_passphrase,
            t!("CredentialVault.capability_passphrase").to_string(),
        ),
    ]
    .into_iter()
    .filter(|(present, _)| *present)
    .map(|(_, label)| {
        div()
            .rounded_full()
            .bg(gpui::hsla(0.0, 0.0, 0.5, 0.08))
            .px_2()
            .py_0p5()
            .text_xs()
            .child(label)
            .into_any_element()
    })
    .collect()
}

fn chip(label: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .rounded_full()
        .bg(cx.theme().muted)
        .px_2()
        .py_0p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label.into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn credential_vault_matches_the_keychain_workspace_layout() {
        let source = include_str!("render.rs");

        assert!(source.contains("Button::new(\"credential-vault-add\")"));
        assert!(source.contains("CredentialVault.create"));
        assert!(source.contains("IconName::Search"));
        assert!(source.contains(".cleanable(true)"));
        assert!(source.contains(".id(\"credential-vault-content\")"));
        assert!(source.contains(".id(\"credential-vault-list\")"));
        assert!(source.contains(".min_h_0()"));
        assert!(source.contains(".overflow_hidden()"));
        assert!(source.contains(".overflow_y_scrollbar()"));
    }

    #[test]
    fn credential_vault_has_distinct_empty_and_search_states() {
        let source = include_str!("render.rs");

        assert!(source.contains("CredentialVault.empty_title"));
        assert!(source.contains("CredentialVault.no_matches"));
        assert!(source.contains("Button::new(\"credential-vault-empty-add\")"));
        assert!(source.contains("Button::new(\"credential-vault-retry\")"));
    }
}
