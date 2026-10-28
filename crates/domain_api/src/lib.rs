//! 三产品拆分后的唯一领域契约层。
//!
//! 这里只放 `navop`、`navopShell`、`navopData` 共用的 trait 与值类型，
//! 不依赖 `db`、`db_view`、`terminal_view` 等具体领域 crate。

mod domain;
mod home;
mod identity;
mod intent;
mod registry;

pub use domain::{Domain, DomainId};
pub use home::{HomeContext, NotifyLevel, TabHost};
pub use identity::{AppIdentity, Product, ProductFeatures};
pub use intent::{
    DatabaseTabMode, RemoteProtocol, TabIntent, TerminalProfileDescriptor, TerminalProfileKind,
};
pub use registry::{ConnectionOpener, DomainRegistry, TabOpener};
