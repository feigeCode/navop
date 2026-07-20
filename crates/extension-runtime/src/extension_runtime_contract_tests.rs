use std::path::PathBuf;

use html_preview::resolve_extension_asset_url;

use crate::{
    ExtensionRuntimeCatalog,
    extension::manifest::{
        ApiVersions, CommandContrib, CommandHandlerContrib, ContributesManifest,
        DocumentExporterContrib, Engines, HtmlPreviewTransformContrib, Manifest, MenuCommandRef,
        MenuContrib, RuntimeSection, WasmRuntime, WasmRuntimeKind,
        contributes::{
            RemoteFileEditorCommandContrib, RemoteFileEditorContrib, RemoteFileEditorLaunchMode,
        },
    },
};

#[test]
fn runtime_catalog_registers_wasm_command_and_db_tree_menu() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .commands
        .push(command("main", "example.sync_table"));
    manifest.contributes.menus.insert(
        "db.tree.table".to_string(),
        vec![MenuContrib {
            command: MenuCommandRef {
                id: "example.sync_table".to_string(),
            },
            label: Some("同步表".to_string()),
            group: Some("extension@10".to_string()),
            when: Some("node.type == 'table'".to_string()),
            requires_active: true,
        }],
    );

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let menu_registry = catalog.db_tree_menu_registry();
    let table_items = menu_registry.items_for_node(db::DbNodeType::Table);

    assert!(
        catalog
            .component_permissions_for_command("example.sync_table")
            .is_ok()
    );
    assert_eq!(1, table_items.len());
    assert_eq!("com.example.tools", table_items[0].extension_id);
    assert_eq!("example.sync_table", table_items[0].command_id);
    assert_eq!("同步表", table_items[0].label);
}

#[test]
fn runtime_catalog_rejects_wasm_command_with_missing_runtime() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .commands
        .push(command("missing", "example.missing"));

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("unknown runtime_id"));
}

#[test]
fn runtime_catalog_exposes_component_permissions_for_command() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .commands
        .push(command("main", "example.search"));

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let permissions = catalog
        .component_permissions_for_command("example.search")
        .unwrap();

    assert_eq!(vec!["db:schema:*", "ui:dialog"], permissions);
}

#[test]
fn runtime_catalog_registers_wasm_html_preview_transform_with_assets() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .html_preview_transforms
        .push(HtmlPreviewTransformContrib {
            id: "example.decorate_html".to_string(),
            runtime_id: "main".to_string(),
            function: "transform-html".to_string(),
            languages: vec!["html".to_string(), "htm".to_string()],
            assets: "assets".to_string(),
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let transforms = catalog.html_preview_transforms_for_language("HTML");

    assert_eq!(1, transforms.len());
    assert_eq!("com.example.tools", transforms[0].extension_id);
    assert_eq!("example.decorate_html", transforms[0].id);
    assert_eq!("com.example.tools::main", transforms[0].runtime_id);
    assert_eq!("transform-html", transforms[0].function);
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/assets"),
        transforms[0].assets_root
    );
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/assets/app.css"),
        resolve_extension_asset_url("onet-extension://com.example.tools/app.css").unwrap()
    );
}

#[test]
fn runtime_catalog_resolves_document_exporter_by_format() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("exporter"));
    manifest
        .contributes
        .document_exporters
        .push(DocumentExporterContrib {
            id: "notes-documents".to_string(),
            display_name: "HTML, PDF and Word".to_string(),
            runtime_id: "exporter".to_string(),
            function: "export-document".to_string(),
            formats: vec!["html".to_string(), "pdf".to_string(), "docx".to_string()],
            output_media_types: vec!["text/html".to_string(), "application/pdf".to_string()],
            priority: 100,
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let exporter = catalog.document_exporter_for_format("PDF").unwrap();

    assert_eq!("notes-documents", exporter.id);
    assert_eq!("com.example.tools::exporter", exporter.runtime_id);
    assert_eq!("export-document", exporter.function);
    assert!(catalog.document_exporter_for_format("odt").is_none());
}

#[test]
fn runtime_catalog_registers_remote_file_editors() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .remote_file_editors
        .push(RemoteFileEditorContrib {
            id: "notepad-plus-plus".to_string(),
            display_name: "Notepad++".to_string(),
            platforms: vec!["windows".to_string()],
            file_masks: vec!["*".to_string()],
            priority: 100,
            command: RemoteFileEditorCommandContrib {
                launch_mode: RemoteFileEditorLaunchMode::MacosOpen,
                program_candidates: vec!["notepad++.exe".to_string()],
                args: vec!["{file}".to_string()],
            },
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let editors = catalog.remote_file_editors();

    assert_eq!(1, editors.len());
    assert_eq!("com.example.tools", editors[0].extension_id);
    assert_eq!("notepad-plus-plus", editors[0].id);
    assert_eq!(
        "com.example.tools::notepad-plus-plus",
        editors[0].editor_key
    );
    assert_eq!("Notepad++", editors[0].display_name);
    assert_eq!(vec!["windows"], editors[0].platforms);
    assert_eq!(vec!["*"], editors[0].file_masks);
    assert_eq!(100, editors[0].priority);
    assert_eq!(
        RemoteFileEditorLaunchMode::MacosOpen,
        editors[0].command.launch_mode
    );
    assert_eq!(vec!["notepad++.exe"], editors[0].command.program_candidates);
    assert_eq!(vec!["{file}"], editors[0].command.args);
}

#[test]
fn runtime_catalog_rejects_remote_editor_without_program_candidates() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .remote_file_editors
        .push(RemoteFileEditorContrib {
            id: "broken".to_string(),
            display_name: "Broken Editor".to_string(),
            platforms: Vec::new(),
            file_masks: Vec::new(),
            priority: 0,
            command: RemoteFileEditorCommandContrib::default(),
        });

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("programCandidates"));
}

