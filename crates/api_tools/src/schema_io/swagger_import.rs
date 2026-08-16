use anyhow::{Result, anyhow};
use serde_json::Value;

use super::import_shared::ImportState;
use super::schema_auth;
use super::schema_path::request_url;
use super::schema_shared::{
    SWAGGER_METHODS, combined_parameters, environment, push_parameter, raw_example, raw_language,
    resolved, sync_headers,
};
use crate::collection_io::ImportedCollection;
use crate::http::{BodyType, FieldType, KeyValue};
use crate::request_store::StoredRequest;

struct ParameterSource<'a> {
    root: &'a Value,
    path_item: &'a Value,
    operation: &'a Value,
}

pub fn import(root: &Value) -> Result<ImportedCollection> {
    let name = root["info"]["title"]
        .as_str()
        .unwrap_or("Imported Swagger")
        .to_string();
    let server = swagger_server(root);
    let has_server = server.is_some();
    let mut state = ImportState::new(root, has_server);
    let paths = root["paths"]
        .as_object()
        .ok_or_else(|| anyhow!("Swagger document has no paths object"))?;
    for (path, path_item) in paths {
        import_path(path, path_item, &mut state);
    }
    Ok(ImportedCollection {
        name: name.clone(),
        folders: state.folders,
        requests: state.requests,
        environment: environment(&name, server),
    })
}

fn swagger_server(root: &Value) -> Option<String> {
    let host = root["host"].as_str()?;
    let scheme = root["schemes"][0].as_str().unwrap_or("https");
    let base_path = root["basePath"].as_str().unwrap_or_default();
    Some(format!("{scheme}://{host}{base_path}"))
}

fn import_path(path: &str, path_item: &Value, state: &mut ImportState<'_>) {
    let root = state.root;
    let has_server = state.has_server;
    for (key, method) in SWAGGER_METHODS {
        let operation = &path_item[key];
        if !operation.is_object() {
            continue;
        }
        let name = operation["operationId"]
            .as_str()
            .or_else(|| operation["summary"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} {path}", method.label()));
        let mut request = StoredRequest::new(name, method);
        request.url = request_url(path, has_server);
        request.folder_id = state.folder_id(operation["tags"][0].as_str());
        import_parameters(
            ParameterSource {
                root,
                path_item,
                operation,
            },
            &mut request,
        );
        request.auth = schema_auth::import_swagger(root, operation);
        sync_headers(&mut request);
        state.requests.push(request);
    }
}

fn import_parameters(source: ParameterSource<'_>, request: &mut StoredRequest) {
    let consumes = source.operation["consumes"]
        .as_array()
        .or_else(|| source.root["consumes"].as_array());
    for parameter in combined_parameters(source.root, source.path_item, source.operation) {
        match parameter["in"].as_str() {
            Some("body") => {
                let schema = resolved(source.root, &parameter["schema"]);
                let content_type = consumes
                    .and_then(|items| items.iter().find_map(Value::as_str))
                    .unwrap_or("application/json");
                import_body(schema, content_type, request);
            }
            Some("formData") => import_form_parameter(&parameter, consumes, request),
            _ => push_parameter(request, &parameter),
        }
    }
}

fn import_body(schema: &Value, content_type: &str, request: &mut StoredRequest) {
    request.body_type = BodyType::Raw;
    request.raw_language = raw_language(content_type);
    request.body = raw_example(&serde_json::json!({"schema": schema}), request.raw_language);
}

fn import_form_parameter(
    parameter: &Value,
    consumes: Option<&Vec<Value>>,
    request: &mut StoredRequest,
) {
    request.body_type = if consumes.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.as_str() == Some("application/x-www-form-urlencoded"))
    }) {
        BodyType::Urlencoded
    } else {
        BodyType::FormData
    };
    let Some(name) = parameter["name"].as_str() else {
        return;
    };
    let mut row = KeyValue::new(
        name,
        parameter
            .get("default")
            .or_else(|| parameter.get("x-example"))
            .map(super::schema_shared::value_string)
            .unwrap_or_default(),
    );
    if parameter["type"].as_str() == Some("file") {
        row.field_type = FieldType::File;
    }
    request.body_rows.push(row);
}
