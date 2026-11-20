//! X11 组件的应用级启动配置。

use semver::Version;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum X11RuntimeConfigError {
    InvalidVersion,
    MissingDataDir,
}

pub(crate) struct X11RuntimeConfig {
    pub(crate) version: Version,
    pub(crate) data_dir: PathBuf,
}

pub(crate) fn parse_runtime_config(
    version: &str,
    data_dir: Option<PathBuf>,
) -> Result<X11RuntimeConfig, X11RuntimeConfigError> {
    let version = Version::parse(version).map_err(|_| X11RuntimeConfigError::InvalidVersion)?;
    let data_dir = data_dir.ok_or(X11RuntimeConfigError::MissingDataDir)?;
    Ok(X11RuntimeConfig { version, data_dir })
}

pub(crate) fn configure_runtime(version: &str) -> Result<(), X11RuntimeConfigError> {
    let config = parse_runtime_config(version, one_core::app_dirs::data_dir())?;
    x11_component::set_navop_version(config.version);
    x11_component::set_data_dir(config.data_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{X11RuntimeConfigError, parse_runtime_config};
    use std::path::PathBuf;

    #[test]
    fn parses_product_version_without_panicking() {
        let config = parse_runtime_config("0.23.4", Some(PathBuf::from("/tmp/navop-data")))
            .expect("合法版本与数据目录应可注入");

        assert_eq!("0.23.4", config.version.to_string());
        assert_eq!(PathBuf::from("/tmp/navop-data"), config.data_dir);
    }

    #[test]
    fn rejects_invalid_version_and_missing_data_dir() {
        assert_eq!(
            Err(X11RuntimeConfigError::InvalidVersion),
            parse_runtime_config("not-semver", Some(PathBuf::from("/tmp/navop-data")))
                .map(|config| (config.version, config.data_dir))
        );
        assert_eq!(
            Err(X11RuntimeConfigError::MissingDataDir),
            parse_runtime_config("0.23.4", None).map(|config| (config.version, config.data_dir))
        );
    }

    #[test]
    fn navop_app_package_version_is_valid() {
        assert!(
            parse_runtime_config(env!("CARGO_PKG_VERSION"), Some(PathBuf::from("data"))).is_ok()
        );
    }
}
