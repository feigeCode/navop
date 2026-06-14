use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use extension_component::{ExtensionDbHost, protocol};

use crate::{ComponentHostState, ComponentRuntime};

#[test]
fn component_runtime_instantiates_guest_declaring_db_imports() {
    instantiate_and_activate(include_str!("../fixtures/imports/db_import_component.wat"));
}

#[test]
fn component_runtime_instantiates_guest_declaring_full_db_imports() {
    instantiate_and_activate(include_str!(
        "../fixtures/imports/full_db_import_component.wat"
    ));
}

#[test]
fn component_runtime_instantiates_guest_declaring_task_imports() {
    instantiate_and_activate(include_str!(
        "../fixtures/imports/task_import_component.wat"
    ));
}

#[test]
fn component_runtime_guest_calls_db_import_during_activate() {
    let runtime = ComponentRuntime::from_wat_for_tests(
        "component",
        include_str!("../fixtures/imports/db_import_call_component.wat"),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let state = ComponentHostState::new("ext", DbImportHost::recording(calls.clone()));

    let (mut store, extension) =
        futures::executor::block_on(runtime.instantiate_with_db(state)).unwrap();

    futures::executor::block_on(extension.call_activate(&mut store)).unwrap();

    assert_eq!(1, calls.load(Ordering::SeqCst));
}

fn instantiate_and_activate(component: &str) {
    let runtime = ComponentRuntime::from_wat_for_tests("component", component).unwrap();
    let state = ComponentHostState::new("ext", DbImportHost::default());

    let (mut store, extension) =
        futures::executor::block_on(runtime.instantiate_with_db(state)).unwrap();

    futures::executor::block_on(extension.call_activate(&mut store)).unwrap();
}

#[derive(Default)]
struct DbImportHost {
    list_connections_calls: Option<Arc<AtomicUsize>>,
}

impl DbImportHost {
    fn recording(list_connections_calls: Arc<AtomicUsize>) -> Self {
        Self {
            list_connections_calls: Some(list_connections_calls),
        }
    }
}

#[async_trait]
impl ExtensionDbHost for DbImportHost {
    fn list_connections(&self) -> Result<Vec<protocol::ConnectionInfo>, protocol::DbError> {
        if let Some(calls) = &self.list_connections_calls {
            calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(vec![protocol::ConnectionInfo {
            id: "conn1".to_string(),
            name: "test".to_string(),
            driver: "PostgreSQL".to_string(),
            database: Some("postgres".to_string()),
        }])
    }

    async fn open_session(
        &self,
        request: protocol::OpenSessionRequest,
    ) -> Result<extension_component::DbSessionResource, protocol::DbError> {
        Ok(extension_component::DbSessionResource::new(
            "ext",
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
        Ok(Vec::new())
    }

    async fn list_schemas(
        &self,
        _session: &extension_component::DbSessionResource,
        _database: String,
    ) -> Result<Vec<String>, protocol::DbError> {
        Ok(Vec::new())
    }

    async fn close_session(
        &self,
        session: &mut extension_component::DbSessionResource,
    ) -> Result<(), protocol::DbError> {
        session.close();
        Ok(())
    }
}
