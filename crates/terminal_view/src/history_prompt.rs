/// 历史提示跟踪状态
///
/// 两态设计：
/// - `Active`: 正常跟踪输入，允许显示下拉
/// - `Dismissed`: 已关闭，等待下一次可打印输入重新激活
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackingState {
    #[default]
    Active,
    Dismissed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryPromptMode {
    #[default]
    InlineSuggest,
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryPromptAccept {
    AppendSuffix(String),
    ReplaceLine(String),
}

#[derive(Clone, Debug, Default)]
pub struct HistoryPromptState {
    mode: HistoryPromptMode,
    input: String,
    search_query: String,
    search_base_input: String,
    tracking_state: TrackingState,
    dropdown_visible: bool,
    matches: Vec<String>,
    selected: Option<usize>,
}

impl HistoryPromptState {
    pub fn from_input(input: impl Into<String>) -> Self {
        Self {
            mode: HistoryPromptMode::InlineSuggest,
            input: input.into(),
            search_query: String::new(),
            search_base_input: String::new(),
            tracking_state: TrackingState::Active,
            dropdown_visible: true,
            matches: Vec::new(),
            selected: None,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn query_input(&self) -> &str {
        match self.mode {
            HistoryPromptMode::InlineSuggest => &self.input,
            HistoryPromptMode::Search => &self.search_query,
        }
    }

    pub fn matches(&self) -> &[String] {
        &self.matches
    }

    pub fn mode(&self) -> HistoryPromptMode {
        self.mode
    }

    pub fn tracking_state(&self) -> TrackingState {
        self.tracking_state
    }

    pub fn is_valid(&self) -> bool {
        self.tracking_state == TrackingState::Active
    }

    pub fn is_active(&self) -> bool {
        self.tracking_state == TrackingState::Active
    }

    pub fn dropdown_visible(&self) -> bool {
        self.dropdown_visible
    }

    pub fn clear(&mut self) {
        self.dismiss();
    }

    pub fn dismiss(&mut self) {
        self.mode = HistoryPromptMode::InlineSuggest;
        self.input.clear();
        self.search_query.clear();
        self.search_base_input.clear();
        self.tracking_state = TrackingState::Dismissed;
        self.dropdown_visible = false;
        self.matches.clear();
        self.selected = None;
    }

    pub fn dismiss_matches(&mut self) {
        self.dropdown_visible = false;
        self.matches.clear();
        self.selected = None;
    }

    /// 兼容旧调用，行为等同于 dismiss。
    pub fn invalidate(&mut self) {
        self.dismiss();
    }

    /// 隐藏下拉但保留跟踪状态和当前 input。
    pub fn hide_dropdown(&mut self) {
        if self.tracking_state == TrackingState::Active {
            self.dropdown_visible = false;
            self.selected = None;
        }
    }

    /// 兼容旧调用，行为等同于 hide_dropdown。
    pub fn suspend(&mut self) {
        self.hide_dropdown();
    }

    pub fn show_dropdown(&mut self) {
        if self.query_input().is_empty() && self.mode == HistoryPromptMode::InlineSuggest {
            return;
        }
        self.tracking_state = TrackingState::Active;
        self.dropdown_visible = true;
    }

    /// 兼容旧调用：保留旧 input，并恢复显示状态。
    pub fn resume_with_input(&mut self, input: String) {
        self.tracking_state = TrackingState::Active;
        self.input = input;
        self.dropdown_visible = !self.input.is_empty();
        self.selected = None;
    }

    pub fn set_input(&mut self, input: String) {
        self.mode = HistoryPromptMode::InlineSuggest;
        self.input = input;
        self.search_query.clear();
        self.search_base_input.clear();
        self.tracking_state = TrackingState::Active;
        self.dropdown_visible = !self.input.is_empty();
        self.selected = None;
    }

    pub fn enter_search(&mut self) {
        if self.tracking_state != TrackingState::Active {
            return;
        }
        self.mode = HistoryPromptMode::Search;
        self.search_base_input = self.input.clone();
        self.search_query.clear();
        self.dropdown_visible = true;
        self.matches.clear();
        self.selected = None;
    }

    pub fn exit_search(&mut self) {
        if self.mode != HistoryPromptMode::Search {
            return;
        }
        self.input = self.search_base_input.clone();
        self.search_query.clear();
        self.search_base_input.clear();
        self.mode = HistoryPromptMode::InlineSuggest;
        self.dropdown_visible = !self.input.is_empty();
        self.matches.clear();
        self.selected = None;
    }

    pub fn append_text(&mut self, text: &str) {
        if self.tracking_state == TrackingState::Dismissed {
            self.tracking_state = TrackingState::Active;
        }
        match self.mode {
            HistoryPromptMode::InlineSuggest => self.input.push_str(text),
            HistoryPromptMode::Search => self.search_query.push_str(text),
        }
        self.dropdown_visible = true;
        self.selected = None;
    }

    pub fn backspace(&mut self) {
        if self.tracking_state != TrackingState::Active {
            return;
        }
        match self.mode {
            HistoryPromptMode::InlineSuggest => {
                self.input.pop();
            }
            HistoryPromptMode::Search => {
                self.search_query.pop();
            }
        }
        self.dropdown_visible = match self.mode {
            HistoryPromptMode::InlineSuggest => !self.input.is_empty(),
            HistoryPromptMode::Search => true,
        };
        if !self.dropdown_visible {
            self.matches.clear();
        }
        self.selected = None;
    }

    pub fn apply_paste(&mut self, text: &str) {
        if text.contains('\n') || text.contains('\r') {
            self.dismiss();
            return;
        }
        self.append_text(text);
    }

    pub fn set_matches(&mut self, matches: Vec<String>) {
        if self.tracking_state != TrackingState::Active {
            self.matches.clear();
            self.selected = None;
            return;
        }

        self.matches = matches;
        if self.matches.is_empty() {
            self.selected = None;
        } else if let Some(selected) = self.selected {
            self.selected = Some(selected.min(self.matches.len().saturating_sub(1)));
        } else if self.mode == HistoryPromptMode::Search {
            self.selected = Some(0);
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected.unwrap_or(0)
    }

    pub fn selected_match(&self) -> Option<&str> {
        self.matches
            .get(self.selected.unwrap_or(0))
            .map(|value| value.as_str())
    }

    pub fn accept_selected_suggestion(&mut self) -> Option<HistoryPromptAccept> {
        let candidate = self.selected_match()?.to_string();
        match self.mode {
            HistoryPromptMode::InlineSuggest => {
                // 尝试前缀匹配的 AppendSuffix 路径
                let query = self.query_input().to_string();
                if let Some(suffix) = candidate.strip_prefix(&query) {
                    if suffix.is_empty() {
                        return None;
                    }
                    let suffix = suffix.to_string();
                    self.input = candidate;
                    self.dropdown_visible = false;
                    self.matches.clear();
                    self.selected = None;
                    return Some(HistoryPromptAccept::AppendSuffix(suffix));
                }
                None
            }
            HistoryPromptMode::Search => {
                self.input = candidate.clone();
                self.search_query.clear();
                self.search_base_input.clear();
                self.mode = HistoryPromptMode::InlineSuggest;
                self.dropdown_visible = false;
                self.matches.clear();
                self.selected = None;
                Some(HistoryPromptAccept::ReplaceLine(candidate))
            }
        }
    }

    /// 逐词接受建议（Ctrl+Right / Alt+F）
    ///
    /// 只接受建议中的下一个"词"（到空格或路径分隔符为止），
    /// 保持同一条建议继续部分接受。
    pub fn accept_next_word(&mut self) -> Option<HistoryPromptAccept> {
        let candidate = self.selected_match()?.to_string();
        let suffix = candidate.strip_prefix(self.query_input())?;
        if suffix.is_empty() {
            return None;
        }

        // 找到下一个词边界
        let trimmed = suffix.trim_start_matches(' ');
        let leading_spaces = suffix.len() - trimmed.len();

        let word_end = if trimmed.is_empty() {
            suffix.len()
        } else {
            let boundary = trimmed
                .find(|c: char| c.is_whitespace() || c == '/')
                .map(|i| {
                    // 如果边界是 '/'，包含它
                    if trimmed.as_bytes().get(i) == Some(&b'/') {
                        i + 1
                    } else {
                        i
                    }
                })
                .unwrap_or(trimmed.len());
            leading_spaces + boundary
        };

        let word = &suffix[..word_end];
        self.input.push_str(word);
        self.dropdown_visible = true;
        // 不清除 selected，保持同一条建议继续部分接受
        Some(HistoryPromptAccept::AppendSuffix(word.to_string()))
    }

    pub fn select_match(&mut self, index: usize) -> Option<String> {
        let candidate = self.matches.get(index)?.clone();
        self.selected = Some(index);
        self.dropdown_visible = true;
        Some(candidate)
    }

    pub fn navigate_previous(&mut self) -> Option<String> {
        if self.tracking_state != TrackingState::Active || self.matches.is_empty() {
            return None;
        }

        match self.mode {
            HistoryPromptMode::InlineSuggest => {
                let next_index = match self.selected {
                    Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
                    None => 0,
                };
                self.selected = Some(next_index);
                self.dropdown_visible = true;
                Some(self.matches[next_index].clone())
            }
            HistoryPromptMode::Search => {
                let next_index = match self.selected {
                    Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
                    None => 0,
                };
                self.selected = Some(next_index);
                self.dropdown_visible = true;
                Some(self.matches[next_index].clone())
            }
        }
    }

    pub fn navigate_next(&mut self) -> Option<String> {
        if self.tracking_state != TrackingState::Active || self.matches.is_empty() {
            return None;
        }

        match self.mode {
            HistoryPromptMode::InlineSuggest => match self.selected {
                None => {
                    self.selected = Some(0);
                    self.dropdown_visible = true;
                    Some(self.matches[0].clone())
                }
                Some(index) if index > 0 => {
                    let next_index = index - 1;
                    self.selected = Some(next_index);
                    self.dropdown_visible = true;
                    Some(self.matches[next_index].clone())
                }
                Some(0) => {
                    self.selected = None;
                    self.dropdown_visible = true;
                    Some(self.input.clone())
                }
                _ => None,
            },
            HistoryPromptMode::Search => {
                let next_index = self.selected.unwrap_or(0).saturating_sub(1);
                self.selected = Some(next_index);
                self.dropdown_visible = true;
                Some(self.matches[next_index].clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryPromptAccept, HistoryPromptMode, HistoryPromptState, TrackingState};

    #[test]
    fn history_prompt_dismiss_resets_input_and_restarts_on_next_character() {
        let mut state = HistoryPromptState::from_input("git");

        state.dismiss();

        assert!(!state.is_active());
        assert_eq!(state.tracking_state(), TrackingState::Dismissed);
        assert_eq!(state.input(), "");
        assert!(!state.dropdown_visible());
        assert!(state.matches().is_empty());

        state.append_text("c");

        assert!(state.is_active());
        assert_eq!(state.input(), "c");
        assert!(state.dropdown_visible());
    }

    #[test]
    fn history_prompt_navigation_does_not_overwrite_input_query() {
        let mut state = HistoryPromptState::from_input("git s");
        state.set_matches(vec![
            "git status".to_string(),
            "git stash".to_string(),
            "git switch".to_string(),
        ]);

        assert_eq!(state.navigate_previous().as_deref(), Some("git status"));
        assert_eq!(state.input(), "git s");
        assert_eq!(state.query_input(), "git s");

        assert_eq!(state.navigate_previous().as_deref(), Some("git stash"));
        assert_eq!(state.input(), "git s");
        assert_eq!(state.query_input(), "git s");
    }

    #[test]
    fn history_prompt_search_mode_tracks_query_without_mutating_shell_input() {
        let mut state = HistoryPromptState::from_input("git st");

        state.enter_search();
        state.append_text("status");

        assert_eq!(state.mode(), HistoryPromptMode::Search);
        assert_eq!(state.input(), "git st");
        assert_eq!(state.query_input(), "status");

        state.exit_search();

        assert_eq!(state.mode(), HistoryPromptMode::InlineSuggest);
        assert_eq!(state.input(), "git st");
        assert_eq!(state.query_input(), "git st");
        assert!(state.is_valid());
    }

    #[test]
    fn history_prompt_search_mode_accepts_selection_as_full_replacement() {
        let mut state = HistoryPromptState::from_input("git st");
        state.enter_search();
        state.append_text("cargo");
        state.set_matches(vec!["cargo test".to_string(), "cargo check".to_string()]);

        let accepted = state.accept_selected_suggestion();

        assert_eq!(
            accepted,
            Some(HistoryPromptAccept::ReplaceLine("cargo test".to_string()))
        );
        assert_eq!(state.mode(), HistoryPromptMode::InlineSuggest);
        assert_eq!(state.input(), "cargo test");
        assert_eq!(state.query_input(), "cargo test");
    }

    #[test]
    fn history_prompt_dismiss_exits_search_and_clears_shell_input() {
        let mut state = HistoryPromptState::from_input("git st");
        state.enter_search();
        state.append_text("cargo");
        state.set_matches(vec!["cargo test".to_string()]);

        state.dismiss();

        assert_eq!(state.mode(), HistoryPromptMode::InlineSuggest);
        assert_eq!(state.input(), "");
        assert_eq!(state.query_input(), "");
        assert!(!state.is_active());
        assert!(!state.dropdown_visible());
        assert!(state.matches().is_empty());
    }

    // ---- 下拉隐藏态测试 ----

    #[test]
    fn hide_dropdown_keeps_tracking_active() {
        let mut state = HistoryPromptState::from_input("git s");
        state.set_matches(vec!["git status".to_string()]);

        state.hide_dropdown();

        assert_eq!(state.tracking_state(), TrackingState::Active);
        assert!(state.is_valid());
        assert_eq!(state.input(), "git s"); // input 保留
        assert!(!state.dropdown_visible());
    }

    #[test]
    fn next_character_after_hidden_dropdown_keeps_existing_input() {
        let mut state = HistoryPromptState::from_input("git s");
        state.set_matches(vec!["git status".to_string()]);

        state.hide_dropdown();
        state.append_text("t");

        assert_eq!(state.tracking_state(), TrackingState::Active);
        assert_eq!(state.input(), "git st");
        assert!(state.dropdown_visible());
    }

    #[test]
    fn hide_dropdown_does_not_reactivate_dismissed_state() {
        let mut state = HistoryPromptState::from_input("git");

        state.dismiss();
        state.hide_dropdown();

        assert_eq!(state.tracking_state(), TrackingState::Dismissed);
    }

    #[test]
    fn hidden_dropdown_accepts_next_character() {
        let mut state = HistoryPromptState::from_input("git");
        state.hide_dropdown();

        state.append_text(" status");

        assert_eq!(state.input(), "git status");
        assert!(state.dropdown_visible());
    }

    // ---- Task 7: 部分接受测试 ----

    #[test]
    fn accept_next_word_returns_first_word_of_suffix() {
        let mut state = HistoryPromptState::from_input("git");
        state.set_matches(vec!["git status --short".to_string()]);

        let result = state.accept_next_word();

        assert_eq!(
            result,
            Some(HistoryPromptAccept::AppendSuffix(" status".to_string()))
        );
        assert_eq!(state.input(), "git status");
    }

    #[test]
    fn accept_next_word_updates_input() {
        let mut state = HistoryPromptState::from_input("git");
        state.set_matches(vec!["git status --short".to_string()]);

        state.accept_next_word();

        assert_eq!(state.input(), "git status");
        // selected 应保持，以便继续部分接受
        assert!(state.selected_match().is_some());
    }

    #[test]
    fn accept_next_word_on_last_word_accepts_all() {
        let mut state = HistoryPromptState::from_input("git status --");
        state.set_matches(vec!["git status --short".to_string()]);

        let result = state.accept_next_word();

        assert_eq!(
            result,
            Some(HistoryPromptAccept::AppendSuffix("short".to_string()))
        );
        assert_eq!(state.input(), "git status --short");
    }

    #[test]
    fn accept_next_word_handles_path_separator() {
        let mut state = HistoryPromptState::from_input("cd ");
        state.set_matches(vec!["cd /usr/local/bin".to_string()]);

        let result = state.accept_next_word();

        // 应包含 '/' 分隔符
        assert_eq!(
            result,
            Some(HistoryPromptAccept::AppendSuffix("/".to_string()))
        );
        assert_eq!(state.input(), "cd /");
    }

    // ---- 非前缀匹配接受测试 ----

    #[test]
    fn accept_non_prefix_match_uses_replace_line() {
        let mut state = HistoryPromptState::from_input("test");
        // "cargo test" 是 token_prefix 匹配，不是前缀匹配
        state.set_matches(vec!["cargo test".to_string()]);

        let result = state.accept_selected_suggestion();

        assert_eq!(result, None);
        assert_eq!(state.input(), "test");
    }
}
