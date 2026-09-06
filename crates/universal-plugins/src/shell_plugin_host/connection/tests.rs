use std::{collections::BTreeMap, path::PathBuf};

use one_core::storage::{ExtensionConnectionParams, StoredConnection};

use super::*;

#[test]
fn launch_materializes_secret_refs_without_secret_values() {
    let params = ExtensionConnectionParams::new(
        "com.example.search",
        "search",
        serde_json::Map::from_iter([("url".into(), "https://example.test".into())]),
        BTreeMap::from([("api_key".into(), "secret-value".into())]),
    )
    .unwrap();
    let mut connection = StoredConnection::new_extension("Search".into(), params, None);
    connection.id = Some(42);
    let launch = ShellConnectionLaunch::new(&connection, &contribution(), &view()).unwrap();

    assert_eq!(
        Some("secret://self/42:api_key"),
        launch.resource.config["credential_refs"]["api_key"].as_str()
    );
    assert!(!launch.resource.config.to_string().contains("secret-value"));
    assert_eq!(42, launch.connection_id());
}

#[test]
fn connection_context_and_close_order_stay_host_owned() {
    let context = include_str!("../context.rs");
    let tab = include_str!("../../shell_plugin_tab.rs");

    assert!(context.contains(".field(\"resource\", connection.resource)"));
    assert!(!context.contains("credentialRefs"));
    assert!(!context.contains("connection.config"));
    let close = tab
        .split("fn close_task(")
        .nth(1)
        .expect("shell close task exists");
    let session = close.find("session.close_all().await").unwrap();
    let release = close.find("deactivate_activation").unwrap();
    assert!(session < release);
}

fn contribution() -> extension_runtime::RegisteredResourceConnectionContribution {
    extension_runtime::RegisteredResourceConnectionContribution {
        extension_id: "com.example.search".into(),
        extension_root: PathBuf::from("/tmp/com.example.search"),
        id: "search".into(),
        label: "Search".into(),
        description: None,
        icon_path: None,
        runtime_id: "com.example.search::main".into(),
        resource_type: "search".into(),
        shell_view_id: Some("explorer".into()),
        form: Default::default(),
    }
}

fn view() -> extension_runtime::RegisteredShellViewContribution {
    extension_runtime::RegisteredShellViewContribution {
        extension_id: "com.example.search".into(),
        extension_version: "1.0.0".into(),
        id: "explorer".into(),
        view_key: "com.example.search::explorer".into(),
        title: "Search".into(),
        description: None,
        icon_path: None,
        extension_root: PathBuf::from("/tmp/com.example.search"),
        entry_path: PathBuf::from("/tmp/com.example.search/ui/explorer.js"),
        surface: extension_runtime::extension::manifest::ShellSurface::Tab,
        singleton: false,
        backends: BTreeMap::from([("search".into(), "com.example.search::main".into())]),
        modules: Default::default(),
        permissions: Vec::new(),
        shell_api_version: "1.0".into(),
        required_gpui_shell_version: "0.2.0".into(),
    }
}
