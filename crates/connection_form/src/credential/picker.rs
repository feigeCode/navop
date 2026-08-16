use gpui::{AppContext, Context, Entity, EventEmitter, SharedString, Window};
use gpui_component::{
    IndexPath,
    select::{SelectEvent, SelectState},
};
use one_core::storage::{CredentialReference, CredentialSummary};

use super::{
    CredentialCapabilities, CredentialField, CredentialSelectItem, CredentialSelectValue,
    build_reference, credential_select_items, load_summaries, normalize_reference,
    summary_matches_reference,
};

#[derive(Clone, Debug)]
pub struct CredentialPickerConfig {
    pub(super) capabilities: CredentialCapabilities,
    pub(super) reference: Option<CredentialReference>,
}

impl CredentialPickerConfig {
    pub fn new(_id: impl Into<SharedString>, capabilities: CredentialCapabilities) -> Self {
        Self {
            capabilities,
            reference: None,
        }
    }

    pub fn reference(mut self, reference: Option<CredentialReference>) -> Self {
        self.reference = reference;
        self
    }
}

#[derive(Clone, Debug)]
pub enum CredentialPickerEvent {
    Changed,
}

pub struct CredentialReferencePicker {
    pub(super) select: Entity<SelectState<Vec<CredentialSelectItem>>>,
    pub(super) summaries: Vec<CredentialSummary>,
    pub(super) capabilities: CredentialCapabilities,
    pub(super) reference: Option<CredentialReference>,
    pub(super) load_error: Option<SharedString>,
}

impl EventEmitter<CredentialPickerEvent> for CredentialReferencePicker {}

pub fn create_credential_picker<T: 'static>(
    config: CredentialPickerConfig,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<CredentialReferencePicker> {
    let (summaries, load_error) = load_summaries(cx);
    create_picker(config, summaries, load_error, window, cx)
}

#[cfg(test)]
pub(crate) fn create_credential_picker_with_summaries<T: 'static>(
    config: CredentialPickerConfig,
    summaries: Vec<CredentialSummary>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<CredentialReferencePicker> {
    create_picker(config, summaries, None, window, cx)
}

fn create_picker<T: 'static>(
    config: CredentialPickerConfig,
    summaries: Vec<CredentialSummary>,
    load_error: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<CredentialReferencePicker> {
    let picker =
        cx.new(|cx| CredentialReferencePicker::new(config, summaries, load_error, window, cx));
    subscribe_to_select(&picker, window, cx);
    picker
}

fn subscribe_to_select<T: 'static>(
    picker: &Entity<CredentialReferencePicker>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let select = picker.read(cx).select.clone();
    let picker = picker.clone();
    cx.subscribe_in(
        &select,
        window,
        move |_, _, event: &SelectEvent<Vec<CredentialSelectItem>>, _, cx| {
            let SelectEvent::Confirm(Some(value)) = event else {
                return;
            };
            picker.update(cx, |picker, cx| {
                picker.apply_selected_value(value.clone(), cx);
                cx.emit(CredentialPickerEvent::Changed);
            });
        },
    )
    .detach();
}

impl CredentialReferencePicker {
    fn new(
        config: CredentialPickerConfig,
        summaries: Vec<CredentialSummary>,
        load_error: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let reference = normalized_reference(config.reference, config.capabilities, &summaries);
        let selected = selected_value(reference.as_ref());
        let items = credential_select_items(&summaries, config.capabilities, reference.as_ref());
        let selected_index = items
            .iter()
            .position(|item| item.value() == &selected)
            .map(IndexPath::new);
        let select =
            cx.new(|cx| SelectState::new(items, selected_index, window, cx).searchable(true));
        Self {
            select,
            summaries,
            capabilities: config.capabilities,
            reference,
            load_error,
        }
    }

    pub fn selected_reference(&self) -> Option<CredentialReference> {
        self.reference.clone()
    }

    pub fn selected_value(&self) -> CredentialSelectValue {
        selected_value(self.reference.as_ref())
    }

    pub fn field_referenced(&self, field: CredentialField) -> bool {
        let Some(reference) = self.reference.as_ref() else {
            return false;
        };
        match field {
            CredentialField::Username => reference.username,
            CredentialField::Password => reference.password,
            CredentialField::PrivateKey => reference.private_key,
            CredentialField::Passphrase => reference.passphrase,
        }
    }

    pub fn set_capabilities(
        &mut self,
        capabilities: CredentialCapabilities,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capabilities = capabilities;
        self.reference = normalized_reference(self.reference.take(), capabilities, &self.summaries);
        self.sync_select(window, cx);
        cx.emit(CredentialPickerEvent::Changed);
    }

    pub fn set_reference(
        &mut self,
        reference: Option<CredentialReference>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reference = normalized_reference(reference, self.capabilities, &self.summaries);
        self.sync_select(window, cx);
        cx.emit(CredentialPickerEvent::Changed);
    }

    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (summaries, load_error) = load_summaries(cx);
        self.summaries = summaries;
        self.load_error = load_error;
        self.reference =
            normalized_reference(self.reference.take(), self.capabilities, &self.summaries);
        self.sync_select(window, cx);
    }

    #[cfg(test)]
    pub(crate) fn select_value(&mut self, value: CredentialSelectValue, cx: &mut Context<Self>) {
        self.apply_selected_value(value, cx);
    }

    #[cfg(test)]
    pub(crate) fn set_capabilities_without_window(
        &mut self,
        capabilities: CredentialCapabilities,
        cx: &mut Context<Self>,
    ) {
        self.capabilities = capabilities;
        self.reference = normalized_reference(self.reference.take(), capabilities, &self.summaries);
        cx.notify();
    }

    fn apply_selected_value(&mut self, value: CredentialSelectValue, cx: &mut Context<Self>) {
        self.reference = build_reference(value, self.capabilities, &self.summaries);
        cx.notify();
    }

    fn sync_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = selected_value(self.reference.as_ref());
        let items =
            credential_select_items(&self.summaries, self.capabilities, self.reference.as_ref());
        self.select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_value(&selected, window, cx);
        });
        cx.notify();
    }
}

fn normalized_reference(
    reference: Option<CredentialReference>,
    capabilities: CredentialCapabilities,
    summaries: &[CredentialSummary],
) -> Option<CredentialReference> {
    reference.map(|reference| {
        let summary = summaries
            .iter()
            .find(|summary| summary_matches_reference(summary, &reference));
        normalize_reference(reference, capabilities, summary)
    })
}

fn selected_value(reference: Option<&CredentialReference>) -> CredentialSelectValue {
    reference
        .map(|reference| CredentialSelectValue::Credential(reference.credential_id))
        .unwrap_or(CredentialSelectValue::Manual)
}
