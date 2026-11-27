use nsh_base::{input, switch, theme};
use nsh_core::prelude::*;
use one_core::storage::traits::Repository as _;
use one_core::storage::{
    CredentialEntry, CredentialRepository, CredentialSummary, DeleteCredentialOutcome,
    SshKeyAlgorithm, generate_ssh_key,
};

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct CredentialDraft {
    name: String,
    username: String,
    password: String,
    private_key_path: String,
    private_key_content: String,
    passphrase: String,
    sync_enabled: bool,
    forward_to_agent: bool,
}

pub(crate) fn credentials() -> impl IntoElement {
    let repository = NavopNshServices::current().and_then(|services| services.credentials.clone());
    let summaries = signal(load_summaries(repository.as_ref()));
    let query = signal(String::new());
    let filtered = computed(move || filter_summaries(&summaries.get(), &query.get()));
    let unlocked = one_core::crypto::has_master_key();
    let status = signal(String::new());
    let show_form = signal(false);
    let editing_id = signal(None::<i64>);
    let name = signal(String::new());
    let username = signal(String::new());
    let password = signal(String::new());
    let private_key_path = signal(String::new());
    let private_key_content = signal(String::new());
    let passphrase = signal(String::new());
    let sync_enabled = signal(false);
    let forward_to_agent = signal(false);
    let key_algorithm = signal(SshKeyAlgorithm::Ed25519);
    let generated_public_key = signal(String::new());

    let refresh = {
        let repository = repository.clone();
        move || {
            summaries.set(load_summaries(repository.as_ref()));
            status.set(String::new());
        }
    };
    let open_create = {
        move || {
            editing_id.set(None);
            name.set(String::new());
            username.set(String::new());
            password.set(String::new());
            private_key_path.set(String::new());
            private_key_content.set(String::new());
            passphrase.set(String::new());
            sync_enabled.set(false);
            forward_to_agent.set(false);
            generated_public_key.set(String::new());
            show_form.set(true);
        }
    };
    let open_edit = {
        let repository = repository.clone();
        move |id: i64| {
            let Some(repository) = repository.as_ref() else {
                status.set("Credential storage is unavailable".to_owned());
                return;
            };
            match repository.get_plaintext(id) {
                Ok(Some(entry)) => {
                    editing_id.set(Some(id));
                    name.set(entry.name);
                    username.set(entry.username.unwrap_or_default());
                    password.set(entry.password.unwrap_or_default());
                    private_key_path.set(entry.private_key_path.unwrap_or_default());
                    private_key_content.set(entry.private_key_content.unwrap_or_default());
                    passphrase.set(entry.passphrase.unwrap_or_default());
                    sync_enabled.set(entry.sync_enabled);
                    forward_to_agent.set(entry.forward_to_agent);
                    generated_public_key.set(String::new());
                    show_form.set(true);
                    status.set(format!("Editing credential #{id}"));
                }
                Ok(None) => status.set(format!("Credential #{id} was not found")),
                Err(error) => status.set(format!("Failed to open credential: {error}")),
            }
        }
    };
    let save = {
        let repository = repository.clone();
        move || {
            let Some(repository) = repository.as_ref() else {
                status.set("Credential storage is unavailable".to_owned());
                return;
            };
            let existing = match editing_id.get_untracked() {
                Some(id) => match repository.get_plaintext(id) {
                    Ok(Some(entry)) => Some(entry),
                    Ok(None) => {
                        status.set(format!("Credential #{id} was not found"));
                        return;
                    }
                    Err(error) => {
                        status.set(format!("Failed to load credential: {error}"));
                        return;
                    }
                },
                None => None,
            };
            let draft = CredentialDraft {
                name: name.get_untracked(),
                username: username.get_untracked(),
                password: password.get_untracked(),
                private_key_path: private_key_path.get_untracked(),
                private_key_content: private_key_content.get_untracked(),
                passphrase: passphrase.get_untracked(),
                sync_enabled: sync_enabled.get_untracked(),
                forward_to_agent: forward_to_agent.get_untracked(),
            };
            let mut entry = match apply_credential_draft(existing, draft) {
                Ok(entry) => entry,
                Err(error) => {
                    status.set(error);
                    return;
                }
            };
            let result = if entry.id.is_some() {
                repository
                    .update(&entry)
                    .map(|_| entry.id.unwrap_or_default())
            } else {
                repository.insert(&mut entry)
            };
            match result {
                Ok(id) => {
                    summaries.set(load_summaries(Some(repository)));
                    show_form.set(false);
                    status.set(format!("Saved credential #{id}"));
                }
                Err(error) => status.set(format!("Failed to save credential: {error}")),
            }
        }
    };
    let delete = {
        let repository = repository.clone();
        move |id: i64| {
            let Some(repository) = repository.as_ref() else {
                status.set("Credential storage is unavailable".to_owned());
                return;
            };
            match repository.delete_checked(id) {
                Ok(DeleteCredentialOutcome::Deleted) => {
                    summaries.set(load_summaries(Some(repository)));
                    status.set(format!("Deleted credential #{id}"));
                }
                Ok(DeleteCredentialOutcome::NotFound) => {
                    status.set(format!("Credential #{id} was not found"));
                }
                Ok(DeleteCredentialOutcome::Referenced(hits)) => {
                    status.set(format!(
                        "Credential #{id} is still used by {} connection(s)",
                        hits.len()
                    ));
                }
                Err(error) => status.set(format!("Delete failed: {error}")),
            }
        }
    };
    let scan_references = {
        let repository = repository.clone();
        move |id: i64| {
            let Some(repository) = repository.as_ref() else {
                status.set("Credential storage is unavailable".to_owned());
                return;
            };
            match repository.referencing_connections(id) {
                Ok(hits) if hits.is_empty() => {
                    status.set(format!("Credential #{id} has no connection references"));
                }
                Ok(hits) => status.set(format!(
                    "Credential #{id} is referenced by: {}",
                    hits.iter()
                        .map(|hit| hit.connection_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Err(error) => status.set(format!("Reference scan failed: {error}")),
            }
        }
    };
    let generate_key = {
        move || {
            let name = name.get_untracked();
            let comment = if name.trim().is_empty() {
                "navop"
            } else {
                name.trim()
            };
            match generate_ssh_key(key_algorithm.get_untracked(), comment) {
                Ok(key) => {
                    private_key_content.set(key.private_key_openssh);
                    generated_public_key.set(format!(
                        "{}\n\nFingerprint: {}",
                        key.public_key_openssh, key.fingerprint
                    ));
                    status.set("Generated a new SSH key pair".to_owned());
                }
                Err(error) => status.set(format!("Key generation failed: {error}")),
            }
        }
    };
    let total = move || summaries.with(Vec::len);
    let visible = move || filtered.with(Vec::len);

    view()
        .size_full()
        .child(ui::page_header(
            "Credentials",
            "Shared encrypted credential vault",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_6()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            text(if unlocked {
                                "Vault unlocked"
                            } else {
                                "Vault locked"
                            })
                            .px_2()
                            .py_1()
                            .rounded_full()
                            .border_1()
                            .border_color(move || {
                                if unlocked {
                                    theme().primary
                                } else {
                                    theme().destructive
                                }
                            })
                            .text_xs()
                            .text_color(move || {
                                if unlocked {
                                    theme().primary
                                } else {
                                    theme().destructive
                                }
                            }),
                        )
                        .child(
                            text(move || format!("{} credentials", total()))
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(view().flex_1())
                        .child(ui::primary_button("New credential").on_press(open_create))
                        .child(ui::secondary_button("Refresh").on_press(refresh)),
                )
                .child(
                    input()
                        .value(move || query.get())
                        .on_change(move |value: &str| query.set(value.to_owned()))
                        .placeholder("Search by name or username")
                        .h(px(36.0))
                        .px_3()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().input)
                        .bg(move || theme().card)
                        .text_sm()
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || status.get())
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .when(show_form.get(), |page| {
                    page.child(credential_form(
                        editing_id,
                        name,
                        username,
                        password,
                        private_key_path,
                        private_key_content,
                        passphrase,
                        sync_enabled,
                        forward_to_agent,
                        key_algorithm,
                        generated_public_key,
                        generate_key,
                        show_form,
                        save,
                    ))
                })
                .child(
                    text(move || format!("{} match(es)", visible()))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(scroll().flex_1().min_h_0().gap_2().children(
                    filtered.with(|items| items.clone()).into_iter().map({
                        let open_edit = open_edit.clone();
                        let delete = delete.clone();
                        let scan_references = scan_references.clone();
                        move |summary| {
                            summary_row(
                                summary,
                                open_edit.clone(),
                                delete.clone(),
                                scan_references.clone(),
                            )
                        }
                    }),
                )),
        )
}

fn load_summaries(repository: Option<&CredentialRepository>) -> Vec<CredentialSummary> {
    repository
        .and_then(|repository| repository.list_summaries().ok())
        .unwrap_or_default()
}

fn filter_summaries(summaries: &[CredentialSummary], query: &str) -> Vec<CredentialSummary> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return summaries.to_vec();
    }
    summaries
        .iter()
        .filter(|summary| summary_matches(summary, &query))
        .cloned()
        .collect()
}

fn summary_matches(summary: &CredentialSummary, query: &str) -> bool {
    summary.name.to_lowercase().contains(query)
        || summary
            .username
            .as_deref()
            .is_some_and(|username| username.to_lowercase().contains(query))
}

fn apply_credential_draft(
    existing: Option<CredentialEntry>,
    draft: CredentialDraft,
) -> Result<CredentialEntry, String> {
    let name = draft.name.trim().to_owned();
    if name.is_empty() {
        return Err("Credential name is required".to_owned());
    }
    let mut entry = existing.unwrap_or_else(|| CredentialEntry::new(&name));
    entry.name = name;
    entry.username = optional_value(draft.username);
    entry.password = optional_value(draft.password);
    entry.private_key_path = optional_value(draft.private_key_path);
    entry.private_key_content = optional_value(draft.private_key_content);
    entry.passphrase = optional_value(draft.passphrase);
    entry.sync_enabled = draft.sync_enabled;
    entry.forward_to_agent = draft.forward_to_agent;
    Ok(entry)
}

fn optional_value(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

#[allow(clippy::too_many_arguments)]
fn credential_form(
    editing_id: Signal<Option<i64>>,
    name: Signal<String>,
    username: Signal<String>,
    password: Signal<String>,
    private_key_path: Signal<String>,
    private_key_content: Signal<String>,
    passphrase: Signal<String>,
    sync_enabled: Signal<bool>,
    forward_to_agent: Signal<bool>,
    key_algorithm: Signal<SshKeyAlgorithm>,
    generated_public_key: Signal<String>,
    generate_key: impl Fn() + Clone + 'static,
    show_form: Signal<bool>,
    save: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    ui::card()
        .gap_3()
        .child(
            text(move || {
                editing_id
                    .get()
                    .map_or("New credential", |_| "Edit credential")
            })
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(form_input("Name", name, "Production SSH"))
                .child(form_input("Username", username, "deploy")),
        )
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(secret_input("Password", password, "Optional"))
                .child(secret_input("Passphrase", passphrase, "Optional")),
        )
        .child(form_input(
            "Private key path",
            private_key_path,
            "~/.ssh/id_ed25519",
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap_2()
                .child(
                    text("Key algorithm")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
                .children(
                    SshKeyAlgorithm::ALL
                        .into_iter()
                        .map(move |algorithm| key_algorithm_button(algorithm, key_algorithm)),
                )
                .child(ui::primary_button("Generate key").on_press(generate_key.clone())),
        )
        .child(form_multiline(
            "Private key content",
            private_key_content,
            "Paste an OpenSSH or PEM private key",
        ))
        .when(!generated_public_key.get().is_empty(), |form| {
            form.child(
                view()
                    .gap_1()
                    .child(
                        text("Generated public key")
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(
                        text(move || generated_public_key.get())
                            .p_2()
                            .rounded(move || theme().radius)
                            .border_1()
                            .border_color(move || theme().border)
                            .text_size(px(11.0))
                            .text_color(move || theme().foreground),
                    ),
            )
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_4()
                .child(toggle(
                    sync_enabled,
                    "Enable cloud sync for this credential",
                ))
                .child(toggle(
                    forward_to_agent,
                    "Add the private key to the local agent and forward it",
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(ui::primary_button("Save").on_press(save))
                .child(ui::secondary_button("Cancel").on_press(move || show_form.set(false))),
        )
}

fn summary_row(
    summary: CredentialSummary,
    edit: impl Fn(i64) + Clone + 'static,
    delete: impl Fn(i64) + Clone + 'static,
    scan_references: impl Fn(i64) + Clone + 'static,
) -> impl IntoElement {
    let id = summary.id;
    let username = summary.username.unwrap_or_else(|| "No username".to_owned());
    let mut kinds = Vec::new();
    if summary.has_password {
        kinds.push("Password");
    }
    if summary.has_private_key_path || summary.has_private_key_content {
        kinds.push("SSH key");
    }
    if summary.has_passphrase {
        kinds.push("Passphrase");
    }
    if summary.has_ssh_expect {
        kinds.push("Account expect");
    }
    if summary.forward_to_agent {
        kinds.push("Agent forwarding");
    }
    if summary.sync_enabled {
        kinds.push("Sync");
    }
    let kind_text = kinds.join(" · ");

    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view()
                .size(px(36.0))
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(9.0))
                .bg(move || theme().muted)
                .child(
                    text("KEY")
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    text(summary.name)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(username)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .when(!kind_text.is_empty(), |row| {
                    row.child(
                        text(kind_text)
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                }),
        )
        .child(
            text(
                summary
                    .team_id
                    .as_deref()
                    .map_or("Personal", |_| "Team credential"),
            )
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(ui::secondary_button("Edit").on_press({
            let edit = edit.clone();
            move || edit(id)
        }))
        .child(ui::secondary_button("References").on_press({
            let scan_references = scan_references.clone();
            move || scan_references(id)
        }))
        .child(ui::secondary_button("Delete").on_press(move || delete(id)))
}

fn key_algorithm_button(
    algorithm: SshKeyAlgorithm,
    selected: Signal<SshKeyAlgorithm>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == algorithm {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == algorithm {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == algorithm,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(algorithm))
        .child(
            text(key_algorithm_label(algorithm))
                .text_xs()
                .text_color(move || theme().foreground),
        )
}

fn key_algorithm_label(algorithm: SshKeyAlgorithm) -> &'static str {
    match algorithm {
        SshKeyAlgorithm::Ed25519 => "Ed25519",
        SshKeyAlgorithm::EcdsaP256 => "ECDSA P-256",
        SshKeyAlgorithm::EcdsaP384 => "ECDSA P-384",
        SshKeyAlgorithm::Rsa3072 => "RSA 3072",
    }
}

fn form_input(
    label: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(180.0))
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(36.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn secret_input(
    label: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(180.0))
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .masked(true)
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(36.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn form_multiline(
    label: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            input()
                .multiline(true)
                .masked(true)
                .value(move || value.get())
                .on_change(move |next: &str| value.set(next.to_owned()))
                .placeholder(placeholder)
                .h(px(130.0))
                .p_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().background)
                .text_size(px(12.0))
                .text_color(move || theme().foreground),
        )
}

fn toggle(checked: Signal<bool>, label: &'static str) -> impl IntoElement {
    switch()
        .checked(move || checked.get())
        .track_class("w-9 h-5 rounded-full bg-muted checked:bg-primary")
        .thumb_class("size-4 rounded-full bg-background checked:translate-x-4")
        .accessibility_label(label)
        .on_change(move |next: &bool| checked.set(*next))
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str, username: Option<&str>) -> CredentialSummary {
        CredentialSummary {
            id: 1,
            name: name.to_owned(),
            username: username.map(str::to_owned),
            has_password: false,
            has_private_key_path: false,
            has_private_key_content: false,
            has_passphrase: false,
            has_ssh_expect: false,
            sync_enabled: false,
            forward_to_agent: false,
            cloud_id: None,
            last_synced_at: None,
            team_id: None,
            owner_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn draft() -> CredentialDraft {
        CredentialDraft {
            name: "Production".to_owned(),
            username: "deploy".to_owned(),
            password: "secret".to_owned(),
            private_key_path: String::new(),
            private_key_content: String::new(),
            passphrase: String::new(),
            sync_enabled: true,
            forward_to_agent: false,
        }
    }

    #[test]
    fn credential_search_matches_name_and_username_case_insensitively() {
        let items = vec![
            summary("Production SSH", Some("deploy")),
            summary("Local database", Some("root")),
        ];
        assert_eq!(filter_summaries(&items, "PROD").len(), 1);
        assert_eq!(filter_summaries(&items, "deploy").len(), 1);
        assert_eq!(filter_summaries(&items, "missing").len(), 0);
    }

    #[test]
    fn credential_draft_updates_core_fields_and_preserves_existing_expect() {
        let mut existing = CredentialEntry::new("Old");
        existing.id = Some(9);
        existing.ssh_expect.username.expect = "login:".to_owned();
        let entry = apply_credential_draft(Some(existing), draft()).unwrap();
        assert_eq!(entry.id, Some(9));
        assert_eq!(entry.name, "Production");
        assert_eq!(entry.username.as_deref(), Some("deploy"));
        assert_eq!(entry.password.as_deref(), Some("secret"));
        assert!(entry.sync_enabled);
        assert_eq!(entry.ssh_expect.username.expect, "login:");
    }

    #[test]
    fn blank_secret_fields_clear_optional_values() {
        let mut input = draft();
        input.password.clear();
        let entry = apply_credential_draft(None, input).unwrap();
        assert!(entry.password.is_none());
    }
}
