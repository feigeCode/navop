//! MongoDB connection facade; SDK implementation lives behind `builtin-mongodb`.

#[cfg(feature = "builtin-mongodb")]
pub use mongodb_runtime::BuiltinMongoConnection as MongoConnectionImpl;
pub use mongodb_runtime::{IpcMongoConnection, MongoConnection};
