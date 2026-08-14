use gpui::SharedString;
use gpui_component::select::SelectItem;
use one_core::storage::{CredentialReference, CredentialSummary};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CredentialCapabilities {
    pub username: bool,
    pub password: bool,
    pub private_key: bool,
    pub passphrase: bool,
}

impl CredentialCapabilities {
    pub const fn login() -> Self {
        Self {
            username: true,
            password: true,
            private_key: false,
            passphrase: false,
        }
    }

    pub const fn password_only() -> Self {
        Self {
            password: true,
            ..Self::empty()
        }
    }

    pub const fn ssh_password() -> Self {
        Self::login()
    }

    pub const fn ssh_private_key() -> Self {
        Self {
            username: true,
            private_key: true,
            passphrase: true,
            ..Self::empty()
        }
    }

    pub const fn private_key() -> Self {
        Self {
            private_key: true,
            passphrase: true,
            ..Self::empty()
        }
    }

    pub const fn username_only() -> Self {
        Self {
            username: true,
            ..Self::empty()
        }
    }

    pub const fn all() -> Self {
        Self {
            username: true,
            password: true,
            private_key: true,
            passphrase: true,
        }
    }

    const fn empty() -> Self {
        Self {
            username: false,
            password: false,
            private_key: false,
            passphrase: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialField {
    Username,
    Password,
    PrivateKey,
    Passphrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialSelectValue {
    Manual,
    Credential(i64),
}

#[derive(Clone, Debug)]
pub struct CredentialSelectItem {
    value: CredentialSelectValue,
    title: SharedString,
}

impl CredentialSelectItem {
    fn new(value: CredentialSelectValue, title: impl Into<SharedString>) -> Self {
        Self {
            value,
            title: title.into(),
        }
    }

    pub fn value(&self) -> &CredentialSelectValue {
        &self.value
    }
}

impl SelectItem for CredentialSelectItem {
    type Value = CredentialSelectValue;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub fn build_reference(
    selection: CredentialSelectValue,
    capabilities: CredentialCapabilities,
    summaries: &[CredentialSummary],
) -> Option<CredentialReference> {
    let CredentialSelectValue::Credential(credential_id) = selection else {
        return None;
    };
    let summary = summaries
        .iter()
        .find(|summary| summary.id == credential_id)?;
    Some(default_reference(credential_id, capabilities, summary))
}

pub fn normalize_reference(
    mut reference: CredentialReference,
    capabilities: CredentialCapabilities,
    summary: Option<&CredentialSummary>,
) -> CredentialReference {
    reference.username &= capabilities.username;
    reference.password &= capabilities.password;
    reference.private_key &= capabilities.private_key;
    reference.passphrase &= capabilities.passphrase;
    normalize_auth_fields(&mut reference);
    if has_selected_field(&reference) {
        return reference;
    }
    summary
        .map(|summary| default_reference(reference.credential_id, capabilities, summary))
        .unwrap_or(reference)
}

pub fn apply_field_selection(
    mut reference: CredentialReference,
    field: CredentialField,
    selected: bool,
) -> CredentialReference {
    match field {
        CredentialField::Username => reference.username = selected,
        CredentialField::Password => {
            reference.password = selected;
            if selected {
                reference.private_key = false;
                reference.passphrase = false;
            }
        }
        CredentialField::PrivateKey => {
            reference.private_key = selected;
            if selected {
                reference.password = false;
            } else {
                reference.passphrase = false;
            }
        }
        CredentialField::Passphrase => reference.passphrase = selected,
    }
    reference
}

pub fn credential_select_items(
    summaries: &[CredentialSummary],
    capabilities: CredentialCapabilities,
    selected: Option<CredentialReference>,
) -> Vec<CredentialSelectItem> {
    let selected_id = selected.map(|reference| reference.credential_id);
    let mut summaries = summaries
        .iter()
        .filter(|summary| {
            has_applicable_field(summary, capabilities) || selected_id == Some(summary.id)
        })
        .collect::<Vec<_>>();
    summaries.sort_by_key(|summary| summary.name.to_lowercase());

    let mut items = vec![CredentialSelectItem::new(
        CredentialSelectValue::Manual,
        "手工输入",
    )];
    items.extend(summaries.into_iter().map(summary_item));
    if let Some(id) = selected_id
        && !items
            .iter()
            .any(|item| item.value == CredentialSelectValue::Credential(id))
    {
        items.push(CredentialSelectItem::new(
            CredentialSelectValue::Credential(id),
            format!("不可用的钥匙串条目 #{id}"),
        ));
    }
    items
}

fn default_reference(
    credential_id: i64,
    capabilities: CredentialCapabilities,
    summary: &CredentialSummary,
) -> CredentialReference {
    let password = capabilities.password && summary.has_password;
    let private_key = !password
        && capabilities.private_key
        && (summary.has_private_key_path || summary.has_private_key_content);
    CredentialReference {
        credential_id,
        username: capabilities.username && summary.username.is_some(),
        password,
        private_key,
        passphrase: private_key && capabilities.passphrase && summary.has_passphrase,
    }
}

fn normalize_auth_fields(reference: &mut CredentialReference) {
    if reference.password && reference.private_key {
        reference.private_key = false;
    }
    if !reference.private_key {
        reference.passphrase = false;
    }
}

fn has_selected_field(reference: &CredentialReference) -> bool {
    reference.username || reference.password || reference.private_key || reference.passphrase
}

fn has_applicable_field(summary: &CredentialSummary, capabilities: CredentialCapabilities) -> bool {
    (capabilities.username && summary.username.is_some())
        || (capabilities.password && summary.has_password)
        || (capabilities.private_key
            && (summary.has_private_key_path || summary.has_private_key_content))
        || (capabilities.passphrase && summary.has_passphrase)
}

fn summary_item(summary: &CredentialSummary) -> CredentialSelectItem {
    CredentialSelectItem::new(
        CredentialSelectValue::Credential(summary.id),
        format!("{}（{}）", summary.name, summary.kind),
    )
}
