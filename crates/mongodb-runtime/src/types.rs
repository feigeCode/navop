//! MongoDB 核心类型定义

use bson::{Bson, Document};
use connection_tunnel::SshTunnelConfig;
use thiserror::Error;

/// MongoDB 错误类型
#[derive(Debug, Error)]
pub enum MongoError {
    #[error("Connection error: {message}")]
    Connection {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Command error: {message}")]
    Command {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Invalid filter: {0}")]
    InvalidFilter(String),

    #[error("Not connected to MongoDB")]
    NotConnected,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("MongoDB server is incompatible with the selected driver: {0}")]
    ServerIncompatible(String),
}

impl MongoError {
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connection {
            message: message.into(),
            source: None,
        }
    }

    pub fn connection_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Connection {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn command(message: impl Into<String>) -> Self {
        Self::Command {
            message: message.into(),
            source: None,
        }
    }

    pub fn command_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Command {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// MongoDB 连接配置
#[derive(Clone, Debug)]
pub struct MongoConnectionConfig {
    pub id: String,
    pub name: String,
    pub connection_string: String,
    pub direct_host: String,
    pub direct_port: u16,
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

/// MongoDB 树形节点类型
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MongoNodeType {
    Connection,
    Database,
    Collection,
}

/// MongoDB 树形节点
#[derive(Clone, Debug)]
pub struct MongoNode {
    pub id: String,
    pub name: String,
    pub node_type: MongoNodeType,
    pub connection_id: String,
    pub database_name: Option<String>,
    pub collection_name: Option<String>,
    pub children: Vec<MongoNode>,
    pub children_loaded: bool,
}

impl MongoNode {
    pub fn new_connection(id: String, name: String) -> Self {
        Self {
            id: id.clone(),
            name,
            node_type: MongoNodeType::Connection,
            connection_id: id,
            database_name: None,
            collection_name: None,
            children: Vec::new(),
            children_loaded: false,
        }
    }

    pub fn new_database(
        id: String,
        name: String,
        connection_id: String,
        database_name: String,
    ) -> Self {
        Self {
            id,
            name,
            node_type: MongoNodeType::Database,
            connection_id,
            database_name: Some(database_name),
            collection_name: None,
            children: Vec::new(),
            children_loaded: false,
        }
    }

    pub fn new_collection(
        id: String,
        name: String,
        connection_id: String,
        database_name: String,
        collection_name: String,
    ) -> Self {
        Self {
            id,
            name,
            node_type: MongoNodeType::Collection,
            connection_id,
            database_name: Some(database_name),
            collection_name: Some(collection_name),
            children: Vec::new(),
            children_loaded: true,
        }
    }

    pub fn set_children(&mut self, children: Vec<MongoNode>) {
        self.children = children;
        self.children_loaded = true;
    }

    pub fn is_expandable(&self) -> bool {
        matches!(
            self.node_type,
            MongoNodeType::Connection | MongoNodeType::Database
        )
    }
}

pub fn document_to_pretty_json(document: &Document) -> Result<String, MongoError> {
    bson_to_pretty_json(&Bson::Document(document.clone()))
}

pub fn bson_to_pretty_json(value: &Bson) -> Result<String, MongoError> {
    let json = value.clone().into_relaxed_extjson();
    serde_json::to_string_pretty(&json).map_err(|e| MongoError::Serialization(e.to_string()))
}

pub fn bson_to_compact_json(value: &Bson) -> Result<String, MongoError> {
    let json = value.clone().into_relaxed_extjson();
    serde_json::to_string(&json).map_err(|e| MongoError::Serialization(e.to_string()))
}

pub fn bson_to_string(value: &Bson) -> String {
    match value {
        Bson::String(value) => value.clone(),
        Bson::Int32(value) => value.to_string(),
        Bson::Int64(value) => value.to_string(),
        Bson::Double(value) => value.to_string(),
        Bson::Boolean(value) => value.to_string(),
        Bson::ObjectId(value) => value.to_hex(),
        _ => format!("{:?}", value),
    }
}
