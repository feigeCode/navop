//! Codex-style Skill 支持。
//!
//! 处理本地 `SKILL.md`:发现、导入、选择、向 prompt 注入目录元数据,并通过工具按需加载完整说明。

mod catalog;
mod context;
mod import;

pub use catalog::{SkillCatalog, SkillLoadError, SkillMetadata};
pub use context::{SkillContext, SkillRef, SkillSummary};
pub use import::{SkillImportError, import_skill_dir};

pub(super) const SKILL_FILE: &str = "SKILL.md";
