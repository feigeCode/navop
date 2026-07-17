use std::{fs, path::PathBuf, process::Command};

use anyhow::Result;
use async_trait::async_trait;
use db::GlobalDbState;
use db_view::{
    extension_menu::DbTreeExtensionActionContext,
    extension_widget::{
        build_extension_widget_model, default_form_values, form_values_to_action_event,
    },
};
use extension_component::{DbSessionResource, ExtensionDbHost, protocol};
use extension_wasm::{ComponentHostState, bindings::onet::extension::ui as WitUi};
use one_core::storage::DatabaseType;

use crate::ExtensionRuntimeCatalog;

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
fn html_preview_transform_runs_registered_wasm_component() {
    let root = tempfile::TempDir::new().unwrap();
    let extension_dir = root.path().join("com.example.html");
    let wasm_dir = extension_dir.join("wasm");
    fs::create_dir_all(&wasm_dir).unwrap();
    fs::write(
        wasm_dir.join("plugin.wasm"),
        html_preview_transform_component_bytes(),
    )
    .unwrap();
    fs::write(
        extension_dir.join("extension.json"),
        r#"{
            "schema_version": 1,
            "id": "com.example.html",
            "name": "HTML Preview",
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
                "htmlPreviewTransforms": [{
                    "id": "html.decorate",
                    "runtimeId": "main",
                    "languages": ["html"],
                    "assets": "assets"
                }]
            }
        }"#,
    )
    .unwrap();

    let manifest = crate::extension::manifest::load_from_dir(&extension_dir).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let result = futures::executor::block_on(catalog.transform_html_preview("html", "<main>Hi"))
        .unwrap()
        .unwrap();

    assert_eq!("<main>Changed</main>", result.html);
}

#[test]
fn installed_mermaid_extension_registers_and_renders_when_fixture_is_provided() {
    let Ok(dir) = std::env::var("NAVOP_INSTALLED_MERMAID_EXTENSION") else {
        return;
    };
    let manifest = crate::extension::manifest::load_from_dir(std::path::Path::new(&dir)).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let renderer = catalog.document_renderer_for_kind("mermaid").unwrap();
    assert_eq!("Mermaid", renderer.display_name);
    let output = futures::executor::block_on(catalog.render_document(
        extension_wasm::DocumentRenderRequest {
            renderer: "mermaid".to_owned(),
            source: "flowchart TD\n A --> B".to_owned(),
            theme: extension_wasm::DocumentRenderTheme {
                dark: false,
                background: 0xf7f6f3,
                foreground: 0x37352f,
                border: 0xd8d8d6,
                muted: 0x9b9a97,
                accent: 0x2383e2,
                danger: 0xeb5757,
                font_family: "Inter, sans-serif".to_owned(),
            },
            available_width: 720.0,
            scale_factor: 1.0,
        },
    ))
    .unwrap()
    .unwrap();
    assert_eq!("image/svg+xml", output.media_type);
    assert!(String::from_utf8(output.bytes).unwrap().contains("<svg"));
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

fn html_preview_transform_component_bytes() -> Vec<u8> {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("html_transform.wat");
    let embedded = dir.path().join("embedded.wasm");
    let component = dir.path().join("html_transform.component.wasm");
    fs::write(
        &core_wat,
        r#"
(module
  (memory (export "cm32p2_memory") 1)
  (data (i32.const 512) "\00\00\00\00\00\04\00\00\14\00\00\00\00\00\00\00\00\00\00\00")
  (data (i32.const 1024) "<main>Changed</main>")
  (func (export "cm32p2_realloc") (param i32 i32 i32 i32) (result i32)
    i32.const 2048)
  (func (export "cm32p2_initialize"))
  (func (export "cm32p2||transform-html") (param i32 i32 i32 i32) (result i32)
    i32.const 512)
  (func (export "cm32p2||transform-html_post") (param i32))
)
"#,
    )
    .unwrap();
    let wit_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
    let embed_output = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            wit_dir.to_str().unwrap(),
            "--world",
            "html-preview-transform",
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
    fs::read(component).unwrap()
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
