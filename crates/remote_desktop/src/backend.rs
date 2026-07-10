use crate::backends::rdp::{HelperProcessConfig, RdpBackend};
use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopProviderManifest,
    RemoteDesktopProviderRegistry, RemoteDesktopRuntime, RemoteDesktopSize,
};
use connection_tunnel::{TunnelGuard, start_proxy_tunnel};

const MIN_RDP_PROVIDER_VERSION: &str = "0.1.4";
const MIN_VNC_PROVIDER_VERSION: &str = "0.1.1";

pub trait RemoteDesktopBackend: Send + 'static {
    fn name(&self) -> &'static str {
        "remote-desktop-backend"
    }

    fn start(
        self: Box<Self>,
        initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime>;
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("remote desktop provider version is not supported")]
pub struct RemoteDesktopProviderVersionError {
    pub protocol: RemoteDesktopProtocol,
    pub installed: String,
    pub required: String,
    pub invalid: bool,
}

pub fn create_backend(options: RemoteDesktopConnectionOptions) -> Box<dyn RemoteDesktopBackend> {
    let registry = RemoteDesktopProviderRegistry::load_default();
    match create_backend_with_registry(options.clone(), &registry) {
        Ok(backend) => backend,
        Err(error) => Box::new(MissingProviderBackend {
            protocol: options.protocol,
            error,
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
    validate_provider_requirement(&provider)?;
    let helper = provider_helper_process(&provider);
    Ok(Box::new(RdpBackend::new_with_helper(options, helper)))
}

pub(crate) fn resolve_proxy_options(
    mut options: RemoteDesktopConnectionOptions,
) -> anyhow::Result<(RemoteDesktopConnectionOptions, Option<TunnelGuard>)> {
    let Some(proxy) = options.proxy.take() else {
        return Ok((options, None));
    };
    let (host, port) = split_destination(&options.destination)?;
    let tunnel = start_proxy_tunnel(proxy, host, port)?;
    let local_addr = tunnel.local_addr();
    options.destination = local_addr.to_string();
    Ok((options, Some(tunnel.into())))
}

fn split_destination(destination: &str) -> anyhow::Result<(String, u16)> {
    let (host, port) = destination
        .trim()
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("remote desktop destination must include a port"))?;
    let host = host.trim_matches(['[', ']']).trim();
    if host.is_empty() {
        anyhow::bail!("remote desktop destination host is required");
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("remote desktop destination port is invalid"))?;
    Ok((host.to_string(), port))
}

fn validate_provider_requirement(provider: &RemoteDesktopProviderManifest) -> anyhow::Result<()> {
    let Some(required) = provider_min_version(provider.protocol) else {
        return Ok(());
    };
    let version = semver::Version::parse(provider.version.trim()).map_err(|_| {
        RemoteDesktopProviderVersionError {
            protocol: provider.protocol,
            installed: display_provider_version(&provider.version).to_string(),
            required: required.to_string(),
            invalid: true,
        }
    })?;
    let required = semver::Version::parse(required)?;
    if version < required {
        return Err(RemoteDesktopProviderVersionError {
            protocol: provider.protocol,
            installed: version.to_string(),
            required: required.to_string(),
            invalid: false,
        }
        .into());
    }
    Ok(())
}

fn provider_min_version(protocol: RemoteDesktopProtocol) -> Option<&'static str> {
    match protocol {
        RemoteDesktopProtocol::Rdp => Some(MIN_RDP_PROVIDER_VERSION),
        RemoteDesktopProtocol::Vnc => Some(MIN_VNC_PROVIDER_VERSION),
    }
}

fn display_provider_version(version: &str) -> &str {
    let version = version.trim();
    if version.is_empty() {
        "<empty>"
    } else {
        version
    }
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
    error: anyhow::Error,
}

impl RemoteDesktopBackend for MissingProviderBackend {
    fn name(&self) -> &'static str {
        "missing-remote-desktop-provider"
    }

    fn start(
        self: Box<Self>,
        _initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime> {
        Err(self
            .error
            .context(format!("{} remote desktop provider", self.protocol.label())))
    }
}

#[cfg(test)]
mod tests {
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

        let error = match super::create_backend_with_registry(
            options(RemoteDesktopProtocol::Rdp),
            &registry,
        ) {
            Ok(_) => panic!("outdated RDP provider should be rejected"),
            Err(error) => error,
        };

        let version_error = error
            .downcast_ref::<RemoteDesktopProviderVersionError>()
            .expect("version error");
        assert_eq!(RemoteDesktopProtocol::Rdp, version_error.protocol);
        assert_eq!("0.1.3", version_error.installed);
        assert_eq!("0.1.4", version_error.required);
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
            "0.1.0",
            "./onetcli-vnc-helper",
        );
        let registry = RemoteDesktopProviderRegistry::load_from_dir(temp.path()).unwrap();

        let error = match super::create_backend_with_registry(
            options(RemoteDesktopProtocol::Vnc),
            &registry,
        ) {
            Ok(_) => panic!("outdated VNC provider should be rejected"),
            Err(error) => error,
        };

        let version_error = error
            .downcast_ref::<RemoteDesktopProviderVersionError>()
            .expect("version error");
        assert_eq!(RemoteDesktopProtocol::Vnc, version_error.protocol);
        assert_eq!("0.1.0", version_error.installed);
        assert_eq!("0.1.1", version_error.required);
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
}
