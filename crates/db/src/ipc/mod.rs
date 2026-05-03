pub mod client;
pub mod connection;
pub mod plugin;
pub mod protocol;
pub mod registry;

pub use connection::ExternalDbConnection;
pub use plugin::ExternalDatabasePlugin;
pub use registry::{
    IpcDriverEntry, IpcDriverManifest, IpcDriverRegistry, IpcDriverTransport,
    EXTERNAL_DRIVER_ID_PARAM,
};
