//! 构建期验证后嵌入的固定组件；运行时只接受这份内嵌归档。

pub const PINNED_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/windows/x11-component/navop-x11-manifest.json"
));

static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/navop-x11-bundled.zip"));

#[derive(Clone, Copy)]
pub struct EmbeddedArchive {
    pub target: &'static str,
    pub bytes: &'static [u8],
}

pub fn archive() -> Option<EmbeddedArchive> {
    (!ARCHIVE.is_empty()).then_some(EmbeddedArchive {
        target: env!("NAVOP_X11_BUNDLED_TARGET"),
        bytes: ARCHIVE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn pinned_manifest_contains_both_supported_windows_targets() {
        let manifest = crate::manifest::ComponentManifest::parse(
            PINNED_MANIFEST,
            &semver::Version::new(0, 23, 4),
        )
        .unwrap();
        for (target, executable) in [
            ("windows-x86_64", "yserver-1.9.0-windows-x86_64/yserver.exe"),
            (
                "windows-aarch64",
                "yserver-1.9.0-windows-aarch64/yserver.exe",
            ),
        ] {
            let artifact = manifest.artifact(target).unwrap();
            assert_eq!(artifact.version, "1.9.0-navop.1");
            assert_eq!(artifact.executable_path, executable);
        }
    }

    #[tokio::test]
    async fn configured_archive_installs_complete_component_offline() {
        let Some(archive) = archive() else {
            assert!(option_env!("NAVOP_X11_BUNDLED_ARCHIVE").is_none());
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "navop-x11-real-bundle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = crate::InstallConfig {
            data_dir: root.clone(),
            public_key_base64: crate::X11_PUBLIC_KEY_BASE64.into(),
            navop_version: semver::Version::new(0, 23, 4),
            target: archive.target.into(),
        };
        let result = crate::installer::prepare(config, crate::X11ServerMode::Embedded).await;
        let component = match result {
            Ok(component) => component,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&root);
                panic!("真实内嵌组件必须能离线完整安装：{error}");
            }
        };
        assert!(component.executable.is_file());
        let package_root = component.executable.parent().unwrap();
        assert!(package_root.join("xauth.exe").is_file());
        for directory in ["compat", "keycodes", "rules", "symbols", "types"] {
            assert!(package_root.join("xkb").join(directory).is_dir());
        }
        assert!(package_root.join("licenses/rust-crates.txt").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn configured_archive_matches_its_pinned_manifest() {
        let Some(archive) = archive() else {
            assert!(option_env!("NAVOP_X11_BUNDLED_ARCHIVE").is_none());
            return;
        };
        let manifest = crate::manifest::ComponentManifest::parse(
            PINNED_MANIFEST,
            &semver::Version::new(0, 23, 4),
        )
        .unwrap();
        let artifact = manifest.artifact(archive.target).unwrap();
        assert_eq!(
            hex::encode(sha2::Sha256::digest(archive.bytes)),
            artifact.sha256
        );
        crate::verify_ed25519(
            archive.bytes,
            &artifact.signature,
            crate::X11_PUBLIC_KEY_BASE64,
        )
        .unwrap();
    }
}
