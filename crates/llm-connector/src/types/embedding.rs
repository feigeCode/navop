//! Embedding API Types

use crate::types::Usage;
use serde::{Deserialize, Serialize};

/// Embedding Request
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbedRequest {
    /// Model to use for embedding
    pub model: String,
    /// Input text to embed. Can be a single string or an array of strings.
    pub input: Vec<String>,
    /// Format of the embeddings. Optional, defaults to "float".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    /// User identifier string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl EmbedRequest {
    /// Create a new embedding request with a single input string
    pub fn new(model: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            input: vec![input.into()],
            ..Default::default()
        }
    }

    /// Create a new embedding request with multiple input strings
    pub fn new_batch(model: impl Into<String>, inputs: Vec<String>) -> Self {
        Self {
            model: model.into(),
            input: inputs,
            ..Default::default()
        }
    }

    /// Set the encoding format
    pub fn with_encoding_format(mut self, format: impl Into<String>) -> Self {
        self.encoding_format = Some(format.into());
        self
    }
}

/// Embedding Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    /// Object type (typically "list")
    pub object: String,
    /// List of embedding data items
    pub data: Vec<EmbeddingData>,
    /// Model used
    pub model: String,
    /// Usage statistics
    pub usage: Usage,
}

/// Single Embedding Data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    /// Object type (typically "embedding")
    pub object: String,
    /// The embedding vector
    pub embedding: Vec<f32>,
    /// Index of the input in the request
    pub index: u32,
}
