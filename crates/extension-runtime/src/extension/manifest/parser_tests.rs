use std::fs;

use super::{ManifestError, RemoteFileEditorLaunchMode, load_from_dir};

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
    assert!(!importer.manual_file_pick.supports_directories);
    assert!(importer.manual_file_pick.directory_prompt.is_none());
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
    assert_eq!(
        RemoteFileEditorLaunchMode::Direct,
        editor.command.launch_mode
    );
    assert_eq!(2, editor.command.program_candidates.len());
    assert_eq!(vec!["{file}"], editor.command.args);
}

#[test]
fn manifest_loads_macos_open_remote_file_editor_launch_mode() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.editor.notepad-minus-minus",
            "name": "Notepad-- External Editor",
            "version": "0.1.1",
            "engines": { "onetcli": ">=0.8.6" },
            "contributes": {
                "remoteFileEditors": [{
                    "id": "notepad-minus-minus",
                    "displayName": "Notepad--",
                    "platforms": ["macos"],
                    "command": {
                        "launchMode": "macos_open",
                        "programCandidates": [
                            "/Applications/Notepad--.app/Contents/MacOS/Notepad--"
                        ],
                        "args": ["{file}"]
                    }
                }]
            }
        }"#,
    );

    let manifest = load_from_dir(tmp.path()).unwrap();
    let command = &manifest.contributes.remote_file_editors[0].command;

    assert_eq!(RemoteFileEditorLaunchMode::MacosOpen, command.launch_mode);
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

#[test]
fn manifest_parses_document_exporters() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_manifest(
        tmp.path(),
        r#"{
            "schema_version": 1,
            "id": "com.navop.exporter.documents",
            "name": "Document Exporter",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.7.0" },
            "runtime": { "wasm": [{ "id": "main", "module": "wasm/main.wasm", "kind": "component" }] },
            "contributes": {
                "documentExporters": [{
                    "id": "documents",
                    "displayName": "HTML, PDF and Word",
                    "runtimeId": "main",
                    "formats": ["html", "pdf", "docx"],
                    "outputMediaTypes": ["text/html", "application/pdf", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"]
                }]
            }
        }"#,
    );

    let manifest = load_from_dir(tmp.path()).unwrap();
    let exporter = &manifest.contributes.document_exporters[0];
    assert_eq!("documents", exporter.id);
    assert_eq!("export-document", exporter.function);
    assert_eq!(vec!["html", "pdf", "docx"], exporter.formats);
}

#[test]
fn manifest_parses_typed_shell_view() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_entry(tmp.path(), "ui/explorer.js");
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/explorer.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["context", "resource"]
        }),
    );

    let manifest = load_from_dir(tmp.path()).unwrap();
    let view = &manifest.contributes.shell_views[0];

    assert_eq!("explorer", view.id);
    assert_eq!("ui/explorer.js", view.entry);
    assert_eq!(
        Some("provider"),
        view.backends.get("main").map(String::as_str)
    );
}

#[test]
fn manifest_rejects_shell_view_entry_escape() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "../escape.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["resource"]
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/contributes/shellViews/explorer/entry", field);
            assert!(reason.contains("escape"), "{reason}");
        }
        other => panic!("expected invalid shell entry, got {other:?}"),
    }
}

#[test]
fn manifest_rejects_missing_shell_view_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/missing.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["resource"]
        }),
    );

    let error = load_from_dir(tmp.path()).unwrap_err();

    assert!(error.to_string().contains("does not exist"), "{error}");
}

#[test]
fn manifest_rejects_unknown_shell_view_field() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_entry(tmp.path(), "ui/explorer.js");
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/explorer.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["resource"],
            "unexpected": true
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::Parse { message, .. } => {
            assert!(message.contains("unknown field"), "{message}")
        }
        other => panic!("expected shell view parse failure, got {other:?}"),
    }
}

#[test]
fn manifest_rejects_removed_ui_host_module() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_entry(tmp.path(), "ui/explorer.js");
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/explorer.js",
            "surface": "tab",
            "modules": ["ui"]
        }),
    );

    let error = load_from_dir(tmp.path()).unwrap_err();
    assert!(
        error.to_string().contains("unknown variant `ui`"),
        "{error}"
    );
}

#[test]
fn manifest_accepts_registered_shell_lifecycle_modules() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_shell_entry(tmp.path(), "ui/explorer.js");
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/explorer.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["job", "event", "blob", "runtime", "log"]
        }),
    );

    let manifest = load_from_dir(tmp.path()).expect("registered shell modules");
    assert_eq!(5, manifest.contributes.shell_views[0].modules.len());
}

#[cfg(unix)]
#[test]
fn manifest_rejects_shell_entry_symlink_escape() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::TempDir::new().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    std::fs::write(outside.path().join("outside.js"), "export default class {}").unwrap();
    std::fs::create_dir_all(tmp.path().join("ui")).unwrap();
    symlink(
        outside.path().join("outside.js"),
        tmp.path().join("ui/explorer.js"),
    )
    .unwrap();
    write_shell_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "explorer",
            "title": "Resources",
            "entry": "ui/explorer.js",
            "surface": "tab",
            "backends": { "main": "provider" },
            "modules": ["context", "resource"]
        }),
    );

    let error = load_from_dir(tmp.path()).unwrap_err();

    assert!(error.to_string().contains("符号链接"), "{error}");
}

