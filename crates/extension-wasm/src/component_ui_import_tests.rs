use super::*;

#[test]
fn generated_ui_imports_expose_action_context_and_open_view() {
    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );
    state.set_action_context(crate::test_support::action_context());

    let context = futures::executor::block_on(
        bindings::onet::extension::ui::Host::current_action_context(&mut state),
    )
    .unwrap()
    .unwrap();
    assert_eq!("search", context.command_id);

    futures::executor::block_on(bindings::onet::extension::ui::Host::open_view(
        &mut state,
        bindings::onet::extension::ui::ViewSpec {
            id: "view-1".to_string(),
            title: "全库搜索".to_string(),
            mode: bindings::onet::extension::ui::ViewMode::Dialog,
            nodes: Vec::new(),
            actions: Vec::new(),
            window: None,
        },
    ))
    .unwrap();

    assert_eq!(1, state.opened_views().len());
    assert_eq!("全库搜索", state.opened_views()[0].title);
}

#[test]
fn generated_ui_imports_expose_progress_resource_methods() {
    let mut state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );

    let progress = futures::executor::block_on(
        bindings::onet::extension::ui::Host::start_progress(&mut state, "Smoke".to_string()),
    )
    .unwrap();
    let borrowed = wasmtime::component::Resource::new_borrow(progress.rep());

    futures::executor::block_on(bindings::onet::extension::ui::HostProgress::set_message(
        &mut state,
        borrowed,
        "Running".to_string(),
    ))
    .unwrap();
    let borrowed = wasmtime::component::Resource::new_borrow(progress.rep());
    futures::executor::block_on(bindings::onet::extension::ui::HostProgress::set_fraction(
        &mut state, borrowed, 1.0,
    ))
    .unwrap();
    let borrowed = wasmtime::component::Resource::new_borrow(progress.rep());
    futures::executor::block_on(bindings::onet::extension::ui::HostProgress::close(
        &mut state, borrowed,
    ))
    .unwrap();
    futures::executor::block_on(bindings::onet::extension::ui::HostProgress::drop(
        &mut state, progress,
    ))
    .unwrap();
}

struct NoopDbHost {
    connections: Vec<extension_component::protocol::ConnectionInfo>,
}

#[async_trait::async_trait]
impl extension_component::ExtensionDbHost for NoopDbHost {
    fn list_connections(
        &self,
    ) -> Result<
        Vec<extension_component::protocol::ConnectionInfo>,
        extension_component::protocol::DbError,
    > {
        Ok(self.connections.clone())
    }

    async fn open_session(
        &self,
        request: extension_component::protocol::OpenSessionRequest,
    ) -> Result<extension_component::DbSessionResource, extension_component::protocol::DbError>
    {
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
        _options: extension_component::protocol::ExecOptions,
    ) -> Result<extension_component::protocol::RowBatch, extension_component::protocol::DbError>
    {
        Ok(extension_component::protocol::RowBatch {
            columns: Vec::new(),
            rows: Vec::new(),
            next_cursor: None,
        })
    }

    async fn list_databases(
        &self,
        _session: &extension_component::DbSessionResource,
    ) -> Result<Vec<String>, extension_component::protocol::DbError> {
        Ok(Vec::new())
    }

    async fn list_schemas(
        &self,
        _session: &extension_component::DbSessionResource,
        _database: String,
    ) -> Result<Vec<String>, extension_component::protocol::DbError> {
        Ok(Vec::new())
    }

    async fn close_session(
        &self,
        session: &mut extension_component::DbSessionResource,
    ) -> Result<(), extension_component::protocol::DbError> {
        session.close();
        Ok(())
    }
}
