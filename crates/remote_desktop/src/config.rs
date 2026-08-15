use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use connection_tunnel::ProxyTunnelConfig;
use one_core::storage::RdpAudioMode;
pub use one_core::storage::{RdpSettings, RemoteDesktopBackendPreference};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDesktopProtocol {
    Rdp,
    Vnc,
}

impl RemoteDesktopProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rdp => "RDP",
            Self::Vnc => "VNC",
        }
    }

    pub fn provider_id(self) -> &'static str {
        match self {
            Self::Rdp => "rdp",
            Self::Vnc => "vnc",
        }
    }
}

#[derive(Clone)]
pub struct RemoteDesktopConnectionOptions {
    pub protocol: RemoteDesktopProtocol,
    pub backend_preference: RemoteDesktopBackendPreference,
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub read_only: bool,
    pub audio_playback: bool,
    pub audio_capture: bool,
    pub shared_folders: Vec<RemoteDesktopSharedFolder>,
    pub rdp: RdpSettings,
    pub proxy: Option<ProxyTunnelConfig>,
}

impl RemoteDesktopConnectionOptions {
    pub fn from_storage_params(params: one_core::storage::RemoteDesktopParams) -> Self {
        let protocol = match params.protocol {
            one_core::storage::RemoteDesktopProtocol::Rdp => RemoteDesktopProtocol::Rdp,
            one_core::storage::RemoteDesktopProtocol::Vnc => RemoteDesktopProtocol::Vnc,
        };
        let backend_preference = match protocol {
            RemoteDesktopProtocol::Rdp => params.backend_preference,
            RemoteDesktopProtocol::Vnc => RemoteDesktopBackendPreference::Canvas,
        };
        let rdp = match protocol {
            RemoteDesktopProtocol::Rdp => params.effective_rdp_settings(),
            RemoteDesktopProtocol::Vnc => RdpSettings::from_legacy_audio_playback(false),
        };
        let audio_playback =
            protocol == RemoteDesktopProtocol::Rdp && rdp.audio.mode == RdpAudioMode::Local;
        let audio_capture = protocol == RemoteDesktopProtocol::Rdp && rdp.audio.capture;
        let shared_folders = rdp
            .resources
            .shared_folders
            .iter()
            .map(|folder| RemoteDesktopSharedFolder {
                name: folder.name.clone(),
                path: PathBuf::from(&folder.path),
                read_only: folder.read_only,
            })
            .collect();
        Self {
            protocol,
            backend_preference,
            destination: format!("{}:{}", params.host, params.port),
            username: params.username,
            password: params.password,
            domain: params.domain,
            read_only: params.read_only,
            audio_playback,
            audio_capture,
            shared_folders,
            rdp,
            proxy: params.proxy.map(storage_proxy_config),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopSharedFolder {
    pub name: String,
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktopSize {
    pub width: u16,
    pub height: u16,
    pub scale_factor: u32,
}

impl fmt::Debug for RemoteDesktopConnectionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteDesktopConnectionOptions")
            .field("protocol", &self.protocol)
            .field("backend_preference", &self.backend_preference)
            .field("destination", &self.destination)
            .field("username_present", &self.username.is_some())
            .field("username_len", &option_len(&self.username))
            .field("password_present", &self.password.is_some())
            .field("domain_present", &self.domain.is_some())
            .field("domain_len", &option_len(&self.domain))
            .field("read_only", &self.read_only)
            .field("audio_playback", &self.audio_playback)
            .field("audio_capture", &self.audio_capture)
            .field("shared_folder_count", &self.shared_folders.len())
            .field("admin_session", &self.rdp.admin_session)
            .field("display_mode", &self.rdp.display.mode)
            .field("gateway_mode", &self.rdp.gateway.mode)
            .field(
                "gateway_hostname_present",
                &self.rdp.gateway.hostname.is_some(),
            )
            .field(
                "gateway_username_present",
                &self.rdp.gateway.username.is_some(),
            )
            .field(
                "gateway_password_present",
                &self.rdp.gateway.password.is_some(),
            )
            .field("proxy", &proxy_debug_label(self.proxy.as_ref()))
            .finish()
    }
}

fn option_len(value: &Option<String>) -> Option<usize> {
    value.as_ref().map(String::len)
}

fn proxy_debug_label(proxy: Option<&ProxyTunnelConfig>) -> Option<&'static str> {
    proxy.map(|proxy| match proxy.proxy_type {
        connection_tunnel::ProxyTunnelType::Socks5 => "socks5",
        connection_tunnel::ProxyTunnelType::Http => "http",
    })
}

fn storage_proxy_config(proxy: one_core::storage::ProxyConfig) -> ProxyTunnelConfig {
    ProxyTunnelConfig {
        proxy_type: match proxy.proxy_type {
            one_core::storage::ProxyType::Socks5 => connection_tunnel::ProxyTunnelType::Socks5,
            one_core::storage::ProxyType::Http => connection_tunnel::ProxyTunnelType::Http,
        },
        host: proxy.host.trim().to_string(),
        port: proxy.port,
        username: normalized_optional(proxy.username),
        password: preserved_secret(proxy.password),
    }
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn preserved_secret(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use one_core::storage::{
        ProxyConfig as StoredProxyConfig, ProxyType as StoredProxyType,
        RdpAudioMode as StoredRdpAudioMode, RdpDisplayMode as StoredRdpDisplayMode,
        RdpGatewayMode as StoredRdpGatewayMode, RdpSettings as StoredRdpSettings,
        RdpSharedFolder as StoredRdpSharedFolder,
        RemoteDesktopBackendPreference as StoredRemoteDesktopBackendPreference,
        RemoteDesktopParams as StoredRemoteDesktopParams,
        RemoteDesktopProtocol as StoredRemoteDesktopProtocol,
    };

    use super::*;

    #[test]
    fn connection_options_debug_redacts_credentials_folders_and_proxy_address() {
        let options = RemoteDesktopConnectionOptions {
            protocol: RemoteDesktopProtocol::Rdp,
            backend_preference: RemoteDesktopBackendPreference::Auto,
            destination: "10.2.178.12:3389".to_string(),
            username: Some("administrator".to_string()),
            password: Some("secret".to_string()),
            domain: Some("private.example".to_string()),
            read_only: false,
            audio_playback: true,
            audio_capture: false,
            shared_folders: vec![RemoteDesktopSharedFolder {
                name: "workspace".to_string(),
                path: PathBuf::from("/Users/rachel/private-project"),
                read_only: true,
            }],
            rdp: StoredRdpSettings::default(),
            proxy: Some(ProxyTunnelConfig {
                proxy_type: connection_tunnel::ProxyTunnelType::Http,
                host: "proxy.private.example".to_string(),
                port: 8443,
                username: Some("proxy-user".to_string()),
                password: Some("proxy-secret".to_string()),
            }),
        };

        let debug = format!("{options:?}");

        assert!(debug.contains("username_present: true"));
        assert!(debug.contains("username_len: Some(13)"));
        assert!(debug.contains("password_present: true"));
        assert!(debug.contains("domain_present: true"));
        assert!(debug.contains("domain_len: Some(15)"));
        assert!(debug.contains("audio_playback: true"));
        assert!(debug.contains("audio_capture: false"));
        assert!(debug.contains("shared_folder_count: 1"));
        assert!(debug.contains("proxy: Some(\"http\")"));
        assert!(!debug.contains("administrator"));
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("private.example"));
        assert!(!debug.contains("workspace"));
        assert!(!debug.contains("private-project"));
        assert!(!debug.contains("proxy-user"));
        assert!(!debug.contains("8443"));
    }

    #[test]
    fn storage_params_convert_to_connection_options_without_saving() {
        let options =
            RemoteDesktopConnectionOptions::from_storage_params(StoredRemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Rdp,
                host: "xrdp.example".to_string(),
                port: 3390,
                username: Some("alice".to_string()),
                password: Some("secret".to_string()),
                credential_reference: None,
                domain: Some("LAB".to_string()),
                read_only: true,
                audio_playback: true,
                proxy: Some(StoredProxyConfig {
                    proxy_type: StoredProxyType::Socks5,
                    host: " proxy.example ".to_string(),
                    port: 1080,
                    username: Some(" proxy-user ".to_string()),
                    password: Some("proxy-secret".to_string()),
                    credential_reference: None,
                }),
                backend_preference: StoredRemoteDesktopBackendPreference::WindowsNative,
                rdp: None,
            });

        assert_eq!(RemoteDesktopProtocol::Rdp, options.protocol);
        assert_eq!(
            StoredRemoteDesktopBackendPreference::WindowsNative,
            options.backend_preference
        );
        assert_eq!("xrdp.example:3390", options.destination);
        assert_eq!(Some("alice".to_string()), options.username);
        assert_eq!(Some("secret".to_string()), options.password);
        assert_eq!(Some("LAB".to_string()), options.domain);
        assert!(options.read_only);
        assert!(options.audio_playback);
        assert!(!options.audio_capture);
        assert!(options.shared_folders.is_empty());
        let proxy = options.proxy.unwrap();
        assert!(proxy.proxy_type == connection_tunnel::ProxyTunnelType::Socks5);
        assert_eq!("proxy.example", proxy.host);
        assert_eq!(Some("proxy-user".to_string()), proxy.username);
        assert_eq!(Some("proxy-secret".to_string()), proxy.password);
    }

