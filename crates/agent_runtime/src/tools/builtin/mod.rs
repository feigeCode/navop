//! 内置工具集合。

mod echo;
mod load_skill;
mod read_skill_file;

pub use echo::EchoTool;
pub use load_skill::LoadSkillTool;
pub use read_skill_file::ReadSkillFileTool;

use crate::tools::ToolRegistry;
use std::sync::Arc;

pub fn default_agent_tools() -> ToolRegistry {
    ToolRegistry::new()
        .with_tool(Arc::new(LoadSkillTool))
        .with_tool(Arc::new(ReadSkillFileTool))
}
