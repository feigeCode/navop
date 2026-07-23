use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const PORTABLE_MARKER_FILE: &str = "navop.portable";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppRunMode {
    Installed,
    Portable { root: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    mode: AppRunMode,
    config_dir: PathBuf,
    data_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
}

impl AppPaths {
    pub fn mode(&self) -> &AppRunMode {
        &self.mode
    }

    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }

    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.as_ref()
    }

    pub fn cache_dir(&self) -> Option<&PathBuf> {
        self.cache_dir.as_ref()
    }

    pub fn is_portable(&self) -> bool {
        matches!(self.mode, AppRunMode::Portable { .. })
    }

    pub fn allows_persistent_master_key(&self) -> bool {
        !self.is_portable()
    }

    pub fn requires_master_key_on_startup(&self, configured: bool) -> bool {
        configured || self.is_portable()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppPathOverrides {
    portable: bool,
    data_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedStartupArguments {
    pub path_overrides: AppPathOverrides,
    pub remaining: Vec<OsString>,
}

#[derive(Clone, Debug)]
pub struct AppPathResolutionContext {
    pub executable_path: PathBuf,
    pub current_dir: PathBuf,
    pub portable_environment: Option<OsString>,
    pub data_dir_environment: Option<OsString>,
}

static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

pub fn parse_startup_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<ParsedStartupArguments> {
    let mut path_overrides = AppPathOverrides::default();
    let mut remaining = Vec::new();
    let mut arguments = arguments.into_iter();
    let mut parse_options = true;

    while let Some(argument) = arguments.next() {
        if parse_options && argument == OsStr::new("--") {
            parse_options = false;
            continue;
        }
        if !parse_options {
            remaining.push(argument);
            continue;
        }
        if argument == OsStr::new("--portable") {
            path_overrides.portable = true;
            continue;
        }
        if argument == OsStr::new("--data-dir") {
            let value = arguments.next().context("--data-dir requires a path")?;
            path_overrides.data_dir = Some(PathBuf::from(value));
            continue;
        }
        if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--data-dir="))
        {
            if value.is_empty() {
                bail!("--data-dir requires a path");
            }
            path_overrides.data_dir = Some(PathBuf::from(value));
            continue;
        }
        remaining.push(argument);
    }

    Ok(ParsedStartupArguments {
        path_overrides,
        remaining,
    })
}

pub fn resolve_app_paths(
    overrides: &AppPathOverrides,
    context: &AppPathResolutionContext,
) -> Result<AppPaths> {
    let executable_root = executable_root(&context.executable_path)?;
    let marker_portable = executable_root.join(PORTABLE_MARKER_FILE).is_file();
    let environment_data_dir = context
        .data_dir_environment
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let portable_environment = context
        .portable_environment
        .as_deref()
        .is_some_and(is_truthy);
    let data_root = if let Some(path) = overrides.data_dir.as_ref() {
        Some(make_absolute(path, &context.current_dir))
    } else if overrides.portable {
        Some(executable_root.join("data"))
    } else if let Some(path) = environment_data_dir.as_ref() {
        Some(make_absolute(path, &context.current_dir))
    } else {
        (portable_environment || marker_portable).then(|| executable_root.join("data"))
    };

    if let Some(data_root) = data_root {
        return Ok(portable_paths(executable_root, data_root));
    }

    Ok(installed_paths()?)
}

pub fn initialize_app_paths(
    overrides: &AppPathOverrides,
    context: &AppPathResolutionContext,
) -> Result<&'static AppPaths> {
    let paths = resolve_app_paths(overrides, context)?;
    prepare_paths(&paths)?;
    APP_PATHS
        .set(paths)
        .map_err(|_| anyhow::anyhow!("application paths were already initialized"))?;
    Ok(APP_PATHS.get().expect("paths just initialized"))
}

pub fn initialized_paths() -> Option<&'static AppPaths> {
    APP_PATHS.get()
}

pub fn is_portable() -> bool {
    initialized_paths().is_some_and(AppPaths::is_portable)
}

pub fn master_key_on_startup_required(configured: bool) -> bool {
    initialized_paths().map_or(configured, |paths| {
        paths.requires_master_key_on_startup(configured)
    })
}

pub fn process_context() -> Result<AppPathResolutionContext> {
    Ok(AppPathResolutionContext {
        executable_path: std::env::current_exe().context("resolve current executable")?,
        current_dir: std::env::current_dir().context("resolve current directory")?,
        portable_environment: std::env::var_os("NAVOP_PORTABLE"),
        data_dir_environment: std::env::var_os("NAVOP_DATA_DIR"),
    })
}

fn portable_paths(root: PathBuf, data_root: PathBuf) -> AppPaths {
    AppPaths {
        mode: AppRunMode::Portable { root },
        config_dir: data_root.join("config"),
        data_dir: Some(data_root.join("state")),
        cache_dir: Some(data_root.join("cache")),
    }
}

fn installed_paths() -> Result<AppPaths> {
    Ok(AppPaths {
        mode: AppRunMode::Installed,
        config_dir: crate::app_dirs::installed_config_dir()?,
        data_dir: crate::app_dirs::installed_data_dir(),
        cache_dir: dirs::cache_dir().map(|root| root.join("navop")),
    })
}

fn prepare_paths(paths: &AppPaths) -> Result<()> {
    if !paths.is_portable() {
        return Ok(());
    }

    prepare_directory(&paths.config_dir)?;
    if let Some(data_dir) = paths.data_dir() {
        prepare_directory(data_dir)?;
    }
    if let Some(cache_dir) = paths.cache_dir() {
        prepare_directory(cache_dir)?;
    }
    Ok(())
}

fn prepare_directory(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("create {}", directory.display()))?;
    let probe = directory.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"navop")
        .with_context(|| format!("portable directory is not writable: {}", probe.display()))?;
    std::fs::remove_file(&probe)
        .with_context(|| format!("remove portable write probe: {}", probe.display()))?;
    Ok(())
}

fn make_absolute(path: &Path, current_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    }
}

fn executable_root(executable_path: &Path) -> Result<PathBuf> {
    let parent = executable_path
        .parent()
        .context("current executable has no parent directory")?;
    let app_bundle = parent
        .file_name()
        .filter(|name| *name == OsStr::new("MacOS"))
        .and_then(|_| parent.parent())
        .filter(|contents| contents.file_name() == Some(OsStr::new("Contents")))
        .and_then(Path::parent)
        .filter(|bundle| bundle.extension() == Some(OsStr::new("app")));

    Ok(app_bundle
        .and_then(Path::parent)
        .unwrap_or(parent)
        .to_path_buf())
}

fn is_truthy(value: &OsStr) -> bool {
    matches!(
        value
            .to_str()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[cfg(test)]
#[path = "app_paths_tests.rs"]
mod tests;
