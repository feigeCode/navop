use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};

pub(crate) fn upload_file_name(path: &Path) -> Result<Vec<u8>> {
    let name = path
        .file_name()
        .context("ZMODEM upload path has no file name")?
        .as_encoded_bytes();
    validate_name(name)?;
    Ok(name.to_vec())
}

pub(crate) fn download_path(directory: &Path, remote_name: &[u8]) -> Result<PathBuf> {
    let name = remote_name
        .rsplit(|byte| matches!(byte, b'/' | b'\\'))
        .next()
        .unwrap_or_default();
    validate_name(name)?;
    Ok(directory.join(String::from_utf8_lossy(name).as_ref()))
}

fn validate_name(name: &[u8]) -> Result<()> {
    if name.is_empty() || matches!(name, b"." | b"..") {
        bail!("ZMODEM file name is empty or unsafe");
    }
    if name.contains(&0) {
        bail!("ZMODEM file name contains a NUL byte");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_uses_only_the_local_basename() {
        let name = upload_file_name(Path::new("/tmp/nested/report.txt")).expect("file name");
        assert_eq!(name, b"report.txt");
    }

    #[test]
    fn download_strips_unix_and_windows_parent_components() {
        let directory = Path::new("/tmp/downloads");
        assert_eq!(
            download_path(directory, b"../../etc/passwd").expect("safe path"),
            directory.join("passwd")
        );
        assert_eq!(
            download_path(directory, br"C:\Windows\system.ini").expect("safe path"),
            directory.join("system.ini")
        );
    }

    #[test]
    fn download_rejects_empty_and_parent_names() {
        let directory = Path::new("/tmp/downloads");
        assert!(download_path(directory, b"").is_err());
        assert!(download_path(directory, b".").is_err());
        assert!(download_path(directory, b"..").is_err());
    }
}
