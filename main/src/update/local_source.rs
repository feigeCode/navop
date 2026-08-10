use std::path::PathBuf;

use url::Url;

/// 将本地更新地址解析为文件路径。
///
/// 支持普通绝对/相对路径以及 `file://` URL。其他带 scheme 的地址
///（例如 HTTP、HTTPS、FTP）会返回 `None`，继续走原有网络更新流程。
pub(crate) fn local_file_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if looks_like_windows_absolute_path(value) {
        return Some(PathBuf::from(value));
    }

    match Url::parse(value) {
        Ok(url) if url.scheme() == "file" => url.to_file_path().ok(),
        Ok(_) => None,
        Err(_) => Some(PathBuf::from(value)),
    }
}

pub(crate) fn is_local_file_source(value: &str) -> bool {
    local_file_path(value).is_some()
}

/// 当 manifest 来自本地文件时，将其中的相对文件地址解析为相对于 manifest
/// 所在目录的路径。绝对路径、`file://` URL 和网络 URL 保持原样。
pub(crate) fn resolve_local_reference(base_source: &str, reference: &str) -> String {
    let Some(reference_path) = local_file_path(reference) else {
        return reference.to_string();
    };
    if reference_path.is_absolute() || looks_like_windows_absolute_path(reference) {
        return reference.to_string();
    }

    let Some(base_path) = local_file_path(base_source) else {
        return reference.to_string();
    };
    let Some(parent) = base_path.parent() else {
        return reference.to_string();
    };

    parent.join(reference_path).to_string_lossy().into_owned()
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_plain_relative_and_absolute_paths() {
        assert_eq!(
            local_file_path("fixtures/latest.json"),
            Some(PathBuf::from("fixtures/latest.json"))
        );
        assert_eq!(
            local_file_path("/tmp/navop/latest.json"),
            Some(PathBuf::from("/tmp/navop/latest.json"))
        );
        assert_eq!(
            local_file_path(r"C:\navop\latest.json"),
            Some(PathBuf::from(r"C:\navop\latest.json"))
        );
    }

    #[test]
    fn converts_file_url_and_decodes_escaped_characters() {
        let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
        let path = temp_dir.path().join("local update.json");
        let file_url = Url::from_file_path(&path)
            .expect("应生成 file URL")
            .to_string();

        assert_eq!(local_file_path(&file_url), Some(path));
    }

    #[test]
    fn leaves_network_and_other_url_schemes_to_the_network_client() {
        assert_eq!(local_file_path("https://example.test/latest.json"), None);
        assert_eq!(local_file_path("http://example.test/latest.json"), None);
        assert_eq!(local_file_path("ftp://example.test/latest.json"), None);
    }

    #[test]
    fn resolves_relative_package_path_from_local_manifest_directory() {
        assert_eq!(
            resolve_local_reference("/tmp/navop/latest.json", "packages/navop.tar.gz"),
            PathBuf::from("/tmp/navop")
                .join("packages/navop.tar.gz")
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(
            resolve_local_reference(
                "/tmp/navop/latest.json",
                "https://example.test/navop.tar.gz"
            ),
            "https://example.test/navop.tar.gz"
        );
        assert_eq!(
            resolve_local_reference("/tmp/navop/latest.json", "/fixtures/navop.tar.gz"),
            "/fixtures/navop.tar.gz"
        );
    }
}
