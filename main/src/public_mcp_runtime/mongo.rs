use gpui::App;
use mongodb_view::bson::{Bson, Document};
use mongodb_view::{GlobalMongoState, MongoFindOptions as FindOptions};
use public_mcp::tools::{
    MongoConnectionSnapshot, MongoConnectionSnapshotProvider, MongoOperation,
    MongoOperationProvider, MongoToolProvider,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{ToolError, ToolHandler, ToolResult};

pub(super) fn mongo_tool_handlers(cx: &App) -> Vec<Arc<dyn ToolHandler>> {
    match cx.try_global::<GlobalMongoState>().cloned() {
        Some(state) => MongoToolProvider::handlers(Arc::new(MongoRuntimeAdapter { state })),
        None => {
            tracing::warn!(
                "Public MCP MongoDB toolset enabled before MongoDB state is initialized"
            );
            MongoToolProvider::empty()
        }
    }
}

struct MongoRuntimeAdapter {
    state: GlobalMongoState,
}

impl MongoConnectionSnapshotProvider for MongoRuntimeAdapter {
    fn list_connections(&self) -> Vec<MongoConnectionSnapshot> {
        self.state
            .connection_ids()
            .into_iter()
            .map(|connection_id| MongoConnectionSnapshot { connection_id })
            .collect()
    }
}

impl MongoOperationProvider for MongoRuntimeAdapter {
    fn execute(
        &self,
        operation: MongoOperation,
        connection_id: &str,
        input: Value,
    ) -> tool_runtime::ToolFuture {
        let state = self.state.clone();
        let connection_id = connection_id.to_string();
        Box::pin(async move {
            let connection =
                state
                    .get_connection(&connection_id)
                    .ok_or_else(|| ToolError::Failed {
                        message: format!("unknown MongoDB connection: {connection_id}"),
                    })?;
            let guard = connection.read().await;
            let database = optional_string(&input, "database")?;
            let collection = optional_string(&input, "collection")?;
            let result = match operation {
                MongoOperation::ListConnections => unreachable!("handled synchronously"),
                MongoOperation::ListDatabases => json!({
                    "connection_id": connection_id,
                    "databases": guard.list_databases().await.map_err(tool_error)?
                }),
                MongoOperation::ListCollections => json!({
                    "connection_id": connection_id,
                    "database": database,
                    "collections": guard.list_collections(required(&database, "database")?).await.map_err(tool_error)?
                }),
                MongoOperation::Find => {
                    let documents = guard
                        .find_documents(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            optional_document(&input, "filter")?,
                            find_options(&input)?,
                        )
                        .await
                        .map_err(tool_error)?;
                    json!({ "connection_id": connection_id, "database": database, "collection": collection, "documents": documents_json(documents) })
                }
                MongoOperation::Aggregate => {
                    let documents = guard
                        .aggregate_documents(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_pipeline(&input)?,
                        )
                        .await
                        .map_err(tool_error)?;
                    json!({ "connection_id": connection_id, "database": database, "collection": collection, "documents": documents_json(documents) })
                }
                MongoOperation::Count => json!({
                    "connection_id": connection_id,
                    "database": database,
                    "collection": collection,
                    "count": guard.count_documents(
                        required(&database, "database")?,
                        required(&collection, "collection")?,
                        optional_document(&input, "filter")?,
                    ).await.map_err(tool_error)?
                }),
                MongoOperation::ListIndexes => json!({
                    "connection_id": connection_id,
                    "database": database,
                    "collection": collection,
                    "indexes": documents_json(guard.list_indexes(
                        required(&database, "database")?,
                        required(&collection, "collection")?,
                    ).await.map_err(tool_error)?)
                }),
                MongoOperation::CreateIndex => {
                    guard
                        .create_index(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_document(&input, "keys")?,
                            optional_string(&input, "name")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::DropIndex => {
                    let name = required_input_string(&input, "name")?;
                    guard
                        .drop_index(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            &name,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::CreateCollection => {
                    guard
                        .create_collection(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::DropDatabase => {
                    guard
                        .drop_database(required(&database, "database")?)
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::GetValidation => json!({
                    "connection_id": connection_id,
                    "database": database,
                    "collection": collection,
                    "validator": guard.get_collection_validation(
                        required(&database, "database")?,
                        required(&collection, "collection")?,
                    ).await.map_err(tool_error)?.map(document_json)
                }),
                MongoOperation::SetValidation => {
                    let validator = if input.get("validator").is_some_and(Value::is_null) {
                        None
                    } else {
                        optional_document(&input, "validator")?
                    };
                    guard
                        .update_collection_validation(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            validator,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::Insert => {
                    guard
                        .insert_document(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_document(&input, "document")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::Replace => {
                    guard
                        .replace_document(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_bson(&input, "id")?,
                            required_document(&input, "document")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::Update => {
                    guard
                        .update_document_fields(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_bson(&input, "id")?,
                            required_document(&input, "set")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::Delete => {
                    guard
                        .delete_document(
                            required(&database, "database")?,
                            required(&collection, "collection")?,
                            required_bson(&input, "id")?,
                        )
                        .await
                        .map_err(tool_error)?;
                    mutation_result(&connection_id, &database, &collection)
                }
                MongoOperation::Explain => json!({
                    "connection_id": connection_id,
                    "database": database,
                    "collection": collection,
                    "explain": document_json(guard.explain_find(
                        required(&database, "database")?,
                        required(&collection, "collection")?,
                        optional_document(&input, "filter")?,
                        find_options(&input)?,
                    ).await.map_err(tool_error)?)
                }),
            };
            Ok(ToolResult::structured(result))
        })
    }
}

fn optional_string(input: &Value, name: &str) -> Result<Option<String>, ToolError> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        _ => Err(invalid(format!("{name} must be a non-empty string"))),
    }
}

fn required_input_string(input: &Value, name: &str) -> Result<String, ToolError> {
    optional_string(input, name)?.ok_or_else(|| invalid(format!("{name} is required")))
}

fn required<'a>(value: &'a Option<String>, name: &str) -> Result<&'a str, ToolError> {
    value
        .as_deref()
        .ok_or_else(|| invalid(format!("{name} is required")))
}

fn optional_document(input: &Value, name: &str) -> Result<Option<Document>, ToolError> {
    input.get(name).map(json_document).transpose()
}

fn required_document(input: &Value, name: &str) -> Result<Document, ToolError> {
    input
        .get(name)
        .ok_or_else(|| invalid(format!("{name} is required")))
        .and_then(json_document)
}

fn required_bson(input: &Value, name: &str) -> Result<Bson, ToolError> {
    let value = input
        .get(name)
        .ok_or_else(|| invalid(format!("{name} is required")))?;
    Bson::try_from(value.clone()).map_err(|error| invalid(format!("invalid {name}: {error}")))
}

fn json_document(value: &Value) -> Result<Document, ToolError> {
    match Bson::try_from(value.clone()).map_err(|error| invalid(error.to_string()))? {
        Bson::Document(document) => Ok(document),
        _ => Err(invalid("MongoDB document must be a JSON object")),
    }
}

fn required_pipeline(input: &Value) -> Result<Vec<Document>, ToolError> {
    input
        .get("pipeline")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("pipeline must be an array"))?
        .iter()
        .map(json_document)
        .collect()
}

fn find_options(input: &Value) -> Result<FindOptions, ToolError> {
    let mut options = FindOptions::default();
    options.sort = optional_document(input, "sort")?;
    options.projection = optional_document(input, "projection")?;
    options.skip = optional_i64(input, "skip")?;
    options.limit = optional_i64(input, "limit")?;
    Ok(options)
}

fn optional_i64(input: &Value, name: &str) -> Result<Option<i64>, ToolError> {
    input
        .get(name)
        .map(|value| {
            value
                .as_i64()
                .filter(|value| *value >= 0)
                .ok_or_else(|| invalid(format!("{name} must be a non-negative integer")))
        })
        .transpose()
}

fn documents_json(documents: Vec<Document>) -> Vec<Value> {
    documents.into_iter().map(document_json).collect()
}

fn document_json(document: Document) -> Value {
    Bson::Document(document).into_relaxed_extjson()
}

fn mutation_result(
    connection_id: &str,
    database: &Option<String>,
    collection: &Option<String>,
) -> Value {
    json!({ "ok": true, "connection_id": connection_id, "database": database, "collection": collection })
}

fn tool_error(error: mongodb_view::MongoError) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::Failed {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::find_options;
    use serde_json::json;

    #[test]
    fn find_options_use_runtime_signed_paging_contract() {
        let options = find_options(&json!({"skip": 10, "limit": 25})).unwrap();

        assert_eq!(Some(10), options.skip);
        assert_eq!(Some(25), options.limit);
    }

    #[test]
    fn find_options_reject_negative_paging_values() {
        assert!(find_options(&json!({"skip": -1})).is_err());
        assert!(find_options(&json!({"limit": -1})).is_err());
    }
}
