#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use base64::Engine as _;
use bson::Document;
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async_from_env,
};
use extension_protocol::blob::WireBytes;
use extension_protocol::blob::{BlobReadParams, BlobReadResult};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::lifecycle::{Capability, InitResult};
use extension_protocol::method;
use extension_protocol::mongodb::MongoFindResult;
use extension_protocol::mongodb::{MongoCommandParams, MongoConnectionConfig, MongoFindParams};
use futures_util::TryStreamExt;
use mongodb::{Client, Database};
use serde_json::Value;

const SOCKET_ENV: &str = "ONETCLI_EXT_SOCKET";

pub async fn run(variant: &'static str) -> anyhow::Result<()> {
    serve_async_from_env(
        MongoDriver {
            variant,
            next_conn_id: AtomicU64::new(1),
        },
        SOCKET_ENV,
    )
    .await
}

struct MongoDriver {
    variant: &'static str,
    next_conn_id: AtomicU64,
}

struct MongoConnection {
    client: Client,
    blobs: HashMap<String, (Vec<u8>, usize)>,
}

#[async_trait]
impl AsyncNativeDriver for MongoDriver {
    async fn init(&self, _params: &Value) -> Result<Value, ProtocolError> {
        let result = InitResult::new(env!("CARGO_PKG_VERSION"))
            .with_api("mongodb", extension_protocol::WIRE_PROTOCOL_VERSION)
            .with_feature(Capability::CANCEL_REQUEST)
            .with_method(method::MONGODB_COMMAND)
            .with_method(method::MONGODB_FIND)
            .with_driver(format!("mongodb-{}", self.variant));
        serde_json::to_value(result).map_err(internal_error)
    }

    async fn open_connection(
        &self,
        params: &Value,
    ) -> Result<AsyncOpenedConnection, ProtocolError> {
        let open: ConnOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let config: MongoConnectionConfig =
            serde_json::from_value(open.config).map_err(invalid_params)?;
        let client = Client::with_uri_str(&config.connection_string)
            .await
            .map_err(connection_error)?;
        let build_info = client
            .database("admin")
            .run_command(bson::doc! { "buildInfo": 1 })
            .await
            .map_err(|error| variant_connection_error(self.variant, error))?;
        if self.variant == "modern"
            && build_info
                .get_array("versionArray")
                .ok()
                .and_then(|version| version.first())
                .and_then(bson::Bson::as_i32)
                .is_some_and(|major| major < 4)
        {
            return Err(ProtocolError::new(
                error_codes::SERVER_INCOMPATIBLE,
                "MongoDB server requires the legacy driver",
            ));
        }
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        Ok(AsyncOpenedConnection {
            conn_id,
            open_result: serde_json::to_value(ConnOpenResult {
                conn_id,
                server_info: None,
            })
            .map_err(internal_error)?,
            connection: Box::new(MongoConnection {
                client,
                blobs: HashMap::new(),
            }),
        })
    }

    async fn call_connless(
        &self,
        method_name: &str,
        _params: &Value,
    ) -> Result<Value, ProtocolError> {
        Err(ProtocolError::new(
            error_codes::METHOD_NOT_FOUND,
            format!("unsupported MongoDB method `{method_name}`"),
        ))
    }
}

