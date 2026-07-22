use crate::SourceNodeId;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceNodeCompatibility {
    Editable,
    SourceEditable,
    PreservedRaw,
    Unparsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub severity: SourceDiagnosticSeverity,
    pub code: &'static str,
    pub message: String,
    pub source_range: Option<Range<usize>>,
    pub node_id: Option<SourceNodeId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentCompatibility {
    pub fully_editable: bool,
    pub partially_editable: bool,
    pub source_only_nodes: Vec<SourceNodeId>,
    pub diagnostics: Vec<SourceDiagnostic>,
}
