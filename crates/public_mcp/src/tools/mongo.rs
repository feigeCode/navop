use serde_json::{Map, Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolHandler, ToolMode, ToolResult, ToolTargetSpec,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MongoConnectionSnapshot {
    pub connection_id: String,
}

pub trait MongoConnectionSnapshotProvider: Send + Sync + 'static {
    fn list_connections(&self) -> Vec<MongoConnectionSnapshot>;
}

pub trait MongoOperationProvider: Send + Sync + 'static {
    fn execute(
        &self,
        operation: MongoOperation,
        connection_id: &str,
        input: Value,
    ) -> tool_runtime::ToolFuture;
}

pub trait MongoRuntimeProvider: MongoConnectionSnapshotProvider + MongoOperationProvider {}

impl<T> MongoRuntimeProvider for T where T: MongoConnectionSnapshotProvider + MongoOperationProvider {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MongoOperation {
    ListConnections,
    ListDatabases,
    ListCollections,
    Find,
    Aggregate,
    Count,
    ListIndexes,
    CreateIndex,
    DropIndex,
    CreateCollection,
    DropDatabase,
    GetValidation,
    SetValidation,
    Insert,
    Replace,
    Update,
    Delete,
    Explain,
}

#[derive(Clone)]
pub struct MongoToolProvider {
    runtime: Arc<dyn MongoRuntimeProvider>,
    operation: MongoOperation,
}

impl MongoToolProvider {
    pub fn handlers(runtime: Arc<dyn MongoRuntimeProvider>) -> Vec<Arc<dyn ToolHandler>> {
        MongoOperation::ALL
            .into_iter()
            .map(|operation| {
                Arc::new(Self {
                    runtime: runtime.clone(),
                    operation,
                }) as Arc<dyn ToolHandler>
            })
            .collect()
    }

    pub fn empty() -> Vec<Arc<dyn ToolHandler>> {
        Self::handlers(Arc::new(EmptyMongoRuntime))
    }

    fn list_connections(&self) -> ToolResult {
        let mut connections = self.runtime.list_connections();
        connections.sort_by(|left, right| left.connection_id.cmp(&right.connection_id));
        ToolResult::structured(json!({
            "connections": connections.into_iter().map(|connection| {
                json!({ "connection_id": connection.connection_id })
            }).collect::<Vec<_>>()
        }))
    }
}

impl ToolHandler for MongoToolProvider {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.operation.id().to_string(),
            title: self.operation.title().to_string(),
            description: self.operation.description().to_string(),
            input_schema: self.operation.input_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: self.operation.annotations(),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        if self.operation == MongoOperation::ListConnections {
            let result = self.list_connections();
            return Box::pin(async move { Ok(result) });
        }
        let runtime = self.runtime.clone();
        let operation = self.operation;
        Box::pin(async move {
            let connection_id = required_string(&input, "connection_id")?;
            runtime.execute(operation, &connection_id, input).await
        })
    }

    fn target_spec(&self) -> ToolTargetSpec {
        if self.operation == MongoOperation::ListConnections {
            ToolTargetSpec::none()
        } else {
            ToolTargetSpec::required(vec![ResourceKind::Mongo])
        }
    }
}

