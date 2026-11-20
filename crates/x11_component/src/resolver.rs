use std::path::PathBuf;

use x11_forwarding::{ServerEndpoint, detect_local_server};

use crate::installer::prepare;
use crate::launch::LaunchSpec;
use crate::metrics::{MetricEvent, record};
use crate::paths::{component_root, default_config, read_installed};
use crate::state::{configured_data_dir, embedded_disable_reason, update_status};
use crate::supervisor::start;
use crate::{X11ComponentError, X11ForwardingStatus, X11Resolution, X11ServerMode};

static RESOLVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn resolve_server(mode: X11ServerMode) -> X11Resolution {
    let _guard = RESOLVE_LOCK.lock().await;
    let resolution = match mode {
        X11ServerMode::External => external().await,
        X11ServerMode::Embedded => embedded(mode).await,
        X11ServerMode::Auto => match external().await {
            Ok(resolution) => Ok(resolution),
            Err(_) => {
                record(MetricEvent::AutoExternalFallback);
                embedded(mode).await
            }
        },
    };
    if resolution.is_err() && mode != X11ServerMode::External {
        record(MetricEvent::EmbeddedFailure);
    }
    let resolution = resolution.unwrap_or_else(disabled);
    update_status(resolution.status.clone());
    resolution
}

pub fn installed_version() -> Option<String> {
    let data_dir = configured_data_dir().ok()?;
    read_installed(&component_root(&data_dir))
}

pub async fn redownload() -> Result<String, X11ComponentError> {
    if !cfg!(target_os = "windows") {
        return Err(X11ComponentError::UnsupportedPlatform(
            std::env::consts::OS.into(),
        ));
    }
    crate::redownload_with_config(default_config()?).await
}

async fn external() -> Result<X11Resolution, X11ComponentError> {
    let proxy = tokio::task::spawn_blocking(detect_local_server)
        .await
        .map_err(|error| X11ComponentError::Process(error.to_string()))?
        .map_err(|error| X11ComponentError::Config(error.to_string()))?;
    let server = endpoint_name(proxy.endpoint());
    Ok(X11Resolution {
        proxy: Some(proxy),
        status: X11ForwardingStatus::External { server },
    })
}

async fn embedded(mode: X11ServerMode) -> Result<X11Resolution, X11ComponentError> {
    if !cfg!(target_os = "windows") {
        return Err(X11ComponentError::UnsupportedPlatform(
            std::env::consts::OS.into(),
        ));
    }
    if let Some(reason) = embedded_disable_reason() {
        return Err(X11ComponentError::EmbeddedDisabled(reason));
    }
    let spec = match manual_binary() {
        Some(executable) => LaunchSpec::manual(configured_data_dir()?, executable)?,
        None => {
            let config = default_config()?;
            let data_dir = config.data_dir.clone();
            let component = prepare(config, mode).await?;
            LaunchSpec::installed(data_dir, component)?
        }
    };
    start(spec).await
}

fn manual_binary() -> Option<PathBuf> {
    std::env::var_os("NAVOP_X11_YSERVER_PATH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn endpoint_name(endpoint: &ServerEndpoint) -> String {
    match endpoint {
        ServerEndpoint::Unix(path) => path.display().to_string(),
        ServerEndpoint::Inet { host, port } => format!("{host}:{port}"),
    }
}

fn disabled(error: X11ComponentError) -> X11Resolution {
    X11Resolution {
        proxy: None,
        status: X11ForwardingStatus::Disabled {
            reason: error.to_string(),
        },
    }
}
