//! 工具运行时:工具规格、调用上下文、观测结果、注册表与路由器。

pub mod builtin;

mod invocation;
mod observation;
mod registry;
mod router;
mod spec;

pub use invocation::ToolInvocation;
pub use observation::{ObservationData, ToolObservation};
pub use registry::{Tool, ToolRegistry};
pub use router::{ToolCall, ToolDispatchContext, ToolRouter};
pub use spec::{ToolName, ToolSpec};
