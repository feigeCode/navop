// Keep connection-form translations, including refresh labels, owned by this crate.
rust_i18n::i18n!("locales", fallback = "en");

pub mod credential;
pub mod declarative;
pub mod middleware_form;
pub mod ssh_auth;
pub mod ssh_tunnel;
pub mod team;

pub use ssh_auth::{SshAuthOption, normalize_ssh_auth_type};
pub use ssh_tunnel::{
    SshConnectionSelectItem, SshTunnelForm, SshTunnelFormConfig, SshTunnelFormValue,
};
