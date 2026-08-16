use serde_json::Value;

use super::{
    CollectionFormat, DocumentEncoding, detect_collection_format, export_openapi, export_swagger,
    import_collection,
};
use crate::http::{BodyType, KeyValue, RawLanguage, RequestMethod};
use crate::request_store::{ApiStore, StoredRequest};

#[test]
fn detects_unquoted_numeric_schema_versions() {
    assert_eq!(
        detect_collection_format("openapi: 3.0\ninfo: {}\npaths: {}").unwrap(),
        CollectionFormat::OpenApi3
    );
    assert_eq!(
        detect_collection_format("swagger: 2.0\ninfo: {}\npaths: {}").unwrap(),
        CollectionFormat::Swagger2
    );
}

#[test]
fn exports_absolute_url_templates_as_schema_paths() {
    let mut request = StoredRequest::new("Get user", RequestMethod::Get);
    request.url = "https://api.example.com/users/{{id}}?draft=true".into();
    request.path_vars = vec![KeyValue::new("id", "42")];
    let store = store_with(request);

    for document in exported_documents(&store) {
        assert!(document["paths"]["/users/{id}"]["get"].is_object());
        assert!(document["paths"]["/users/%7B%7Bid%7D%7D"].is_null());
    }
}

#[test]
fn imports_parameterized_openapi_form_media_types() {
    let document = r#"
openapi: 3.0.3
info: {title: Forms, version: 1.0.0}
paths:
  /upload:
    post:
      requestBody:
        content:
          multipart/form-data; boundary=navop:
            schema:
              type: object
              properties:
                asset:
                  type: string
                  format: binary
  /login:
    post:
      requestBody:
        content:
          application/x-www-form-urlencoded; charset=utf-8:
            schema:
              type: object
              properties:
                username:
                  type: string
                  example: navop
"#;
    let imported = import_collection(document).unwrap();
    let login = imported
        .requests
        .iter()
        .find(|request| request.url.ends_with("/login"))
        .unwrap();
    let upload = imported
        .requests
        .iter()
        .find(|request| request.url.ends_with("/upload"))
        .unwrap();

    assert_eq!(login.body_type, BodyType::Urlencoded);
    assert_eq!(login.body_rows, vec![KeyValue::new("username", "navop")]);
    assert_eq!(upload.body_type, BodyType::FormData);
    assert_eq!(upload.body_rows[0].field_type, crate::http::FieldType::File);
}

#[test]
fn preserves_json_string_examples_as_valid_json() {
    let document = r#"
openapi: 3.0.3
info: {title: Strings, version: 1.0.0}
paths:
  /echo:
    post:
      requestBody:
        content:
          application/json:
            example: hello
"#;
    let imported = import_collection(document).unwrap();
    let request = &imported.requests[0];

    assert_eq!(request.raw_language, RawLanguage::Json);
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).unwrap(),
        "hello"
    );
}

#[test]
fn imports_swagger_body_language_from_consumes() {
    let document = r#"
swagger: '2.0'
info: {title: XML, version: 1.0.0}
consumes: [application/xml]
paths:
  /echo:
    post:
      parameters:
        - name: body
          in: body
          schema:
            type: string
            example: <message>Hello</message>
"#;
    let imported = import_collection(document).unwrap();
    let request = &imported.requests[0];

    assert_eq!(request.body_type, BodyType::Raw);
    assert_eq!(request.raw_language, RawLanguage::Xml);
    assert_eq!(request.body, "<message>Hello</message>");
}

#[test]
fn exports_only_path_parameters_present_in_the_template() {
    let mut request = StoredRequest::new("Get user", RequestMethod::Get);
    request.url = "/users/{{id}}".into();
    request.path_vars = vec![KeyValue::new("stale", "unused")];
    let store = store_with(request);

    for document in exported_documents(&store) {
        let parameters = document["paths"]["/users/{id}"]["get"]["parameters"]
            .as_array()
            .unwrap();
        let path_parameters = parameters
            .iter()
            .filter(|parameter| parameter["in"] == "path")
            .collect::<Vec<_>>();
        assert_eq!(path_parameters.len(), 1);
        assert_eq!(path_parameters[0]["name"], "id");
        assert_ne!(path_parameters[0]["name"], "stale");
    }
}

#[test]
fn exports_json_body_schema_matching_the_example_type() {
    let mut request = StoredRequest::new("Array body", RequestMethod::Post);
    request.url = "/items".into();
    request.body_type = BodyType::Raw;
    request.raw_language = RawLanguage::Json;
    request.body = "[1, 2]".into();
    let store = store_with(request);

    let openapi = exported_openapi(&store);
    assert_eq!(
        openapi["paths"]["/items"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["type"],
        "array"
    );
    let swagger = exported_swagger(&store);
    assert_eq!(
        swagger["paths"]["/items"]["post"]["parameters"][0]["schema"]["type"],
        "array"
    );
}

#[test]
fn resolves_chained_local_references_for_request_bodies_and_schemas() {
    let document = r#"
openapi: 3.0.3
info: {title: References, version: 1.0.0}
components:
  requestBodies:
    Alias:
      $ref: '#/components/requestBodies/Actual'
    Actual:
      content:
        application/x-www-form-urlencoded:
          schema:
            $ref: '#/components/schemas/Alias'
  schemas:
    Alias:
      $ref: '#/components/schemas/Actual'
    Actual:
      type: object
      properties:
        name:
          type: string
          example: Navop
paths:
  /users:
    post:
      requestBody:
        $ref: '#/components/requestBodies/Alias'
"#;
    let imported = import_collection(document).unwrap();
    let request = &imported.requests[0];

    assert_eq!(request.body_type, BodyType::Urlencoded);
    assert_eq!(request.body_rows, vec![KeyValue::new("name", "Navop")]);
}

fn store_with(request: StoredRequest) -> ApiStore {
    ApiStore {
        requests: vec![request],
        ..ApiStore::default()
    }
}

fn exported_documents(store: &ApiStore) -> [Value; 2] {
    [exported_openapi(store), exported_swagger(store)]
}

fn exported_openapi(store: &ApiStore) -> Value {
    let text = export_openapi("Regression", store, DocumentEncoding::Json).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn exported_swagger(store: &ApiStore) -> Value {
    let text = export_swagger("Regression", store, DocumentEncoding::Json).unwrap();
    serde_json::from_str(&text).unwrap()
}
