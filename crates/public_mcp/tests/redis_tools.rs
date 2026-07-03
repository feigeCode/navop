use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::permissions::PermissionMode;
use public_mcp::tools::{
    PublicMcpToolContext, PublicMcpToolRegistry, RedisCommandExecution,
    RedisCommandExecutionProvider, RedisConnectionSnapshot, RedisConnectionSnapshotProvider,
    RedisToolProvider, ToolRuntimeMcpProvider,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tool_runtime::ToolRegistry;

#[tokio::test]
async fn redis_provider_lists_runtime_connections() {
    let runtime_registry =
        ToolRegistry::new(RedisToolProvider::handlers(Arc::new(FakeRedisSnapshots {
            connections: vec![
                RedisConnectionSnapshot {
                    connection_id: "redis-b".to_string(),
                },
                RedisConnectionSnapshot {
                    connection_id: "redis-a".to_string(),
                },
            ],
        })));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        runtime_registry,
    ))]);

    let result = registry
        .call_tool(
            "redis.list_connections",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect("redis list connections tool should run");

    assert_eq!(
        Some(json!({
            "connections": [
                { "connection_id": "redis-a" },
                { "connection_id": "redis-b" }
            ]
        })),
        result.structured_content
    );
}

#[tokio::test]
async fn redis_provider_executes_runtime_command() {
    let registry = redis_command_registry(Some(2));
    let approver = Arc::new(RecordingApprover::approved());

    let result = registry
        .call_tool(
            "redis.command",
            Some(serde_json::Map::from_iter([
                ("connection_id".to_string(), json!("redis-a")),
                ("db".to_string(), json!(2)),
                ("command".to_string(), json!("PING")),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Allow,
                approver: PublicMcpApprovalManager::new(approver.clone()),
            },
        )
        .await
        .expect("redis execute command tool should run");

    assert_eq!(
        Some(redis_command_result(json!(2))),
        result.structured_content
    );
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!("redis.command", requests[0].tool_name);
}

#[test]
fn redis_command_is_registered_as_mutating() {
    let registry = ToolRegistry::new(RedisToolProvider::empty());
    let tool = registry
        .get("redis.command", tool_runtime::ToolAdapter::Mcp)
        .expect("redis.command tool should be registered");

    assert_eq!(
        json!(["connection_id", "command"]),
        tool.input_schema["required"]
    );
    assert_eq!("redis.command", tool.id);
    assert!(!tool.annotations.read_only);
    assert!(tool.annotations.destructive);
}

#[tokio::test]
async fn redis_execute_command_alias_still_calls_runtime_command() {
    let registry = redis_command_registry(None);
    let approver = Arc::new(RecordingApprover::approved());

    let result = registry
        .call_tool(
            "redis.execute_command",
            Some(serde_json::Map::from_iter([
                ("connection_id".to_string(), json!("redis-a")),
                ("command".to_string(), json!("PING")),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Allow,
                approver: PublicMcpApprovalManager::new(approver.clone()),
            },
        )
        .await
        .expect("legacy redis.execute_command alias should run");

    assert_eq!(
        Some(redis_command_result(Value::Null)),
        result.structured_content
    );
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!("redis.execute_command", requests[0].tool_name);
}

fn redis_command_registry(db: Option<u8>) -> PublicMcpToolRegistry {
    let runtime_registry =
        ToolRegistry::new(RedisToolProvider::handlers(Arc::new(FakeRedisRuntime {
            connections: vec![RedisConnectionSnapshot {
                connection_id: "redis-a".to_string(),
            }],
            execution: RedisCommandExecution {
                connection_id: "redis-a".to_string(),
                db,
                command: "PING".to_string(),
                result: json!({ "type": "status", "value": "PONG" }),
                display: "OK: PONG".to_string(),
            },
        })));
    PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        runtime_registry,
    ))])
}

fn redis_command_result(db: Value) -> Value {
    json!({
        "connection_id": "redis-a",
        "db": db,
        "command": "PING",
        "result": {
            "type": "status",
            "value": "PONG"
        },
        "display": "OK: PONG"
    })
}

#[derive(Clone)]
struct FakeRedisSnapshots {
    connections: Vec<RedisConnectionSnapshot>,
}

impl RedisConnectionSnapshotProvider for FakeRedisSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        self.connections.clone()
    }
}

impl RedisCommandExecutionProvider for FakeRedisSnapshots {
    fn execute_command(
        &self,
        connection_id: &str,
        _db: Option<u8>,
        _command: &str,
    ) -> tool_runtime::ToolFuture {
        let connection_id = connection_id.to_string();
        Box::pin(async move {
            Err(tool_runtime::ToolError::Failed {
                message: format!("unknown Redis connection: {connection_id}"),
            })
        })
    }
}

#[derive(Clone)]
struct FakeRedisRuntime {
    connections: Vec<RedisConnectionSnapshot>,
    execution: RedisCommandExecution,
}

impl RedisConnectionSnapshotProvider for FakeRedisRuntime {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        self.connections.clone()
    }
}

impl RedisCommandExecutionProvider for FakeRedisRuntime {
    fn execute_command(
        &self,
        _connection_id: &str,
        _db: Option<u8>,
        _command: &str,
    ) -> tool_runtime::ToolFuture {
        let execution = self.execution.clone();
        Box::pin(async move { Ok(tool_runtime::ToolResult::structured(execution.into_json())) })
    }
}

#[derive(Clone)]
struct RecordingApprover {
    requests: Arc<Mutex<Vec<PublicMcpApprovalRequest>>>,
}

impl RecordingApprover {
    fn approved() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<PublicMcpApprovalRequest> {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .clone()
    }
}

impl PublicMcpApprover for RecordingApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .push(request);
        Box::pin(async { PublicMcpApprovalOutcome::Approved })
    }
}
