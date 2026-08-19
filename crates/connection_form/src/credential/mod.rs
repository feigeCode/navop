mod model;
mod picker;
mod render;
mod repository;
mod runtime;

pub(crate) use model::reference_is_unavailable;
use model::summary_matches_reference;
pub use model::{
    CredentialCapabilities, CredentialField, CredentialSelectItem, CredentialSelectValue,
    build_reference, credential_select_items, normalize_reference,
};
#[cfg(test)]
pub(crate) use picker::create_credential_picker_with_summaries;
pub use picker::{
    CredentialPickerConfig, CredentialPickerEvent, CredentialReferencePicker,
    create_credential_picker,
};
use repository::load_summaries;
pub use runtime::{resolve_connection_for_runtime, resolve_ssh_for_runtime};

#[cfg(test)]
mod picker_tests;
#[cfg(test)]
mod tests;
