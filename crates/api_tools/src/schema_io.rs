use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::collection_io::ImportedCollection;
use crate::request_store::ApiStore;

mod export_shared;
mod import_shared;
mod openapi_export;
mod openapi_import;
mod schema_auth;
mod schema_path;
mod schema_shared;
mod swagger_export;
mod swagger_import;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionFormat {
    PostmanV2_1,
    OpenApi3,
    Swagger2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentEncoding {
    Json,
    Yaml,
}

pub fn detect_collection_format(text: &str) -> Result<CollectionFormat> {
    let root = parse_document(text)?;
    if version_matches(&root["openapi"], "3") {
        return Ok(CollectionFormat::OpenApi3);
    }
    if version_matches(&root["swagger"], "2") {
        return Ok(CollectionFormat::Swagger2);
    }
    let schema = root["info"]["schema"].as_str().unwrap_or_default();
    if schema.contains("postman") || root["item"].is_array() {
        return Ok(CollectionFormat::PostmanV2_1);
    }
    Err(anyhow!("unsupported collection format"))
}

fn version_matches(value: &Value, expected_major: &str) -> bool {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_number().map(ToString::to_string))
        .is_some_and(|version| version.starts_with(expected_major))
}

pub fn import_collection(text: &str) -> Result<ImportedCollection> {
    match detect_collection_format(text)? {
        CollectionFormat::PostmanV2_1 => crate::collection_io::import_postman_v2_1(text),
        CollectionFormat::OpenApi3 => openapi_import::import(&parse_document(text)?),
        CollectionFormat::Swagger2 => swagger_import::import(&parse_document(text)?),
    }
}

pub fn export_openapi(name: &str, store: &ApiStore, encoding: DocumentEncoding) -> Result<String> {
    encode_document(&openapi_export::export(name, store)?, encoding)
}

pub fn export_swagger(name: &str, store: &ApiStore, encoding: DocumentEncoding) -> Result<String> {
    encode_document(&swagger_export::export(name, store)?, encoding)
}

fn parse_document(text: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)?;
    Ok(serde_json::to_value(yaml)?)
}

fn encode_document(value: &Value, encoding: DocumentEncoding) -> Result<String> {
    match encoding {
        DocumentEncoding::Json => Ok(serde_json::to_string_pretty(value)?),
        DocumentEncoding::Yaml => Ok(serde_yaml::to_string(value)?),
    }
}

#[cfg(test)]
#[path = "schema_io_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "schema_io_regression_tests.rs"]
mod regression_tests;

#[cfg(test)]
#[path = "schema_io_compatibility_tests.rs"]
mod compatibility_tests;
