use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::Path;

pub(super) fn read_optional_text(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub(super) fn read_optional_json_object(path: &Path) -> Result<Map<String, Value>> {
    let text = read_optional_text(path)?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text)? {
        Value::Object(object) => Ok(object),
        _ => bail!("Claude config root must be a JSON object"),
    }
}

pub(super) fn write_user_only_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(&tmp_path)?;
    file.write_all(&bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }

    fs::rename(tmp_path, path)?;
    Ok(())
}
