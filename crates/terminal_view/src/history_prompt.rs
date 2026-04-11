#[derive(Clone, Debug, Default)]
pub struct HistoryPromptState {
    input: String,
    valid: bool,
    matches: Vec<String>,
    selected: Option<usize>,
    original_input: Option<String>,
}

impl HistoryPromptState {
    pub fn from_input(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            valid: true,
            matches: Vec::new(),
            selected: None,
            original_input: None,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn query_input(&self) -> &str {
        self.original_input.as_deref().unwrap_or(&self.input)
    }

    pub fn matches(&self) -> &[String] {
        &self.matches
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.valid = true;
        self.matches.clear();
        self.selected = None;
        self.original_input = None;
    }

    pub fn dismiss_matches(&mut self) {
        self.matches.clear();
        self.selected = None;
        self.original_input = None;
    }

    pub fn invalidate(&mut self) {
        self.valid = false;
        self.matches.clear();
        self.selected = None;
        self.original_input = None;
    }

    pub fn set_input(&mut self, input: String) {
        self.input = input;
        self.valid = true;
        self.selected = None;
        self.original_input = None;
    }

    pub fn append_text(&mut self, text: &str) {
        self.input.push_str(text);
        self.valid = true;
        self.selected = None;
        self.original_input = None;
    }

    pub fn backspace(&mut self) {
        self.input.pop();
        self.valid = true;
        self.selected = None;
        self.original_input = None;
    }

    pub fn apply_paste(&mut self, text: &str) {
        if text.contains('\n') || text.contains('\r') {
            self.invalidate();
            return;
        }
        self.append_text(text);
    }

    pub fn set_matches(&mut self, matches: Vec<String>) {
        if !self.valid {
            self.matches.clear();
            self.selected = None;
            self.original_input = None;
            return;
        }

        self.matches = matches;
        if self.matches.is_empty() {
            self.selected = None;
            self.original_input = None;
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
        self.selected = Some(self.selected.unwrap_or(0));
        self.original_input = None;
        Some(suffix)
    }

    pub fn select_match(&mut self, index: usize) -> Option<String> {
        let candidate = self.matches.get(index)?.clone();
        if self.original_input.is_none() {
            self.original_input = Some(self.input.clone());
        }
        self.selected = Some(index);
        self.input = candidate.clone();
        Some(candidate)
    }

    pub fn navigate_previous(&mut self) -> Option<String> {
        if !self.valid || self.matches.is_empty() {
            return None;
        }

        if self.original_input.is_none() {
            self.original_input = Some(self.input.clone());
        }

        let next_index = match self.selected {
            Some(index) => (index + 1).min(self.matches.len().saturating_sub(1)),
            None => 0,
        };
        self.selected = Some(next_index);
        self.input = self.matches[next_index].clone();
        Some(self.input.clone())
    }

    pub fn navigate_next(&mut self) -> Option<String> {
        if !self.valid || self.matches.is_empty() {
            return None;
        }

        match self.selected {
            None => {
                if self.original_input.is_none() {
                    self.original_input = Some(self.input.clone());
                }
                self.selected = Some(0);
                self.input = self.matches[0].clone();
                Some(self.input.clone())
            }
            Some(index) if index > 0 => {
                let next_index = index - 1;
                self.selected = Some(next_index);
                self.input = self.matches[next_index].clone();
                Some(self.input.clone())
            }
            Some(0) => {
                let original = self.original_input.clone().unwrap_or_default();
                self.selected = None;
                self.original_input = None;
                self.input = original.clone();
                Some(original)
            }
            _ => None,
        }
    }
}
