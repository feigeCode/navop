use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use extension_component::{ActionContext, ExtensionDbHost, ViewActionEvent, protocol};

use crate::{ComponentHostState, ComponentRuntime, WasmRuntimeConfig};

#[test]
fn coverage_plugin_declares_manifest_and_exercises_component_runtime() {
    let runtime = ComponentRuntime::from_file(
        "coverage",
        &coverage_wat_path(),
        WasmRuntimeConfig::default(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let state = ComponentHostState::new(
        "com.onetcli.coverage-wasm",
        CoverageDbHost {
            list_connections_calls: calls.clone(),
        },
    );
    let (mut store, extension) =
        futures::executor::block_on(runtime.instantiate_with_db(state)).unwrap();

    futures::executor::block_on(extension.call_activate(&mut store)).unwrap();
    futures::executor::block_on(extension.call_run_action(&mut store)).unwrap();
    futures::executor::block_on(extension.call_deactivate(&mut store)).unwrap();

    assert_eq!(2, calls.load(Ordering::SeqCst));
}

#[test]
fn coverage_plugin_accepts_view_action_callback() {
    let runtime = ComponentRuntime::from_file(
        "coverage",
        &coverage_wat_path(),
        WasmRuntimeConfig::default(),
    )
    .unwrap();
    let state = ComponentHostState::new(
        "com.onetcli.coverage-wasm",
        CoverageDbHost {
            list_connections_calls: Arc::new(AtomicUsize::new(0)),
        },
    );

    futures::executor::block_on(runtime.handle_view_action_with_db(
        state,
        action_context(),
        ViewActionEvent {
            view_id: "coverage".to_string(),
            action_id: "run".to_string(),
            fields: Vec::new(),
        },
    ))
    .unwrap();
}

fn coverage_wat_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/coverage-plugin/wasm/coverage_component.wat")
}

struct CoverageDbHost {
    list_connections_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ExtensionDbHost for CoverageDbHost {
    fn list_connections(&self) -> Result<Vec<protocol::ConnectionInfo>, protocol::DbError> {
        self.list_connections_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![protocol::ConnectionInfo {
            id: "conn1".to_string(),
            name: "Coverage".to_string(),
            driver: "PostgreSQL".to_string(),
            database: Some("postgres".to_string()),
        }])
    }

    async fn open_session(
        &self,
        request: protocol::OpenSessionRequest,
    ) -> Result<extension_component::DbSessionResource, protocol::DbError> {
        Ok(extension_component::DbSessionResource::new(
            "com.onetcli.coverage-wasm",
            request.connection_id,
            "session-1",
        ))
    }

    async fn execute(
        &self,
        _session: &extension_component::DbSessionResource,
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
        _session: &extension_component::DbSessionResource,
    ) -> Result<Vec<String>, protocol::DbError> {
        Ok(vec!["postgres".to_string()])
    }

    async fn list_schemas(
        &self,
        _session: &extension_component::DbSessionResource,
        _database: String,
    ) -> Result<Vec<String>, protocol::DbError> {
        Ok(vec!["public".to_string()])
    }

    async fn close_session(
        &self,
        session: &mut extension_component::DbSessionResource,
    ) -> Result<(), protocol::DbError> {
        session.close();
        Ok(())
    }
}

fn action_context() -> ActionContext {
    ActionContext {
        extension_id: "com.onetcli.coverage-wasm".to_string(),
        command_id: "coverage.run".to_string(),
        node_id: "table-1".to_string(),
        node_name: "users".to_string(),
        node_type: "table".to_string(),
        database_type: "PostgreSQL".to_string(),
        connection_id: "conn1".to_string(),
    }
}