impl MongoOperation {
    const ALL: [Self; 18] = [
        Self::ListConnections,
        Self::ListDatabases,
        Self::ListCollections,
        Self::Find,
        Self::Aggregate,
        Self::Count,
        Self::ListIndexes,
        Self::CreateIndex,
        Self::DropIndex,
        Self::CreateCollection,
        Self::DropDatabase,
        Self::GetValidation,
        Self::SetValidation,
        Self::Insert,
        Self::Replace,
        Self::Update,
        Self::Delete,
        Self::Explain,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::ListConnections => "mongo.list_connections",
            Self::ListDatabases => "mongo.list_databases",
            Self::ListCollections => "mongo.list_collections",
            Self::Find => "mongo.find",
            Self::Aggregate => "mongo.aggregate",
            Self::Count => "mongo.count",
            Self::ListIndexes => "mongo.list_indexes",
            Self::CreateIndex => "mongo.create_index",
            Self::DropIndex => "mongo.drop_index",
            Self::CreateCollection => "mongo.create_collection",
            Self::DropDatabase => "mongo.drop_database",
            Self::GetValidation => "mongo.get_validation",
            Self::SetValidation => "mongo.set_validation",
            Self::Insert => "mongo.insert",
            Self::Replace => "mongo.replace",
            Self::Update => "mongo.update",
            Self::Delete => "mongo.delete",
            Self::Explain => "mongo.explain",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::ListConnections => "List MongoDB connections",
            Self::ListDatabases => "List MongoDB databases",
            Self::ListCollections => "List MongoDB collections",
            Self::Find => "Find MongoDB documents",
            Self::Aggregate => "Aggregate MongoDB documents",
            Self::Count => "Count MongoDB documents",
            Self::ListIndexes => "List MongoDB indexes",
            Self::CreateIndex => "Create MongoDB index",
            Self::DropIndex => "Drop MongoDB index",
            Self::CreateCollection => "Create MongoDB collection",
            Self::DropDatabase => "Drop MongoDB database",
            Self::GetValidation => "Get MongoDB validation",
            Self::SetValidation => "Set MongoDB validation",
            Self::Insert => "Insert MongoDB document",
            Self::Replace => "Replace MongoDB document",
            Self::Update => "Update MongoDB document fields",
            Self::Delete => "Delete MongoDB document",
            Self::Explain => "Explain MongoDB find",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::ListConnections => "List active MongoDB connections in the running Navop app.",
            _ => {
                "Execute this MongoDB operation through an active connection owned by the running Navop app. JSON document values use MongoDB Extended JSON and are converted to BSON in the Rust host."
            }
        }
    }

    fn annotations(self) -> ToolAnnotations {
        match self {
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateCollection
            | Self::DropDatabase
            | Self::SetValidation
            | Self::Insert
            | Self::Replace
            | Self::Update
            | Self::Delete => ToolAnnotations::mutating(self.title()),
            Self::Aggregate | Self::Find | Self::Explain => {
                ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Medium)
            }
            _ => ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Low),
        }
    }

    fn input_schema(self) -> Value {
        if self == Self::ListConnections {
            return object_schema([], []);
        }
        let mut properties = vec![(
            "connection_id",
            string_schema("Active MongoDB connection id"),
        )];
        let mut required = vec!["connection_id"];
        if self.needs_database() {
            properties.push(("database", string_schema("MongoDB database name")));
            required.push("database");
        }
        if self.needs_collection() {
            properties.push(("collection", string_schema("MongoDB collection name")));
            required.push("collection");
        }
        match self {
            Self::Find | Self::Explain => {
                properties.extend([
                    (
                        "filter",
                        object_value_schema("MongoDB Extended JSON filter"),
                    ),
                    ("sort", object_value_schema("MongoDB sort document")),
                    (
                        "projection",
                        object_value_schema("MongoDB projection document"),
                    ),
                    ("skip", integer_schema(0)),
                    ("limit", integer_schema(0)),
                ]);
            }
            Self::Aggregate => {
                properties.push((
                    "pipeline",
                    array_object_schema("MongoDB aggregation pipeline"),
                ));
                required.push("pipeline");
            }
            Self::Count => properties.push((
                "filter",
                object_value_schema("MongoDB Extended JSON filter"),
            )),
            Self::CreateIndex => {
                properties.push(("keys", object_value_schema("MongoDB index key document")));
                properties.push(("name", string_schema("Optional index name")));
                required.push("keys");
            }
            Self::DropIndex => {
                properties.push(("name", string_schema("Index name")));
                required.push("name");
            }
            Self::SetValidation => properties.push((
                "validator",
                nullable_object_schema("Validator document, or null to clear"),
            )),
            Self::Insert => {
                properties.push(("document", object_value_schema("Document to insert")));
                required.push("document");
            }
            Self::Replace => {
                properties.push((
                    "id",
                    json!({ "description": "MongoDB Extended JSON _id value" }),
                ));
                properties.push(("document", object_value_schema("Replacement document")));
                required.extend(["id", "document"]);
            }
            Self::Update => {
                properties.push((
                    "id",
                    json!({ "description": "MongoDB Extended JSON _id value" }),
                ));
                properties.push(("set", object_value_schema("Fields passed to $set")));
                required.extend(["id", "set"]);
            }
            Self::Delete => {
                properties.push((
                    "id",
                    json!({ "description": "MongoDB Extended JSON _id value" }),
                ));
                required.push("id");
            }
            _ => {}
        }
        object_schema_vec(properties, required)
    }

    fn needs_database(self) -> bool {
        !matches!(self, Self::ListConnections | Self::ListDatabases)
    }

    fn needs_collection(self) -> bool {
        !matches!(
            self,
            Self::ListConnections
                | Self::ListDatabases
                | Self::ListCollections
                | Self::DropDatabase
        )
    }
}

fn required_string(input: &Value, name: &str) -> Result<String, ToolError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed {
            message: format!("{name} must be a non-empty string"),
        })
}

fn object_schema<const P: usize, const R: usize>(
    properties: [(&str, Value); P],
    required: [&str; R],
) -> Value {
    object_schema_vec(
        properties.into_iter().collect(),
        required.into_iter().collect(),
    )
}

fn object_schema_vec(properties: Vec<(&str, Value)>, required: Vec<&str>) -> Value {
    let properties = properties
        .into_iter()
        .map(|(name, schema)| (name.to_string(), schema))
        .collect::<Map<_, _>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn string_schema(description: &str) -> Value {
    json!({ "type": "string", "minLength": 1, "description": description })
}

fn object_value_schema(description: &str) -> Value {
    json!({ "type": "object", "description": description })
}

fn nullable_object_schema(description: &str) -> Value {
    json!({ "type": ["object", "null"], "description": description })
}

fn array_object_schema(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "object" }, "description": description })
}

fn integer_schema(minimum: u64) -> Value {
    json!({ "type": "integer", "minimum": minimum })
}

struct EmptyMongoRuntime;

impl MongoConnectionSnapshotProvider for EmptyMongoRuntime {
    fn list_connections(&self) -> Vec<MongoConnectionSnapshot> {
        Vec::new()
    }
}

impl MongoOperationProvider for EmptyMongoRuntime {
    fn execute(
        &self,
        _operation: MongoOperation,
        connection_id: &str,
        _input: Value,
    ) -> tool_runtime::ToolFuture {
        let connection_id = connection_id.to_string();
        Box::pin(async move {
            Err(ToolError::Failed {
                message: format!("unknown MongoDB connection: {connection_id}"),
            })
        })
    }
}
