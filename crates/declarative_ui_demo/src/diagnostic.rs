use std::collections::HashSet;

use crate::NodePath;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Compile,
    Binding,
    Render,
    Runtime,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DiagnosticCode {
    UnknownTag,
    UnsupportedClass,
    DuplicateIdentity,
    EmptyAttribute,
    UnsupportedAttribute,
    MissingAttribute,
    ConflictingAttributes,
    MissingBinding,
    ComponentRenderFailed,
    ComponentPanicked,
    RuntimeActionFailed,
    ReconciliationFailed,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub phase: DiagnosticPhase,
    pub code: DiagnosticCode,
    pub message: String,
    pub path: Option<NodePath>,
    pub span: Option<SourceSpan>,
}

impl Diagnostic {
    pub fn error(phase: DiagnosticPhase, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            phase,
            code,
            message: message.into(),
            path: None,
            span: None,
        }
    }

    pub fn warning(
        phase: DiagnosticPhase,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            phase,
            code,
            message: message.into(),
            path: None,
            span: None,
        }
    }

    pub fn at_path(mut self, path: NodePath) -> Self {
        self.path = Some(path);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
    seen: HashSet<Diagnostic>,
}

impl Diagnostics {
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items
            .iter()
            .filter(|item| item.severity == DiagnosticSeverity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items
            .iter()
            .filter(|item| item.severity == DiagnosticSeverity::Warning)
    }

    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn phase(&self, phase: DiagnosticPhase) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(move |item| item.phase == phase)
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        if self.seen.insert(diagnostic.clone()) {
            self.items.push(diagnostic);
        }
    }

    pub(crate) fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        for diagnostic in diagnostics {
            self.push(diagnostic);
        }
    }

    pub(crate) fn replace_phase(
        &mut self,
        phase: DiagnosticPhase,
        diagnostics: impl IntoIterator<Item = Diagnostic>,
    ) {
        self.items.retain(|item| item.phase != phase);
        self.seen.retain(|item| item.phase != phase);
        self.extend(diagnostics);
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Diagnostic, DiagnosticCode, DiagnosticPhase, Diagnostics, NodePath};

    #[test]
    fn identical_diagnostics_are_deduplicated_but_distinct_paths_are_retained() {
        let first = Diagnostic::warning(
            DiagnosticPhase::Binding,
            DiagnosticCode::MissingBinding,
            "missing state",
        )
        .at_path(NodePath(vec![0]));
        let second_path = first.clone().at_path(NodePath(vec![1]));
        let mut diagnostics = Diagnostics::default();

        diagnostics.push(first.clone());
        diagnostics.extend([first, second_path]);

        assert_eq!(2, diagnostics.len());
        assert_eq!(2, diagnostics.phase(DiagnosticPhase::Binding).count());
    }
}
