//! 三产品拆分后的唯一领域契约层。
//!
//! 这里只放 `navop`、`navopShell`、`navopData` 共用的 trait 与值类型，
//! 不依赖 `db`、`db_view`、`terminal_view` 等具体领域 crate。

mod domain;
mod file_open;
mod form;
mod home;
mod identity;
mod intent;
mod kind;
mod registry;
mod resources;
mod settings;
mod shell_extras;
mod tab_restore;

pub use domain::{Domain, DomainId};
pub use file_open::FileOpenHandler;
pub use form::{ConnectionFormProvider, ConnectionFormRequest, ConnectionSavedCallback};
pub use home::{HomeContext, NotifyLevel, TabHost};
pub use identity::{AppIdentity, Product, ProductFeatures};
pub use intent::{
    DatabaseTabMode, RemoteProtocol, TabIntent, TerminalProfileDescriptor, TerminalProfileKind,
};
pub use kind::{ConnectionKindDescriptor, ConnectionKindGroup};
pub use resources::{ActiveResourceIds, ActiveResourceProvider};
pub use registry::{ConnectionOpener, DomainRegistry, TabOpener};
pub use settings::SettingsSectionProvider;
pub use shell_extras::{ShellExtras, SidebarColors, WslDistributionInfo};
pub use tab_restore::TabRestorer;

/// 端口转发运行时句柄：契约层统一暴露，避免各产品直接依赖 `port_forwarding`。
pub use port_forwarding::PortForwardingRuntime;
