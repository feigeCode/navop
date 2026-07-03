use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Result;
use async_trait::async_trait;
use connection_import_protocol::{CandidateFile, HostAccessError, ImportRecordKind, Platform};
use db::GlobalDbState;
use db_view::{
    extension_menu::DbTreeExtensionActionContext,
    extension_widget::{
        build_extension_widget_model, default_form_values, form_values_to_action_event,
    },
};
use extension_component::{
    DbSessionResource, ExtensionConnectionImportHost, ExtensionDbHost, protocol,
};
use extension_wasm::{ComponentHostState, bindings::onet::extension::ui as WitUi};
use one_core::storage::DatabaseType;

use crate::{
    ExtensionRuntimeCatalog,
    connection_import_provider::{
        ManifestConnectionImportHost, preview_manifest_connection_importers,
    },
};

#[test]
fn connection_import_provider_lists_manifest_importers_with_scoped_ids() {
    let tmp = tempfile::TempDir::new().unwrap();
    let extension_dir = tmp.path().join("navicat");
    std::fs::create_dir_all(&extension_dir).unwrap();
    std::fs::write(
        extension_dir.join("extension.json"),
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
                    "kind": "component"
                }]
            },
            "contributes": {
                "connectionImporters": [{
                    "id": "navicat",
                    "runtimeId": "navicat-importer",
                    "displayName": "Navicat",
                    "outputKinds": ["database"],
                    "platforms": ["macos"],
                    "candidateFiles": [{
                        "id": "navicat-conn",
                        "platform": "macos",
                        "path": "~/Library/Navicat/conn.plist"
                    }]
                }]
            }
        }"#,
    )
    .unwrap();

    let importers =
        crate::connection_import_provider::list_manifest_connection_importers(tmp.path()).unwrap();

    assert_eq!(1, importers.len());
    assert_eq!(
        "com.onetcli.importer.navicat/navicat",
        importers[0].descriptor.id
    );
    assert_eq!("Navicat", importers[0].descriptor.display_name);
    assert_eq!("navicat-importer", importers[0].runtime_id);
    assert_eq!(extension_dir, importers[0].extension_dir);
}

#[test]
fn connection_import_provider_previews_dbeaver_and_termius_wasm_fixtures() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "dbeaver",
            extension_id: "com.onetcli.importer.dbeaver",
            importer_id: "dbeaver",
            runtime_id: "dbeaver-importer",
            display_name: "DBeaver",
            output_kind: "database",
            component_name: "dbeaver.component.wasm",
            core_wat: dbeaver_importer_core_wat(),
        },
    );
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "termius",
            extension_id: "com.onetcli.importer.termius",
            importer_id: "termius",
            runtime_id: "termius-importer",
            display_name: "Termius",
            output_kind: "ssh",
            component_name: "termius.component.wasm",
            core_wat: termius_importer_core_wat(),
        },
    );

    let records = futures::executor::block_on(preview_manifest_connection_importers(
        tmp.path(),
        &[
            "com.onetcli.importer.dbeaver/dbeaver".to_string(),
            "com.onetcli.importer.termius/termius".to_string(),
        ],
        true,
    ))
    .unwrap();

    assert_eq!(2, records.len());
    assert!(records.iter().any(|record| {
        record.kind == ImportRecordKind::Database && record.display_name == "prod-mysql"
    }));
    assert!(
        records
            .iter()
            .any(|record| record.kind == ImportRecordKind::Ssh && record.display_name == "prod-ssh")
    );
}

#[test]
fn manifest_connection_import_host_requires_manifest_fs_read_permission() {
    let tmp = tempfile::TempDir::new().unwrap();
    let candidate_path = tmp.path().join("connections.json");
    fs::write(&candidate_path, "{}").unwrap();
    let host = ManifestConnectionImportHost::new(
        vec![CandidateFile {
            id: "connections".to_string(),
            platform: Some(Platform::Macos),
            path: candidate_path.to_string_lossy().to_string(),
        }],
        Vec::<String>::new(),
    );

    let error = host
        .read_file("connections")
        .expect_err("manifest permission must gate file reads");

    assert_eq!(
        HostAccessError::PermissionDenied(candidate_path.to_string_lossy().to_string()),
        error
    );
}

