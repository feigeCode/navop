use std::sync::{OnceLock, RwLock};

use semver::{Version, VersionReq};
use thiserror::Error;

use super::schema::{CURRENT_SCHEMA_VERSION, Manifest};

pub struct HostApiVersions {
    pub extension: ApiVersion,
    pub database: ApiVersion,
    pub ui: ApiVersion,
    pub task: ApiVersion,
    pub connection: ApiVersion,
}

impl HostApiVersions {
    pub const fn current() -> Self {
        Self {
            extension: ApiVersion::new(1, 0),
            database: ApiVersion::new(1, 0),
            ui: ApiVersion::new(1, 0),
            task: ApiVersion::new(1, 0),
            connection: ApiVersion::new(1, 0),
        }
    }

    pub fn version_for(&self, api_name: &str) -> Option<ApiVersion> {
        Some(match api_name {
            "extension" => self.extension,
            "database" => self.database,
            "ui" => self.ui,
            "task" => self.task,
            "connection" => self.connection,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub fn parse(s: &str) -> Result<Self, ApiVersionParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ApiVersionParseError::Empty);
        }
        let mut parts = trimmed.split('.');
        let major = parts
            .next()
            .ok_or(ApiVersionParseError::MissingMajor)?
            .parse::<u16>()
            .map_err(|_| ApiVersionParseError::InvalidMajor(trimmed.to_string()))?;
        let minor = parts
            .next()
            .unwrap_or("0")
            .parse::<u16>()
            .map_err(|_| ApiVersionParseError::InvalidMinor(trimmed.to_string()))?;
        Ok(Self { major, minor })
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiVersionParseError {
    #[error("API 版本为空")]
    Empty,
    #[error("API 版本缺少 major")]
    MissingMajor,
    #[error("API major 不是合法数字: {0}")]
    InvalidMajor(String),
    #[error("API minor 不是合法数字: {0}")]
    InvalidMinor(String),
}

#[derive(Debug, Error)]
pub enum CompatibilityError {
    #[error("manifest schema 版本 {found} 高于宿主支持的最高版本 {max},请升级 onetcli")]
    SchemaVersionTooNew { found: u32, max: u32 },

    #[error("manifest schema 版本 {found} 不合法(必须 >= 1)")]
    SchemaVersionInvalid { found: u32 },

    #[error("engines.onetcli 字段为空,需要声明依赖的 onetcli 版本范围")]
    EnginesOnetcliMissing,

    #[error("engines.onetcli {required:?} 不是合法 SemVer range: {reason}")]
    EnginesOnetcliInvalid { required: String, reason: String },

    #[error("扩展要求 onetcli {required:?},当前 onetcli 版本 {current},请升级或寻找兼容版本")]
    HostVersionMismatch { required: String, current: String },

    #[error("扩展 api.{api} = {required:?} 不合法: {reason}")]
    ApiVersionParse {
        api: &'static str,
        required: String,
        reason: String,
    },

    #[error("扩展 api.{api} 需要 {required},宿主提供 {offered},MAJOR 版本不兼容")]
    ApiMajorMismatch {
        api: &'static str,
        required: ApiVersion,
        offered: ApiVersion,
    },

    #[error("扩展 api.{api} 需要 {required},宿主仅提供 {offered}(MINOR 版本低于需求)")]
    ApiMinorBehind {
        api: &'static str,
        required: ApiVersion,
        offered: ApiVersion,
    },
}

pub fn check_compatibility(
    manifest: &Manifest,
    host_version: &Version,
    host_apis: &HostApiVersions,
) -> Result<(), CompatibilityError> {
    if manifest.schema_version == 0 {
        return Err(CompatibilityError::SchemaVersionInvalid {
            found: manifest.schema_version,
        });
    }
    if manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(CompatibilityError::SchemaVersionTooNew {
            found: manifest.schema_version,
            max: CURRENT_SCHEMA_VERSION,
        });
    }
    if manifest.engines.onetcli.trim().is_empty() {
        return Err(CompatibilityError::EnginesOnetcliMissing);
    }
    let req = VersionReq::parse(&manifest.engines.onetcli).map_err(|e| {
        CompatibilityError::EnginesOnetcliInvalid {
            required: manifest.engines.onetcli.clone(),
            reason: e.to_string(),
        }
    })?;
    if !req.matches(host_version) {
        return Err(CompatibilityError::HostVersionMismatch {
            required: manifest.engines.onetcli.clone(),
            current: host_version.to_string(),
        });
    }
    for (api_name, required_str) in manifest.api.all_iter() {
        let required =
            ApiVersion::parse(required_str).map_err(|e| CompatibilityError::ApiVersionParse {
                api: api_name,
                required: required_str.to_string(),
                reason: e.to_string(),
            })?;
        let offered = host_apis
            .version_for(api_name)
            .unwrap_or_else(|| ApiVersion::new(0, 0));
        if required.major != offered.major {
            return Err(CompatibilityError::ApiMajorMismatch {
                api: api_name,
                required,
                offered,
            });
        }
        if offered.minor < required.minor {
            return Err(CompatibilityError::ApiMinorBehind {
                api: api_name,
                required,
                offered,
            });
        }
    }
    Ok(())
}

static HOST_VERSION_OVERRIDE: OnceLock<RwLock<Option<Version>>> = OnceLock::new();

pub fn set_current_host_version(version: &str) -> Result<(), semver::Error> {
    let version = Version::parse(version)?;
    if let Ok(mut current) = host_version_override().write() {
        *current = Some(version);
    }
    Ok(())
}

pub fn current_host_version() -> Version {
    if let Some(version) = host_version_override()
        .read()
        .ok()
        .and_then(|version| version.clone())
    {
        return version;
    }
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

fn host_version_override() -> &'static RwLock<Option<Version>> {
    HOST_VERSION_OVERRIDE.get_or_init(|| RwLock::new(None))
}
