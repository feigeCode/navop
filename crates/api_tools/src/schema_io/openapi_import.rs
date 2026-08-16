use anyhow::Result;
use serde_json::Value;

use super::import_shared::ImportState;
use super::schema_auth;
use super::schema_path::request_url;
use super::schema_shared::{
    OPENAPI_METHODS, combined_parameters, environment, push_parameter, raw_example, raw_language,
    resolved, rows_from_schema, sync_headers,
};
use crate::collection_io::ImportedCollection;
use crate::http::BodyType;
use crate::request_store::StoredRequest;

pub fn import(root: &Value) -> Result<ImportedCollection> {
    let name = root["info"]["title"]
        .as_str()
        .unwrap_or("Imported OpenAPI")
        .to_string();
    let server = root["servers"][0]["url"].as_str().map(str::to_string);
    let has_server = server.is_some();
    let mut state = ImportState::new(root, has_server);
    if let Some(paths) = root["paths"].as_object() {
        for (path, path_item) in paths {
            import_path(path, resolved(root, path_item), &mut state);
        }
    }
    Ok(ImportedCollection {
        name: name.clone(),
        folders: state.folders,
        requests: state.requests,
        environment: environment(&name, server),
    })
}

fn import_path(path: &str, path_item: &Value, state: &mut ImportState<'_>) {
    let root = state.root;
    let has_server = state.has_server;
    for (key, method) in OPENAPI_METHODS {
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
        for parameter in combined_parameters(root, path_item, operation) {
            push_parameter(&mut request, &parameter);
        }
        import_body(root, operation, &mut request);
        request.auth = schema_auth::import_openapi(root, operation);
        sync_headers(&mut request);
        state.requests.push(request);
    }
}

fn import_body(root: &Value, operation: &Value, request: &mut StoredRequest) {
    let body = resolved(root, &operation["requestBody"]);
    let Some(content) = body["content"].as_object() else {
        return;
    };
    let selected = preferred_media(content);
    let Some((content_type, media)) = selected else {
        return;
    };
    let media_type = base_media_type(content_type);
    if media_type == "application/x-www-form-urlencoded" {
        request.body_type = BodyType::Urlencoded;
        request.body_rows = rows_from_schema(resolved(root, &media["schema"]));
    } else if media_type == "multipart/form-data" {
        request.body_type = BodyType::FormData;
        request.body_rows = rows_from_schema(resolved(root, &media["schema"]));
    } else {
        request.body_type = BodyType::Raw;
        request.raw_language = raw_language(media_type);
        request.body = raw_example(media, request.raw_language);
    }
}

fn preferred_media(content: &serde_json::Map<String, Value>) -> Option<(&str, &Value)> {
    const PREFERRED: [&str; 3] = [
        "application/json",
        "application/x-www-form-urlencoded",
        "multipart/form-data",
    ];
    for preferred in PREFERRED {
        if let Some((content_type, media)) = content
            .iter()
            .find(|(content_type, _)| base_media_type(content_type) == preferred)
        {
            return Some((content_type, media));
        }
    }
    content
        .iter()
        .next()
        .map(|(content_type, media)| (content_type.as_str(), media))
}

fn base_media_type(content_type: &str) -> &str {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
}
