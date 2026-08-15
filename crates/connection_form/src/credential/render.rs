use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme, Disableable, checkbox::Checkbox, h_flex, select::Select, v_flex,
};
use one_core::storage::CredentialSummary;

use super::{CredentialField, CredentialReferencePicker};

impl Render for CredentialReferencePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.selected_summary();
        let checkboxes = self.field_checkboxes(cx);
        let messages = self.messages(summary, cx);

        v_flex()
            .w_full()
            .gap_2()
            .child(
                Select::new(&self.select)
                    .w_full()
                    .cleanable(false)
                    .placeholder("手工输入"),
            )
            .when(!checkboxes.is_empty(), |this| {
                this.child(h_flex().w_full().flex_wrap().gap_3().children(checkboxes))
            })
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

    fn field_checkboxes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut fields = Vec::new();
        if self.capabilities.username {
            fields.push(self.field_checkbox(CredentialField::Username, "用户名", false, cx));
        }
        if self.capabilities.password {
            fields.push(self.field_checkbox(CredentialField::Password, "密码", false, cx));
        }
        if self.capabilities.private_key {
            fields.push(self.field_checkbox(CredentialField::PrivateKey, "私钥", false, cx));
        }
        if self.capabilities.passphrase {
            let disabled = !self.field_referenced(CredentialField::PrivateKey);
            fields.push(self.field_checkbox(CredentialField::Passphrase, "私钥密码", disabled, cx));
        }
        if self.reference.is_some() {
            fields
        } else {
            Vec::new()
        }
    }

    fn field_checkbox(
        &self,
        field: CredentialField,
        label: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Checkbox::new(format!("{}-{field:?}", self.id))
            .label(label)
            .checked(self.field_referenced(field))
            .disabled(disabled)
            .on_click(cx.listener(move |this, selected, window, cx| {
                this.set_field_selection(field, *selected, window, cx);
            }))
            .into_any_element()
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
                messages.push(message("钥匙串中暂无可用凭据，请先创建。", true, cx));
            }
            return messages;
        };
        messages.extend(self.missing_field_warnings(summary, cx));
        let sync_status = if summary.sync_enabled {
            "已参与个人端到端加密同步"
        } else {
            "仅本地"
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
            warnings.push(message("当前钥匙串条目已不再包含用户名。", false, cx));
        }
        if self.field_referenced(CredentialField::Password) && !summary.has_password {
            warnings.push(message("当前钥匙串条目已不再包含密码。", false, cx));
        }
        let has_key = summary.has_private_key_path || summary.has_private_key_content;
        if self.field_referenced(CredentialField::PrivateKey) && !has_key {
            warnings.push(message("当前钥匙串条目已不再包含私钥。", false, cx));
        }
        if self.field_referenced(CredentialField::Passphrase) && !summary.has_passphrase {
            warnings.push(message("当前钥匙串条目已不再包含私钥密码。", false, cx));
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
