use std::io::ErrorKind;
use std::path::Path;

use rand::{RngCore as _, rngs::OsRng};

use crate::{X11ComponentError, X11ServerMode};

/// 每台机器使用持久随机编号，避免清单变化时灰度组漂移。
pub fn persistent_rollout_id(root: &Path) -> Result<u64, X11ComponentError> {
    std::fs::create_dir_all(root).map_err(config_error)?;
    let path = root.join("rollout-id");
    match std::fs::read_to_string(&path) {
        Ok(text) => return parse_rollout_id(&text),
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(config_error(error)),
        Err(_) => {}
    }
    let id = OsRng.next_u64();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            use std::io::Write as _;
            file.write_all(id.to_string().as_bytes())
                .map_err(config_error)?;
            Ok(id)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => persistent_rollout_id(root),
        Err(error) => Err(config_error(error)),
    }
}

/// Auto 遵守远程灰度；显式 Embedded 是 dogfood 入口，External 不触发内置。
pub fn should_use_embedded(
    root: &Path,
    mode: X11ServerMode,
    percent: u8,
) -> Result<bool, X11ComponentError> {
    match mode {
        X11ServerMode::External => Ok(false),
        X11ServerMode::Embedded => Ok(true),
        X11ServerMode::Auto => {
            if percent >= 100 {
                return Ok(true);
            }
            if percent == 0 {
                return Ok(false);
            }
            Ok((persistent_rollout_id(root)? % 100) < u64::from(percent))
        }
    }
}

fn parse_rollout_id(text: &str) -> Result<u64, X11ComponentError> {
    text.trim()
        .parse()
        .map_err(|_| X11ComponentError::Config("rollout-id 已损坏".into()))
}

fn config_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Config(error.to_string())
}

#[cfg(test)]
mod tests;
