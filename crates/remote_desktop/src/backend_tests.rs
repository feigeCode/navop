use std::fs;

use tempfile::TempDir;

use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopProviderRegistry,
    backend::RemoteDesktopProviderVersionError,
};
use connection_tunnel::{ProxyTunnelConfig, ProxyTunnelType, TunnelGuard};

#[test]
fn create_backend_with_registry_requires_installed_provider() {
    let options = options(RemoteDesktopProtocol::Rdp);
    let registry = RemoteDesktopProviderRegistry::empty();

    let err = match super::create_backend_with_registry(options, &registry) {
        Ok(_) => panic!("expected missing provider error"),
        Err(error) => error,
    };

    assert!(err.to_string().contains("RDP"));
}

#[test]
fn create_backend_with_registry_uses_provider_helper() {
    let temp = TempDir::new().unwrap();
    let provider_dir = temp.path().join("rdp");
    fs::create_dir_all(&provider_dir).unwrap();
    fs::write(provider_dir.join("onetcli-rdp-helper"), b"helper").unwrap();
    fs::write(
        provider_dir.join("remote_desktop_provider.json"),
        provider_json("rdp", "RDP", "rdp", "./onetcli-rdp-helper"),
    )
    .unwrap();
    let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

    let backend =
        super::create_backend_with_registry(options(RemoteDesktopProtocol::Rdp), &registry)
            .unwrap();

    assert_eq!("remote-desktop-helper", backend.name());
}

#[test]
fn create_backend_with_registry_rejects_outdated_rdp_provider() {
    let temp = TempDir::new().unwrap();
    write_provider(
        temp.path(),
        "rdp",
        "RDP",
        "rdp",
        "0.1.3",
        "./onetcli-rdp-helper",
    );
    let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

    let error =
        match super::create_backend_with_registry(options(RemoteDesktopProtocol::Rdp), &registry) {
            Ok(_) => panic!("outdated RDP provider should be rejected"),
            Err(error) => error,
        };

    let version_error = error
        .downcast_ref::<RemoteDesktopProviderVersionError>()
        .expect("version error");
    assert_eq!(RemoteDesktopProtocol::Rdp, version_error.protocol);
    assert_eq!("0.1.3", version_error.installed);
    assert_eq!("0.3.0", version_error.required);
    assert!(!version_error.invalid);
}

#[test]
fn create_backend_with_registry_rejects_outdated_vnc_provider() {
    let temp = TempDir::new().unwrap();
    write_provider(
        temp.path(),
        "vnc",
        "VNC",
        "vnc",
        "0.2.1",
        "./onetcli-vnc-helper",
    );
    let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

    let error =
        match super::create_backend_with_registry(options(RemoteDesktopProtocol::Vnc), &registry) {
            Ok(_) => panic!("outdated VNC provider should be rejected"),
            Err(error) => error,
        };

    let version_error = error
        .downcast_ref::<RemoteDesktopProviderVersionError>()
        .expect("version error");
    assert_eq!(RemoteDesktopProtocol::Vnc, version_error.protocol);
    assert_eq!("0.2.1", version_error.installed);
    assert_eq!("0.2.2", version_error.required);
    assert!(!version_error.invalid);
}

#[test]
fn proxied_options_use_loopback_destination_and_keep_guard() {
    let mut options = options(RemoteDesktopProtocol::Rdp);
    options.proxy = Some(ProxyTunnelConfig {
        proxy_type: ProxyTunnelType::Socks5,
        host: "127.0.0.1".to_string(),
        port: 9,
        username: None,
        password: None,
    });

    let (resolved, guard) = super::resolve_proxy_options(options).unwrap();

    let destination = resolved
        .destination
        .split_once(':')
        .expect("proxied destination should contain a port");
    assert!(
        destination
            .0
            .parse::<std::net::IpAddr>()
            .unwrap()
            .is_loopback()
    );
    assert!(matches!(guard, Some(TunnelGuard::Proxy(_))));
}

fn options(protocol: RemoteDesktopProtocol) -> RemoteDesktopConnectionOptions {
    RemoteDesktopConnectionOptions {
        protocol,
        destination: "127.0.0.1:3389".to_string(),
        username: None,
        password: None,
        domain: None,
        read_only: false,
        audio_playback: false,
        audio_capture: false,
        shared_folders: Vec::new(),
        proxy: None,
    }
}

fn write_provider(
    root: &std::path::Path,
    dir: &str,
    name: &str,
    protocol: &str,
    version: &str,
    command: &str,
) {
    let provider_dir = root.join(dir);
    fs::create_dir_all(&provider_dir).unwrap();
    fs::write(
        provider_dir.join("remote_desktop_provider.json"),
        provider_json_with_version(id_for_dir(dir), name, protocol, version, command),
    )
    .unwrap();
}

fn id_for_dir(dir: &str) -> &str {
    dir.strip_prefix("aaa-").unwrap_or(dir)
}

fn provider_json(id: &str, name: &str, protocol: &str, command: &str) -> String {
    provider_json_with_version(id, name, protocol, "1.2.3", command)
}

fn provider_json_with_version(
    id: &str,
    name: &str,
    protocol: &str,
    version: &str,
    command: &str,
) -> String {
    format!(
        r#"{{
                "id": "{id}",
                "name": "{name}",
                "description": "{name} provider",
                "version": "{version}",
                "protocol": "{protocol}",
                "entry": {{ "command": "{command}" }},
                "capabilities": {{
                    "resize": "remote_resize",
                    "clipboard_text": true,
                    "cursor_shape": true,
                    "audio": false,
                    "file_transfer": false
                }}
            }}"#
    )
}