#[test]
fn runtime_catalog_loads_compatible_extensions_from_composite_root() {
    let root = tempfile::TempDir::new().unwrap();
    write_composite_manifest(
        root.path(),
        "com.example.echo",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "commands": [{
                    "id": "example.echo",
                    "title": "Echo",
                    "handler": {
                        "kind": "wasm",
                        "runtime_id": "main"
                    }
                }]
            }
        }"#,
    );
    write_composite_manifest(
        root.path(),
        "com.example.echo.backup-0.0.9",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo Backup",
            "version": "0.0.9",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            }
        }"#,
    );
    write_composite_manifest(
        root.path(),
        ".com.example.echo.install-backup-1-0",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo Transaction Backup",
            "version": "0.0.8",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            }
        }"#,
    );
    std::fs::create_dir_all(root.path().join("_staging")).unwrap();
    std::fs::create_dir_all(root.path().join("noise")).unwrap();

    let report =
        ExtensionRuntimeCatalog::from_installed_composite_root_with_report(root.path()).unwrap();
    let catalog = report.catalog;

    assert!(
        catalog
            .component_permissions_for_command("example.echo")
            .is_ok()
    );
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id, "com.example.echo");
    assert_eq!(
        report.loaded[0].wasm_runtimes,
        vec!["com.example.echo::main".to_string()]
    );
}

#[test]
fn runtime_catalog_rebuild_after_uninstall_drops_db_tree_menu() {
    let root = tempfile::TempDir::new().unwrap();
    write_composite_manifest(
        root.path(),
        "com.example.cleanup",
        r#"{
            "schema_version": 1,
            "id": "com.example.cleanup",
            "name": "Cleanup",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "commands": [{
                    "id": "cleanup.run",
                    "title": "Cleanup",
                    "handler": {
                        "kind": "wasm",
                        "runtime_id": "main"
                    }
                }],
                "menus": {
                    "db.tree.table": [{
                        "command": "cleanup.run",
                        "label": "Cleanup",
                        "group": "extension@10"
                    }]
                }
            }
        }"#,
    );

    let before = ExtensionRuntimeCatalog::from_installed_composite_root(root.path()).unwrap();
    assert_eq!(
        1,
        before
            .db_tree_menu_registry()
            .items_for_node(db::DbNodeType::Table)
            .len()
    );

    std::fs::remove_dir_all(root.path().join("com.example.cleanup")).unwrap();
    let after = ExtensionRuntimeCatalog::from_installed_composite_root(root.path()).unwrap();

    assert!(
        after
            .db_tree_menu_registry()
            .items_for_node(db::DbNodeType::Table)
            .is_empty()
    );
}

fn wasm_runtime(id: &str) -> WasmRuntime {
    WasmRuntime {
        id: id.to_string(),
        module: "./wasm/plugin.wasm".to_string(),
        kind: WasmRuntimeKind::Component,
        timeout_ms: 5_000,
        max_memory_mb: 64,
        fuel_per_call: 100_000_000,
    }
}

fn command(runtime_id: &str, command_id: &str) -> CommandContrib {
    CommandContrib {
        id: command_id.to_string(),
        title: "Sync Table".to_string(),
        category: String::new(),
        icon: None,
        enablement_when: Some("node.type == 'table'".to_string()),
        handler: CommandHandlerContrib {
            kind: "wasm".to_string(),
            runtime_id: runtime_id.to_string(),
            function: Some("run".to_string()),
        },
    }
}

fn base_manifest() -> Manifest {
    Manifest {
        schema_version: 1,
        id: "com.example.tools".to_string(),
        name: "Example Tools".to_string(),
        version: "0.1.0".to_string(),
        publisher: String::new(),
        license: String::new(),
        homepage: String::new(),
        repository: String::new(),
        icon: String::new(),
        description_i18n: String::new(),
        description: String::new(),
        categories: vec![],
        keywords: vec![],
        engines: Engines {
            onetcli: ">=0.1.0".to_string(),
        },
        api: ApiVersions::default(),
        activation: vec![],
        permissions: vec!["db:schema:*".to_string(), "ui:dialog".to_string()],
        runtime: RuntimeSection::default(),
        contributes: ContributesManifest::default(),
        manifest_dir: PathBuf::from("/tmp/com.example.tools"),
    }
}

fn write_composite_manifest(root: &std::path::Path, id: &str, content: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.json"), content).unwrap();
}
