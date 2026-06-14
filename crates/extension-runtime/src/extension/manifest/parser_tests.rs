use std::fs;

use super::{ManifestError, load_from_dir};

fn write_manifest(dir: &std::path::Path, body: &str) {
    fs::write(dir.join("extension.json"), body).unwrap();
}

#[test]
fn manifest_loads_composite_contributions() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.example.analytics",
            "name": "Analytics Suite",
            "version": "1.2.3",
            "description": "SQL analytics helpers",
            "engines": { "onetcli": ">=0.4.0" },
            "runtime": {
                "wasm": [{
                    "id": "ui",
                    "module": "./wasm/analytics.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "languages": [{
                    "id": "analytics.sql",
                    "name": "Analytics SQL",
                    "path": "./languages/sql",
                    "file_extensions": ["asql"]
                }],
                "menus": {
                    "db.tree.table": [{
                        "command": "analytics.inspect_table",
                        "label": "Inspect table"
                    }]
                }
            }
        }"#,
    );

    let manifest = load_from_dir(tmp.path()).unwrap();

    assert_eq!("com.example.analytics", manifest.id);
    assert_eq!(tmp.path(), manifest.manifest_dir);
    assert_eq!("1.0", manifest.api.extension);
    assert_eq!(1, manifest.runtime.wasm.len());
    assert_eq!("ui", manifest.runtime.wasm[0].id);
    assert_eq!(1, manifest.contributes.languages.len());
    assert_eq!("analytics.sql", manifest.contributes.languages[0].id);
    let menu = &manifest.contributes.menus["db.tree.table"][0];
    assert_eq!("analytics.inspect_table", menu.command.id);
    assert!(menu.requires_active);
}

#[test]
fn manifest_rejects_wasm_module_path_escape() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.example.bad",
            "name": "Bad",
            "version": "1.0.0",
            "engines": { "onetcli": ">=0.4.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "../escape.wasm",
                    "kind": "component"
                }]
            }
        }"#,
    );

    let err = load_from_dir(tmp.path()).unwrap_err();

    match err {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/wasm/main/module", field);
            assert!(reason.contains("逃逸"));
        }
        other => panic!("expected invalid wasm path, got {other:?}"),
    }
}
