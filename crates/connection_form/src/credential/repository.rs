use gpui::{App, SharedString};
use one_core::storage::{CredentialRepository, CredentialSummary, GlobalStorageState};
use rust_i18n::t;

pub(super) fn load_summaries(cx: &App) -> (Vec<CredentialSummary>, Option<SharedString>) {
    let Some(repository) = cx
        .try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<CredentialRepository>())
    else {
        return (Vec::new(), None);
    };
    match repository.list_summaries() {
        Ok(summaries) => (summaries, None),
        Err(_) => (Vec::new(), Some(t!("Credential.load_failed").into())),
    }
}
