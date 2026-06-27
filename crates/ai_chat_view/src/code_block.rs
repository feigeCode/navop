use std::sync::Arc;

use gpui::{App, SharedString, Window};
use gpui_component::IconName;

pub use crate::code_block_parse::{FencedCodeBlock, extract_fenced_code_blocks};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlockActionPreview {
    pub language: Option<String>,
    pub code: String,
    pub action_ids: Vec<String>,
}

#[derive(Clone)]
pub enum LanguageMatcher {
    Exact(Vec<&'static str>),
    Prefix(&'static str),
    Custom(Arc<dyn Fn(&str) -> bool + Send + Sync>),
    Any,
}

impl LanguageMatcher {
    pub fn exact(lang: &'static str) -> Self {
        Self::Exact(vec![lang])
    }

    pub fn exact_many(langs: Vec<&'static str>) -> Self {
        Self::Exact(langs)
    }

    pub fn sql() -> Self {
        Self::Exact(vec![
            "sql",
            "mysql",
            "postgresql",
            "postgres",
            "sqlite",
            "mssql",
            "oracle",
            "plsql",
        ])
    }

    pub fn shell() -> Self {
        Self::Exact(vec![
            "bash",
            "sh",
            "shell",
            "zsh",
            "fish",
            "powershell",
            "ps1",
            "cmd",
            "batch",
        ])
    }

    pub fn python() -> Self {
        Self::Exact(vec!["python", "py", "python3"])
    }

    pub fn rust() -> Self {
        Self::Exact(vec!["rust", "rs"])
    }

    pub fn javascript() -> Self {
        Self::Exact(vec!["javascript", "js", "typescript", "ts", "jsx", "tsx"])
    }

    pub fn matches(&self, lang: Option<&str>) -> bool {
        match self {
            LanguageMatcher::Exact(langs) => lang.is_some_and(|l| {
                langs
                    .iter()
                    .any(|expected| expected.eq_ignore_ascii_case(l))
            }),
            LanguageMatcher::Prefix(prefix) => {
                lang.is_some_and(|l| l.to_lowercase().starts_with(&prefix.to_lowercase()))
            }
            LanguageMatcher::Custom(f) => lang.is_some_and(|l| f(l)),
            LanguageMatcher::Any => true,
        }
    }
}

pub type CodeBlockActionCallback =
    Arc<dyn Fn(String, Option<String>, &mut Window, &mut App) + Send + Sync>;

#[derive(Clone)]
pub struct CodeBlockAction {
    pub id: SharedString,
    pub icon: IconName,
    pub label: Option<SharedString>,
    pub matcher: LanguageMatcher,
    pub callback: CodeBlockActionCallback,
}

impl CodeBlockAction {
    pub fn new(id: impl Into<SharedString>) -> CodeBlockActionBuilder {
        CodeBlockActionBuilder {
            id: id.into(),
            icon: IconName::SquareTerminal,
            label: None,
            matcher: LanguageMatcher::Any,
            callback: None,
        }
    }
}

pub struct CodeBlockActionBuilder {
    id: SharedString,
    icon: IconName,
    label: Option<SharedString>,
    matcher: LanguageMatcher,
    callback: Option<CodeBlockActionCallback>,
}

impl CodeBlockActionBuilder {
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn matcher(mut self, matcher: LanguageMatcher) -> Self {
        self.matcher = matcher;
        self
    }

    pub fn on_click<F>(mut self, f: F) -> Self
    where
        F: Fn(String, Option<String>, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.callback = Some(Arc::new(f));
        self
    }

    pub fn build(self) -> Option<CodeBlockAction> {
        self.callback.map(|callback| CodeBlockAction {
            id: self.id,
            icon: self.icon,
            label: self.label,
            matcher: self.matcher,
            callback,
        })
    }
}

#[derive(Clone, Default)]
pub struct CodeBlockActionRegistry {
    actions: Vec<CodeBlockAction>,
}

impl CodeBlockActionRegistry {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    pub fn register(&mut self, action: CodeBlockAction) {
        self.actions.push(action);
    }

    pub fn get_actions_for_lang(&self, lang: Option<&str>) -> Vec<&CodeBlockAction> {
        self.actions
            .iter()
            .filter(|action| action.matcher.matches(lang))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn action_previews_for_markdown(&self, markdown: &str) -> Vec<CodeBlockActionPreview> {
        extract_fenced_code_blocks(markdown)
            .into_iter()
            .filter_map(|block| {
                let action_ids: Vec<String> = self
                    .get_actions_for_lang(block.language.as_deref())
                    .into_iter()
                    .map(|action| action.id.to_string())
                    .collect();
                (!action_ids.is_empty()).then(|| CodeBlockActionPreview {
                    language: block.language,
                    code: block.code,
                    action_ids,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_matcher_recognizes_common_dialects() {
        let matcher = LanguageMatcher::sql();

        assert!(matcher.matches(Some("sql")));
        assert!(matcher.matches(Some("PostgreSQL")));
        assert!(!matcher.matches(Some("rust")));
    }

    #[test]
    fn registry_filters_actions_by_language() {
        let sql_action = CodeBlockAction::new("run-sql")
            .matcher(LanguageMatcher::sql())
            .on_click(|_, _, _, _| {})
            .build()
            .expect("action should build");
        let any_action = CodeBlockAction::new("copy")
            .on_click(|_, _, _, _| {})
            .build()
            .expect("action should build");
        let mut registry = CodeBlockActionRegistry::new();
        registry.register(sql_action);
        registry.register(any_action);

        assert_eq!(registry.get_actions_for_lang(Some("sql")).len(), 2);
        assert_eq!(registry.get_actions_for_lang(Some("rust")).len(), 1);
    }

    #[test]
    fn registry_builds_action_previews_for_markdown_code_blocks() {
        let sql_action = CodeBlockAction::new("run-sql")
            .matcher(LanguageMatcher::sql())
            .on_click(|_, _, _, _| {})
            .build()
            .expect("action should build");
        let shell_action = CodeBlockAction::new("paste-shell")
            .matcher(LanguageMatcher::shell())
            .on_click(|_, _, _, _| {})
            .build()
            .expect("action should build");
        let mut registry = CodeBlockActionRegistry::new();
        registry.register(sql_action);
        registry.register(shell_action);

        let previews = registry.action_previews_for_markdown(
            "```sql\nselect 1;\n```\n```bash\necho hi\n```\n```rust\nfn main(){}\n```",
        );

        assert_eq!(2, previews.len());
        assert_eq!(Some("sql"), previews[0].language.as_deref());
        assert_eq!(vec!["run-sql"], previews[0].action_ids);
        assert_eq!(Some("bash"), previews[1].language.as_deref());
        assert_eq!(vec!["paste-shell"], previews[1].action_ids);
    }
}