#[test]
fn runtime_catalog_loads_wasm_coverage_fixture_manifest() {
    let fixture_dir = coverage_fixture_dir();
    let manifest = crate::extension::manifest::load_from_dir(&fixture_dir).unwrap();

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let registry = catalog.db_tree_menu_registry();
    let table_items = registry.items_for_node(db::DbNodeType::Table);

    assert!(
        catalog
            .component_permissions_for_command("coverage.run")
            .is_ok()
    );
    assert_eq!(1, table_items.len());
    assert_eq!("coverage.run", table_items[0].command_id);
}

#[test]
fn db_tree_action_runs_registered_wasm_component() {
    let fixture_dir = coverage_fixture_dir();
    let manifest = crate::extension::manifest::load_from_dir(&fixture_dir).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();

    let views = futures::executor::block_on(catalog.run_db_tree_component_action(
        DbTreeExtensionActionContext {
            extension_id: "com.onetcli.coverage-wasm".to_string(),
            command_id: "coverage.run".to_string(),
            node_id: "table-1".to_string(),
            node_name: "users".to_string(),
            node_type: db::DbNodeType::Table,
            database_type: DatabaseType::PostgreSQL,
            connection_id: "conn1".to_string(),
        },
        GlobalDbState::new(),
    ))
    .unwrap();

    assert!(views.is_empty());
}

#[test]
fn wasm_open_view_output_builds_extension_widget_event() {
    let mut state = ComponentHostState::new("com.example.tools", NoopDbHost);
    state.set_action_context(extension_component::ActionContext {
        extension_id: "com.example.tools".to_string(),
        command_id: "example.sync_table".to_string(),
        node_id: "table-1".to_string(),
        node_name: "users".to_string(),
        node_type: "table".to_string(),
        database_type: "PostgreSQL".to_string(),
        connection_id: "conn-1".to_string(),
    });

    let context =
        futures::executor::block_on(WitUi::Host::current_action_context(&mut state)).unwrap();
    assert_eq!("users", context.unwrap().node_name);
    futures::executor::block_on(WitUi::Host::open_view(&mut state, wit_form_view())).unwrap();

    let model = build_extension_widget_model(&state.opened_views()[0]).unwrap();
    let values = default_form_values(&model);
    let event = form_values_to_action_event(&model.id, &model.actions[0].id, &values);

    assert_eq!("sync-form", model.id);
    assert_eq!("users", values["target"].as_str());
    assert_eq!("run", event.action_id);
    assert!(
        event
            .fields
            .iter()
            .any(|field| field.id == "target" && field.value == "users")
    );
}

fn coverage_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../extension-wasm/fixtures/coverage-plugin")
        .canonicalize()
        .unwrap()
}

struct WasmImporterFixture<'a> {
    extension_dir: &'a str,
    extension_id: &'a str,
    importer_id: &'a str,
    runtime_id: &'a str,
    display_name: &'a str,
    output_kind: &'a str,
    component_name: &'a str,
    core_wat: &'a str,
}

fn write_wasm_importer_extension(composite_root: &Path, fixture: WasmImporterFixture<'_>) {
    let extension_dir = composite_root.join(fixture.extension_dir);
    let wasm_dir = extension_dir.join("wasm");
    fs::create_dir_all(&wasm_dir).unwrap();
    write_component_from_core_wat(&wasm_dir.join(fixture.component_name), fixture.core_wat);
    fs::write(
        extension_dir.join("extension.json"),
        format!(
            r#"{{
                "schema_version": 1,
                "id": "{extension_id}",
                "name": "{display_name} Importer",
                "version": "0.1.0",
                "engines": {{ "onetcli": ">=0.7.0" }},
                "runtime": {{
                    "wasm": [{{
                        "id": "{runtime_id}",
                        "module": "wasm/{component_name}",
                        "kind": "component"
                    }}]
                }},
                "contributes": {{
                    "connectionImporters": [{{
                        "id": "{importer_id}",
                        "runtimeId": "{runtime_id}",
                        "displayName": "{display_name}",
                        "outputKinds": ["{output_kind}"],
                        "platforms": ["macos"]
                    }}]
                }}
            }}"#,
            extension_id = fixture.extension_id,
            display_name = fixture.display_name,
            runtime_id = fixture.runtime_id,
            component_name = fixture.component_name,
            importer_id = fixture.importer_id,
            output_kind = fixture.output_kind,
        ),
    )
    .unwrap();
}

