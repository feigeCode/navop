use async_trait::async_trait;
use extension_component::{ExtensionDbHost, protocol};

pub struct NoopDbHost {
    pub connections: Vec<protocol::ConnectionInfo>,
}

pub fn connection(id: &str) -> protocol::ConnectionInfo {
    protocol::ConnectionInfo {
        id: id.to_string(),
        name: "test".to_string(),
        driver: "PostgreSQL".to_string(),
        database: Some("postgres".to_string()),
    }
}

pub fn action_context() -> extension_component::ActionContext {
    extension_component::ActionContext {
        extension_id: "ext".to_string(),
        command_id: "search".to_string(),
        node_id: "node-1".to_string(),
        node_name: "public".to_string(),
        node_type: "schema".to_string(),
        database_type: "PostgreSQL".to_string(),
        connection_id: "conn-1".to_string(),
    }
}

pub const MINIMAL_EXTENSION_COMPONENT: &str = r#"
(component
    (type $ui-instance
        (instance
            (type (;0;) (record (field "id" string) (field "value" string)))
            (export (;1;) "field-value" (type (eq 0)))
            (type (;2;) (list 1))
            (type (;3;) (record (field "view-id" string) (field "action-id" string) (field "fields" 2)))
            (export (;4;) "view-action-event" (type (eq 3)))
        )
    )
    (import "onet:extension/ui" (instance $ui (type $ui-instance)))
    (alias export $ui "view-action-event" (type $view-action-event))
    (type $handle-view-action-ty (func (param "event" $view-action-event)))
    (core module $m
        (type (;0;) (func))
        (type (;1;) (func (param i32 i32 i32 i32 i32 i32)))
        (memory 1)
        (func (;0;) (param i32 i32 i32 i32) (result i32) (i32.const 2048))
        (func (;1;) (type 0))
        (func (;2;) (type 0))
        (func (;3;) (type 1) (param i32 i32 i32 i32 i32 i32))
        (func (;4;) (type 0))
        (export "memory" (memory 0))
        (export "realloc" (func 0))
        (export "activate" (func 1))
        (export "run-action" (func 2))
        (export "handle-view-action" (func 3))
        (export "deactivate" (func 4))
    )
    (core instance $i (instantiate $m))
    (alias core export $i "memory" (core memory $memory))
    (alias core export $i "realloc" (core func $realloc))
    (func $activate (canon lift (core func $i "activate")))
    (func $run-action (canon lift (core func $i "run-action")))
    (func $handle-view-action (type $handle-view-action-ty)
        (canon lift
            (core func $i "handle-view-action")
            (memory $memory)
            (realloc $realloc)
            string-encoding=utf8
        ))
    (func $deactivate (canon lift (core func $i "deactivate")))
    (export "activate" (func $activate))
    (export "run-action" (func $run-action))
    (export "handle-view-action" (func $handle-view-action) (func (type $handle-view-action-ty)))
    (export "deactivate" (func $deactivate))
)
"#;

#[async_trait]
impl ExtensionDbHost for NoopDbHost {
    fn list_connections(&self) -> Result<Vec<protocol::ConnectionInfo>, protocol::DbError> {
        Ok(self.connections.clone())
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
            columns: vec![protocol::Column {
                name: "value".to_string(),
                type_name: "text".to_string(),
                nullable: false,
            }],
            rows: vec![vec![protocol::DbValue::Text("ok".to_string())]],
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
