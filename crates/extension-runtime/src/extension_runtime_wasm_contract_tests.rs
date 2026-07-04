use std::path::PathBuf;

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