fn write_component_from_core_wat(component: &Path, wat: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("importer.wat");
    let embedded = dir.path().join("embedded.wasm");
    fs::write(&core_wat, wat).unwrap();

    let wit_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
    let embed_output = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            wit_dir.to_str().unwrap(),
            "--world",
            "connection-importer",
            core_wat.to_str().unwrap(),
            "-o",
            embedded.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        embed_output.status.success(),
        "component embed failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed_output.stdout),
        String::from_utf8_lossy(&embed_output.stderr)
    );

    let new_output = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            embedded.to_str().unwrap(),
            "-o",
            component.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        new_output.status.success(),
        "component new failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&new_output.stdout),
        String::from_utf8_lossy(&new_output.stderr)
    );
}

fn dbeaver_importer_core_wat() -> &'static str {
    include_str!("../../extension-wasm/fixtures/connection-import/dbeaver_importer_core.wat")
}

fn termius_importer_core_wat() -> &'static str {
    include_str!("../../extension-wasm/fixtures/connection-import/termius_importer_core.wat")
}

fn wit_form_view() -> WitUi::ViewSpec {
    WitUi::ViewSpec {
        id: "sync-form".to_string(),
        title: "同步表".to_string(),
        mode: WitUi::ViewMode::Dialog,
        nodes: vec![WitUi::UiNode::Form(vec![WitUi::UiField {
            id: "target".to_string(),
            label: "目标表".to_string(),
            kind: WitUi::FieldKind::Select,
            required: true,
            value: Some("users".to_string()),
            source: Some(WitUi::FieldSource::StaticOptions(vec![
                WitUi::SelectOption {
                    value: "users".to_string(),
                    label: "users".to_string(),
                },
            ])),
        }])],
        actions: vec![WitUi::UiAction {
            id: "run".to_string(),
            label: "执行".to_string(),
            style: WitUi::ActionStyle::Primary,
        }],
        window: None,
    }
}

struct NoopDbHost;

#[async_trait]
impl ExtensionDbHost for NoopDbHost {
    fn list_connections(&self) -> Result<Vec<protocol::ConnectionInfo>, protocol::DbError> {
        Ok(Vec::new())
    }

    async fn open_session(
        &self,
        request: protocol::OpenSessionRequest,
    ) -> Result<DbSessionResource, protocol::DbError> {
        Ok(DbSessionResource::new(
            "com.example.tools",
            request.connection_id,
            "session-1",
        ))
    }

    async fn execute(
        &self,
        _session: &DbSessionResource,
        _sql: String,
        _options: protocol::ExecOptions,
    ) -> Result<protocol::RowBatch, protocol::DbError> {
        Ok(protocol::RowBatch {
            columns: Vec::new(),
            rows: Vec::new(),
            next_cursor: None,
        })
    }

    async fn list_databases(
        &self,
        _session: &DbSessionResource,
    ) -> Result<Vec<String>, protocol::DbError> {
        Ok(Vec::new())
    }

    async fn list_schemas(
        &self,
        _session: &DbSessionResource,
        _database: String,
    ) -> Result<Vec<String>, protocol::DbError> {
        Ok(Vec::new())
    }

    async fn close_session(
        &self,
        session: &mut DbSessionResource,
    ) -> Result<(), protocol::DbError> {
        session.close();
        Ok(())
    }
}
