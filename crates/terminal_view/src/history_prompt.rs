#[derive(Clone, Debug, Default)]
pub struct HistoryPromptState {
    input: String,
    tracking: bool,
    matches: Vec<String>,
    selected: Option<usize>,
}

impl HistoryPromptState {
    pub fn from_input(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            tracking: true,
            matches: Vec::new(),
            selected: None,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn query_input(&self) -> &str {
        &self.input
    }

    pub fn matches(&self) -> &[String] {
        &self.matches
    }

    pub fn is_valid(&self) -> bool {
        self.tracking
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.tracking = true;
        self.matches.clear();
        self.selected = None;
    }

    pub fn dismiss_matches(&mut self) {
        self.matches.clear();
        self.selected = None;
    }

    pub fn invalidate(&mut self) {
        self.tracking = false;
        self.matches.clear();
        self.selected = None;
    }

    pub fn set_input(&mut self, input: String) {
        self.input = input;
        self.tracking = true;
        self.selected = None;
    }

    pub fn append_text(&mut self, text: &str) {
        if !self.tracking {
            return;
        }
        self.input.push_str(text);
        self.selected = None;
    }

    pub fn backspace(&mut self) {
        if !self.tracking {
            return;
        }
        self.input.pop();
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

    pub fn accept_selected_suggestion(&mut self) -> Option<String> {
        let candidate = self.selected_match()?.to_string();
        let suffix = candidate.strip_prefix(self.query_input())?.to_string();
        if suffix.is_empty() {
            return None;
        }
        self.input = candidate;
        self.selected = None;
        Some(suffix)
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

        let next_index = match self.selected {
            Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
            None => 0,
        };
        self.selected = Some(next_index);
        Some(self.matches[next_index].clone())
    }

    pub fn navigate_next(&mut self) -> Option<String> {
        if !self.tracking || self.matches.is_empty() {
            return None;
        }

        match self.selected {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryPromptState;

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
}
