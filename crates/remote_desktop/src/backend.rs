use crate::backends::rdp::{HelperProcessConfig, RdpBackend};
use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopProviderManifest,
    RemoteDesktopProviderRegistry, RemoteDesktopRuntime, RemoteDesktopSize,
};
use connection_tunnel::{TunnelGuard, start_proxy_tunnel};

const MIN_RDP_PROVIDER_VERSION: &str = "0.3.0";
const MIN_VNC_PROVIDER_VERSION: &str = "0.2.0";

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
#[path = "backend_tests.rs"]
mod tests;