    #[test]
    fn vnc_storage_params_disable_audio_playback() {
        let options =
            RemoteDesktopConnectionOptions::from_storage_params(StoredRemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Vnc,
                host: "vnc.example".to_string(),
                port: 5900,
                username: None,
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: true,
                proxy: None,
                backend_preference: StoredRemoteDesktopBackendPreference::WindowsNative,
                rdp: None,
            });

        assert_eq!(RemoteDesktopProtocol::Vnc, options.protocol);
        assert_eq!(
            StoredRemoteDesktopBackendPreference::Canvas,
            options.backend_preference
        );
        assert!(!options.audio_playback);
    }

    #[test]
    fn complete_native_rdp_settings_map_into_runtime_options() {
        let mut rdp = StoredRdpSettings::default();
        rdp.admin_session = true;
        rdp.display.mode = StoredRdpDisplayMode::Fixed;
        rdp.display.width = 2560;
        rdp.display.height = 1440;
        rdp.audio.mode = StoredRdpAudioMode::Remote;
        rdp.audio.capture = true;
        rdp.resources.shared_folders.push(StoredRdpSharedFolder {
            name: "workspace".to_string(),
            path: "D:\\workspace".to_string(),
            read_only: true,
        });
        rdp.gateway.mode = StoredRdpGatewayMode::Explicit;
        rdp.gateway.hostname = Some("gateway.example".to_string());
        rdp.gateway.username = Some("gateway-user".to_string());
        rdp.gateway.password = Some("gateway-secret".to_string());

        let options =
            RemoteDesktopConnectionOptions::from_storage_params(StoredRemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Rdp,
                host: "rdp.example".to_string(),
                port: 3389,
                username: Some("alice".to_string()),
                password: Some("secret".to_string()),
                domain: Some("CORP".to_string()),
                read_only: false,
                audio_playback: true,
                proxy: None,
                backend_preference: StoredRemoteDesktopBackendPreference::WindowsNative,
                rdp: Some(rdp.clone()),
            });

        assert_eq!(rdp, options.rdp);
        assert!(!options.audio_playback);
        assert!(options.audio_capture);
        assert_eq!(1, options.shared_folders.len());
        assert_eq!(
            PathBuf::from("D:\\workspace"),
            options.shared_folders[0].path
        );
        let debug = format!("{options:?}");
        assert!(debug.contains("gateway_mode: Explicit"));
        assert!(debug.contains("gateway_hostname_present: true"));
        assert!(!debug.contains("gateway.example"));
        assert!(!debug.contains("gateway-user"));
        assert!(!debug.contains("gateway-secret"));
    }
}
