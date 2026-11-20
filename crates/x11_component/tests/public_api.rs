use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use x11_component::{
    X11ForwardingStatus, X11ServerMode, configured_mode, current_status, installed_version,
    set_configured_mode, set_data_dir, verify_ed25519,
};

#[test]
fn mode_serializes_snake_case_and_defaults_auto() {
    assert_eq!(X11ServerMode::default(), X11ServerMode::Auto);
    assert_eq!(
        serde_json::to_string(&X11ServerMode::External).unwrap(),
        "\"external\""
    );
    assert_eq!(
        serde_json::from_str::<X11ServerMode>("\"embedded\"").unwrap(),
        X11ServerMode::Embedded
    );
}

#[test]
fn configured_mode_is_thread_safe() {
    set_configured_mode(X11ServerMode::Embedded);
    let threads = (0..4)
        .map(|_| std::thread::spawn(|| configured_mode()))
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), X11ServerMode::Embedded);
    }
    set_configured_mode(X11ServerMode::Auto);
    assert_eq!(configured_mode(), X11ServerMode::Auto);
}

#[test]
fn initial_status_is_disabled() {
    assert!(matches!(
        current_status(),
        X11ForwardingStatus::Disabled { .. }
    ));
}

#[test]
fn injected_data_dir_controls_installed_version() {
    let root = std::env::temp_dir().join(format!("x11-installed-{}", std::process::id()));
    let component = root.join("components/x11");
    std::fs::create_dir_all(component.join("versions/9.8.7")).unwrap();
    std::fs::write(component.join("current"), "9.8.7").unwrap();
    set_data_dir(root.clone());
    assert_eq!(installed_version().as_deref(), Some("9.8.7"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn valid_ed25519_signature_is_accepted() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let public = BASE64.encode(VerifyingKey::from(&signing).to_bytes());
    let signature = BASE64.encode(signing.sign(b"payload").to_bytes());
    verify_ed25519(b"payload", &signature, &public).unwrap();
}

#[test]
fn wrong_key_and_truncated_signature_are_rejected() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let wrong = SigningKey::from_bytes(&[8; 32]);
    let signature = signing.sign(b"payload").to_bytes();
    let wrong_public = BASE64.encode(VerifyingKey::from(&wrong).to_bytes());
    let signature_base64 = BASE64.encode(signature);
    assert!(
        verify_ed25519(b"payload", &signature_base64, &wrong_public).is_err(),
        "换公钥后必须验签失败"
    );
    let truncated = BASE64.encode(&signature[..63]);
    let public = BASE64.encode(VerifyingKey::from(&signing).to_bytes());
    assert!(verify_ed25519(b"payload", &truncated, &public).is_err());
}
