use serde_json::Value;

use super::{
    CollectionFormat, DocumentEncoding, detect_collection_format, export_openapi, export_swagger,
    import_collection,
};
use crate::http::{
    AuthTarget, AuthType, BodyType, FieldType, KeyValue, RawLanguage, RequestMethod,
};
use crate::request_store::{ApiEnvironment, ApiStore, StoredFolder, StoredRequest};

const OPENAPI_YAML: &str = r#"
openapi: 3.0.3
info:
  title: Pet API
  version: 1.0.0
servers:
  - url: https://api.example.com/v1
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
paths:
  /users/{id}:
    parameters:
      - name: id
        in: path
        required: true
        schema:
          type: string
          example: "42"
      - name: limit
        in: query
        schema:
          type: integer
          example: 5
    post:
      tags: [Users]
      operationId: updateUser
      parameters:
        - name: limit
          in: query
          schema:
            type: integer
            example: 10
        - name: X-Trace
          in: header
          schema:
            type: string
            example: trace-1
      security:
        - bearerAuth: []
      requestBody:
        content:
          application/json:
            example:
              name: Navop
"#;

const SWAGGER_JSON: &str = r#"{
  "swagger": "2.0",
  "info": {"title": "Upload API", "version": "1.0.0"},
  "host": "upload.example.com",
  "basePath": "/v2",
  "schemes": ["https"],
  "securityDefinitions": {
    "queryKey": {"type": "apiKey", "name": "api_key", "in": "query"}
  },
  "paths": {
    "/files/{id}": {
      "post": {
        "tags": ["Files"],
        "summary": "Upload file",
        "security": [{"queryKey": []}],
        "consumes": ["multipart/form-data"],
        "parameters": [
          {"name": "id", "in": "path", "required": true, "type": "string", "default": "7"},
          {"name": "caption", "in": "formData", "type": "string", "default": "cover"},
          {"name": "asset", "in": "formData", "type": "file"}
        ]
      }
    }
  }
}"#;

#[test]
fn detects_json_and_yaml_collection_formats() {
    assert_eq!(
        detect_collection_format(OPENAPI_YAML).unwrap(),
        CollectionFormat::OpenApi3
    );
    assert_eq!(
        detect_collection_format(SWAGGER_JSON).unwrap(),
        CollectionFormat::Swagger2
    );
    assert_eq!(
        detect_collection_format(
            r#"{"info":{"schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#
        )
        .unwrap(),
        CollectionFormat::PostmanV2_1
    );
    assert!(detect_collection_format("name: unknown").is_err());
}

#[test]
fn imports_openapi_parameters_body_auth_and_server() {
    let imported = import_collection(OPENAPI_YAML).unwrap();
    let request = &imported.requests[0];

    assert_eq!(imported.name, "Pet API");
    assert_eq!(imported.folders[0].name, "Users");
    assert_eq!(request.name, "updateUser");
    assert_eq!(request.method, RequestMethod::Post);
    assert_eq!(request.url, "{{baseUrl}}/users/{{id}}");
    assert_eq!(request.path_vars[0], KeyValue::new("id", "42"));
    assert_eq!(request.params[0], KeyValue::new("limit", "10"));
    assert_eq!(request.header_rows[0], KeyValue::new("X-Trace", "trace-1"));
    assert_eq!(request.body_type, BodyType::Raw);
    assert_eq!(request.raw_language, RawLanguage::Json);
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).unwrap()["name"],
        "Navop"
    );
    assert_eq!(request.auth.auth_type, AuthType::Bearer);
    assert!(request.auth.token.is_empty());
    assert_eq!(
        imported.environment.unwrap().variables[0],
        KeyValue::new("baseUrl", "https://api.example.com/v1")
    );
}

