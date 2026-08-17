use crate::collection_io::{export_postman_v2_1, import_postman_v2_1};
use crate::http::{
    AuthTarget, AuthType, BodyType, FieldType, KeyValue, RawLanguage, RequestMethod,
};
use crate::request_store::{ApiEnvironment, ApiStore, StoredFolder};

#[test]
fn imports_nested_requests_parameters_auth_and_files() {
    let json = r#"{
      "info": {"name": "Demo"},
      "variable": [{"key": "baseUrl", "value": "https://api.test", "disabled": false}],
      "item": [{
        "name": "Users",
        "item": [{
          "name": "Upload",
          "request": {
            "method": "POST",
            "url": {
              "raw": "{{baseUrl}}/users/:id?draft=true",
              "query": [{"key": "draft", "value": "true", "disabled": true}],
              "variable": [{"key": "id", "value": "42"}]
            },
            "header": [{"key": "X-Trace", "value": "1"}],
            "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}"}]},
            "body": {
              "mode": "formdata",
              "formdata": [
                {"key": "label", "value": "avatar", "type": "text"},
                {"key": "file", "type": "file", "src": "/tmp/avatar.png"}
              ]
            }
          }
        }]
      }]
    }"#;

    let imported = import_postman_v2_1(json).unwrap();

    assert_eq!(imported.name, "Demo");
    assert_eq!(imported.folders.len(), 1);
    assert_eq!(imported.requests.len(), 1);
    let request = &imported.requests[0];
    assert_eq!(request.method, RequestMethod::Post);
    assert_eq!(request.body_type, BodyType::FormData);
    assert_eq!(request.auth.auth_type, AuthType::Bearer);
    assert_eq!(request.path_vars[0].key, "id");
    assert!(!request.params[0].enabled);
    assert_eq!(request.body_rows[1].field_type, FieldType::File);
    assert_eq!(
        request.body_rows[1].file_path.as_deref(),
        Some("/tmp/avatar.png")
    );
    let environment = imported.environment.unwrap();
    assert_eq!(environment.base_url.as_deref(), Some("https://api.test"));
    assert!(environment.variables.iter().all(|row| row.key != "baseUrl"));
}

#[test]
fn imports_raw_body_language_and_api_key_target() {
    let json = r#"{
      "info": {"name": "Raw"},
      "item": [{
        "name": "Create",
        "request": {
          "method": "PATCH",
          "url": "https://api.test/items",
          "auth": {
            "type": "apikey",
            "apikey": [
              {"key": "key", "value": "X-Key"},
              {"key": "value", "value": "secret"},
              {"key": "in", "value": "query"}
            ]
          },
          "body": {
            "mode": "raw",
            "raw": "<item />",
            "options": {"raw": {"language": "xml"}}
          }
        }
      }]
    }"#;

    let imported = import_postman_v2_1(json).unwrap();
    let request = &imported.requests[0];

    assert_eq!(request.body_type, BodyType::Raw);
    assert_eq!(request.raw_language, RawLanguage::Xml);
    assert_eq!(request.auth.auth_type, AuthType::ApiKey);
    assert_eq!(request.auth.add_to, AuthTarget::Query);
}

#[test]
fn export_then_import_preserves_nested_http_request_details() {
    let folder = StoredFolder::new("Users", None);
    let mut request = crate::request_store::StoredRequest::new("Create", RequestMethod::Post);
    request.folder_id = Some(folder.id.clone());
    request.url = "{{baseUrl}}/users".into();
    request.body_type = BodyType::Urlencoded;
    request.body_rows = vec![crate::http::KeyValue::new("name", "Navop")];
    request.auth.auth_type = AuthType::Basic;
    request.auth.username = "user".into();
    request.auth.password = "pass".into();
    let store = ApiStore {
        folders: vec![folder],
        requests: vec![request],
        ..ApiStore::default()
    };

    let json = export_postman_v2_1("Navop", &store).unwrap();
    let imported = import_postman_v2_1(&json).unwrap();

    assert!(json.contains("https://schema.getpostman.com/json/collection/v2.1.0/collection.json"));
    assert_eq!(imported.folders[0].name, "Users");
    assert_eq!(imported.requests[0].body_type, BodyType::Urlencoded);
    assert_eq!(imported.requests[0].auth.auth_type, AuthType::Basic);
    assert_eq!(imported.requests[0].body_rows[0].value, "Navop");
}

#[test]
fn postman_export_uses_the_environment_base_url_as_the_canonical_variable() {
    let mut environment = ApiEnvironment::new("Production");
    environment.base_url = Some("https://api.current.test".into());
    environment.variables = vec![
        KeyValue::new("baseUrl", "https://api.stale.test"),
        KeyValue::new("tenant", "navop"),
    ];
    let store = ApiStore {
        active_environment_id: Some(environment.id.clone()),
        environments: vec![environment],
        ..ApiStore::default()
    };

    let json = export_postman_v2_1("Navop", &store).unwrap();
    let exported: serde_json::Value = serde_json::from_str(&json).unwrap();
    let variables = exported["variable"].as_array().unwrap();

    assert_eq!(
        variables
            .iter()
            .filter(|variable| variable["key"] == "baseUrl")
            .count(),
        1
    );
    assert_eq!(
        variables
            .iter()
            .find(|variable| variable["key"] == "baseUrl")
            .unwrap()["value"],
        "https://api.current.test"
    );
    assert!(variables.iter().any(|variable| variable["key"] == "tenant"));
}

#[test]
fn invalid_postman_json_returns_an_error() {
    assert!(import_postman_v2_1("{not-json").is_err());
}
