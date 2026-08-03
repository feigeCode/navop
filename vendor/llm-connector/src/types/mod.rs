//! Core types for llm-connector

pub mod message_block;
pub mod request;
pub mod response;
pub mod responses;

#[cfg(feature = "streaming")]
pub mod streaming;

pub mod embedding;

pub use message_block::*;
pub use request::*;
pub use response::*;
pub use responses::*;

#[cfg(feature = "streaming")]
pub use streaming::*;

pub use embedding::*;
// ChatMessage is the same as Message
pub type ChatMessage = request::Message;
