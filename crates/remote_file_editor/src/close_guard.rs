#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseIntercept {
    Allow,
    Prompt,
    Ignore,
}

pub fn decide_close_intercept(is_dirty: bool, prompt_open: bool) -> CloseIntercept {
    if !is_dirty {
        CloseIntercept::Allow
    } else if prompt_open {
        CloseIntercept::Ignore
    } else {
        CloseIntercept::Prompt
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseIntercept, decide_close_intercept};

    #[test]
    fn allows_close_when_editor_is_clean() {
        assert_eq!(decide_close_intercept(false, false), CloseIntercept::Allow);
    }

    #[test]
    fn prompts_once_for_unsaved_changes() {
        assert_eq!(decide_close_intercept(true, false), CloseIntercept::Prompt);
    }

    #[test]
    fn ignores_repeated_close_while_prompt_is_open() {
        assert_eq!(decide_close_intercept(true, true), CloseIntercept::Ignore);
    }
}
