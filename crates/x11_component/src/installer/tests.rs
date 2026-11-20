use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::X11ComponentError;
use crate::process::{PidFile, instance_path};
use crate::state::set_active_version;

fn test_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "x11-installer-{name}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn active_version_cannot_be_replaced_by_force_redownload() {
    let _guard = crate::test_support::global_state_lock();
    set_active_version(Some("1.9.0".into()));
    let root = test_root("local-active");
    let error = super::ensure_replacement_allowed(&root, "1.9.0").unwrap_err();
    assert!(matches!(error, X11ComponentError::Install(_)));
    assert!(error.to_string().contains("重启应用"));
    set_active_version(None);
    assert!(super::ensure_replacement_allowed(&root, "1.9.0").is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn another_process_active_version_cannot_be_replaced() {
    let root = test_root("other-active");
    let record = PidFile::capture(
        std::process::id(),
        &std::env::current_exe().unwrap(),
        "1.9.0",
    )
    .unwrap();
    record.write(&instance_path(&root, &record)).unwrap();
    let error = super::ensure_replacement_allowed(&root, &record.version).unwrap_err();
    assert!(matches!(error, X11ComponentError::Install(_)));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn commit_rechecks_active_version_before_replacement() {
    let root = test_root("commit-active");
    let staging = root.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::create_dir_all(root.join("versions/1.9.0")).unwrap();
    let record = PidFile::capture(
        std::process::id(),
        &std::env::current_exe().unwrap(),
        "1.9.0",
    )
    .unwrap();
    record.write(&instance_path(&root, &record)).unwrap();
    let error = super::commit(&staging, &root, &record.version)
        .await
        .unwrap_err();
    assert!(matches!(error, X11ComponentError::Install(_)));
    assert!(root.join("versions/1.9.0").exists());
    assert!(staging.exists(), "拒绝提交时不能移动 staging");
    std::fs::remove_dir_all(root).unwrap();
}

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use sha2::{Digest as _, Sha256};

use crate::bundle::EmbeddedArchive;
use crate::{InstallConfig, X11ServerMode};

fn embedded_config(root: &std::path::Path, target: &str, signing: &SigningKey) -> InstallConfig {
    InstallConfig {
        data_dir: root.to_path_buf(),
        // 离线分支不得读取这个地址；直连标志也必须无关紧要。
        manifest_url: "http://127.0.0.1:1/must-not-be-requested.json".into(),
        public_key_base64: BASE64.encode(VerifyingKey::from(signing).to_bytes()),
        navop_version: semver::Version::new(0, 23, 4),
        target: target.into(),
        direct_http: false,
    }
}

fn zip_payload(executable: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("yserver.exe", executable),
        ("share/xkb/keymap", b"keymap".as_slice()),
    ] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn artifact_json(version: &str, payload: &[u8], signing: &SigningKey) -> serde_json::Value {
    serde_json::json!({
        "version": version,
        "url": "https://example.invalid/must-not-be-requested.zip",
        "sha256": hex::encode(Sha256::digest(payload)),
        "signature": BASE64.encode(signing.sign(payload).to_bytes()),
        "min_vulkan": "1.3.0",
        "format": "zip",
        "executable_path": "yserver.exe"
    })
}

fn embedded_manifest(payload: &[u8], signing: &SigningKey) -> String {
    serde_json::json!({
        "schema": 1,
        "components": {
            "x11-server": {
                "channel": "stable",
                "pinned_for_navop": ">=0.23.4, <0.24.0",
                "rollout_percent": 100,
                "artifacts": {
                    "windows-x86_64": artifact_json("1.9.0", payload, signing)
                }
            }
        }
    })
    .to_string()
}

async fn install_test_archive(
    config: &InstallConfig,
    purpose: super::InstallPurpose,
    manifest: &str,
    target: &str,
    payload: &[u8],
) -> Result<super::InstalledComponent, X11ComponentError> {
    super::install_embedded_manifest(
        config,
        purpose,
        manifest,
        EmbeddedArchive {
            // 生产引用编译期静态字节；测试将短小的临时夹具提升为同样的生命周期。
            target: Box::leak(target.to_owned().into_boxed_str()),
            bytes: Box::leak(payload.to_vec().into_boxed_slice()),
        },
    )
    .await
}

#[tokio::test]
async fn embedded_install_selects_target_without_http() {
    let root = test_root("embedded-offline");
    let signing = SigningKey::from_bytes(&[41; 32]);
    let payload = zip_payload(b"embedded-x64");
    let arm_payload = zip_payload(b"embedded-arm64");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&embedded_manifest(&payload, &signing)).unwrap();
    manifest["components"]["x11-server"]["artifacts"]["windows-aarch64"] =
        artifact_json("2.0.0", &arm_payload, &signing);
    let manifest = manifest.to_string();
    let config = embedded_config(&root, "windows-x86_64", &signing);

    let installed = install_test_archive(
        &config,
        super::InstallPurpose::Resolve(X11ServerMode::Embedded),
        &manifest,
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap();
    assert_eq!(installed.version, "1.9.0");
    let version_root = root.join("components/x11/versions/1.9.0");
    assert_eq!(
        std::fs::read(version_root.join("yserver.exe")).unwrap(),
        b"embedded-x64"
    );
    assert_eq!(
        std::fs::read(version_root.join("share/xkb/keymap")).unwrap(),
        b"keymap"
    );
    assert!(!root.join("components/x11/versions/2.0.0").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn embedded_target_mismatch_fails_closed() {
    let root = test_root("embedded-target");
    let signing = SigningKey::from_bytes(&[42; 32]);
    let payload = zip_payload(b"embedded-x64");
    let manifest = embedded_manifest(&payload, &signing);
    let config = embedded_config(&root, "windows-aarch64", &signing);
    let error = install_test_archive(
        &config,
        super::InstallPurpose::Resolve(X11ServerMode::Embedded),
        &manifest,
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, X11ComponentError::MissingArtifact(_)));
    assert!(!root.join("components/x11/current").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn embedded_bad_digest_does_not_commit() {
    let root = test_root("embedded-digest");
    let signing = SigningKey::from_bytes(&[43; 32]);
    let payload = zip_payload(b"embedded");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&embedded_manifest(&payload, &signing)).unwrap();
    manifest["components"]["x11-server"]["artifacts"]["windows-x86_64"]["sha256"] =
        "0".repeat(64).into();
    let config = embedded_config(&root, "windows-x86_64", &signing);
    let error = install_test_archive(
        &config,
        super::InstallPurpose::Resolve(X11ServerMode::Embedded),
        &manifest.to_string(),
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, X11ComponentError::Sha256Mismatch { .. }));
    assert!(!root.join("components/x11/current").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn embedded_bad_signature_does_not_commit() {
    let root = test_root("embedded-signature");
    let signing = SigningKey::from_bytes(&[44; 32]);
    let wrong = SigningKey::from_bytes(&[45; 32]);
    let payload = zip_payload(b"embedded");
    let manifest = embedded_manifest(&payload, &wrong);
    let config = embedded_config(&root, "windows-x86_64", &signing);
    let error = install_test_archive(
        &config,
        super::InstallPurpose::Resolve(X11ServerMode::Embedded),
        &manifest,
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, X11ComponentError::SignatureInvalid(_)));
    assert!(!root.join("components/x11/current").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn embedded_force_reinstall_verifies_and_replaces_existing_files() {
    let root = test_root("embedded-force");
    let signing = SigningKey::from_bytes(&[46; 32]);
    let payload = zip_payload(b"original");
    let manifest = embedded_manifest(&payload, &signing);
    let config = embedded_config(&root, "windows-x86_64", &signing);
    install_test_archive(
        &config,
        super::InstallPurpose::Resolve(X11ServerMode::Embedded),
        &manifest,
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap();
    let executable = root.join("components/x11/versions/1.9.0/yserver.exe");
    std::fs::write(&executable, b"tampered").unwrap();

    install_test_archive(
        &config,
        super::InstallPurpose::Redownload,
        &manifest,
        "windows-x86_64",
        &payload,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(executable).unwrap(), b"original");
    std::fs::remove_dir_all(root).unwrap();
}
