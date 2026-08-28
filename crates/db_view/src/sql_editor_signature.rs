//! SQL signature help: locate the active function call at the cursor and build
//! LSP signature help from the metadata snapshot's routine catalog.
//!
//! Call analysis is a pure function from the `db` crate against a local,
//! synchronous `SqlSchema` snapshot, so tests are plain unit tests. The
//! provider mirrors the long-lived default completion/hover providers: schema
//! refresh replaces the inner source atomically without replacing the trait
//! object (spec §25.1).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use db::sql_editor::signature::{
    SqlRoutineSignature, SqlSignatureHelp, signature_help,
};
use gpui::{App, AppContext, Task, Window};
use gpui_component::Rope;
use gpui_component::input::SignatureHelpProvider;
use lsp_types::{
    Documentation, ParameterInformation, ParameterLabel,
    SignatureHelp as LspSignatureHelp, SignatureInformation,
};

use crate::sql_editor::SqlSchema;

/// Long-lived default signature help provider.
#[derive(Clone)]
pub struct DefaultSqlSignatureHelpProvider {
    sources: Rc<RefCell<SqlSignatureSources>>,
}

#[derive(Clone)]
pub(crate) struct SqlSignatureSources {
    pub(crate) schema: Arc<SqlSchema>,
}

impl Default for SqlSignatureSources {
    fn default() -> Self {
        Self {
            schema: Arc::new(SqlSchema::default()),
        }
    }
}

impl DefaultSqlSignatureHelpProvider {
    pub fn new(schema: SqlSchema) -> Self {
        Self {
            sources: Rc::new(RefCell::new(SqlSignatureSources {
                schema: Arc::new(schema),
            })),
        }
    }

    /// Atomically replace the schema snapshot while keeping the provider alive.
    pub fn set_schema(&self, schema: SqlSchema) {
        self.sources.borrow_mut().schema = Arc::new(schema);
    }

    fn snapshot(&self) -> SqlSignatureSources {
        self.sources.borrow().clone()
    }
}