#[test]
fn imports_swagger_form_file_and_query_api_key() {
    let imported = import_collection(SWAGGER_JSON).unwrap();
    let request = &imported.requests[0];

    assert_eq!(request.url, "{{baseUrl}}/files/{{id}}");
    assert_eq!(request.path_vars[0], KeyValue::new("id", "7"));
    assert_eq!(request.body_type, BodyType::FormData);
    assert_eq!(
        request
            .body_rows
            .iter()
            .find(|row| row.key == "caption")
            .unwrap(),
        &KeyValue::new("caption", "cover")
    );
    assert_eq!(
        request
            .body_rows
            .iter()
            .find(|row| row.key == "asset")
            .unwrap()
            .field_type,
        FieldType::File
    );
    assert_eq!(request.auth.auth_type, AuthType::ApiKey);
    assert_eq!(request.auth.key, "api_key");
    assert_eq!(request.auth.add_to, AuthTarget::Query);
    assert_eq!(
        imported.environment.unwrap().variables[0].value,
        "https://upload.example.com/v2"
    );
}

#[test]
fn exports_openapi_json_and_yaml_with_http_semantics() {
    let store = sample_store();
    let json = export_openapi("Navop API", &store, DocumentEncoding::Json).unwrap();
    let doc: Value = serde_json::from_str(&json).unwrap();
    let operation = &doc["paths"]["/users/{id}"]["post"];

    assert_eq!(doc["openapi"], "3.0.3");
    assert_eq!(doc["servers"][0]["url"], "https://api.example.com/v1");
    assert_eq!(operation["tags"][0], "Users");
    assert_eq!(operation["parameters"][0]["in"], "query");
    assert_eq!(
        operation["requestBody"]["content"]["multipart/form-data"]["schema"]["properties"]["asset"]
            ["format"],
        "binary"
    );
    let scheme_name = operation["security"][0]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    assert_eq!(
        doc["components"]["securitySchemes"][scheme_name]["in"],
        "query"
    );

    let yaml = export_openapi("Navop API", &store, DocumentEncoding::Yaml).unwrap();
    assert_eq!(
        detect_collection_format(&yaml).unwrap(),
        CollectionFormat::OpenApi3
    );
}

#[test]
fn exports_swagger_form_data_and_round_trips() {
    let store = sample_store();
    let json = export_swagger("Navop API", &store, DocumentEncoding::Json).unwrap();
    let doc: Value = serde_json::from_str(&json).unwrap();
    let operation = &doc["paths"]["/users/{id}"]["post"];

    assert_eq!(doc["swagger"], "2.0");
    assert_eq!(doc["host"], "api.example.com");
    assert_eq!(doc["basePath"], "/v1");
    assert_eq!(operation["consumes"][0], "multipart/form-data");
    assert!(
        operation["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|parameter| parameter["name"] == "asset"
                && parameter["in"] == "formData"
                && parameter["type"] == "file")
    );

    let imported = import_collection(&json).unwrap();
    let request = &imported.requests[0];
    assert_eq!(request.method, RequestMethod::Post);
    assert_eq!(request.url, "{{baseUrl}}/users/{{id}}");
    assert_eq!(request.body_type, BodyType::FormData);
    assert_eq!(request.auth.auth_type, AuthType::ApiKey);
    assert_eq!(request.auth.add_to, AuthTarget::Query);
}

fn sample_store() -> ApiStore {
    let folder = StoredFolder::new("Users", None);
    let mut request = StoredRequest::new("Upload user asset", RequestMethod::Post);
    request.folder_id = Some(folder.id.clone());
    request.url = "{{baseUrl}}/users/{{id}}".into();
    request.params = vec![KeyValue::new("draft", "true")];
    request.path_vars = vec![KeyValue::new("id", "42")];
    request.body_type = BodyType::FormData;
    let mut asset = KeyValue::new("asset", "");
    asset.field_type = FieldType::File;
    request.body_rows = vec![KeyValue::new("caption", "avatar"), asset];
    request.auth.auth_type = AuthType::ApiKey;
    request.auth.key = "api_key".into();
    request.auth.add_to = AuthTarget::Query;
    ApiStore {
        folders: vec![folder],
        requests: vec![request],
        environments: vec![ApiEnvironment {
            id: "env".into(),
            name: "Production".into(),
            variables: vec![KeyValue::new("baseUrl", "https://api.example.com/v1")],
        }],
        active_environment_id: Some("env".into()),
        ..ApiStore::default()
    }
}
