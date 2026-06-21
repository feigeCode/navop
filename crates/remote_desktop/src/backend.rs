use crate::backends::rdp::{HelperProcessConfig, RdpBackend};
use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopProviderManifest,
    RemoteDesktopProviderRegistry, RemoteDesktopRuntime, RemoteDesktopSize,
};

pub trait RemoteDesktopBackend: Send + 'static {
    fn name(&self) -> &'static str {
        "remote-desktop-backend"
    }

    fn start(
        self: Box<Self>,
        initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime>;
}

pub fn create_backend(options: RemoteDesktopConnectionOptions) -> Box<dyn RemoteDesktopBackend> {
    let registry = RemoteDesktopProviderRegistry::load_default();
    match create_backend_with_registry(options.clone(), &registry) {
        Ok(backend) => backend,
        Err(error) => Box::new(MissingProviderBackend {
            protocol: options.protocol,
            reason: error.to_string(),
        }),
    }
}

pub fn create_backend_with_registry(
    options: RemoteDesktopConnectionOptions,
    registry: &RemoteDesktopProviderRegistry,
) -> anyhow::Result<Box<dyn RemoteDesktopBackend>> {
    let provider = registry.find(options.protocol).ok_or_else(|| {
        anyhow::anyhow!(
            "{} remote desktop provider is not installed",
            options.protocol.label()
        )
    })?;
    let helper = provider_helper_process(&provider);
    Ok(Box::new(RdpBackend::new_with_helper(options, helper)))
}

fn provider_helper_process(provider: &RemoteDesktopProviderManifest) -> HelperProcessConfig {
    let command = std::path::Path::new(&provider.entry.command);
    if command.is_absolute() {
        return HelperProcessConfig::new(
            command.to_path_buf(),
            provider.entry.args.clone(),
            provider.command_working_dir(),
        );
    }
    HelperProcessConfig::new(
        provider.manifest_dir.join(command),
        provider.entry.args.clone(),
        provider.command_working_dir(),
    )
}

struct MissingProviderBackend {
    protocol: RemoteDesktopProtocol,
    reason: String,
}

impl RemoteDesktopBackend for MissingProviderBackend {
    fn name(&self) -> &'static str {
        "missing-remote-desktop-provider"
    }

    fn start(
        self: Box<Self>,
        _initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime> {
        anyhow::bail!("{}: {}", self.protocol.label(), self.reason)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::{
        RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopProviderRegistry,
    };

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

    fn options(protocol: RemoteDesktopProtocol) -> RemoteDesktopConnectionOptions {
        RemoteDesktopConnectionOptions {
            protocol,
            destination: "127.0.0.1:3389".to_string(),
            username: None,
            password: None,
            domain: None,
            read_only: false,
        }
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
                }}
            }}"#
        )
    }
}
