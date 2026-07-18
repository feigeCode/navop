use async_trait::async_trait;
use base64::Engine as _;
use bson::{Bson, Document, doc};
use extension_host::{HostError, NativeDriverManifest, ProcessRpcSession};
use extension_protocol::blob::WireBytes;
use extension_protocol::blob::{BlobReadResult, DEFAULT_BLOB_CHUNK_BYTES};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::method;
use extension_protocol::mongodb::{
    MongoBsonDocument, MongoCommandParams, MongoFindParams, MongoFindResult,
};
use std::sync::Arc;

use crate::{MongoConnection, MongoConnectionConfig, MongoError, MongoFindOptions};

/// IPC facade placeholder for the UI/runtime boundary.
///
/// The native sidecar protocol is already defined and the modern/legacy
/// sidecars share the generic process session. CRUD methods are deliberately
/// explicit here so enabling the default-off feature never links the MongoDB
/// SDK into the UI crate; the wire-backed implementations can be filled in
/// without changing the UI trait.
pub struct IpcMongoConnection {
    config: MongoConnectionConfig,
    manifest: Option<NativeDriverManifest>,
    session: Option<Arc<ProcessRpcSession>>,
    conn_id: Option<u64>,
}

impl IpcMongoConnection {
    pub fn new(config: MongoConnectionConfig) -> Self {
        Self {
            config,
            manifest: None,
            session: None,
            conn_id: None,
        }
    }

    pub fn with_manifest(manifest: NativeDriverManifest, config: MongoConnectionConfig) -> Self {
        Self {
            config,
            manifest: Some(manifest),
            session: None,
            conn_id: None,
        }
    }
}

