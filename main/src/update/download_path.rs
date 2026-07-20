use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const UPDATE_ROOT_DIR: &str = "navop-update";
const DOWNLOAD_DIR_PREFIX: &str = "download";
const PARTIAL_FILE_SUFFIX: &str = ".part";
static DOWNLOAD_ATTEMPT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn build_download_path(version: &str, download_url: &str) -> Result<PathBuf, String> {
    let file_name = download_file_name(version, download_url);
    let attempt_id = DOWNLOAD_ATTEMPT_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("读取系统时间失败: {err}"))?
        .as_nanos();
    let dir = std::env::temp_dir().join(UPDATE_ROOT_DIR).join(format!(
        "{DOWNLOAD_DIR_PREFIX}-{}-{timestamp}-{attempt_id}",
        std::process::id()
    ));
    Ok(dir.join(file_name))
}

pub(crate) fn partial_download_path(download_path: &Path) -> Result<PathBuf, String> {
    let file_name = download_path
        .file_name()
        .ok_or_else(|| format!("更新文件名无效: {}", download_path.display()))?;
    let mut partial_name = file_name.to_os_string();
    partial_name.push(PARTIAL_FILE_SUFFIX);
    Ok(download_path.with_file_name(partial_name))
}

pub(crate) fn update_root_for_download(download_path: &Path) -> Option<&Path> {
    let root = download_path.parent()?.parent()?;
    (root.file_name()?.to_str()? == UPDATE_ROOT_DIR).then_some(root)
}

fn download_file_name(version: &str, download_url: &str) -> String {
    let extension = url_file_name(download_url)
        .map(archive_extension)
        .unwrap_or_default();
    let base_name = format!("navop-update-{}", version.replace('/', "-"));
    if extension.is_empty() {
        base_name
    } else {
        format!("{base_name}{extension}")
    }
}

fn url_file_name(download_url: &str) -> Option<&str> {
    let without_fragment = download_url.split('#').next()?;
    let without_query = without_fragment.split('?').next()?;
    without_query.rsplit('/').next()
}

fn archive_extension(file_name: &str) -> String {
    for extension in [".tar.gz", ".tgz", ".zip"] {
        if file_name.ends_with(extension) {
            return extension.to_string();
        }
    }

    Path::new(file_name)
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn download_file_name_preserves_archive_suffix() {
        assert_eq!(
            download_file_name(
                "0.3.2",
                "https://example.com/navop-x86_64-apple-darwin.tar.gz?source=r2"
            ),
            "navop-update-0.3.2.tar.gz"
        );
        assert_eq!(
            download_file_name(
                "0.3.2",
                "https://example.com/navop-x86_64-pc-windows-msvc.zip"
            ),
            "navop-update-0.3.2.zip"
        );
    }

    #[test]
    fn build_download_path_is_unique_per_attempt() {
        let url = "https://example.com/navop-aarch64-apple-darwin.tar.gz";

        let first = build_download_path("v0.8.13", url).expect("应生成首次下载路径");
        let second = build_download_path("v0.8.13", url).expect("应生成第二次下载路径");

        assert_ne!(first.parent(), second.parent());
        assert_eq!(
            first
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name),
            Some(OsStr::new("navop-update"))
        );
        assert_eq!(
            update_root_for_download(&first),
            first.parent().and_then(Path::parent)
        );
    }

    #[test]
    fn arbitrary_download_path_has_no_managed_update_root() {
        let path = std::env::temp_dir().join("unmanaged-download/navop.tar.gz");

        assert_eq!(update_root_for_download(&path), None);
    }

    #[test]
    fn partial_download_path_keeps_final_archive_untouched() {
        let final_path = Path::new("/tmp/navop-update.tar.gz");

        assert_eq!(
            partial_download_path(final_path).expect("应生成临时文件路径"),
            PathBuf::from("/tmp/navop-update.tar.gz.part")
        );
    }
}
