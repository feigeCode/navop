use crate::{
    ComponentHostState, bindings,
    test_support::{NoopDbHost, connection},
};

#[test]
fn component_host_state_stores_db_session_resources() {
    use extension_component::{DbSessionResource, protocol::ConnectionInfo};

    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );
    let resource = state
        .table_mut()
        .push(DbSessionResource::new("ext", "conn", "session"))
        .unwrap();

    assert_eq!("ext", state.extension_id());
    assert_eq!(
        "session",
        state.table().get(&resource).unwrap().session_id()
    );
    assert_eq!(Vec::<ConnectionInfo>::new(), state.db_host().connections);
}

#[test]
fn component_host_state_implements_generated_db_import_traits() {
    fn assert_db<T>()
    where
        T: bindings::onet::extension::db::Host + bindings::onet::extension::db::HostSession,
    {
    }

    assert_db::<ComponentHostState<NoopDbHost>>();
}

#[test]
fn generated_db_import_lists_connections_through_host() {
    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: vec![connection("conn1")],
        },
    );

    let connections = futures::executor::block_on(
        bindings::onet::extension::db::Host::list_connections(&mut state),
    )
    .unwrap()
    .unwrap();

    assert_eq!(1, connections.len());
    assert_eq!("conn1", connections[0].id);
    assert_eq!("test", connections[0].name);
    assert_eq!("PostgreSQL", connections[0].driver);
}

#[test]
fn generated_db_import_opens_session_resource() {
    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );

    let resource = futures::executor::block_on(bindings::onet::extension::db::Host::open_session(
        &mut state,
        "conn1".to_string(),
        Some("postgres".to_string()),
    ))
    .unwrap()
    .unwrap();

    let session = state.table().get(&resource).unwrap();
    assert_eq!("conn1", session.connection_id());
    assert_eq!("session-1", session.session_id());
}

#[test]
fn generated_db_session_execute_maps_row_batch() {
    use extension_component::DbSessionResource;

    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );
    let resource = state
        .table_mut()
        .push(DbSessionResource::new("ext", "conn1", "session-1"))
        .unwrap();

    let batch = futures::executor::block_on(bindings::onet::extension::db::HostSession::execute(
        &mut state,
        resource,
        "select 1".to_string(),
        bindings::onet::extension::db::ExecOptions {
            max_rows: Some(10),
            timeout_ms: Some(1000),
            streaming: false,
        },
    ))
    .unwrap()
    .unwrap();

    assert_eq!("value", batch.columns[0].name);
    assert!(matches!(
        batch.rows[0][0],
        bindings::onet::extension::db::DbValue::Text(ref value) if value == "ok"
    ));
}

#[test]
fn generated_db_session_close_marks_resource_closed() {
    use extension_component::DbSessionResource;

    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );
    let resource = state
        .table_mut()
        .push(DbSessionResource::new("ext", "conn1", "session-1"))
        .unwrap();

    futures::executor::block_on(bindings::onet::extension::db::HostSession::close(
        &mut state,
        wasmtime::component::Resource::new_borrow(resource.rep()),
    ))
    .unwrap()
    .unwrap();

    assert!(state.table().get(&resource).unwrap().is_closed());
}
