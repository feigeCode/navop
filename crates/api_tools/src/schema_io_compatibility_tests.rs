use serde_json::Value;

use super::{DocumentEncoding, export_openapi, export_swagger, import_collection};
use crate::http::{KeyValue, RequestMethod};
use crate::request_store::{ApiStore, StoredRequest};

#[test]
fn imports_openapi_trace_operations() {
    let document = r#"
openapi: 3.0.3
info: {title: Trace, version: 1.0.0}
paths:
  /diagnostics:
    trace:
      operationId: traceDiagnostics
"#;
    let imported = import_collection(document).unwrap();

    assert_eq!(imported.requests.len(), 1);
    assert_eq!(imported.requests[0].method.label(), "TRACE");
}

#[test]
fn emits_unique_operation_ids_for_duplicate_request_names() {
    let first = request("Lookup", RequestMethod::Get, "/users");
    let second = request("Lookup", RequestMethod::Get, "/teams");
    let store = store(vec![first, second]);

    for document in exported_documents(&store) {
        assert_eq!(document["paths"]["/users"]["get"]["operationId"], "lookup");
        assert_eq!(
            document["paths"]["/teams"]["get"]["operationId"],
            "lookup_2"
        );
    }
}

#[test]
fn rejects_duplicate_path_and_method_exports_instead_of_overwriting() {
    let first = request("First", RequestMethod::Get, "/users");
    let second = request("Second", RequestMethod::Get, "/users");
    let store = store(vec![first, second]);

    let openapi = export_openapi("Duplicates", &store, DocumentEncoding::Json);
    let swagger = export_swagger("Duplicates", &store, DocumentEncoding::Json);

    assert!(openapi.unwrap_err().to_string().contains("GET /users"));
    assert!(swagger.unwrap_err().to_string().contains("GET /users"));
}

#[test]
fn exports_swagger_cookies_as_a_cookie_header() {
    let mut request = request("Cookie", RequestMethod::Get, "/session");
    request.cookies = vec![
        KeyValue::new("session", "abc"),
        KeyValue::new("theme", "dark"),
    ];
    let document = exported_swagger(&store(vec![request]));
    let parameters = document["paths"]["/session"]["get"]["parameters"]
        .as_array()
        .unwrap();
    let cookie = parameters
        .iter()
        .find(|parameter| parameter["name"] == "Cookie")
        .unwrap();

    assert_eq!(cookie["in"], "header");
    assert_eq!(cookie["default"], "session=abc; theme=dark");
}

#[test]
fn imports_openapi_named_media_examples() {
    let document = r#"
openapi: 3.0.3
info: {title: Examples, version: 1.0.0}
paths:
  /echo:
    post:
      requestBody:
        content:
          application/json:
            examples:
              greeting:
                value: hello
"#;
    let imported = import_collection(document).unwrap();

    assert_eq!(
        serde_json::from_str::<Value>(&imported.requests[0].body).unwrap(),
        "hello"
    );
}

#[test]
fn resolves_openapi_path_item_references() {
    let document = r#"
openapi: 3.1.0
info: {title: Paths, version: 1.0.0}
components:
  pathItems:
    Users:
      get:
        operationId: listUsers
paths:
  /users:
    $ref: '#/components/pathItems/Users'
"#;
    let imported = import_collection(document).unwrap();

    assert_eq!(imported.requests.len(), 1);
    assert_eq!(imported.requests[0].name, "listUsers");
}

#[test]
fn accepts_openapi_documents_without_paths() {
    let document = r#"
openapi: 3.1.0
info: {title: Webhooks, version: 1.0.0}
webhooks: {}
"#;
    let imported = import_collection(document).unwrap();

    assert!(imported.requests.is_empty());
}

fn request(name: &str, method: RequestMethod, url: &str) -> StoredRequest {
    let mut request = StoredRequest::new(name, method);
    request.url = url.into();
    request
}

fn store(requests: Vec<StoredRequest>) -> ApiStore {
    ApiStore {
        requests,
        ..ApiStore::default()
    }
}

fn exported_documents(store: &ApiStore) -> [Value; 2] {
    [exported_openapi(store), exported_swagger(store)]
}

fn exported_openapi(store: &ApiStore) -> Value {
    let text = export_openapi("Compatibility", store, DocumentEncoding::Json).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn exported_swagger(store: &ApiStore) -> Value {
    let text = export_swagger("Compatibility", store, DocumentEncoding::Json).unwrap();
    serde_json::from_str(&text).unwrap()
}