#[cfg(unix)]
#[test]
fn manifest_rejects_auto_restart_without_restart_budget() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_ipc_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "main",
            "entry": { "command": "bin/provider" },
            "auto_restart": true,
            "max_restart_attempts": 0
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/max_restart_attempts", field);
            assert!(reason.contains("1"), "{reason}");
        }
        other => panic!("expected invalid restart policy, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn manifest_rejects_unsupported_ipc_transport() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_ipc_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "main",
            "entry": { "command": "bin/provider" },
            "transport": { "kind": "stdio" }
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/transport/kind", field);
            assert!(reason.contains("local_socket"), "{reason}");
        }
        other => panic!("expected invalid IPC transport, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn manifest_rejects_ipc_working_directory_escape() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_ipc_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "main",
            "entry": {
                "command": "bin/provider",
                "working_dir": "../outside"
            }
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/entry/working_dir", field);
            assert!(reason.contains("逃逸"), "{reason}");
        }
        other => panic!("expected invalid IPC working directory, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn manifest_rejects_absolute_ipc_working_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_ipc_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "main",
            "entry": {
                "command": "bin/provider",
                "working_dir": "/tmp/runtime"
            }
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/entry/working_dir", field);
            assert!(reason.contains("绝对路径"), "{reason}");
        }
        other => panic!("expected invalid IPC working directory, got {other:?}"),
    }
}

#[test]
fn manifest_requires_spawn_permission_for_resolved_ipc_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "id": "com.example.resources",
        "name": "Resources",
        "version": "0.1.0",
        "engines": { "onetcli": ">=0.1.0" },
        "permissions": ["spawn:./bin/provider"],
        "runtime": {
            "ipc": [{
                "id": "main",
                "entry": {
                    "command": "./bin/provider",
                    "working_dir": "runtime"
                }
            }]
        }
    });
    write_manifest(
        tmp.path(),
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/entry/command", field);
            assert!(reason.contains("spawn:./runtime/bin/provider"), "{reason}");
        }
        other => panic!("expected missing spawn permission, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn manifest_rejects_ipc_command_that_relies_on_path_lookup() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_ipc_manifest(
        tmp.path(),
        serde_json::json!({
            "id": "main",
            "entry": { "command": "provider" }
        }),
    );

    match load_from_dir(tmp.path()).unwrap_err() {
        ManifestError::InvalidField { field, reason } => {
            assert_eq!("/runtime/ipc/main/entry/command", field);
            assert!(reason.contains("PATH"), "{reason}");
        }
        other => panic!("expected PATH lookup rejection, got {other:?}"),
    }
}

fn write_shell_manifest(dir: &std::path::Path, view: serde_json::Value) {
    let manifest = serde_json::json!({
        "schema_version": 1,
        "id": "com.example.resources",
        "name": "Resources",
        "version": "0.1.0",
        "engines": { "onetcli": ">=0.1.0", "gpui_shell": "0.2.0" },
        "api": { "shell": "1.0" },
        "permissions": ["shell:exec", "spawn:./bin/provider"],
        "runtime": {
            "ipc": [{
                "id": "provider",
                "entry": { "command": "./bin/provider" }
            }]
        },
        "contributes": { "shellViews": [view] }
    });
    write_manifest(dir, &serde_json::to_string_pretty(&manifest).unwrap());
}

fn write_shell_entry(dir: &std::path::Path, entry: &str) {
    let path = dir.join(entry);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "export default class View {}").unwrap();
}

#[cfg(unix)]
fn write_ipc_manifest(dir: &std::path::Path, runtime: serde_json::Value) {
    let manifest = serde_json::json!({
        "schema_version": 1,
        "id": "com.example.resources",
        "name": "Resources",
        "version": "0.1.0",
        "engines": { "onetcli": ">=0.1.0" },
        "permissions": ["spawn:./bin/provider"],
        "runtime": { "ipc": [runtime] }
    });
    write_manifest(dir, &serde_json::to_string_pretty(&manifest).unwrap());
}

#[test]
fn reference_resource_plugin_manifests_are_parser_valid() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let examples = [
        ("nacos", "com.navop.nacos"),
        ("elasticsearch", "com.navop.elasticsearch"),
        ("rocketmq", "com.navop.rocketmq"),
        ("kafka", "com.navop.kafka"),
        ("docker", "com.navop.docker"),
        ("kubernetes", "com.navop.kubernetes"),
        ("api-test", "com.navop.api-test"),
    ];

    for (name, expected_id) in examples {
        let directory = repo_root
            .join("docs/extension-resource-plugins/examples")
            .join(name);
        let manifest = load_from_dir(&directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));

        assert_eq!(expected_id, manifest.id);
        assert_eq!(1, manifest.runtime.ipc.len());
        assert_eq!("main", manifest.runtime.ipc[0].id);
        assert!(!manifest.contributes.connections.is_empty());
    }
}
