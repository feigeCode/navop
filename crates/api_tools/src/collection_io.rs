use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::http::{
    AuthConfig, AuthTarget, AuthType, BodyType, FieldType, KeyValue, RawLanguage, RequestMethod,
};
use crate::request_store::{ApiEnvironment, ApiStore, StoredFolder, StoredRequest};

pub struct ImportedCollection {
    pub name: String,
    pub folders: Vec<StoredFolder>,
    pub requests: Vec<StoredRequest>,
    pub environment: Option<ApiEnvironment>,
}

pub fn import_postman_v2_1(text: &str) -> Result<ImportedCollection> {
    let root: Value = serde_json::from_str(text)?;
    let name = root["info"]["name"]
        .as_str()
        .unwrap_or("Imported Collection")
        .to_string();
    let environment = import_variables(&root, &name);
    let mut folders = Vec::new();
    let mut requests = Vec::new();
    for item in root["item"]
        .as_array()
        .ok_or_else(|| anyhow!("Postman collection has no item array"))?
    {
        import_item(item, None, &mut folders, &mut requests)?;
    }
    Ok(ImportedCollection {
        name,
        folders,
        requests,
        environment,
    })
}

fn import_variables(root: &Value, name: &str) -> Option<ApiEnvironment> {
    let variables = root["variable"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            if item["disabled"].as_bool().unwrap_or(false) {
                return None;
            }
            Some(KeyValue::new(
                item["key"].as_str()?,
                item["value"].as_str().unwrap_or_default(),
            ))
        })
        .collect::<Vec<_>>();
    (!variables.is_empty()).then(|| ApiEnvironment {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: name.to_string(),
        variables,
    })
}

fn import_item(
    item: &Value,
    parent_id: Option<String>,
    folders: &mut Vec<StoredFolder>,
    requests: &mut Vec<StoredRequest>,
) -> Result<()> {
    let children = item["item"].as_array().map_or(&[][..], Vec::as_slice);
    let current_parent = if children.is_empty() {
        parent_id
    } else {
        let folder = StoredFolder::new(item["name"].as_str().unwrap_or("Folder"), parent_id);
        let id = folder.id.clone();
        folders.push(folder);
        if item.get("request").is_some() {
            requests.push(import_request(item, Some(id.clone()))?);
        }
        for child in children {
            import_item(child, Some(id.clone()), folders, requests)?;
        }
        return Ok(());
    };
    if item.get("request").is_some() {
        requests.push(import_request(item, current_parent)?);
    }
    Ok(())
}

fn import_request(item: &Value, folder_id: Option<String>) -> Result<StoredRequest> {
    let request = &item["request"];
    let method = parse_method(request["method"].as_str().unwrap_or("GET"));
    let mut stored = StoredRequest::new(item["name"].as_str().unwrap_or("Request"), method);
    stored.folder_id = folder_id;
    stored.url = postman_url_raw(&request["url"]);
    stored.params = postman_rows(request["url"]["query"].as_array());
    stored.path_vars = postman_rows(request["url"]["variable"].as_array());
    stored.header_rows = postman_rows(request["header"].as_array());
    stored.headers = stored
        .header_rows
        .iter()
        .filter(|row| !row.key.is_empty())
        .map(|row| format!("{}: {}", row.key, row.value))
        .collect::<Vec<_>>()
        .join("\n");
    stored.cookies = postman_rows(request["cookie"].as_array());
    stored.auth = import_auth(&request["auth"]);
    import_body(request.get("body"), &mut stored);
    Ok(stored)
}

fn postman_url_raw(url: &Value) -> String {
    url.as_str()
        .map(str::to_string)
        .or_else(|| url["raw"].as_str().map(str::to_string))
        .unwrap_or_default()
}

fn postman_rows(items: Option<&Vec<Value>>) -> Vec<KeyValue> {
    items
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let key = item["key"].as_str()?.to_string();
            let field_type = (item["type"].as_str() == Some("file"))
                .then_some(FieldType::File)
                .unwrap_or_default();
            Some(KeyValue {
                key,
                value: item["value"].as_str().unwrap_or_default().to_string(),
                enabled: !item["disabled"].as_bool().unwrap_or(false),
                field_type,
                file_path: item["src"].as_str().map(str::to_string),
            })
        })
        .collect()
}

fn import_body(body: Option<&Value>, stored: &mut StoredRequest) {
    let Some(body) = body else { return };
    match body["mode"].as_str().unwrap_or("none") {
        "formdata" => {
            stored.body_type = BodyType::FormData;
            stored.body_rows = postman_rows(body["formdata"].as_array());
        }
        "urlencoded" => {
            stored.body_type = BodyType::Urlencoded;
            stored.body_rows = postman_rows(body["urlencoded"].as_array());
        }
        "raw" | "json" | "xml" | "text" | "html" | "javascript" => {
            stored.body_type = BodyType::Raw;
            stored.body = body["raw"].as_str().unwrap_or_default().to_string();
            stored.raw_language = match body["options"]["raw"]["language"].as_str() {
                Some("xml") => RawLanguage::Xml,
                Some("text") => RawLanguage::Text,
                Some("html") => RawLanguage::Html,
                Some("javascript") => RawLanguage::Javascript,
                _ if body["mode"] == "xml" => RawLanguage::Xml,
                _ if body["mode"] == "text" => RawLanguage::Text,
                _ => RawLanguage::Json,
            };
        }
        _ => {}
    }
}

