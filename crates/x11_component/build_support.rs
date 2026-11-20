use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub const BUNDLED_ARCHIVE_ENV: &str = "NAVOP_X11_BUNDLED_ARCHIVE";
pub const BUNDLED_TARGET_ENV: &str = "NAVOP_X11_BUNDLED_TARGET";
pub const BUNDLED_ARCHIVE_FILE: &str = "navop-x11-bundled.zip";

pub struct EmbeddedBuild {
    pub target: &'static str,
    pub source: PathBuf,
}

pub fn target_key(target_os: &str, target_arch: &str) -> Result<&'static str, String> {
    match (target_os, target_arch) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("windows", "aarch64") => Ok("windows-aarch64"),
        _ => Err(format!(
            "{BUNDLED_ARCHIVE_ENV} 仅支持 Windows x86_64/aarch64，实际为 {target_os}-{target_arch}"
        )),
    }
}

/// 没有环境变量时写空占位文件，保留原源码构建；指定后所有输入错误都终止构建。
pub fn configure(
    archive: Option<OsString>,
    target_os: &str,
    target_arch: &str,
    out_dir: &Path,
) -> Result<Option<EmbeddedBuild>, String> {
    let output = out_dir.join(BUNDLED_ARCHIVE_FILE);
    let Some(archive) = archive else {
        fs::write(&output, [])
            .map_err(|error| format!("无法写入空嵌入占位 {}：{error}", output.display()))?;
        return Ok(None);
    };
    if archive.is_empty() {
        return Err(format!("{BUNDLED_ARCHIVE_ENV} 不能为空字符串"));
    }
    let target = target_key(target_os, target_arch)?;
    let source = PathBuf::from(archive);
    if !source.is_absolute() {
        return Err(format!(
            "{BUNDLED_ARCHIVE_ENV} 必须是绝对 zip 路径：{}",
            source.display()
        ));
    }
    if source.extension().and_then(|value| value.to_str()) != Some("zip") {
        return Err(format!(
            "{BUNDLED_ARCHIVE_ENV} 必须指向 .zip 文件：{}",
            source.display()
        ));
    }
    let source = fs::canonicalize(&source).map_err(|error| {
        format!(
            "{BUNDLED_ARCHIVE_ENV} 路径无法解析 {}：{error}",
            source.display()
        )
    })?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("无法读取嵌入 zip {}：{error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!("嵌入路径不是普通文件：{}", source.display()));
    }
    fs::copy(&source, &output).map_err(|error| {
        format!(
            "无法复制嵌入 zip {} 到 {}：{error}",
            source.display(),
            output.display()
        )
    })?;
    Ok(Some(EmbeddedBuild { target, source }))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "navop-x11-build-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn target_keys_are_explicit_and_fail_closed() {
        assert_eq!(target_key("windows", "x86_64").unwrap(), "windows-x86_64");
        assert_eq!(target_key("windows", "aarch64").unwrap(), "windows-aarch64");
        assert!(target_key("windows", "x86").is_err());
        assert!(target_key("macos", "aarch64").is_err());
    }

    #[test]
    fn missing_archive_preserves_source_build() {
        let out = test_dir("source");
        assert!(configure(None, "macos", "arm64", &out).unwrap().is_none());
        assert_eq!(fs::read(out.join(BUNDLED_ARCHIVE_FILE)).unwrap(), b"");
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn archive_path_must_be_absolute_existing_zip() {
        let out = test_dir("invalid-path");
        let relative = OsString::from("relative.zip");
        assert!(configure(Some(relative), "windows", "x86_64", &out).is_err());
        let missing = out.join("missing.zip");
        assert!(configure(Some(missing.into_os_string()), "windows", "x86_64", &out).is_err());
        let wrong_extension = out.join("archive.tar");
        fs::write(&wrong_extension, b"not zip").unwrap();
        assert!(
            configure(
                Some(wrong_extension.into_os_string()),
                "windows",
                "x86_64",
                &out
            )
            .is_err()
        );
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn configured_archive_is_copied_with_matching_target() {
        let out = test_dir("embedded");
        let source = out.join("source.zip");
        fs::write(&source, b"signed zip bytes").unwrap();
        let embedded = configure(
            Some(source.clone().into_os_string()),
            "windows",
            "aarch64",
            &out,
        )
        .unwrap()
        .unwrap();
        assert_eq!(embedded.target, "windows-aarch64");
        assert_eq!(
            fs::read(out.join(BUNDLED_ARCHIVE_FILE)).unwrap(),
            b"signed zip bytes"
        );
        fs::remove_dir_all(out).unwrap();
    }
}
