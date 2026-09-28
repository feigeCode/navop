//! 工具动作类别:这次调用「在做什么」的**声明**。
//!
//! UI 用它选动词短语(「读取 src/main.rs」而不是「fs.read · src/main.rs」)。因此
//! 这里只接受**声明**,不接受猜测:
//!
//! - 外部 agent 走 ACP 时,类别来自协议字段 `ToolCall.kind`(agent 自己声明的);
//! - 本地运行时由 [`ToolAction::from_tool_name`] 按**第一方工具注册名**归类。
//!
//! 两边都拿不到声明时一律落到 [`ToolAction::Other`],UI 退回显示工具名——
//! 宁可显示 `echo`,也不把任意工具名猜成「读取」。

use serde::{Deserialize, Serialize};

/// 一次工具调用的动作类别。
///
/// 变体与 ACP 协议声明的工具类别一一对应,便于协议层直接映射。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAction {
    /// 读取文件或数据。
    Read,
    /// 修改文件或内容。
    Edit,
    /// 删除文件或数据。
    Delete,
    /// 移动 / 重命名。
    Move,
    /// 检索信息。
    Search,
    /// 运行命令或代码。
    Execute,
    /// 内部推理 / 规划。
    Think,
    /// 拉取外部数据。
    Fetch,
    /// 切换会话模式。
    SwitchMode,
    /// 未声明类别(默认)。
    #[default]
    Other,
}

impl ToolAction {
    /// 第一方工具注册名 → 动作类别。
    ///
    /// **只认 navop 自己注册的工具名**;表外的名字(包含扩展 / 外部 agent 任意标题)
    /// 一律返回 [`ToolAction::Other`],由 UI 显示原名。
    pub fn from_tool_name(name: &str) -> Self {
        match name {
            // 读取
            "read_file" | "read_skill_file" | "sftp.read" | "db.query" => ToolAction::Read,
            // 写入 / 修改
            "write_file" | "sftp.write" | "sftp.upload" => ToolAction::Edit,
            // 执行
            "exec_command" | "ssh.exec" | "terminal.exec" | "terminal_exec" | "db.exec" => {
                ToolAction::Execute
            }
            _ => ToolAction::Other,
        }
    }

    /// 该类别是否值得用一个动词短语代替工具名。
    ///
    /// `Other` 时 UI 应直接显示工具名:没有声明就不要编一个动词。
    pub fn has_verb(self) -> bool {
        !matches!(self, ToolAction::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_names_map_to_declared_actions() {
        assert_eq!(ToolAction::Read, ToolAction::from_tool_name("read_file"));
        assert_eq!(ToolAction::Read, ToolAction::from_tool_name("sftp.read"));
        assert_eq!(ToolAction::Edit, ToolAction::from_tool_name("write_file"));
        assert_eq!(ToolAction::Edit, ToolAction::from_tool_name("sftp.upload"));
        assert_eq!(ToolAction::Execute, ToolAction::from_tool_name("ssh.exec"));
        assert_eq!(ToolAction::Execute, ToolAction::from_tool_name("db.exec"));
    }

    #[test]
    fn unknown_names_are_never_guessed_into_a_verb() {
        // 表外名字(扩展工具、外部 agent 的自由标题)必须留在 Other。
        for name in ["fs.read", "echo", "mcp__fs__read", "读一下这个文件", ""] {
            assert_eq!(
                ToolAction::Other,
                ToolAction::from_tool_name(name),
                "{name} 不应被猜成某个动词"
            );
            assert!(!ToolAction::from_tool_name(name).has_verb());
        }
    }
}
