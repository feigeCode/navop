use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::permissions::PermissionMode;
use public_mcp::tools::{
    PublicMcpToolContext, PublicMcpToolRegistry, RedisCommandExecutionProvider,
    RedisConnectionSnapshot, RedisConnectionSnapshotProvider, RedisToolProvider,
    ToolRuntimeMcpProvider,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tool_runtime::{ResourceKind, ResourcePool, ResourceRef, ToolRegistry};

#[test]
fn redis_convenience_tools_are_registered_with_expected_risk() {
    let registry = ToolRegistry::new(RedisToolProvider::empty());
    let keys = registry
        .get("redis.keys", tool_runtime::ToolAdapter::Mcp)
        .expect("redis.keys should be registered");
    let get = registry
        .get("redis.get", tool_runtime::ToolAdapter::Mcp)
        .expect("redis.get should be registered");
    let set = registry
        .get("redis.set", tool_runtime::ToolAdapter::Mcp)
        .expect("redis.set should be registered");

    assert_eq!(
        json!(["connection_id", "pattern"]),
        keys.input_schema["required"]
    );
    assert!(keys.annotations.read_only);
    assert!(!keys.annotations.destructive);
    assert_eq!(
        json!(["connection_id", "key"]),
        get.input_schema["required"]
    );
    assert!(get.annotations.read_only);
    assert!(!get.annotations.destructive);
    assert_eq!(
        json!(["connection_id", "key", "value"]),
        set.input_schema["required"]
    );
    assert!(!set.annotations.read_only);
    assert!(set.annotations.destructive);
}

#[tokio::test]
async fn redis_get_calls_runtime_without_approval_in_safe_mode() {
    let runtime = Arc::new(RecordingRedisRuntime::new());
    let registry = registry(runtime.clone());

    let result = registry
        .call_tool(
            "redis.get",
            Some(serde_json::Map::from_iter([
                ("target".to_string(), json!("redis-a")),
                ("key".to_string(), json!("user:1")),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect("redis.get should run");

    assert_eq!(Some(result_for("GET user:1")), result.structured_content);
    assert_eq!(
        vec![RecordedCommand::new("redis-a", None, "GET user:1")],
        runtime.commands()
    );
}

#[tokio::test]
async fn redis_get_resolves_target_with_redis_resource_kind() {
    let runtime = Arc::new(RecordingRedisRuntime::new());
    let resource_pool = ResourcePool::new()
        .with_resource(ResourceRef::new("db-prod", ResourceKind::Mysql, "prod"))
        .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "prod"));
    let registry = registry_with_pool(runtime.clone(), resource_pool);

    let result = registry
        .call_tool(
            "redis.get",
            Some(serde_json::Map::from_iter([
                ("target".to_string(), json!("prod")),
                ("key".to_string(), json!("user:1")),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect("redis.get target kind should disambiguate resource pool");

    assert_eq!(Some(result_for("GET user:1")), result.structured_content);
    assert_eq!(
        vec![RecordedCommand::new("redis-a", None, "GET user:1")],
        runtime.commands()
    );
}

#[tokio::test]
async fn redis_set_runs_without_approval_in_allow_mode() {
    let runtime = Arc::new(RecordingRedisRuntime::new());
    let approver = Arc::new(RecordingApprover::approved());
    let registry = registry(runtime.clone());

    let result = registry
        .call_tool(
            "redis.set",
            Some(serde_json::Map::from_iter([
                ("target".to_string(), json!("redis-a")),
                ("key".to_string(), json!("user:1")),
                ("value".to_string(), json!("Ada")),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Allow,
                approver: PublicMcpApprovalManager::new(approver.clone()),
            },
        )
        .await
        .expect("redis.set should run in auto mode");

    assert_eq!(
        Some(result_for("SET user:1 Ada")),
        result.structured_content
    );
    assert!(approver.requests().is_empty());
    assert_eq!(
        vec![RecordedCommand::new("redis-a", None, "SET user:1 Ada")],
        runtime.commands()
    );
}

fn registry(runtime: Arc<RecordingRedisRuntime>) -> PublicMcpToolRegistry {
    let runtime_registry = ToolRegistry::new(RedisToolProvider::handlers(runtime));
    PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        runtime_registry,
    ))])
}

fn registry_with_pool(
    runtime: Arc<RecordingRedisRuntime>,
    resource_pool: ResourcePool,
) -> PublicMcpToolRegistry {
    let runtime_registry = ToolRegistry::new(RedisToolProvider::handlers(runtime));
    PublicMcpToolRegistry::new(vec![Arc::new(
        ToolRuntimeMcpProvider::new(runtime_registry).with_resource_pool(resource_pool),
    )])
}

fn result_for(command: &str) -> Value {
    json!({
        "connection_id": "redis-a",
        "db": null,
        "command": command,
        "result": {
            "type": "status",
            "value": "OK"
        },
        "display": command
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedCommand {
    connection_id: String,
    db: Option<u8>,
    command: String,
}

impl RecordedCommand {
    fn new(connection_id: &str, db: Option<u8>, command: &str) -> Self {
        Self {
            connection_id: connection_id.to_string(),
            db,
            command: command.to_string(),
        }
    }
}

#[derive(Default)]
struct RecordingRedisRuntime {
    commands: Mutex<Vec<RecordedCommand>>,
}

impl RecordingRedisRuntime {
    fn new() -> Self {
        Self::default()
    }

    fn commands(&self) -> Vec<RecordedCommand> {
        self.commands.lock().expect("commands lock").clone()
    }
}

impl RedisConnectionSnapshotProvider for RecordingRedisRuntime {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        vec![RedisConnectionSnapshot {
            connection_id: "redis-a".to_string(),
        }]
    }
}

impl RedisCommandExecutionProvider for RecordingRedisRuntime {
    fn execute_command(
        &self,
        connection_id: &str,
        db: Option<u8>,
        command: &str,
    ) -> tool_runtime::ToolFuture {
        let command_record = RecordedCommand::new(connection_id, db, command);
        self.commands
            .lock()
            .expect("commands lock")
            .push(command_record);
        let command = command.to_string();
        Box::pin(async move { Ok(tool_runtime::ToolResult::structured(result_for(&command))) })
    }
}

#[derive(Clone, Default)]
struct RecordingApprover {
    requests: Arc<Mutex<Vec<PublicMcpApprovalRequest>>>,
}

impl RecordingApprover {
    fn approved() -> Self {
        Self::default()
    }

    fn requests(&self) -> Vec<PublicMcpApprovalRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl PublicMcpApprover for RecordingApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        self.requests.lock().expect("requests lock").push(request);
        Box::pin(async { PublicMcpApprovalOutcome::Approved })
    }
}
