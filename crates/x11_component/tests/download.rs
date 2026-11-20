mod common;

use common::{
    ArtifactSpec, MockServer, TestDir, manifest, manifest_artifact, public_key, unused_url,
};
use ed25519_dalek::SigningKey;
use x11_component::{
    InstallConfig, X11ComponentError, X11ServerMode, install_with_config,
    install_with_config_for_mode, redownload_with_config,
};

fn config(dir: &TestDir, server: &MockServer, signing: &SigningKey) -> InstallConfig {
    InstallConfig {
        data_dir: dir.path().to_path_buf(),
        manifest_url: format!("{}/manifest.json", server.url),
        public_key_base64: public_key(signing),
        navop_version: semver::Version::new(0, 23, 4),
        target: "windows-x86_64".into(),
        direct_http: true,
    }
}

fn component_root(dir: &TestDir) -> std::path::PathBuf {
    dir.path().join("components/x11")
}

#[tokio::test]
async fn successful_signature_installs_atomically() {
    let dir = TestDir::new("success");
    let signing = SigningKey::from_bytes(&[11; 32]);
    let payload = b"signed-yserver";
    let server = MockServer::new(&[]);
    let manifest_text = manifest(&format!("{}/yserver.exe", server.url), payload, &signing);
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/yserver.exe", 200, payload),
    ]);
    let config = config(&dir, &server, &signing);

    assert_eq!(install_with_config(config).await.unwrap(), "1.9.0");
    let root = component_root(&dir);
    assert_eq!(
        std::fs::read(root.join("versions/1.9.0/yserver.exe")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read_to_string(root.join("current")).unwrap(),
        "1.9.0"
    );
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-")
    }));
}

#[tokio::test]
async fn offline_download_fails_without_current_version() {
    let dir = TestDir::new("offline");
    let signing = SigningKey::from_bytes(&[12; 32]);
    let config = InstallConfig {
        data_dir: dir.path().to_path_buf(),
        manifest_url: unused_url(),
        public_key_base64: public_key(&signing),
        navop_version: semver::Version::new(0, 23, 4),
        target: "windows-x86_64".into(),
        direct_http: true,
    };
    assert!(matches!(
        install_with_config(config).await,
        Err(X11ComponentError::Download(_))
    ));
    assert!(!component_root(&dir).join("current").exists());
}

#[tokio::test]
async fn sha_mismatch_cleans_staging_and_disables_embedded() {
    let dir = TestDir::new("sha");
    let signing = SigningKey::from_bytes(&[13; 32]);
    let payload = b"truncated";
    let server = MockServer::new(&[]);
    let manifest_text = manifest(&format!("{}/artifact", server.url), b"expected", &signing);
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let error = install_with_config(config(&dir, &server, &signing))
        .await
        .unwrap_err();
    assert!(matches!(error, X11ComponentError::Sha256Mismatch { .. }));
    assert!(matches!(
        x11_component::current_status(),
        x11_component::X11ForwardingStatus::Disabled { reason }
            if reason.contains("SHA256") && reason.contains("禁用")
    ));
    assert!(!component_root(&dir).join("current").exists());
}

#[tokio::test]
async fn duplicate_install_reuses_artifact() {
    let dir = TestDir::new("duplicate");
    let signing = SigningKey::from_bytes(&[16; 32]);
    let payload = b"payload";
    let server = MockServer::new(&[]);
    let manifest_text = manifest(&format!("{}/artifact", server.url), payload, &signing);
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let config = config(&dir, &server, &signing);
    install_with_config(config.clone()).await.unwrap();
    install_with_config(config).await.unwrap();
    assert_eq!(server.hits("/manifest.json"), 2);
    assert_eq!(server.hits("/artifact"), 1);
}

#[tokio::test]
async fn force_redownload_fetches_the_artifact_again() {
    let dir = TestDir::new("force-redownload");
    let signing = SigningKey::from_bytes(&[20; 32]);
    let payload = b"payload";
    let server = MockServer::new(&[]);
    let manifest_text = manifest(&format!("{}/artifact", server.url), payload, &signing);
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let config = config(&dir, &server, &signing);
    install_with_config(config.clone()).await.unwrap();
    redownload_with_config(config).await.unwrap();
    assert_eq!(server.hits("/artifact"), 2);
}

