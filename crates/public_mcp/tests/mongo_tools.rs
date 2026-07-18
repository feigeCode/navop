use public_mcp::tools::{
    MongoConnectionSnapshot, MongoConnectionSnapshotProvider, MongoOperation,
    MongoOperationProvider, MongoToolProvider,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tool_runtime::{ToolAdapter, ToolContext, ToolRegistry, ToolResult};

#[derive(Default)]
struct FakeMongoRuntime {
    calls: Mutex<Vec<(MongoOperation, String, Value)>>,
}

impl MongoConnectionSnapshotProvider for FakeMongoRuntime {
    fn list_connections(&self) -> Vec<MongoConnectionSnapshot> {
        vec![
            MongoConnectionSnapshot {
                connection_id: "mongo-b".to_string(),
            },
            MongoConnectionSnapshot {
                connection_id: "mongo-a".to_string(),
            },
        ]
    }
}

impl MongoOperationProvider for FakeMongoRuntime {
    fn execute(
        &self,
        operation: MongoOperation,
        connection_id: &str,
        input: Value,
    ) -> tool_runtime::ToolFuture {
        self.calls
            .lock()
            .unwrap()
            .push((operation, connection_id.to_string(), input.clone()));
        Box::pin(async move { Ok(ToolResult::structured(json!({ "ok": true }))) })
    }
}

#[tokio::test]
async fn mongo_tools_expose_trait_operations_and_dispatch_without_business_logic() {
    let runtime = Arc::new(FakeMongoRuntime::default());
    let registry = ToolRegistry::new(MongoToolProvider::handlers(runtime.clone()));
    let names = registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .map(|tool| tool.id)
        .collect::<Vec<_>>();

    for expected in [
        "mongo.list_connections",
        "mongo.list_databases",
        "mongo.list_collections",
        "mongo.find",
        "mongo.aggregate",
        "mongo.count",
        "mongo.list_indexes",
        "mongo.create_index",
        "mongo.drop_index",
        "mongo.create_collection",
        "mongo.drop_database",
        "mongo.get_validation",
        "mongo.set_validation",
        "mongo.insert",
        "mongo.replace",
        "mongo.update",
        "mongo.delete",
        "mongo.explain",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }

    let list = registry
        .call(
            "mongo.list_connections",
            json!({}),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .unwrap();
    assert_eq!(
        json!({
            "connections": [
                { "connection_id": "mongo-a" },
                { "connection_id": "mongo-b" }
            ]
        }),
        list.structured_content
    );

    registry
        .call(
            "mongo.find",
            json!({
                "connection_id": "mongo-a",
                "database": "app",
                "collection": "users",
                "filter": { "active": true },
                "limit": 20
            }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .unwrap();

    assert_eq!(
        vec![(
            MongoOperation::Find,
            "mongo-a".to_string(),
            json!({
                "connection_id": "mongo-a",
                "database": "app",
                "collection": "users",
                "filter": { "active": true },
                "limit": 20
            })
        )],
        *runtime.calls.lock().unwrap()
    );
}
