use zeroize::Zeroize;

const TERMSRV_PREFIX: &str = "TERMSRV/";
#[cfg(target_os = "windows")]
const HANDOFF_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(target_os = "windows")]
const NAVOP_CREDENTIAL_MARKER: &str = "Navop temporary MSTSC credential";

pub(super) struct MstscCredentials {
    pub(super) target: String,
    pub(super) username: String,
    pub(super) password: String,
}

pub(super) struct MstscCredentialInput<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) username: Option<&'a str>,
    pub(super) password: Option<&'a str>,
    pub(super) domain: Option<&'a str>,
}

impl Drop for MstscCredentials {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

pub(super) fn mstsc_credentials(input: MstscCredentialInput<'_>) -> Option<MstscCredentials> {
    let username = input.username.filter(|value| !value.is_empty())?;
    let password = input.password.filter(|value| !value.is_empty())?;
    let username = if username.contains(|character| matches!(character, '\\' | '@')) {
        username.to_string()
    } else if let Some(domain) = input.domain.filter(|value| !value.is_empty()) {
        format!("{domain}\\{username}")
    } else {
        username.to_string()
    };
    Some(MstscCredentials {
        target: format!(
            "{TERMSRV_PREFIX}{}",
            super::format_host_port(input.host, input.port)
        ),
        username,
        password: password.to_string(),
    })
}

#[cfg(target_os = "windows")]
#[path = "mstsc_credentials_windows.rs"]
mod windows_credential;

#[cfg(target_os = "windows")]
pub(super) use windows_credential::store_temporary;
