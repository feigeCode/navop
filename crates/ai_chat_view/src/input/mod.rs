//! Agent 输入框模块:顶部目标 Context Bar + 多行输入 + `@` 提及 + 图片附件 + 底部执行参数工具栏。

mod agent_input;
mod attachment;
mod context;
mod mention;

pub use agent_input::{AgentInput, AgentInputEvent};
pub use attachment::ImageAttachment;
pub use context::{
    AgentComposerContext, ComposerAgentOption, ComposerMenuOption, ComposerModel,
    ComposerModelOption, ComposerPlanItem, ComposerScope, ComposerSubAgentItem, ComposerTarget,
};
pub use mention::{MentionCompletionProvider, MentionItem};
