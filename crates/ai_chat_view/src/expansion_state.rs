//! 过程块展开态的显式覆盖表。
//!
//! 用「显式 bool 覆盖表」而不是「已展开集合」：用户手动**收起**一个正在直播的
//! reasoning 块也必须被记住，用集合表达不出来。
//!
//! key 必须是稳定身份（消息 `Uuid` / `TurnId` / 工具调用 id），**不得**使用数组下标、
//! 显示文本或每次 render 生成的随机值。

use std::collections::HashMap;

/// 一次会话内的展开态覆盖表。
#[derive(Clone, Debug, Default)]
pub struct ExpansionState {
    overrides: HashMap<String, bool>,
}

impl ExpansionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读取显式覆盖；`None` 表示「用户没表态」，由调用方决定默认值。
    pub fn override_of(&self, id: &str) -> Option<bool> {
        self.overrides.get(id).copied()
    }

    /// 显式设置展开/收起。
    pub fn set(&mut self, id: impl Into<String>, expanded: bool) {
        self.overrides.insert(id.into(), expanded);
    }

    /// 切换并返回新态；从未表态时以 `default` 为起点。
    pub fn toggle(&mut self, id: &str, default: bool) -> bool {
        let next = !self.override_of(id).unwrap_or(default);
        self.set(id.to_string(), next);
        next
    }

    /// 结合默认值解析当前应否展开。
    pub fn is_expanded(&self, id: &str, default: bool) -> bool {
        self.override_of(id).unwrap_or(default)
    }

    /// 新提交时清空：跨轮的 id 与索引都会失效。
    pub fn clear(&mut self) {
        self.overrides.clear();
    }

    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_collapse_of_a_live_block_is_remembered() {
        let mut state = ExpansionState::new();

        // 直播中的过程块默认为展开；用户手动收起。
        assert!(state.is_expanded("block-1", true));
        state.set("block-1", false);

        assert_eq!(Some(false), state.override_of("block-1"));
        assert!(!state.is_expanded("block-1", true));
    }

    #[test]
    fn toggle_starts_from_default_and_flips() {
        let mut state = ExpansionState::new();

        // 默认展开的块，第一次 toggle 是收起。
        assert!(!state.toggle("block-2", true));
        assert!(!state.is_expanded("block-2", true));
        assert!(state.toggle("block-2", true));
        assert!(state.is_expanded("block-2", false));
    }

    #[test]
    fn unknown_id_falls_back_to_default() {
        let state = ExpansionState::new();

        assert_eq!(None, state.override_of("missing"));
        assert!(state.is_expanded("missing", true));
        assert!(!state.is_expanded("missing", false));
    }

    #[test]
    fn clear_drops_every_override_across_turns() {
        let mut state = ExpansionState::new();
        state.set("a", true);
        state.set("b", false);

        state.clear();

        assert!(state.is_empty());
        assert_eq!(0, state.len());
        assert_eq!(None, state.override_of("a"));
    }
}
