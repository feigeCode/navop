mod compatibility;
mod document;
mod fingerprint;
mod history;
mod inline;
mod node;
mod operations;
mod parser;
mod patch;
mod projection;
mod selection;
mod table;
mod transaction;

pub use compatibility::{
    DocumentCompatibility, SourceDiagnostic, SourceDiagnosticSeverity, SourceNodeCompatibility,
};
pub use document::SourceMarkdownDocument;
pub use fingerprint::SourceFingerprint;
pub use history::{SourceEditTransaction, SourceHistory, SourceParseScope};
pub use node::{
    SourceBlock, SourceBlockKind, SourceImageMap, SourceInlineKind, SourceInlineNode,
    SourceLinkMap, SourceNodeId,
};
pub use operations::{
    BlockMoveDirection, InlineFormat, ListFormat, SourceOperationError, TableCellAddress,
};
pub use parser::SourceParseError;
pub use patch::{PatchError, apply_edits, validate_expected_changes};
pub use projection::{ProjectionError, ProjectionResult, reconcile_projection};
pub use selection::{ActiveInlineSource, SourceSelection};
pub use table::{SourceTableCell, SourceTableMap, SourceTableRow};
pub use transaction::{SourceEdit, SourceEditOrigin, SourceTransaction};
