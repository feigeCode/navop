use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialQuery {
    pub service: String,
    pub account: String,
}

impl CredentialQuery {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }
}

pub trait CredentialStore {
    fn get_password(&self, query: &CredentialQuery) -> Option<String>;
}

pub struct NoopCredentialStore;

impl CredentialStore for NoopCredentialStore {
    fn get_password(&self, _query: &CredentialQuery) -> Option<String> {
        None
    }
}

pub struct SystemCredentialStore;

impl CredentialStore for SystemCredentialStore {
    fn get_password(&self, query: &CredentialQuery) -> Option<String> {
        system_password(query)
    }
}

#[cfg(target_os = "macos")]
fn system_password(query: &CredentialQuery) -> Option<String> {
    command_output(
        "security",
        &[
            "find-generic-password",
            "-s",
            &query.service,
            "-a",
            &query.account,
            "-w",
        ],
    )
}

#[cfg(target_os = "linux")]
fn system_password(query: &CredentialQuery) -> Option<String> {
    command_output(
        "secret-tool",
        &[
            "lookup",
            "service",
            &query.service,
            "account",
            &query.account,
        ],
    )
}

#[cfg(target_os = "windows")]
fn system_password(_query: &CredentialQuery) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn system_password(_query: &CredentialQuery) -> Option<String> {
    None
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let password = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!password.is_empty()).then_some(password)
}
