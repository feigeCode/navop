//! MongoDB native sidecar wire contract.
//!
//! BSON is carried as Base64 `WireBytes` so JSON framing never coerces BSON
//! numeric widths, ObjectId, Decimal128, binary subtype, or timestamps.

use serde::{Deserialize, Serialize};

use crate::blob::WireBytes;
use crate::conn::ConnId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoConnectionConfig {
    pub connection_string: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoBsonDocument {
    pub bson: WireBytes,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoFindOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<MongoBsonDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<MongoBsonDocument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoFindParams {
    pub conn_id: ConnId,
    pub database: String,
    pub collection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<MongoBsonDocument>,
    #[serde(default)]
    pub options: MongoFindOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoFindResult {
    #[serde(default)]
    pub documents: Vec<MongoBsonDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents_blob_id: Option<String>,
    #[serde(default)]
    pub document_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MongoCommandParams {
    pub conn_id: ConnId,
    pub database: String,
    pub command: MongoBsonDocument,
}
