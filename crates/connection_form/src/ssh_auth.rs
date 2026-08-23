/// SSH authentication methods shared by connection forms.
///
/// The enum deliberately contains only the stable, non-secret values used by
/// connection forms. Runtime authentication is still represented by `ssh::SshAuth`
/// and persisted connection records by `one-core::storage::SshAuthMethod`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SshAuthOption {
    #[default]
    Password,
    PrivateKey,
    PrivateKeyContent,
    Agent,
    Pageant,
    AutoPublicKey,
}

impl SshAuthOption {
    pub const ALL: &'static [Self] = &[
        Self::Password,
        Self::PrivateKey,
        Self::PrivateKeyContent,
        Self::Agent,
        Self::Pageant,
        Self::AutoPublicKey,
    ];

    /// Authentication methods currently supported by database SSH tunnels.
    pub const TUNNEL: &'static [Self] = &[
        Self::Password,
        Self::PrivateKey,
        Self::Agent,
        Self::Pageant,
        Self::AutoPublicKey,
    ];

    pub const fn value(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
            Self::PrivateKeyContent => "private_key_content",
            Self::Agent => "agent",
            Self::Pageant => "pageant",
            Self::AutoPublicKey => "auto_public_key",
        }
    }

    pub const fn label_i18n_key(self) -> &'static str {
        match self {
            Self::Password => "ConnectionForm.ssh_auth_password",
            Self::PrivateKey => "ConnectionForm.ssh_auth_private_key",
            Self::PrivateKeyContent => "ConnectionForm.ssh_auth_private_key_content",
            Self::Agent => "ConnectionForm.ssh_auth_agent",
            Self::Pageant => "ConnectionForm.ssh_auth_pageant",
            Self::AutoPublicKey => "ConnectionForm.ssh_auth_auto_public_key",
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Password => t!("ConnectionForm.ssh_auth_password").to_string(),
            Self::PrivateKey => t!("ConnectionForm.ssh_auth_private_key").to_string(),
            Self::PrivateKeyContent => {
                t!("ConnectionForm.ssh_auth_private_key_content").to_string()
            }
            Self::Agent => t!("ConnectionForm.ssh_auth_agent").to_string(),
            Self::Pageant => t!("ConnectionForm.ssh_auth_pageant").to_string(),
            Self::AutoPublicKey => t!("ConnectionForm.ssh_auth_auto_public_key").to_string(),
        }
    }

    pub const fn requires_password(self) -> bool {
        matches!(self, Self::Password)
    }

    pub const fn requires_private_key(self) -> bool {
        matches!(self, Self::PrivateKey)
    }

    pub const fn requires_private_key_content(self) -> bool {
        matches!(self, Self::PrivateKeyContent)
    }
}

pub fn normalize_ssh_auth_type(auth_type: &str) -> &str {
    let auth_type = auth_type.trim();
    if auth_type.eq_ignore_ascii_case("private_key_material") {
        "private_key_content"
    } else if auth_type.eq_ignore_ascii_case("private_key_content") {
        "private_key_content"
    } else if auth_type.eq_ignore_ascii_case("private_key") {
        "private_key"
    } else if auth_type.eq_ignore_ascii_case("agent") {
        "agent"
    } else if auth_type.eq_ignore_ascii_case("pageant") {
        "pageant"
    } else if auth_type.eq_ignore_ascii_case("auto_public_key")
        || auth_type.eq_ignore_ascii_case("auto_publickey")
    {
        "auto_public_key"
    } else {
        "password"
    }
}

#[cfg(test)]
mod tests {
    use super::{SshAuthOption, normalize_ssh_auth_type};

    #[test]
    fn shared_ssh_auth_options_include_pageant() {
        assert_eq!(
            SshAuthOption::ALL
                .iter()
                .map(|option| option.value())
                .collect::<Vec<_>>(),
            vec![
                "password",
                "private_key",
                "private_key_content",
                "agent",
                "pageant",
                "auto_public_key",
            ]
        );
        assert_eq!(
            SshAuthOption::Pageant.label_i18n_key(),
            "ConnectionForm.ssh_auth_pageant"
        );
    }

    #[test]
    fn shared_ssh_auth_type_normalization_preserves_pageant() {
        assert_eq!(normalize_ssh_auth_type(" Pageant "), "pageant");
        assert_eq!(normalize_ssh_auth_type("Auto_PublicKey"), "auto_public_key");
        assert_eq!(
            normalize_ssh_auth_type("private_key_material"),
            "private_key_content"
        );
    }
}
use rust_i18n::t;
