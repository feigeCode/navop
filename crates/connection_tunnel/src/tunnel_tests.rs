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
fn pageant_auth_type_builds_pageant_authentication() {
    let tunnel = SshTunnelConfig {
        auth_type: "pageant".to_string(),
        ..Default::default()
    };

    let auth = build_auth(&tunnel).expect("Pageant 认证不需要额外凭据字段");

    assert!(matches!(auth, SshAuth::Pageant));
}
