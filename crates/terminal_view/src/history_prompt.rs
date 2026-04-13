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
    tracking: bool,
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
            tracking: true,
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

    pub fn is_valid(&self) -> bool {
        self.tracking
    }

    pub fn clear(&mut self) {
        self.mode = HistoryPromptMode::InlineSuggest;
        self.input.clear();
        self.search_query.clear();
        self.search_base_input.clear();
        self.tracking = true;
        self.matches.clear();
        self.selected = None;
    }

    pub fn dismiss_matches(&mut self) {
        self.matches.clear();
        self.selected = None;
    }

    pub fn invalidate(&mut self) {
        if self.mode == HistoryPromptMode::Search {
            self.input = self.search_base_input.clone();
            self.search_query.clear();
            self.search_base_input.clear();
            self.mode = HistoryPromptMode::InlineSuggest;
        }
        self.tracking = false;
        self.matches.clear();
        self.selected = None;
    }

    pub fn set_input(&mut self, input: String) {
        self.mode = HistoryPromptMode::InlineSuggest;
        self.input = input;
        self.search_query.clear();
        self.search_base_input.clear();
        self.tracking = true;
        self.selected = None;
    }

    pub fn enter_search(&mut self) {
        if !self.tracking {
            return;
        }
        self.mode = HistoryPromptMode::Search;
        self.search_base_input = self.input.clone();
        self.search_query.clear();
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
        self.matches.clear();
        self.selected = None;
    }

    pub fn append_text(&mut self, text: &str) {
        if !self.tracking {
            return;
        }
        match self.mode {
            HistoryPromptMode::InlineSuggest => self.input.push_str(text),
            HistoryPromptMode::Search => self.search_query.push_str(text),
        }
        self.selected = None;
    }

    pub fn backspace(&mut self) {
        if !self.tracking {
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
        self.selected = None;
    }

    pub fn apply_paste(&mut self, text: &str) {
        if text.contains('\n') || text.contains('\r') {
            self.invalidate();
            return;
        }
        self.append_text(text);
    }

    pub fn set_matches(&mut self, matches: Vec<String>) {
        if !self.tracking {
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
                let suffix = candidate.strip_prefix(self.query_input())?.to_string();
                if suffix.is_empty() {
                    return None;
                }
                self.input = candidate;
                self.selected = None;
                Some(HistoryPromptAccept::AppendSuffix(suffix))
            }
            HistoryPromptMode::Search => {
                self.input = candidate.clone();
                self.search_query.clear();
                self.search_base_input.clear();
                self.mode = HistoryPromptMode::InlineSuggest;
                self.matches.clear();
                self.selected = None;
                Some(HistoryPromptAccept::ReplaceLine(candidate))
            }
        }
    }

    pub fn select_match(&mut self, index: usize) -> Option<String> {
        let candidate = self.matches.get(index)?.clone();
        self.selected = Some(index);
        Some(candidate)
    }

    pub fn navigate_previous(&mut self) -> Option<String> {
        if !self.tracking || self.matches.is_empty() {
            return None;
        }

        match self.mode {
            HistoryPromptMode::InlineSuggest => {
                let next_index = match self.selected {
                    Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
                    None => 0,
                };
                self.selected = Some(next_index);
                Some(self.matches[next_index].clone())
            }
            HistoryPromptMode::Search => {
                let next_index = match self.selected {
                    Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
                    None => 0,
                };
                self.selected = Some(next_index);
                Some(self.matches[next_index].clone())
            }
        }
    }

    pub fn navigate_next(&mut self) -> Option<String> {
        if !self.tracking || self.matches.is_empty() {
            return None;
        }

        match self.mode {
            HistoryPromptMode::InlineSuggest => match self.selected {
                None => {
                    self.selected = Some(0);
                    Some(self.matches[0].clone())
                }
                Some(index) if index > 0 => {
                    let next_index = index - 1;
                    self.selected = Some(next_index);
                    Some(self.matches[next_index].clone())
                }
                Some(0) => {
                    self.selected = None;
                    Some(self.input.clone())
                }
                _ => None,
            },
            HistoryPromptMode::Search => {
                let next_index = self.selected.unwrap_or(0).saturating_sub(1);
                self.selected = Some(next_index);
                Some(self.matches[next_index].clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryPromptAccept, HistoryPromptMode, HistoryPromptState};

    #[test]
    fn history_prompt_invalidate_suspends_tracking_until_clear() {
        let mut state = HistoryPromptState::from_input("git");

        state.invalidate();
        state.append_text(" status");

        assert!(!state.is_valid());
        assert_eq!(state.input(), "git");
        assert!(state.matches().is_empty());

        state.clear();
        state.append_text("cargo");

        assert!(state.is_valid());
        assert_eq!(state.input(), "cargo");
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
    fn history_prompt_invalidate_exits_search_and_restores_shell_input() {
        let mut state = HistoryPromptState::from_input("git st");
        state.enter_search();
        state.append_text("cargo");
        state.set_matches(vec!["cargo test".to_string()]);

        state.invalidate();

        assert_eq!(state.mode(), HistoryPromptMode::InlineSuggest);
        assert_eq!(state.input(), "git st");
        assert_eq!(state.query_input(), "git st");
        assert!(!state.is_valid());
        assert!(state.matches().is_empty());
    }
}