#[tokio::test]
async fn damaged_version_directory_is_repaired_atomically() {
    let dir = TestDir::new("repair");
    let signing = SigningKey::from_bytes(&[21; 32]);
    let payload = b"payload";
    let server = MockServer::new(&[]);
    let manifest_text = manifest(&format!("{}/artifact", server.url), payload, &signing);
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let damaged = component_root(&dir).join("versions/1.9.0");
    std::fs::create_dir_all(&damaged).unwrap();
    std::fs::write(damaged.join("stale-resource"), b"stale").unwrap();
    install_with_config(config(&dir, &server, &signing))
        .await
        .unwrap();
    assert_eq!(std::fs::read(damaged.join("yserver.exe")).unwrap(), payload);
    assert!(!damaged.join("stale-resource").exists());
    assert_eq!(server.hits("/artifact"), 1);
}

#[tokio::test]
async fn zip_preserves_runtime_resources() {
    let dir = TestDir::new("zip");
    let signing = SigningKey::from_bytes(&[17; 32]);
    let payload = zip_bytes(&[
        ("bin/yserver.exe", b"executable"),
        ("xkb/keymap", b"keymap"),
        ("licenses/yserver.txt", b"license"),
    ]);
    let server = MockServer::new(&[]);
    let manifest_text = manifest_artifact(ArtifactSpec {
        url: &format!("{}/artifact.zip", server.url),
        payload: &payload,
        signing: &signing,
        format: "zip",
        executable_path: "bin/yserver.exe",
    });
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact.zip", 200, &payload),
    ]);
    install_with_config(config(&dir, &server, &signing))
        .await
        .unwrap();
    let installed = component_root(&dir).join("versions/1.9.0");
    assert_eq!(
        std::fs::read(installed.join("bin/yserver.exe")).unwrap(),
        b"executable"
    );
    assert_eq!(
        std::fs::read(installed.join("xkb/keymap")).unwrap(),
        b"keymap"
    );
    assert_eq!(
        std::fs::read(installed.join("licenses/yserver.txt")).unwrap(),
        b"license"
    );
}

#[tokio::test]
async fn zip_parent_traversal_is_rejected() {
    let dir = TestDir::new("zip-traversal");
    let signing = SigningKey::from_bytes(&[18; 32]);
    let payload = zip_bytes(&[("../escape", b"bad")]);
    let server = MockServer::new(&[]);
    let manifest_text = manifest_artifact(ArtifactSpec {
        url: &format!("{}/artifact.zip", server.url),
        payload: &payload,
        signing: &signing,
        format: "zip",
        executable_path: "yserver.exe",
    });
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact.zip", 200, &payload),
    ]);
    assert!(matches!(
        install_with_config(config(&dir, &server, &signing)).await,
        Err(X11ComponentError::Install(_))
    ));
    assert!(!component_root(&dir).join("escape").exists());
}

#[tokio::test]
async fn auto_respects_rollout_and_embedded_allows_dogfood() {
    let dir = TestDir::new("rollout");
    let signing = SigningKey::from_bytes(&[19; 32]);
    let payload = b"payload";
    let server = MockServer::new(&[]);
    let mut value: serde_json::Value = serde_json::from_str(&manifest(
        &format!("{}/artifact", server.url),
        payload,
        &signing,
    ))
    .unwrap();
    value["components"]["x11-server"]["rollout_percent"] = 0.into();
    let manifest_text = value.to_string();
    server.set_routes(&[
        ("/manifest.json", 200, manifest_text.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let config = config(&dir, &server, &signing);
    assert!(matches!(
        install_with_config_for_mode(config.clone(), X11ServerMode::Auto).await,
        Err(X11ComponentError::EmbeddedDisabled(_))
    ));
    assert_eq!(server.hits("/artifact"), 0);
    install_with_config_for_mode(config, X11ServerMode::Embedded)
        .await
        .unwrap();
    assert_eq!(server.hits("/artifact"), 1);
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
