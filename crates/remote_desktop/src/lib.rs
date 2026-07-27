pub mod backend;
pub mod capabilities;
pub mod config;
pub mod framebuffer;
pub mod helper_protocol;
pub mod input;
pub mod output;
pub mod output_mailbox;
pub mod provider;
pub mod provider_registry;
pub mod runtime;

pub mod backends;

mod helper_event_debug;
mod helper_input;
mod helper_protocol_debug;
mod input_debug;
mod output_debug;

pub use backend::{RemoteDesktopBackend, RemoteDesktopProviderVersionError, create_backend};
pub use capabilities::{RemoteDesktopCapabilities, ResizeSupport};
pub use config::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopSharedFolder,
    RemoteDesktopSize,
};
pub use connection_tunnel::{ProxyTunnelConfig, ProxyTunnelType};
pub use framebuffer::RgbaFramebuffer;
pub use input::{RemoteDesktopInput, RemoteKey, RemoteMouseButton, RemoteNamedKey};
pub use output::{
    RemoteDesktopFrameRect, RemoteDesktopOutput, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};
pub use provider::{
    PROVIDER_MANIFEST_FILE, RemoteDesktopProviderEntry, RemoteDesktopProviderManifest,
    RemoteDesktopProviderUi,
};
pub use provider_registry::{
    RemoteDesktopProviderLoadedEntry, RemoteDesktopProviderRegistry,
    RemoteDesktopProviderRegistryLoadReport, RemoteDesktopProviderSkippedEntry,
    default_provider_dir, default_provider_dirs,
};
pub use runtime::RemoteDesktopRuntime;

#[cfg(test)]
mod provider_registry_tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::{
        RemoteDesktopProtocol, RemoteDesktopProviderRegistry, default_provider_dir,
        default_provider_dirs,
    };

    #[test]
    fn provider_registry_loads_sorted_providers() {
        let temp = TempDir::new().unwrap();
        write_provider(temp.path(), "vnc", "VNC", "vnc", "./onetcli-vnc-helper");
        write_provider(temp.path(), "rdp", "RDP", "rdp", "./onetcli-rdp-helper");

        let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

        let ids: Vec<&str> = registry
            .providers()
            .iter()
            .map(|provider| provider.id.as_str())
            .collect();
        assert_eq!(vec!["rdp", "vnc"], ids);
        assert_eq!(
            Some("rdp".to_string()),
            registry
                .find(RemoteDesktopProtocol::Rdp)
                .map(|provider| provider.id)
        );
    }

    #[test]
    fn provider_registry_keeps_first_duplicate_id() {
        let temp = TempDir::new().unwrap();
        write_provider(temp.path(), "rdp", "RDP One", "rdp", "./one-helper");
        write_provider(temp.path(), "rdp-copy", "RDP Two", "rdp", "./two-helper");
        fs::rename(
            temp.path().join("rdp-copy"),
            temp.path().join("aaa-rdp-copy"),
        )
        .unwrap();
        fs::write(
            temp.path()
                .join("aaa-rdp-copy")
                .join("remote_desktop_provider.json"),
            provider_json("rdp", "RDP Two", "rdp", "./two-helper"),
        )
        .unwrap();

        let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

        assert_eq!(1, registry.providers().len());
        assert_eq!(
            temp.path().join("aaa-rdp-copy"),
            registry.providers()[0].manifest_dir
        );
    }

    #[test]
    fn provider_registry_reports_invalid_manifests() {
        let temp = TempDir::new().unwrap();
        write_provider(temp.path(), "vnc", "VNC", "vnc", "./onetcli-vnc-helper");
        fs::create_dir_all(temp.path().join("broken")).unwrap();
        fs::write(
            temp.path()
                .join("broken")
                .join("remote_desktop_provider.json"),
            r#"{
                "id":"",
                "name":"Broken",
                "protocol":"rdp",
                "entry":{"command":"./helper"},
                "capabilities":{
                    "resize":"remote_resize",
                    "clipboard_text":true,
                    "cursor_shape":true,
                    "audio":false,
                    "file_transfer":false
                }
            }"#,
        )
        .unwrap();

        let report = RemoteDesktopProviderRegistry::load_from_dir_with_report(temp.path()).unwrap();

        assert_eq!(1, report.loaded.len());
        assert_eq!(1, report.skipped.len());
        assert!(report.skipped[0].error.contains("id"));
    }

    #[test]
    fn provider_entry_command_remains_relative_when_file_exists_in_manifest_dir() {
        let temp = TempDir::new().unwrap();
        write_provider(temp.path(), "rdp", "RDP", "rdp", "./onetcli-rdp-helper");
        fs::write(
            temp.path().join("rdp").join("onetcli-rdp-helper"),
            b"helper",
        )
        .unwrap();

        let provider =
            RemoteDesktopProviderRegistry::load_provider_from_dir(&temp.path().join("rdp"))
                .unwrap()
                .unwrap();

        assert_eq!("./onetcli-rdp-helper", provider.entry.command);
    }

    #[test]
    fn default_provider_dirs_use_override_when_present() {
        let temp = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR", temp.path());
        }

        assert_eq!(vec![temp.path().to_path_buf()], default_provider_dirs());
        assert_eq!(temp.path().to_path_buf(), default_provider_dir());

        unsafe {
            std::env::remove_var("ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR");
        }
    }

    fn write_provider(
        root: &std::path::Path,
        dir: &str,
        name: &str,
        protocol: &str,
        command: &str,
    ) {
        let provider_dir = root.join(dir);
        fs::create_dir_all(&provider_dir).unwrap();
        fs::write(
            provider_dir.join("remote_desktop_provider.json"),
            provider_json(dir, name, protocol, command),
        )
        .unwrap();
    }

    fn provider_json(id: &str, name: &str, protocol: &str, command: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "name": "{name}",
                "description": "{name} provider",
                "version": "1.2.3",
                "protocol": "{protocol}",
                "entry": {{ "command": "{command}" }},
                "capabilities": {{
                    "resize": "remote_resize",
                    "clipboard_text": true,
                    "cursor_shape": true,
                    "audio": false,
                    "file_transfer": false
                }},
                "ui": {{ "default_port": 3389 }}
            }}"#
        )
    }
}