impl SignatureHelpProvider for DefaultSqlSignatureHelpProvider {
    fn signature_help(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<LspSignatureHelp>>> {
        let text = text.to_string();
        let schema = self.snapshot().schema;
        cx.background_spawn(async move { Ok(build_lsp_signature_help(&text, offset, &schema)) })
    }
}

/// Full signature help pipeline: run the pure call analysis and render the
/// result into LSP types against the schema's routine catalog.
pub fn build_lsp_signature_help(
    text: &str,
    offset: usize,
    schema: &SqlSchema,
) -> Option<LspSignatureHelp> {
    let routines = routines_from_schema(schema);
    if routines.is_empty() {
        return None;
    }
    let help = signature_help(text, offset, &routines)?;
    Some(help_to_lsp(&help))
}

/// Build the routine catalog from the schema's `(signature, doc)` fragments.
pub fn routines_from_schema(schema: &SqlSchema) -> Vec<SqlRoutineSignature> {
    schema
        .functions
        .iter()
        .filter_map(|(fragment, doc)| routine_signature_from_fragment(fragment, doc))
        .collect()
}

/// Parse a `name(arg1, arg2)` fragment into a [`SqlRoutineSignature`].
pub fn routine_signature_from_fragment(
    fragment: &str,
    doc: &str,
) -> Option<SqlRoutineSignature> {
    let trimmed = fragment.trim();
    if trimmed.is_empty() {
        return None;
    }
    let open = trimmed.find('(').unwrap_or(trimmed.len());
    let identity = trimmed[..open].trim();
    if identity.is_empty() {
        return None;
    }
    let parameters = if trimmed[open..].starts_with('(') && trimmed.ends_with(')') {
        split_top_level_params(&trimmed[open + 1..trimmed.len() - 1])
    } else {
        Vec::new()
    };
    Some(SqlRoutineSignature {
        identity: identity.to_string(),
        label: trimmed.to_string(),
        parameters,
        return_type: None,
        documentation: (!doc.is_empty()).then(|| doc.to_string()),
    })
}

/// Split comma-separated parameters at the top level, ignoring nested parens
/// (e.g. `DECIMAL(10,2)` stays one parameter).
pub fn split_top_level_params(params: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(params[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = params[start..].trim();
    if !tail.is_empty() {
        result.push(tail.to_string());
    }
    result
}

/// Convert the pure analysis result into LSP signature help.
pub fn help_to_lsp(help: &SqlSignatureHelp) -> LspSignatureHelp {
    let active_parameter = help.active_parameter as u32;
    // Prefer the first overload whose parameter list can host the cursor, so a
    // multi-arg overload is shown over a short one (spec §9.8 overload).
    let active_signature = help
        .signatures
        .iter()
        .position(|signature| signature.parameters.len() > help.active_parameter)
        .unwrap_or(0) as u32;

    let signatures = help
        .signatures
        .iter()
        .enumerate()
        .map(|(index, signature)| {
            let parameters = signature
                .parameters
                .iter()
                .map(|name| ParameterInformation {
                    label: ParameterLabel::Simple(name.clone()),
                    documentation: None,
                })
                .collect::<Vec<_>>();
            let mut information = SignatureInformation {
                label: signature.label.clone(),
                documentation: signature
                    .documentation
                    .clone()
                    .map(Documentation::String),
                parameters: Some(parameters),
                active_parameter: None,
            };
            if index as u32 == active_signature {
                information.active_parameter = Some(active_parameter);
            }
            information
        })
        .collect();

    LspSignatureHelp {
        signatures,
        active_signature: Some(active_signature),
        active_parameter: Some(active_parameter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_with_functions() -> SqlSchema {
        SqlSchema::default().with_functions([
            ("count(*)", "Count all rows"),
            ("count(col)", "Count non-NULL values"),
            ("coalesce(a, b, c)", "Return first non-NULL"),
            (
                "substring(s, start, len)",
                "Extract a substring",
            ),
            ("noargs()", "Takes no arguments"),
        ])
    }

    #[test]
    fn parse_simple_fragment() {
        let signature = routine_signature_from_fragment("count(*)", "Count all rows").unwrap();
        assert_eq!("count", signature.identity);
        assert_eq!(vec!["*"], signature.parameters);
        assert_eq!(Some("Count all rows".to_string()), signature.documentation);
    }

    #[test]
    fn parse_multi_param_fragment() {
        let signature =
            routine_signature_from_fragment("coalesce(a, b, c)", "").unwrap();
        assert_eq!(vec!["a", "b", "c"], signature.parameters);
        assert_eq!("coalesce(a, b, c)", signature.label);
    }

    #[test]
    fn nested_parens_stay_one_parameter() {
        assert_eq!(
            vec!["DECIMAL(10,2)", "x"],
            split_top_level_params("DECIMAL(10,2), x")
        );
        assert_eq!(vec!["a", "b"], split_top_level_params("a, b"));
        assert_eq!(Vec::<String>::new(), split_top_level_params(""));
    }

    #[test]
    fn missing_name_is_rejected() {
        assert!(routine_signature_from_fragment("", "").is_none());
        assert!(routine_signature_from_fragment("   ", "doc").is_none());
        assert!(routine_signature_from_fragment("(a, b)", "").is_none());
    }

    #[test]
    fn build_help_returns_none_without_routines() {
        assert!(build_lsp_signature_help("SELECT upper(ab", 13, &SqlSchema::default()).is_none());
    }

    #[test]
    fn build_help_highlights_active_parameter_inside_call() {
        let schema = schema_with_functions();
        let text = "SELECT coalesce(a, b"; // cursor after `b`
        let cursor = text.len();
        let help = build_lsp_signature_help(text, cursor, &schema).unwrap();
        assert_eq!(1, help.signatures.len());
        assert_eq!(Some(1), help.active_parameter);
        assert_eq!(Some(1), help.signatures[0].active_parameter);
        assert_eq!(Some(0), help.active_signature);
    }

    #[test]
    fn empty_call_sees_first_parameter() {
        let schema = schema_with_functions();
        let text = "SELECT coalesce(";
        let help = build_lsp_signature_help(text, text.len(), &schema).unwrap();
        assert_eq!(Some(0), help.active_parameter);
    }

    #[test]
    fn cursor_after_closing_paren_returns_none() {
        let schema = schema_with_functions();
        let text = "SELECT count(*)";
        assert_eq!(
            None,
            build_lsp_signature_help(text, text.len(), &schema).map(|h| h.active_parameter)
        );
    }
}