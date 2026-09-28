//! 工具运行时:工具规格、调用上下文、观测结果、注册表与路由器。

pub mod builtin;

mod action;
mod invocation;
mod observation;
mod registry;
mod router;
mod runtime_adapter;
mod spec;

pub use action::ToolAction;
pub use invocation::ToolInvocation;
pub use observation::{FileChange, ObservationData, ToolObservation};
pub use registry::{Tool, ToolRegistry, ToolRegistryError};
pub use router::{ToolCall, ToolDispatchContext, ToolRouter};
pub use runtime_adapter::*;
pub use spec::{ToolName, ToolNameAllocator, ToolSchemaError, ToolSpec};