#[async_trait]
impl AsyncDriverConnection for MongoConnection {
    async fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::MONGODB_COMMAND => {
                let params: MongoCommandParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let command = decode_document(params.command.bson)?;
                let result = self
                    .database(&params.database)
                    .run_command(command)
                    .await
                    .map_err(command_error)?;
                serde_json::to_value(encode_document(&result)?).map_err(internal_error)
            }
            method::MONGODB_FIND => {
                let params: MongoFindParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let filter = params
                    .filter
                    .map(|document| decode_document(document.bson))
                    .transpose()?
                    .unwrap_or_default();
                let collection = self
                    .client
                    .database(&params.database)
                    .collection::<Document>(&params.collection);
                let options = mongodb::options::FindOptions::builder()
                    .limit(params.options.limit)
                    .skip(params.options.skip.map(|skip| skip.max(0) as u64))
                    .sort(
                        params
                            .options
                            .sort
                            .map(|document| decode_document(document.bson))
                            .transpose()?,
                    )
                    .projection(
                        params
                            .options
                            .projection
                            .map(|document| decode_document(document.bson))
                            .transpose()?,
                    )
                    .build();
                let mut cursor = collection
                    .find(filter)
                    .with_options(options)
                    .await
                    .map_err(command_error)?;
                let mut documents = Vec::new();
                let mut packed = Vec::new();
                while let Some(document) = cursor.try_next().await.map_err(command_error)? {
                    let bytes = bson::to_vec(&document).map_err(internal_error)?;
                    packed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                    packed.extend_from_slice(&bytes);
                    documents.push(encode_document(&document)?);
                }
                if packed.len() as u64 > extension_protocol::blob::INLINE_BLOB_THRESHOLD_BYTES {
                    let document_count = documents.len() as u64;
                    let blob_id = format!("mongo-docs-{}", BLOB_ID.fetch_add(1, Ordering::Relaxed));
                    self.blobs.insert(blob_id.clone(), (packed, 0));
                    documents.clear();
                    return serde_json::to_value(MongoFindResult {
                        documents,
                        documents_blob_id: Some(blob_id),
                        document_count,
                        cursor_id: None,
                    })
                    .map_err(internal_error);
                }
                let document_count = documents.len() as u64;
                serde_json::to_value(MongoFindResult {
                    documents,
                    documents_blob_id: None,
                    document_count,
                    cursor_id: None,
                })
                .map_err(internal_error)
            }
            extension_protocol::method::BLOB_READ => {
                let params: BlobReadParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let Some((bytes, offset)) = self.blobs.get_mut(&params.blob_id) else {
                    return Err(ProtocolError::new(
                        error_codes::RESOURCE_CLOSED,
                        "blob is closed",
                    ));
                };
                let end = (*offset + params.effective_max_bytes() as usize).min(bytes.len());
                let chunk = &bytes[*offset..end];
                *offset = end;
                serde_json::to_value(BlobReadResult {
                    data: base64::engine::general_purpose::STANDARD.encode(chunk),
                    bytes_read: chunk.len() as u32,
                    done: end == bytes.len(),
                })
                .map_err(internal_error)
            }
            extension_protocol::method::BLOB_CLOSE => {
                let blob_id = params
                    .get("blob_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_params("blob_id is required"))?;
                self.blobs.remove(blob_id);
                Ok(Value::Null)
            }
            _ => Err(ProtocolError::new(
                error_codes::METHOD_NOT_FOUND,
                format!("unsupported MongoDB method `{method_name}`"),
            )),
        }
    }

    async fn close(&mut self) {
        self.blobs.clear();
    }
}

static BLOB_ID: AtomicU64 = AtomicU64::new(1);

impl MongoConnection {
    fn database(&self, name: &str) -> Database {
        self.client.database(name)
    }
}

fn decode_document(value: WireBytes) -> Result<Document, ProtocolError> {
    let bytes = match value {
        WireBytes::Utf8(value) => value.into_bytes(),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(invalid_params)?,
    };
    bson::from_slice(&bytes).map_err(invalid_params)
}

fn encode_document(
    document: &Document,
) -> Result<extension_protocol::mongodb::MongoBsonDocument, ProtocolError> {
    let bytes = bson::to_vec(document).map_err(internal_error)?;
    Ok(extension_protocol::mongodb::MongoBsonDocument {
        bson: WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(bytes)),
    })
}

fn invalid_params(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, error.to_string())
}
fn connection_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, error.to_string())
}

fn variant_connection_error(variant: &str, error: impl std::fmt::Display) -> ProtocolError {
    let message = error.to_string();
    if variant == "modern"
        && (message.contains("wire version")
            || message.contains("incompatible server")
            || message.contains("requires at least"))
    {
        ProtocolError::new(error_codes::SERVER_INCOMPATIBLE, message)
    } else {
        ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, message)
    }
}
fn command_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::EXTENSION_CUSTOM_START, error.to_string())
}
fn internal_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INTERNAL_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bson_document_round_trip_is_binary_safe() {
        let document = bson::doc! { "n": bson::Bson::Int64(i64::MAX), "ok": true };
        let wire = encode_document(&document).unwrap();
        assert!(matches!(wire.bson, WireBytes::Base64(_)));
    }
}
