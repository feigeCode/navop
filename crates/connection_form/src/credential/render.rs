use gpui::{AnyElement, Context, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::{ActiveTheme, h_flex, select::Select};
use one_core::storage::CredentialSummary;
use rust_i18n::t;

use super::{CredentialField, CredentialReferencePicker};

impl Render for CredentialReferencePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.selected_summary();
        let messages = self.messages(summary, cx);

        h_flex()
            .w_full()
            .gap_2()
            .child(
                Select::new(&self.select)
                    .w_full()
                    .cleanable(false)
                    .placeholder(t!("Credential.manual_input")),
            )
            .children(messages)
    }
}

impl CredentialReferencePicker {
    pub(super) fn selected_summary(&self) -> Option<&CredentialSummary> {
        let reference = self.reference.as_ref()?;
        self.summaries
            .iter()
            .find(|summary| super::summary_matches_reference(summary, reference))
    }

    fn messages(
        &self,
        summary: Option<&CredentialSummary>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut messages = Vec::new();
        if let Some(error) = &self.load_error {
            messages.push(message(error.clone(), false, cx));
            return messages;
        }
        let Some(summary) = summary else {
            if self.summaries.is_empty() {
                messages.push(message(t!("Credential.empty"), true, cx));
            }
            return messages;
        };
        messages.extend(self.missing_field_warnings(summary, cx));
        let sync_status = if summary.sync_enabled {
            t!("Credential.sync_enabled")
        } else {
            t!("Credential.local_only")
        };
        messages.push(message(sync_status, true, cx));
        messages
    }

    fn missing_field_warnings(
        &self,
        summary: &CredentialSummary,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut warnings = Vec::new();
        if self.field_referenced(CredentialField::Username) && summary.username.is_none() {
            warnings.push(message(t!("Credential.missing_username"), false, cx));
        }
        if self.field_referenced(CredentialField::Password) && !summary.has_password {
            warnings.push(message(t!("Credential.missing_password"), false, cx));
        }
        let has_key = summary.has_private_key_path || summary.has_private_key_content;
        if self.field_referenced(CredentialField::PrivateKey) && !has_key {
            warnings.push(message(t!("Credential.missing_private_key"), false, cx));
        }
        if self.field_referenced(CredentialField::Passphrase) && !summary.has_passphrase {
            warnings.push(message(t!("Credential.missing_passphrase"), false, cx));
        }
        warnings
    }
}

fn message(
    text: impl Into<gpui::SharedString>,
    muted: bool,
    cx: &Context<CredentialReferencePicker>,
) -> AnyElement {
    let color = if muted {
        cx.theme().muted_foreground
    } else {
        cx.theme().foreground
    };
    div()
        .text_xs()
        .text_color(color)
        .child(text.into())
        .into_any_element()
}
