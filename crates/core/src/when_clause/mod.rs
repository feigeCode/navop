mod parser;
mod tokenizer;

use parser::Parser;
use serde_json::Value as JsonValue;
use tokenizer::tokenize;

#[derive(Debug, Clone, Default)]
pub struct WhenContext {
    root: JsonValue,
}

impl WhenContext {
    pub fn from_json(root: JsonValue) -> Self {
        Self { root }
    }

    pub(super) fn get_path(&self, path: &str) -> Option<&JsonValue> {
        let mut current = &self.root;
        for part in path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WhenClauseError {
    #[error("invalid when clause `{input}`: {message}")]
    Invalid { input: String, message: String },
}

pub fn evaluate(source: &str, context: &WhenContext) -> Result<bool, WhenClauseError> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Ok(true);
    }

    let tokens = tokenize(trimmed)?;
    let mut parser = Parser::new(trimmed, tokens);
    let expr = parser.parse_expr()?;
    parser.expect_end()?;
    Ok(expr.eval(context).truthy())
}

pub(super) fn invalid(input: &str, message: impl Into<String>) -> WhenClauseError {
    WhenClauseError::Invalid {
        input: input.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> WhenContext {
        WhenContext::from_json(json!({
            "connection": { "kind": "postgresql", "id": "conn-1" },
            "node": { "type": "table", "name": "users" },
            "editor": { "focus": true, "language": "sql" },
            "selection": { "empty": false }
        }))
    }

    #[test]
    fn empty_and_true_clauses_are_visible() {
        assert!(evaluate("", &ctx()).unwrap());
        assert!(evaluate("true", &ctx()).unwrap());
    }

    #[test]
    fn equality_and_inequality_use_context_paths() {
        assert!(evaluate("connection.kind == 'postgresql'", &ctx()).unwrap());
        assert!(evaluate("node.type != 'schema'", &ctx()).unwrap());
        assert!(!evaluate("node.name == 'orders'", &ctx()).unwrap());
    }

    #[test]
    fn boolean_operators_respect_precedence() {
        assert!(evaluate("editor.focus && node.type == 'table'", &ctx()).unwrap());
        assert!(evaluate("node.type == 'view' || node.type == 'table'", &ctx()).unwrap());
        assert!(evaluate("!selection.empty", &ctx()).unwrap());
        assert!(!evaluate("!(node.type == 'table')", &ctx()).unwrap());
    }

    #[test]
    fn in_operator_accepts_string_arrays() {
        assert!(evaluate("node.type in ['table', 'view']", &ctx()).unwrap());
        assert!(!evaluate("connection.kind in ['mysql', 'sqlite']", &ctx()).unwrap());
    }

    #[test]
    fn invalid_syntax_returns_error_instead_of_panicking() {
        let err = evaluate("node.type ==", &ctx()).unwrap_err();
        assert!(err.to_string().contains("when clause"));
    }
}
