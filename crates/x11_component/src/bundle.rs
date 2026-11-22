//! 构建期验证后嵌入的固定组件；运行时只接受这份内嵌归档。
//!
//! 是否内嵌由 `bundled` 特性决定，默认关闭：组件分发策略未定期间，普通构建与
//! 发布构建都不把 zip 塞进二进制（`archive()` 恒为 `None`，`Embedded` 模式会
//! 明确报「当前构建未内嵌 X11 组件」，`Auto` 模式退回外置 X server）。
//! 需要内嵌时显式开启 `--features x11_component/bundled`。

pub const PINNED_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/windows/x11-component/navop-x11-manifest.json"
));

#[cfg(feature = "bundled")]
static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/navop-x11-bundled.zip"));

#[derive(Clone, Copy)]
pub struct EmbeddedArchive {
    pub target: &'static str,
    pub bytes: &'static [u8],
}

/// 本次构建内嵌的归档；未启用 `bundled` 特性或构建期没提供 zip 时为 `None`。
pub fn archive() -> Option<EmbeddedArchive> {
    #[cfg(feature = "bundled")]
    {
        return (!ARCHIVE.is_empty()).then_some(EmbeddedArchive {
            target: env!("NAVOP_X11_BUNDLED_TARGET"),
            bytes: ARCHIVE,
        });
    }
    #[cfg(not(feature = "bundled"))]
    {
        None
    }
}

/// 当前构建是否内嵌了固定 X11 组件。
pub fn bundled() -> bool {
    archive().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    /// 默认（未开 `bundled`）构建必须不内嵌任何归档，`Embedded` 模式因此走
    /// 「当前构建未内嵌」的错误路径；开启特性且构建期给了 zip 时必须有归档。
    #[test]
    fn bundled_feature_controls_the_archive() {
        #[cfg(feature = "bundled")]
        {
            if option_env!("NAVOP_X11_BUNDLED_ARCHIVE").is_some() {
                assert!(
                    archive().is_some(),
                    "提供归档后启用 bundled 特性必须内嵌组件"
                );
            }
        }
        #[cfg(not(feature = "bundled"))]
        assert!(archive().is_none(), "未启用 bundled 特性时不得内嵌组件");
    }

    #[test]
    fn pinned_manifest_contains_all_supported_targets() {
        let manifest = crate::manifest::ComponentManifest::parse(
            PINNED_MANIFEST,
            &semver::Version::new(0, 23, 4),
        )
        .unwrap();
        for (target, version, executable) in [
            (
                "windows-x86_64",
                "1.9.0-navop.1",
                "yserver-1.9.0-windows-x86_64/yserver.exe",
            ),
            (
                "windows-aarch64",
                "1.9.0-navop.1",
                "yserver-1.9.0-windows-aarch64/yserver.exe",
            ),
            (
                "macos-aarch64",
                "1.9.0-navop.2",
                "yserver-1.9.0-navop.2-macos-aarch64/yserver",
            ),
        ] {
            let artifact = manifest.artifact(target).unwrap();
            assert_eq!(artifact.version, version);
            assert_eq!(artifact.executable_path, executable);
        }
    }

    #[test]
    fn pinned_manifest_covers_the_next_release_line() {
        crate::manifest::ComponentManifest::parse(PINNED_MANIFEST, &semver::Version::new(0, 24, 0))
            .expect("0.24.0 发布线必须能解析内置清单");
        assert!(
            crate::manifest::ComponentManifest::parse(
                PINNED_MANIFEST,
                &semver::Version::new(0, 25, 0),
            )
            .is_err(),
            "超出固定范围的版本必须被拒绝"
        );
    }

    #[tokio::test]
    async fn configured_archive_installs_complete_component_offline() {
        let Some(archive) = archive() else {
            // 默认构建不内嵌组件；本用例只在真实内嵌构建里跑。
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
        if archive.target.starts_with("macos-") {
            for relative in [
                "yserver-xauth",
                "lib/libvulkan.1.dylib",
                "lib/libvulkan_kosmickrisp.dylib",
                "icd.d/kosmickrisp_mesa_icd.aarch64.json",
            ] {
                assert!(package_root.join(relative).is_file(), "缺少 {relative}");
            }
            let icd = std::fs::read_to_string(
                package_root.join("icd.d/kosmickrisp_mesa_icd.aarch64.json"),
            )
            .unwrap();
            assert!(icd.contains("@@YSERVER_ROOT@@/lib/libvulkan_kosmickrisp.dylib"));
            assert!(!icd.contains(".tmp-"));
        } else {
            assert!(package_root.join("xauth.exe").is_file());
        }
        for directory in ["compat", "keycodes", "rules", "symbols", "types"] {
            assert!(package_root.join("xkb").join(directory).is_dir());
        }
        assert!(package_root.join("licenses/rust-crates.txt").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn configured_archive_launches_real_macos_server() {
        let Some(archive) = archive() else {
            return;
        };
        if archive.target != "macos-aarch64" {
            return;
        }
        // GitHub 的 Apple Silicon runner 可能没有 Metal 设备。CI 先用随包
        // loader/ICD 独立运行 vulkaninfo，确认设备存在后才设置这个门禁；
        // 探测成功后若 yserver 启动失败，测试仍然失败，不能掩盖生产回归。
        let gpu_test_enabled = std::env::var("NAVOP_X11_REAL_GPU_TEST")
            .map(|value| matches!(value.as_str(), "1" | "true"))
            .unwrap_or(false);
        let in_ci =
            std::env::var_os("CI").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some();
        if in_ci && !gpu_test_enabled {
            eprintln!("跳过真实 Metal 启动：CI 的 Vulkan 能力探测未通过");
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "navop-x11-real-launch-{}-{}",
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
        let component = crate::installer::prepare(config, crate::X11ServerMode::Embedded)
            .await
            .expect("真实内嵌组件安装");
        let spec =
            crate::launch::LaunchSpec::installed(root.clone(), component).expect("真实启动参数");
        let probe = crate::supervisor::start(spec.clone()).await.map(|_| ());
        let icd = std::fs::read_to_string(
            spec.executable
                .parent()
                .unwrap()
                .join("icd.d/kosmickrisp_mesa_icd.aarch64.json"),
        )
        .unwrap();
        assert!(icd.contains(&spec.executable.parent().unwrap().display().to_string()));
        assert!(!icd.contains(".tmp-"));
        if probe.is_err() {
            eprintln!("启动根目录：{}", root.display());
            eprintln!("endpoint：{:?}", spec.endpoint());
            eprintln!(
                "yserver 日志：{}",
                root.join("components/x11/yserver-macos.log").display()
            );
            if let Ok(log) = std::fs::read_to_string(root.join("components/x11/yserver-macos.log"))
            {
                eprintln!("{log}");
            }
        } else {
            let _ = std::fs::remove_dir_all(&root);
        }
        probe.expect("内嵌 yserver 真实 X11 握手");
    }

    #[test]
    fn configured_archive_matches_its_pinned_manifest() {
        let Some(archive) = archive() else {
            // 默认构建不内嵌组件；本用例只在真实内嵌构建里跑。
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
