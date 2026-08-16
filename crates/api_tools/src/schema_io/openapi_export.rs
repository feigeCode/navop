use anyhow::Result;
use serde_json::{Map, Value, json};

use super::export_shared::{OperationIds, OperationTarget, insert_operation};
use super::schema_auth;
use super::schema_path::{path_parameter_rows, request_path};
use super::schema_shared::{
    active_server, example_schema, folder_name, object_schema, parameter_schema,
};
use crate::http::{BodyType, KeyValue};
use crate::request_store::{ApiStore, StoredRequest};

struct OperationContext<'a> {
    path: &'a str,
    operation_id: &'a str,
    tag: Option<&'a str>,
    schemes: &'a mut Map<String, Value>,
}

pub fn export(name: &str, store: &ApiStore) -> Result<Value> {
    let mut paths = Map::new();
    let mut schemes = Map::new();
    let mut operation_ids = OperationIds::default();
    for request in &store.requests {
        let path = request_path(&request.url);
        let operation_id = operation_ids.next(&request.name, request.method);
        let operation = operation(
            request,
            &mut OperationContext {
                path: &path,
                operation_id: &operation_id,
                tag: folder_name(store, request),
                schemes: &mut schemes,
            },
        );
        insert_operation(
            &mut paths,
            OperationTarget {
                path,
                method: request.method.label().to_ascii_lowercase(),
            },
            operation,
        )?;
    }
    let mut root = json!({
        "openapi": "3.0.3",
        "info": {"title": name, "version": "1.0.0"},
        "paths": paths
    });
    if let Some(server) = active_server(store) {
        root["servers"] = json!([{"url": server}]);
    }
    if !schemes.is_empty() {
        root["components"] = json!({"securitySchemes": schemes});
    }
    Ok(root)
}

fn operation(request: &StoredRequest, context: &mut OperationContext<'_>) -> Map<String, Value> {
    let mut operation = Map::from_iter([
        ("summary".into(), json!(request.name)),
        ("operationId".into(), json!(context.operation_id)),
        ("responses".into(), json!({"200": {"description": "OK"}})),
    ]);
    if let Some(tag) = context.tag {
        operation.insert("tags".into(), json!([tag]));
    }
    let parameters = export_parameters(request, context.path);
    if !parameters.is_empty() {
        operation.insert("parameters".into(), Value::Array(parameters));
    }
    if let Some(body) = export_body(request) {
        operation.insert("requestBody".into(), body);
    }
    if let Some((name, scheme)) = schema_auth::export(&request.auth, false) {
        context.schemes.insert(name.clone(), scheme);
        operation.insert("security".into(), security_requirement(name));
    }
    operation
}

fn export_parameters(request: &StoredRequest, path: &str) -> Vec<Value> {
    let mut parameters = Vec::new();
    push_rows(&mut parameters, &request.params, "query");
    push_rows(&mut parameters, &request.header_rows, "header");
    push_rows(&mut parameters, &request.cookies, "cookie");
    push_rows(
        &mut parameters,
        &path_parameter_rows(path, &request.path_vars),
        "path",
    );
    parameters
}

fn push_rows(parameters: &mut Vec<Value>, rows: &[KeyValue], location: &str) {
    let required = location == "path";
    parameters.extend(
        rows.iter()
            .filter(|row| row.enabled && !row.key.is_empty())
            .filter(|row| {
                location != "header"
                    || (!row.key.eq_ignore_ascii_case("content-type")
                        && !row.key.eq_ignore_ascii_case("host"))
            })
            .map(|row| {
                json!({
                    "name": row.key,
                    "in": location,
                    "required": required,
                    "schema": parameter_schema(row)
                })
            }),
    );
}

fn export_body(request: &StoredRequest) -> Option<Value> {
    match request.body_type {
        BodyType::None => None,
        BodyType::Raw => {
            let content_type = request.raw_language.content_type();
            let example = serde_json::from_str(&request.body)
                .unwrap_or_else(|_| Value::String(request.body.clone()));
            let schema = example_schema(&example);
            Some(request_body(
                content_type,
                json!({"schema": schema, "example": example}),
            ))
        }
        BodyType::Urlencoded => Some(request_body(
            "application/x-www-form-urlencoded",
            json!({"schema": object_schema(&request.body_rows)}),
        )),
        BodyType::FormData => Some(request_body(
            "multipart/form-data",
            json!({"schema": object_schema(&request.body_rows)}),
        )),
    }
}

fn request_body(content_type: &str, media: Value) -> Value {
    let content = Map::from_iter([(content_type.to_string(), media)]);
    json!({"content": content})
}

fn security_requirement(name: String) -> Value {
    Value::Array(vec![Value::Object(Map::from_iter([(
        name,
        Value::Array(Vec::new()),
    )]))])
}