#[async_trait]
impl MongoConnection for IpcMongoConnection {
    fn config(&self) -> &MongoConnectionConfig {
        &self.config
    }
    async fn connect(&mut self) -> Result<(), MongoError> {
        let manifest = self.manifest.clone().ok_or_else(|| {
            MongoError::Internal("MongoDB native driver is not configured".into())
        })?;
        let session = Arc::new(
            ProcessRpcSession::start(manifest.process_session_config(
                env!("CARGO_PKG_VERSION"),
                uuid::Uuid::new_v4().to_string(),
            ))
            .await
            .map_err(host_error)?,
        );
        let wire = extension_protocol::mongodb::MongoConnectionConfig {
            connection_string: self.config.connection_string.clone(),
            direct_host: Some(self.config.direct_host.clone()),
            direct_port: Some(self.config.direct_port),
        };
        let open = ConnOpenParams::new(
            manifest.id,
            serde_json::to_value(wire).map_err(serialization_error)?,
        );
        let result: ConnOpenResult = session
            .request(
                method::CONN_OPEN,
                serde_json::to_value(open).map_err(serialization_error)?,
            )
            .await
            .map_err(host_error)?;
        self.session = Some(session);
        self.conn_id = Some(result.conn_id);
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<(), MongoError> {
        if let (Some(session), Some(conn_id)) = (&self.session, self.conn_id) {
            let _ = session
                .request_value(method::CONN_CLOSE, serde_json::json!({"conn_id": conn_id}))
                .await;
            session.shutdown().await;
        }
        self.session = None;
        self.conn_id = None;
        Ok(())
    }
    async fn ping(&self) -> Result<(), MongoError> {
        self.command_document("admin", doc! { "ping": 1 })
            .await
            .map(|_| ())
    }
    fn is_connected(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| !session.is_closed())
    }
    async fn list_databases(&self) -> Result<Vec<String>, MongoError> {
        let result = self
            .command_document("admin", doc! { "listDatabases": 1, "nameOnly": true })
            .await?;
        bson_documents(&result, "databases")?
            .into_iter()
            .map(|document| bson_string(&document, "name"))
            .collect()
    }
    async fn list_collections(&self, database: &str) -> Result<Vec<String>, MongoError> {
        let result = self
            .command_document(database, doc! { "listCollections": 1, "nameOnly": true })
            .await?;
        cursor_first_batch(&result)?
            .into_iter()
            .map(|document| bson_string(&document, "name"))
            .collect()
    }
    async fn create_collection(&self, database: &str, collection: &str) -> Result<(), MongoError> {
        self.command_document(database, doc! { "create": collection })
            .await
            .map(|_| ())
    }
    async fn drop_database(&self, database: &str) -> Result<(), MongoError> {
        self.command_document(database, doc! { "dropDatabase": 1 })
            .await
            .map(|_| ())
    }
    async fn aggregate_documents(
        &self,
        database: &str,
        collection: &str,
        pipeline: Vec<Document>,
    ) -> Result<Vec<Document>, MongoError> {
        let result = self
            .command_document(
                database,
                doc! { "aggregate": collection, "pipeline": pipeline, "cursor": {} },
            )
            .await?;
        cursor_first_batch(&result)
    }
    async fn list_indexes(
        &self,
        database: &str,
        collection: &str,
    ) -> Result<Vec<Document>, MongoError> {
        let result = self
            .command_document(database, doc! { "listIndexes": collection, "cursor": {} })
            .await?;
        cursor_first_batch(&result)
    }
    async fn create_index(
        &self,
        database: &str,
        collection: &str,
        keys: Document,
        name: Option<String>,
    ) -> Result<(), MongoError> {
        let name = name.unwrap_or_else(|| index_name(&keys));
        self.command_document(
            database,
            doc! { "createIndexes": collection, "indexes": [{ "key": keys, "name": name }] },
        )
        .await
        .map(|_| ())
    }
    async fn drop_index(
        &self,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<(), MongoError> {
        self.command_document(database, doc! { "dropIndexes": collection, "index": name })
            .await
            .map(|_| ())
    }
    async fn get_collection_validation(
        &self,
        database: &str,
        collection: &str,
    ) -> Result<Option<Document>, MongoError> {
        let result = self
            .command_document(
                database,
                doc! { "listCollections": 1, "filter": { "name": collection } },
            )
            .await?;
        let mut batch = cursor_first_batch(&result)?;
        Ok(batch
            .pop()
            .and_then(|document| document.get_document("options").ok().cloned())
            .and_then(|options| options.get_document("validator").ok().cloned()))
    }
    async fn update_collection_validation(
        &self,
        database: &str,
        collection: &str,
        validator: Option<Document>,
    ) -> Result<(), MongoError> {
        let mut command = doc! { "collMod": collection };
        command.insert(
            "validator",
            validator
                .map(Bson::Document)
                .unwrap_or(Bson::Document(Document::new())),
        );
        self.command_document(database, command).await.map(|_| ())
    }
    async fn find_documents(
        &self,
        database: &str,
        collection: &str,
        filter: Option<Document>,
        options: MongoFindOptions,
    ) -> Result<Vec<Document>, MongoError> {
        let session = self.session.as_ref().ok_or(MongoError::NotConnected)?;
        let conn_id = self.conn_id.ok_or(MongoError::NotConnected)?;
        let params = MongoFindParams {
            conn_id,
            database: database.into(),
            collection: collection.into(),
            filter: filter.map(document_wire),
            options: extension_protocol::mongodb::MongoFindOptions {
                limit: options.limit,
                skip: options.skip,
                sort: options.sort.map(document_wire),
                projection: options.projection.map(document_wire),
            },
        };
        let result: MongoFindResult = session
            .request(
                method::MONGODB_FIND,
                serde_json::to_value(params).map_err(serialization_error)?,
            )
            .await
            .map_err(host_error)?;
        if let Some(blob_id) = result.documents_blob_id {
            let mut packed = Vec::new();
            let mut done = false;
            while !done {
                let chunk: BlobReadResult = session
                    .request(
                        method::BLOB_READ,
                        serde_json::json!({
                            "blob_id": blob_id,
                            "max_bytes": DEFAULT_BLOB_CHUNK_BYTES
                        }),
                    )
                    .await
                    .map_err(host_error)?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(chunk.data)
                    .map_err(serialization_error)?;
                packed.extend_from_slice(&bytes);
                done = chunk.done;
            }
            let _ = session
                .request_value(
                    method::BLOB_CLOSE,
                    serde_json::json!({ "blob_id": blob_id }),
                )
                .await;
            decode_packed_documents(&packed)
        } else {
            result.documents.into_iter().map(decode_document).collect()
        }
    }
    async fn count_documents(
        &self,
        database: &str,
        collection: &str,
        filter: Option<Document>,
    ) -> Result<i64, MongoError> {
        let result = self
            .command_document(
                database,
                doc! { "count": collection, "query": filter.unwrap_or_default() },
            )
            .await?;
        bson_i64(&result, "n")
    }
    async fn insert_document(
        &self,
        database: &str,
        collection: &str,
        document: Document,
    ) -> Result<(), MongoError> {
        self.command_document(
            database,
            doc! { "insert": collection, "documents": [document] },
        )
        .await
        .map(|_| ())
    }
    async fn replace_document(
        &self,
        database: &str,
        collection: &str,
        id: Bson,
        document: Document,
    ) -> Result<(), MongoError> {
        self.command_document(
            database,
            doc! { "update": collection, "updates": [{ "q": { "_id": id }, "u": document }] },
        )
        .await
        .map(|_| ())
    }
    async fn update_document_fields(
        &self,
        database: &str,
        collection: &str,
        id: Bson,
        fields: Document,
    ) -> Result<(), MongoError> {
        self.command_document(
            database,
            doc! { "update": collection, "updates": [{ "q": { "_id": id }, "u": { "$set": fields } }] },
        )
        .await
        .map(|_| ())
    }
    async fn delete_document(
        &self,
        database: &str,
        collection: &str,
        id: Bson,
    ) -> Result<(), MongoError> {
        self.command_document(
            database,
            doc! { "delete": collection, "deletes": [{ "q": { "_id": id }, "limit": 1 }] },
        )
        .await
        .map(|_| ())
    }
    async fn explain_find(
        &self,
        database: &str,
        collection: &str,
        filter: Option<Document>,
        options: MongoFindOptions,
    ) -> Result<Document, MongoError> {
        let mut find = doc! { "find": collection, "filter": filter.unwrap_or_default() };
        if let Some(limit) = options.limit {
            find.insert("limit", limit);
        }
        if let Some(skip) = options.skip {
            find.insert("skip", skip);
        }
        if let Some(sort) = options.sort {
            find.insert("sort", sort);
        }
        if let Some(projection) = options.projection {
            find.insert("projection", projection);
        }
        self.command_document(database, doc! { "explain": find })
            .await
    }
}

impl IpcMongoConnection {
    async fn command_document(
        &self,
        database: &str,
        command: Document,
    ) -> Result<Document, MongoError> {
        let session = self.session.as_ref().ok_or(MongoError::NotConnected)?;
        let conn_id = self.conn_id.ok_or(MongoError::NotConnected)?;
        let params = MongoCommandParams {
            conn_id,
            database: database.into(),
            command: document_wire(command),
        };
        let result: MongoBsonDocument = session
            .request(
                method::MONGODB_COMMAND,
                serde_json::to_value(params).map_err(serialization_error)?,
            )
            .await
            .map_err(host_error)?;
        decode_document(result)
    }
}

fn document_wire(document: Document) -> MongoBsonDocument {
    MongoBsonDocument {
        bson: WireBytes::Base64(
            base64::engine::general_purpose::STANDARD
                .encode(bson::to_vec(&document).unwrap_or_default()),
        ),
    }
}

fn decode_document(document: MongoBsonDocument) -> Result<Document, MongoError> {
    let WireBytes::Base64(value) = document.bson else {
        return Err(MongoError::Serialization(
            "MongoDB BSON must be Base64".into(),
        ));
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(serialization_error)?;
    bson::from_slice(&bytes).map_err(serialization_error)
}

fn decode_packed_documents(bytes: &[u8]) -> Result<Vec<Document>, MongoError> {
    let mut offset = 0;
    let mut documents = Vec::new();
    while offset < bytes.len() {
        if bytes.len() - offset < 4 {
            return Err(MongoError::Serialization(
                "truncated BSON blob length".into(),
            ));
        }
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if bytes.len() - offset < length {
            return Err(MongoError::Serialization(
                "truncated BSON blob document".into(),
            ));
        }
        documents
            .push(bson::from_slice(&bytes[offset..offset + length]).map_err(serialization_error)?);
        offset += length;
    }
    Ok(documents)
}

#[cfg(test)]
mod blob_tests {
    use super::*;

    #[test]
    fn packed_bson_documents_round_trip() {
        let first = bson::to_vec(&doc! { "n": 1 }).unwrap();
        let second = bson::to_vec(&doc! { "n": 2 }).unwrap();
        let mut packed = Vec::new();
        for bytes in [&first, &second] {
            packed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            packed.extend_from_slice(bytes);
        }
        let decoded = decode_packed_documents(&packed).unwrap();
        assert_eq!(2, decoded.len());
        assert_eq!(Some(&Bson::Int32(1)), decoded[0].get("n"));
        assert_eq!(Some(&Bson::Int32(2)), decoded[1].get("n"));
    }
}

fn host_error(error: HostError) -> MongoError {
    match error {
        HostError::Protocol(error)
            if error.code == extension_protocol::error::error_codes::SERVER_INCOMPATIBLE =>
        {
            MongoError::ServerIncompatible(error.message.clone())
        }
        other => MongoError::connection(other.to_string()),
    }
}

fn serialization_error(error: impl std::fmt::Display) -> MongoError {
    MongoError::Serialization(error.to_string())
}

fn cursor_first_batch(result: &Document) -> Result<Vec<Document>, MongoError> {
    result
        .get_document("cursor")
        .map_err(|error| MongoError::Serialization(error.to_string()))?
        .get_array("firstBatch")
        .map_err(|error| MongoError::Serialization(error.to_string()))?
        .iter()
        .map(|value| match value {
            Bson::Document(document) => Ok(document.clone()),
            other => Err(MongoError::Serialization(format!(
                "expected BSON document in cursor batch, got {other:?}"
            ))),
        })
        .collect()
}

fn bson_documents(result: &Document, field: &str) -> Result<Vec<Document>, MongoError> {
    result
        .get_array(field)
        .map_err(|error| MongoError::Serialization(error.to_string()))?
        .iter()
        .map(|value| match value {
            Bson::Document(document) => Ok(document.clone()),
            other => Err(MongoError::Serialization(format!(
                "expected BSON document in `{field}`, got {other:?}"
            ))),
        })
        .collect()
}

fn bson_string(document: &Document, field: &str) -> Result<String, MongoError> {
    document
        .get_str(field)
        .map(str::to_string)
        .map_err(|error| MongoError::Serialization(error.to_string()))
}

fn bson_i64(document: &Document, field: &str) -> Result<i64, MongoError> {
    document
        .get_i64(field)
        .or_else(|_| document.get_i32(field).map(i64::from))
        .map_err(|error| MongoError::Serialization(error.to_string()))
}

fn index_name(keys: &Document) -> String {
    keys.iter()
        .map(|(field, value)| format!("{field}_{}", value.as_i32().unwrap_or(1)))
        .collect::<Vec<_>>()
        .join("_")
}
