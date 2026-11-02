use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const PORTABLE_MARKER_FILE: &str = "navop.portable";

/// debug 构建的默认数据目录名。开发态必须与已安装版本完全隔离，
/// 否则 `cargo run` 会直接写生产配置（连接库、设置、登录态）。
/// 与 `.env.local` 中常用的 `NAVOP_DATA_DIR=~/.navop-dev` 保持一致。
pub const DEVELOPMENT_DATA_DIR_NAME: &str = ".navop-dev";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppRunMode {
    Installed,
    Development { root: PathBuf },
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

    /// debug 构建使用独立的开发数据目录。
    pub fn is_development(&self) -> bool {
        matches!(self.mode, AppRunMode::Development { .. })
    }

    /// 只有正式安装态才允许迁移生产目录、注册文件关联等宿主级副作用。
    pub fn uses_installed_directories(&self) -> bool {
        matches!(self.mode, AppRunMode::Installed)
    }

    pub fn allows_persistent_master_key(&self) -> bool {
        true
    }

    pub fn requires_master_key_on_startup(
        &self,
        configured: bool,
        portable_remember_master_key: bool,
    ) -> bool {
        if self.is_portable() {
            !portable_remember_master_key
        } else {
            configured
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppPathOverrides {
    portable: bool,
    data_dir: Option<PathBuf>,
}

impl AppPathOverrides {
    /// 把配置/数据根目录挪到指定位置（等同 `--data-dir`）。
    ///
    /// 测试用它把写入引到临时目录：配置写入路径默认指向用户真实配置目录，
    /// 而测试里的设置起步是默认值，一次保存就会把用户真实 `settings.json`
    /// 覆盖成默认值。
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            portable: false,
            data_dir: Some(data_dir.into()),
        }
    }
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
    /// debug 构建的默认隔离目录；release 构建恒为 `None`。
    pub development_data_dir: Option<PathBuf>,
}

static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct AppPathNames {
    portable_marker: &'static str,
    development_data_dir_name: &'static str,
    cache_dir_name: &'static str,
}

const DEFAULT_APP_PATH_NAMES: AppPathNames = AppPathNames {
    portable_marker: PORTABLE_MARKER_FILE,
    development_data_dir_name: DEVELOPMENT_DATA_DIR_NAME,
    cache_dir_name: "navop",
};

static APP_PATH_NAMES: OnceLock<AppPathNames> = OnceLock::new();

/// 在任何路径解析之前按产品身份覆盖便携标记、开发目录与缓存目录名。
pub fn configure_app_path_names(
    portable_marker: &'static str,
    development_data_dir_name: &'static str,
    cache_dir_name: &'static str,
) {
    let _ = APP_PATH_NAMES.set(AppPathNames {
        portable_marker,
        development_data_dir_name,
        cache_dir_name,
    });
}

fn app_path_names() -> &'static AppPathNames {
    APP_PATH_NAMES.get().unwrap_or(&DEFAULT_APP_PATH_NAMES)
}

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
    if let Some(data_root) = requested_data_root(overrides, context, &executable_root) {
        return Ok(portable_paths(executable_root, data_root));
    }
    if let Some(development_root) = context.development_data_dir.as_ref() {
        return Ok(development_paths(development_root.clone()));
    }

    Ok(installed_paths()?)
}

/// 用户显式声明的便携数据根。显式参数与便携环境变量优先于 debug 隔离目录。
fn requested_data_root(
    overrides: &AppPathOverrides,
    context: &AppPathResolutionContext,
    executable_root: &Path,
) -> Option<PathBuf> {
    if let Some(path) = overrides.data_dir.as_ref() {
        return Some(make_absolute(path, &context.current_dir));
    }
    if overrides.portable {
        return Some(executable_root.join("data"));
    }
    let environment_data_dir = context
        .data_dir_environment
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(path) = environment_data_dir.as_ref() {
        return Some(make_absolute(path, &context.current_dir));
    }
    let portable_environment = context
        .portable_environment
        .as_deref()
        .is_some_and(is_truthy);
    let marker_portable = executable_root
        .join(app_path_names().portable_marker)
        .is_file();

    (portable_environment || marker_portable).then(|| executable_root.join("data"))
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

/// 当前进程是否运行在正式安装态。未初始化时按安装态处理，保持旧行为。
pub fn is_installed() -> bool {
    initialized_paths().is_none_or(AppPaths::uses_installed_directories)
}

pub fn master_key_on_startup_required(
    configured: bool,
    portable_remember_master_key: bool,
) -> bool {
    initialized_paths().map_or(configured, |paths| {
        paths.requires_master_key_on_startup(configured, portable_remember_master_key)
    })
}

pub fn process_context() -> Result<AppPathResolutionContext> {
    let executable_path = std::env::current_exe().context("resolve current executable")?;
    Ok(AppPathResolutionContext {
        development_data_dir: development_data_dir_for(&executable_path),
        executable_path,
        current_dir: std::env::current_dir().context("resolve current directory")?,
        portable_environment: std::env::var_os("NAVOP_PORTABLE"),
        data_dir_environment: std::env::var_os("NAVOP_DATA_DIR"),
    })
}

/// 本次运行该不该用独立的开发目录。
///
/// 只按 `cfg!(debug_assertions)` 判断不够：`cargo run --release`、`cargo build --release`
/// 后直接执行 `target/release/navop`、以及本地 `cargo bundle` 出来的 `.app`，过去都套用
/// 「正式安装态」的路径规则，会直接读写 `~/.config/navop` 与生产 keychain（issue: 开发态
/// 配置污染生产）。因此额外按**可执行文件来源**判定：只要 exe 还在 cargo 产物目录里，
/// 就是自测构建，一律隔离。
fn development_data_dir_for(executable: &Path) -> Option<PathBuf> {
    if !cfg!(debug_assertions) && !runs_from_source_build(executable) {
        return None;
    }

    dirs::home_dir().map(|home| home.join(app_path_names().development_data_dir_name))
}

/// 可执行文件是否来自源码构建：位于 cargo 的 `target/{debug,release}`（含 `targets`，
/// 兼容自定义 `CARGO_TARGET_DIR`）或 `cargo install` 的 `~/.cargo/bin`。
fn runs_from_source_build(executable: &Path) -> bool {
    let components: Vec<&OsStr> = executable
        .components()
        .map(|component| component.as_os_str())
        .collect();
    let within_cargo_target = components
        .iter()
        .any(|component| matches!(component.to_str(), Some("target" | "targets")))
        && components
            .iter()
            .any(|component| matches!(component.to_str(), Some("debug" | "release")));
    if within_cargo_target {
        return true;
    }

    dirs::home_dir()
        .is_some_and(|home| executable.starts_with(home.join(".cargo").join("bin")))
}

fn development_paths(root: PathBuf) -> AppPaths {
    AppPaths {
        mode: AppRunMode::Development { root: root.clone() },
        config_dir: root.join("config"),
        data_dir: Some(root.join("state")),
        cache_dir: Some(root.join("cache")),
    }
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
        cache_dir: dirs::cache_dir().map(|root| root.join(app_path_names().cache_dir_name)),
    })
}

fn prepare_paths(paths: &AppPaths) -> Result<()> {
    if paths.uses_installed_directories() {
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
        .with_context(|| format!("application directory is not writable: {}", probe.display()))?;
    std::fs::remove_file(&probe)
        .with_context(|| format!("remove write probe: {}", probe.display()))?;
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
