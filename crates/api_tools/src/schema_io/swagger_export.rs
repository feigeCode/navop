use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use super::export_shared::{OperationIds, OperationTarget, insert_operation};
use super::schema_auth;
use super::schema_path::{path_parameter_rows, request_path};
use super::schema_shared::{active_server, example_schema, folder_name};
use crate::http::{BodyType, FieldType, KeyValue, RawLanguage};
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
        if request.method == crate::http::RequestMethod::Trace {
            return Err(anyhow!("Swagger 2 does not support TRACE requests"));
        }
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
        "swagger": "2.0",
        "info": {"title": name, "version": "1.0.0"},
        "paths": paths
    });
    apply_server(&mut root, active_server(store).as_deref());
    if !schemes.is_empty() {
        root["securityDefinitions"] = Value::Object(schemes);
    }
    Ok(root)
}

fn apply_server(root: &mut Value, server: Option<&str>) {
    let Some(server) = server.and_then(|value| url::Url::parse(value).ok()) else {
        return;
    };
    if let Some(host) = server.host_str() {
        root["host"] = json!(match server.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        });
    }
    root["basePath"] = json!(if server.path().is_empty() {
        "/"
    } else {
        server.path()
    });
    root["schemes"] = json!([server.scheme()]);
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
    let mut parameters = export_parameters(request, context.path);
    export_body(request, &mut parameters, &mut operation);
    if !parameters.is_empty() {
        operation.insert("parameters".into(), Value::Array(parameters));
    }
    if let Some((name, scheme)) = schema_auth::export(&request.auth, true) {
        context.schemes.insert(name.clone(), scheme);
        operation.insert(
            "security".into(),
            Value::Array(vec![Value::Object(Map::from_iter([(
                name,
                Value::Array(Vec::new()),
            )]))]),
        );
    }
    operation
}

fn export_parameters(request: &StoredRequest, path: &str) -> Vec<Value> {
    let mut parameters = Vec::new();
    push_rows(&mut parameters, &request.params, "query");
    push_rows(&mut parameters, &request.header_rows, "header");
    if let Some(cookie) = cookie_header(&request.cookies) {
        parameters.push(cookie);
    }
    push_rows(
        &mut parameters,
        &path_parameter_rows(path, &request.path_vars),
        "path",
    );
    parameters
}

fn cookie_header(rows: &[KeyValue]) -> Option<Value> {
    let value = rows
        .iter()
        .filter(|row| row.enabled && !row.key.is_empty())
        .map(|row| format!("{}={}", row.key, row.value))
        .collect::<Vec<_>>()
        .join("; ");
    if value.is_empty() {
        return None;
    }
    Some(json!({
        "name": "Cookie",
        "in": "header",
        "required": false,
        "type": "string",
        "default": value
    }))
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
                let mut parameter = json!({
                    "name": row.key,
                    "in": location,
                    "required": required,
                    "type": "string"
                });
                if !row.value.is_empty() {
                    parameter["default"] = json!(row.value);
                }
                parameter
            }),
    );
}

fn export_body(
    request: &StoredRequest,
    parameters: &mut Vec<Value>,
    operation: &mut Map<String, Value>,
) {
    match request.body_type {
        BodyType::None => {}
        BodyType::Raw => {
            operation.insert(
                "consumes".into(),
                json!([content_type(request.raw_language)]),
            );
            let example = serde_json::from_str(&request.body)
                .unwrap_or_else(|_| Value::String(request.body.clone()));
            let mut schema = example_schema(&example);
            schema["example"] = example;
            parameters.push(json!({
                "name": "body",
                "in": "body",
                "required": true,
                "schema": schema
            }));
        }
        BodyType::Urlencoded => {
            operation.insert(
                "consumes".into(),
                json!(["application/x-www-form-urlencoded"]),
            );
            push_form_rows(parameters, &request.body_rows);
        }
        BodyType::FormData => {
            operation.insert("consumes".into(), json!(["multipart/form-data"]));
            push_form_rows(parameters, &request.body_rows);
        }
    }
}

fn push_form_rows(parameters: &mut Vec<Value>, rows: &[KeyValue]) {
    parameters.extend(
        rows.iter()
            .filter(|row| row.enabled && !row.key.is_empty())
            .map(|row| {
                let field_type = if row.field_type == FieldType::File {
                    "file"
                } else {
                    "string"
                };
                let mut parameter = json!({"name": row.key, "in": "formData", "type": field_type});
                if !row.value.is_empty() && row.field_type != FieldType::File {
                    parameter["default"] = json!(row.value);
                }
                parameter
            }),
    );
}

fn content_type(language: RawLanguage) -> &'static str {
    language.content_type()
}
