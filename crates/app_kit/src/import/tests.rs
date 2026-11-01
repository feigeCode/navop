//! `import` 模块的纯逻辑单测（不依赖 gpui App 上下文）。

use super::*;

#[test]
fn every_product_has_supported_kinds() {
    let kinds = supported_connection_types();
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|kind| {
        let canonical = ConnectionType::from_str(kind).to_string();
        &canonical == kind
    }));
}

#[test]
fn future_bundle_version_is_rejected() {
    let mut bundle = ConnectionBundle {
        version: BUNDLE_VERSION + 1,
        exported_at: 0,
        app_id: None,
        workspaces: vec![],
        connections: vec![],
    };
    let error = ensure_supported_version(&bundle).unwrap_err();
    assert!(error.to_string().contains("高于当前支持"), "{error}");
    bundle.version = BUNDLE_VERSION;
    assert!(ensure_supported_version(&bundle).is_ok());
}

#[test]
fn broken_params_json_is_rejected_before_writing() {
    let connection = BundleConnection {
        name: "坏包".to_string(),
        connection_type: "Database".to_string(),
        params: "{not json".to_string(),
        workspace: None,
        remark: None,
        selected_databases: None,
        sync_enabled: true,
    };
    let error = validate_params(&connection).unwrap_err();
    assert!(error.to_string().contains("params 不是合法 JSON"));
    let ok = BundleConnection {
        params: r#"{"host":"127.0.0.1"}"#.to_string(),
        ..connection
    };
    assert!(validate_params(&ok).is_ok());
}

#[test]
fn expands_tilde_quotes_and_scheme_in_source_path() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(
        expand_source_path("~/Desktop/one-hub.db"),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("  ~/Desktop/one-hub.db  "),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("\"~/Desktop/one-hub.db\""),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("file:///tmp/navop-connections.json"),
        PathBuf::from("/tmp/navop-connections.json")
    );
    assert_eq!(expand_source_path("~"), home);
}

#[test]
fn keeps_absolute_and_unknown_variable_paths() {
    let absolute = PathBuf::from("/tmp/navop-connections.json");
    assert_eq!(expand_source_path("/tmp/navop-connections.json"), absolute);
    // 未定义的变量原样保留，避免把 `%VAR%` 吞成空路径。
    assert_eq!(
        expand_source_path("%NAVOP_TESTS_UNSET_VAR%\\one-hub.db"),
        PathBuf::from("%NAVOP_TESTS_UNSET_VAR%\\one-hub.db")
    );
    // 只有前缀是 `~` 才算 home，`~user` 之类不动。
    assert_eq!(
        expand_source_path("~someone/db"),
        PathBuf::from("~someone/db")
    );
}