fn import_auth(auth: &Value) -> AuthConfig {
    let mut result = AuthConfig::default();
    match auth["type"].as_str() {
        Some("bearer") => {
            result.auth_type = AuthType::Bearer;
            result.token = auth_value(auth["bearer"].as_array(), "token");
        }
        Some("basic") => {
            result.auth_type = AuthType::Basic;
            result.username = auth_value(auth["basic"].as_array(), "username");
            result.password = auth_value(auth["basic"].as_array(), "password");
        }
        Some("apikey") => {
            result.auth_type = AuthType::ApiKey;
            result.key = auth_value(auth["apikey"].as_array(), "key");
            result.value = auth_value(auth["apikey"].as_array(), "value");
            result.add_to = if auth_value(auth["apikey"].as_array(), "in") == "query" {
                AuthTarget::Query
            } else {
                AuthTarget::Header
            };
        }
        _ => {}
    }
    result
}

fn auth_value(items: Option<&Vec<Value>>, key: &str) -> String {
    items
        .unwrap_or(&Vec::new())
        .iter()
        .find(|item| item["key"].as_str() == Some(key))
        .and_then(|item| item["value"].as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn export_postman_v2_1(name: &str, store: &ApiStore) -> Result<String> {
    let mut items = Vec::new();
    for request in store
        .requests
        .iter()
        .filter(|request| request.folder_id.is_none())
    {
        items.push(export_request_item(request));
    }
    for folder in store
        .folders
        .iter()
        .filter(|folder| folder.parent_id.is_none())
    {
        items.push(export_folder(folder, store));
    }
    let variables = store
        .active_environment_id
        .as_deref()
        .and_then(|id| store.environments.iter().find(|env| env.id == id))
        .or_else(|| store.environments.first())
        .map(|env| {
            env.variables
                .iter()
                .filter(|row| row.enabled && !row.key.is_empty())
                .map(|row| json!({"key": row.key, "value": row.value}))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "info": {
            "_postman_id": uuid::Uuid::new_v4().to_string(),
            "name": name,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": items,
        "variable": variables
    }))?)
}

fn export_folder(folder: &StoredFolder, store: &ApiStore) -> Value {
    let requests = store
        .requests
        .iter()
        .filter(|request| request.folder_id.as_deref() == Some(folder.id.as_str()))
        .map(export_request_item);
    let folders = store
        .folders
        .iter()
        .filter(|child| child.parent_id.as_deref() == Some(folder.id.as_str()))
        .map(|child| export_folder(child, store));
    json!({"name": folder.name, "item": requests.chain(folders).collect::<Vec<_>>()})
}

fn export_request_item(request: &StoredRequest) -> Value {
    let mut result = json!({
        "name": request.name,
        "request": {
            "method": request.method.label(),
            "header": export_rows(&request.header_rows),
            "url": {
                "raw": request.url,
                "query": export_rows(&request.params),
                "variable": export_rows(&request.path_vars)
            },
            "cookie": export_rows(&request.cookies),
            "body": export_body(request)
        }
    });
    if request.auth.auth_type != AuthType::None {
        result["request"]["auth"] = export_auth(&request.auth);
    }
    result
}

fn export_rows(rows: &[KeyValue]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let mut value = json!({"key": row.key, "value": row.value});
            if !row.enabled {
                value["disabled"] = json!(true);
            }
            if row.field_type == FieldType::File {
                value["type"] = json!("file");
                if let Some(path) = &row.file_path {
                    value["src"] = json!(path);
                }
            }
            value
        })
        .collect()
}

fn export_body(request: &StoredRequest) -> Value {
    match request.body_type {
        BodyType::None => json!({"mode": "none"}),
        BodyType::FormData => {
            json!({"mode": "formdata", "formdata": export_rows(&request.body_rows)})
        }
        BodyType::Urlencoded => {
            json!({"mode": "urlencoded", "urlencoded": export_rows(&request.body_rows)})
        }
        BodyType::Raw => json!({
            "mode": match request.raw_language {
                RawLanguage::Json => "raw",
                RawLanguage::Xml => "xml",
                RawLanguage::Text => "text",
                RawLanguage::Html => "html",
                RawLanguage::Javascript => "javascript",
            },
            "raw": request.body,
            "options": {"raw": {"language": request.raw_language.label().to_ascii_lowercase()}}
        }),
    }
}

fn export_auth(auth: &AuthConfig) -> Value {
    match auth.auth_type {
        AuthType::Bearer => {
            json!({"type": "bearer", "bearer": [{"key": "token", "value": auth.token}]})
        }
        AuthType::Basic => json!({"type": "basic", "basic": [
            {"key": "username", "value": auth.username},
            {"key": "password", "value": auth.password}
        ]}),
        AuthType::ApiKey => json!({"type": "apikey", "apikey": [
            {"key": "key", "value": auth.key},
            {"key": "value", "value": auth.value},
            {"key": "in", "value": if auth.add_to == AuthTarget::Query {"query"} else {"header"}}
        ]}),
        AuthType::None => json!({}),
    }
}

fn parse_method(method: &str) -> RequestMethod {
    match method.to_ascii_uppercase().as_str() {
        "POST" => RequestMethod::Post,
        "PUT" => RequestMethod::Put,
        "DELETE" => RequestMethod::Delete,
        "PATCH" => RequestMethod::Patch,
        "HEAD" => RequestMethod::Head,
        "OPTIONS" => RequestMethod::Options,
        "TRACE" => RequestMethod::Trace,
        _ => RequestMethod::Get,
    }
}

#[cfg(test)]
#[path = "collection_io_tests.rs"]
mod tests;
