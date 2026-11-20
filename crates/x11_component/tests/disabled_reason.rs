mod common;

use common::{MockServer, TestDir, manifest, public_key};
use ed25519_dalek::SigningKey;
use x11_component::{
    InstallConfig, X11ComponentError, X11ForwardingStatus, X11ServerMode, install_with_config,
    resolve_server, set_data_dir, set_navop_version,
};

#[tokio::test]
async fn signature_disable_reason_survives_install_and_resolve() {
    let dir = TestDir::new("disabled-reason");
    let signing = SigningKey::from_bytes(&[31; 32]);
    let wrong = SigningKey::from_bytes(&[32; 32]);
    let payload = b"signed-yserver";
    let server = MockServer::new(&[]);
    let wrong_manifest = manifest(&format!("{}/artifact", server.url), payload, &wrong);
    server.set_routes(&[
        ("/manifest.json", 200, wrong_manifest.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    let config = InstallConfig {
        data_dir: dir.path().to_path_buf(),
        manifest_url: format!("{}/manifest.json", server.url),
        public_key_base64: public_key(&signing),
        navop_version: semver::Version::new(0, 23, 4),
        target: "windows-x86_64".into(),
        direct_http: true,
    };
    set_data_dir(dir.path().to_path_buf());
    set_navop_version(semver::Version::new(0, 23, 4));
    let error = install_with_config(config.clone()).await.unwrap_err();
    assert!(
        matches!(&error, X11ComponentError::SignatureInvalid(_)),
        "应返回验签错误，实际：{error:?}"
    );
    let root = dir.path().join("components/x11");
    assert!(!root.join("current").exists());
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-")
    }));
    assert_signature_disabled(x11_component::current_status());
    assert_platform_aware_disabled(resolve_server(X11ServerMode::Embedded).await.status);

    let correct_manifest = manifest(&format!("{}/artifact", server.url), payload, &signing);
    server.set_routes(&[
        ("/manifest.json", 200, correct_manifest.as_bytes()),
        ("/artifact", 200, payload),
    ]);
    install_with_config(config).await.unwrap();
    assert_platform_aware_disabled(resolve_server(X11ServerMode::Embedded).await.status);
}

fn assert_platform_aware_disabled(status: X11ForwardingStatus) {
    #[cfg(not(target_os = "windows"))]
    assert!(
        matches!(status, X11ForwardingStatus::Disabled { reason } if reason.contains("不支持内置 X11"))
    );
    #[cfg(target_os = "windows")]
    assert_signature_disabled(status);
}

fn assert_signature_disabled(status: X11ForwardingStatus) {
    let X11ForwardingStatus::Disabled { reason } = status else {
        panic!("永久禁用后不得恢复内置服务");
    };
    assert!(reason.contains("验签"), "应保留验签原因：{reason}");
    assert!(!reason.contains("握手"), "不得误报为握手回退：{reason}");
}
