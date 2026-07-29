const MAX_PROMPT_HISTORY: usize = 100;

#[derive(Default)]
pub(crate) struct PromptHistory {
    entries: Vec<String>,
    cursor: Option<usize>,
    draft: Option<String>,
}

impl PromptHistory {
    pub(crate) fn record(&mut self, prompt: &str) {
        if prompt.trim().is_empty() {
            return;
        }
        self.cursor = None;
        self.draft = None;
        if self.entries.last().is_some_and(|entry| entry == prompt) {
            return;
        }
        self.entries.push(prompt.to_string());
        if self.entries.len() > MAX_PROMPT_HISTORY {
            self.entries.remove(0);
        }
    }

    pub(crate) fn previous(&mut self, current: &str) -> Option<String> {
        let next_cursor = match self.cursor {
            Some(0) => return None,
            Some(cursor) => cursor - 1,
            None => {
                let next_cursor = self.entries.len().checked_sub(1)?;
                self.draft = Some(current.to_string());
                next_cursor
            }
        };
        self.cursor = Some(next_cursor);
        self.entries.get(next_cursor).cloned()
    }

    pub(crate) fn next(&mut self) -> Option<String> {
        let cursor = self.cursor?;
        if cursor + 1 < self.entries.len() {
            let next_cursor = cursor + 1;
            self.cursor = Some(next_cursor);
            return self.entries.get(next_cursor).cloned();
        }
        self.cursor = None;
        self.draft.take()
    }

    pub(crate) fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROMPT_HISTORY, PromptHistory};

    #[test]
    fn prompt_history_browses_recent_entries_and_restores_draft() {
        let mut history = PromptHistory::default();
        history.record("first");
        history.record("second");

        assert_eq!(Some("second".to_string()), history.previous("draft"));
        assert_eq!(Some("first".to_string()), history.previous("second"));
        assert_eq!(Some("second".to_string()), history.next());
        assert_eq!(Some("draft".to_string()), history.next());
        assert_eq!(None, history.next());
    }

    #[test]
    fn prompt_history_ignores_empty_and_consecutive_duplicates() {
        let mut history = PromptHistory::default();
        history.record("");
        history.record("   ");
        history.record("same");
        history.record("same");

        assert_eq!(Some("same".to_string()), history.previous(""));
        assert_eq!(None, history.previous("same"));
    }

    #[test]
    fn prompt_history_preserves_surrounding_whitespace() {
        let mut history = PromptHistory::default();
        history.record("  select *\n");

        assert_eq!(
            Some("  select *\n".to_string()),
            history.previous("current draft")
        );
    }

    #[test]
    fn prompt_history_trims_oldest_entries() {
        let mut history = PromptHistory::default();
        for index in 0..=MAX_PROMPT_HISTORY {
            history.record(&format!("prompt-{index}"));
        }

        for index in (1..=MAX_PROMPT_HISTORY).rev() {
            assert_eq!(
                Some(format!("prompt-{index}")),
                history.previous(if index == MAX_PROMPT_HISTORY {
                    "draft"
                } else {
                    ""
                })
            );
        }
        assert_eq!(None, history.previous(""));
    }

    #[test]
    fn recording_a_prompt_resets_active_browsing() {
        let mut history = PromptHistory::default();
        history.record("first");
        assert_eq!(Some("first".to_string()), history.previous("draft"));

        history.record("second");

        assert_eq!(Some("second".to_string()), history.previous("new draft"));
        assert_eq!(Some("new draft".to_string()), history.next());
    }
}
