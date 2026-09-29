use crate::{SshTunnelConfig, build_auth, resolve_tunnel_destination};
use ssh::SshAuth;

#[test]
fn tunnel_destination_uses_explicit_target() {
    let tunnel = SshTunnelConfig {
        enabled: true,
        target_host: Some("mongo.internal".to_string()),
        target_port: Some(27018),
        ..Default::default()
    };

    assert_eq!(
        ("mongo.internal".to_string(), 27018),
        resolve_tunnel_destination("localhost", 27017, Some(&tunnel))
    );
}

#[test]
fn tunnel_destination_falls_back_to_direct_target() {
    let tunnel = SshTunnelConfig {
        enabled: true,
        ..Default::default()
    };

    assert_eq!(
        ("db.local".to_string(), 27017),
        resolve_tunnel_destination("db.local", 27017, Some(&tunnel))
    );
}

#[test]
fn combined_auth_type_builds_a_password_then_key_chain() {
    let tunnel = SshTunnelConfig {
        auth_type: "password_and_private_key".to_string(),
        password: Some("secret".to_string()),
        private_key_path: Some("/keys/id_ed25519".to_string()),
        ..Default::default()
    };

    let auth = build_auth(&tunnel).expect("组合认证需要同时提供密码与私钥");

    match auth {
        SshAuth::Chain(steps) => {
            assert_eq!(2, steps.len(), "组合认证应包含密码与密钥两个因素");
            assert!(
                matches!(&steps[0], SshAuth::Password(password) if password == "secret"),
                "第一个因素应为密码"
            );
            assert!(
                matches!(&steps[1], SshAuth::PrivateKey { key_path, .. } if key_path == "/keys/id_ed25519"),
                "第二个因素应为私钥"
            );
        }
        _ => panic!("期望「密码 + 密钥」认证链"),
    }
}

#[test]
fn combined_auth_type_requires_a_key_factor() {
    let tunnel = SshTunnelConfig {
        auth_type: "password_and_private_key".to_string(),
        password: Some("secret".to_string()),
        ..Default::default()
    };

    assert!(
        build_auth(&tunnel).is_err(),
        "组合认证缺少密钥因素时应报错，而不是静默降级为单密码"
    );
}

#[test]
fn pageant_auth_type_builds_pageant_authentication() {
    let tunnel = SshTunnelConfig {
        auth_type: "pageant".to_string(),
        ..Default::default()
    };

    let auth = build_auth(&tunnel).expect("Pageant 认证不需要额外凭据字段");

    assert!(matches!(auth, SshAuth::Pageant));
}
