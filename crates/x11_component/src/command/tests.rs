use std::path::PathBuf;

use x11_forwarding::ServerEndpoint;

use super::{CommandSpec, EndpointPlan, random_loopback_endpoint};

#[test]
fn random_endpoint_is_loopback_with_matching_display() {
    let plan = EndpointPlan::random_tcp().unwrap();
    let endpoint = plan.endpoint();
    let ServerEndpoint::Inet { host, port } = &endpoint else {
        panic!("应为 TCP")
    };
    assert_eq!(host, "127.0.0.1");
    let display = plan.display_number().unwrap();
    assert!((100..=59_535).contains(&display));
    assert_eq!(*port, 6000 + display);
    let same = EndpointPlan::persisted(endpoint.clone());
    assert_eq!(
        same.endpoint(),
        ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: *port,
        }
    );
}

#[test]
fn installed_tcp_listens_only_on_loopback() {
    let endpoint = ServerEndpoint::Inet {
        host: "127.0.0.1".into(),
        port: 6010,
    };
    let spec =
        CommandSpec::installed_tcp(PathBuf::from("yserver"), &endpoint, PathBuf::from("auth"));
    spec.validate_secure_endpoint().unwrap();
    assert_eq!(spec.display_argument(), ":10");
    let mut expected = vec![":10", "-auth", "auth"];
    #[cfg(target_os = "macos")]
    expected.push("--rootless");
    expected.extend(["--listen", "127.0.0.1"]);
    assert_eq!(spec.arguments(), expected);
}

#[test]
fn installed_tcp_display_covers_full_valid_range() {
    for (port, display) in [(6000, ":0"), (6010, ":10"), (u16::MAX, ":59535")] {
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port,
        };
        let spec =
            CommandSpec::installed_tcp(PathBuf::from("yserver"), &endpoint, PathBuf::from("auth"));
        spec.validate_secure_endpoint().unwrap();
        assert_eq!(spec.display_argument(), display);
    }
}

#[test]
fn installed_tcp_rejects_non_loopback_endpoint() {
    let endpoint = ServerEndpoint::Inet {
        host: "0.0.0.0".into(),
        port: 6010,
    };
    let spec =
        CommandSpec::installed_tcp(PathBuf::from("yserver"), &endpoint, PathBuf::from("auth"));
    assert!(spec.validate_secure_endpoint().is_err());
}

#[test]
fn installed_tcp_rejects_port_without_display() {
    let endpoint = ServerEndpoint::Inet {
        host: "127.0.0.1".into(),
        port: 5999,
    };
    let spec =
        CommandSpec::installed_tcp(PathBuf::from("yserver"), &endpoint, PathBuf::from("auth"));
    assert!(spec.validate_secure_endpoint().is_err());
}

#[test]
fn manual_upstream_tcp_all_interfaces_is_rejected() {
    let endpoint = ServerEndpoint::Inet {
        host: "127.0.0.1".into(),
        port: 6010,
    };
    let spec = CommandSpec::manual_upstream_tcp(
        PathBuf::from("yserver"),
        &endpoint,
        PathBuf::from("auth"),
    );
    let error = spec.validate_secure_endpoint().unwrap_err().to_string();
    assert!(error.contains("手动"));
    assert!(error.contains("不支持 TCP"));
    let mut expected = vec![":10", "-auth", "auth"];
    #[cfg(target_os = "macos")]
    expected.push("--rootless");
    expected.extend(["-listen", "tcp"]);
    assert_eq!(spec.arguments(), expected);
}

#[test]
fn unix_socket_manual_launch_arguments() {
    let endpoint = ServerEndpoint::Unix("/tmp/.X11-unix/X17".into());
    let spec = CommandSpec::unix(PathBuf::from("yserver"), &endpoint, PathBuf::from("auth"));
    spec.validate_secure_endpoint().unwrap();
    assert_eq!(spec.display_argument(), ":17");
    let mut expected = vec![":17", "-auth", "auth"];
    #[cfg(target_os = "macos")]
    expected.push("--rootless");
    expected.extend(["-nolisten", "tcp"]);
    assert_eq!(spec.arguments(), expected);
}

#[test]
fn helper_rejects_non_loopback_tcp() {
    let endpoint = ServerEndpoint::Inet {
        host: "0.0.0.0".into(),
        port: 6000,
    };
    assert!(random_loopback_endpoint(&endpoint).is_none());
}
