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
fn manifest_parses_connection_importers() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.importer.navicat",
            "name": "Navicat Importer",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.7.0" },
            "runtime": {
                "wasm": [{
                    "id": "navicat-importer",
                    "module": "wasm/navicat_importer.wasm",
                    "kind": "component",
                    "timeout_ms": 5000,
                    "max_memory_mb": 64
                }]
            },
            "permissions": [
                "fs:read:~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"
            ],
            "contributes": {
                "connectionImporters": [{
                    "id": "navicat",
                    "runtimeId": "navicat-importer",
                    "displayName": "Navicat",
                    "description": "Import database connections from Navicat",
                    "icon": "database",
                    "outputKinds": ["database"],
                    "platforms": ["macos"],
                    "manualFilePick": {
                        "prompt": "选择 Navicat 导出的 connection.ncx 文件"
                    },
                    "candidateFiles": [{
                        "id": "navicat-macos-cc-conn",
                        "platform": "macos",
                        "path": "~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"
                    }]
                }]
            }
        }"#,
    );

    let manifest = load_from_dir(tmp.path()).unwrap();
    let importer = &manifest.contributes.connection_importers[0];

    assert_eq!("navicat", importer.id);
    assert_eq!("navicat-importer", importer.runtime_id);
    assert_eq!("Navicat", importer.display_name);
    assert_eq!(Some("database"), importer.icon.as_deref());
    assert_eq!(vec!["database"], importer.output_kinds);
    assert_eq!(vec!["macos"], importer.platforms);
    assert_eq!(
        Some("选择 Navicat 导出的 connection.ncx 文件"),
        importer.manual_file_pick.prompt.as_deref()
    );
    assert_eq!(1, importer.candidate_files.len());
    assert_eq!("navicat-macos-cc-conn", importer.candidate_files[0].id);
    assert_eq!("macos", importer.candidate_files[0].platform);
    assert_eq!(
        "~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist",
        importer.candidate_files[0].path
    );
}

#[test]
fn manifest_loads_remote_file_editor_contributions() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.editor.notepad-plus-plus",
            "name": "Notepad++ External Editor",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "contributes": {
                "remoteFileEditors": [{
                    "id": "notepad-plus-plus",
                    "displayName": "Notepad++",
                    "platforms": ["windows"],
                    "fileMasks": ["*"],
                    "priority": 100,
                    "command": {
                        "programCandidates": [
                            "${env:ProgramFiles}\\Notepad++\\notepad++.exe",
                            "${env:ProgramFiles(x86)}\\Notepad++\\notepad++.exe"
                        ],
                        "args": ["{file}"]
                    }
                }]
            }
        }"#,
    );

    let manifest = load_from_dir(tmp.path()).unwrap();
    let editor = &manifest.contributes.remote_file_editors[0];

    assert_eq!("notepad-plus-plus", editor.id);
    assert_eq!("Notepad++", editor.display_name);
    assert_eq!(vec!["windows"], editor.platforms);
    assert_eq!(vec!["*"], editor.file_masks);
    assert_eq!(100, editor.priority);
    assert_eq!(2, editor.command.program_candidates.len());
    assert_eq!(vec!["{file}"], editor.command.args);
}

#[test]
fn manifest_accepts_windows_env_fs_permissions_for_connection_importers() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.importer.dbeaver",
            "name": "DBeaver Importer",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.7.0" },
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

    let manifest = load_from_dir(tmp.path()).unwrap();

    assert_eq!("com.onetcli.importer.dbeaver", manifest.id);
    assert_eq!(1, manifest.contributes.connection_importers.len());
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
