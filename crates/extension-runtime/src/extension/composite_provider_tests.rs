use std::fs;

use super::{CompositeExtensionProvider, ExtensionKind, ExtensionProvider};

#[test]
fn composite_provider_lists_compatible_extensions_and_skips_noise() {
    let tmp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("_staging")).unwrap();
    fs::create_dir_all(tmp.path().join("random-dir")).unwrap();
    write_manifest(tmp.path(), "foo", &valid_manifest("com.example.foo", "Foo"));
    write_manifest(
        tmp.path(),
        "bad",
        r#"{
            "schema_version": 1,
            "id": "com.example.bad",
            "name": "Bad",
            "version": "1.0.0",
            "engines": { "onetcli": ">=99.0.0" }
        }"#,
    );

    let list = CompositeExtensionProvider
        .list_installed(tmp.path())
        .expect("list composite extensions");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::Composite, list[0].kind);
    assert_eq!("com.example.foo", list[0].name);
    assert_eq!("1.0.0", list[0].version);
    assert_eq!("Test extension", list[0].description);
}

#[test]
fn composite_provider_lists_connection_importer_with_windows_env_permission() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        "dbeaver",
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.importer.dbeaver",
            "name": "DBeaver Importer",
            "version": "0.1.0",
            "description": "Import database connections from DBeaver",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "dbeaver-importer",
                    "module": "wasm/dbeaver_importer_wasm.wasm",
                    "kind": "component"
                }]
            },
            "permissions": [
                "fs:read:%APPDATA%/DBeaverData/workspace6/General/.dbeaver/data-sources.json"
            ],
            "contributes": {
                "connectionImporters": [{
                    "id": "dbeaver",
                    "runtimeId": "dbeaver-importer",
                    "displayName": "DBeaver",
                    "outputKinds": ["database"],
                    "platforms": ["windows"],
                    "candidateFiles": [{
                        "id": "dbeaver-windows-data-sources",
                        "platform": "windows",
                        "path": "%APPDATA%/DBeaverData/workspace6/General/.dbeaver/data-sources.json"
                    }]
                }]
            }
        }"#,
    );
    fs::create_dir_all(tmp.path().join("dbeaver/wasm")).unwrap();
    fs::write(
        tmp.path().join("dbeaver/wasm/dbeaver_importer_wasm.wasm"),
        [],
    )
    .unwrap();

    let list = CompositeExtensionProvider
        .list_installed(tmp.path())
        .expect("list composite extensions");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::Composite, list[0].kind);
    assert_eq!("com.onetcli.importer.dbeaver", list[0].name);
    assert_eq!(
        "Import database connections from DBeaver",
        list[0].description
    );
}

#[test]
fn composite_provider_install_from_dir_returns_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(tmp.path(), "foo", &valid_manifest("com.example.foo", "Foo"));

    let summary = CompositeExtensionProvider
        .install_from_dir(&tmp.path().join("foo"))
        .expect("install composite extension");

    assert_eq!(ExtensionKind::Composite, summary.kind);
    assert_eq!("com.example.foo", summary.name);
    assert_eq!("1.0.0", summary.version);
    assert_eq!("Test extension", summary.description);
    assert_eq!(tmp.path().join("foo"), summary.path);
}

#[test]
fn composite_provider_uninstall_removes_directory_and_returns_manifest_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(tmp.path(), "foo", &valid_manifest("com.example.foo", "Foo"));
    let dir = tmp.path().join("foo");

    let name = CompositeExtensionProvider
        .uninstall(&dir)
        .expect("uninstall composite extension");

    assert_eq!("com.example.foo", name);
    assert!(!dir.exists());
}

#[test]
fn composite_provider_uninstall_falls_back_to_directory_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("orphan");
    fs::create_dir_all(&dir).unwrap();

    let name = CompositeExtensionProvider
        .uninstall(&dir)
        .expect("uninstall orphan composite directory");

    assert_eq!("orphan", name);
    assert!(!dir.exists());
}

fn write_manifest(root: &std::path::Path, subdir: &str, content: &str) {
    let dir = root.join(subdir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("extension.json"), content).unwrap();
}

fn valid_manifest(id: &str, name: &str) -> String {
    format!(
        r#"{{
            "schema_version": 1,
            "id": "{id}",
            "name": "{name}",
            "version": "1.0.0",
            "description": "Test extension",
            "icon": "Package",
            "engines": {{ "onetcli": ">=0.1.0" }}
        }}"#
    )
}
