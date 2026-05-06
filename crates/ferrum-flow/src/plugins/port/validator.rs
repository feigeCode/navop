use std::fmt::Display;

use crate::{PluginContext, Port, PortKind};

/// Validates whether an edge may be created between two ports.
pub trait EdgeValidator: Send + Sync {
    fn validate(
        &self,
        from: &Port,
        to: &Port,
        ctx: &PluginContext,
    ) -> Result<(), EdgeValidationError>;
}

#[derive(Debug, Clone)]
pub struct EdgeValidationError {
    code: EdgeValidationErrorCode,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeValidationErrorCode {
    /// The two ports are not one output and one input.
    KindMismatch,
    /// Both ports belong to the same node.
    SameNode,
    /// Port types are incompatible (reserved for stricter validators).
    TypeMismatch,
    /// Target input already has a connection (reserved for stricter validators).
    AlreadyConnected,
    /// Plugin-specific failure reason.
    Custom(String),
}

impl EdgeValidationError {
    pub fn new(code: EdgeValidationErrorCode, message: String) -> Self {
        Self { code, message }
    }

    pub fn kind_mismatch(message: String) -> Self {
        Self::new(EdgeValidationErrorCode::KindMismatch, message)
    }

    pub fn same_node(message: String) -> Self {
        Self::new(EdgeValidationErrorCode::SameNode, message)
    }

    pub fn type_mismatch(message: String) -> Self {
        Self::new(EdgeValidationErrorCode::TypeMismatch, message)
    }

    pub fn already_connected(message: String) -> Self {
        Self::new(EdgeValidationErrorCode::AlreadyConnected, message)
    }

    pub fn custom(ty: String, message: String) -> Self {
        Self::new(EdgeValidationErrorCode::Custom(ty), message)
    }

    pub fn code(&self) -> &EdgeValidationErrorCode {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for EdgeValidationErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdgeValidationErrorCode::KindMismatch => write!(f, "KindMismatch"),
            EdgeValidationErrorCode::SameNode => write!(f, "SameNode"),
            EdgeValidationErrorCode::TypeMismatch => write!(f, "TypeMismatch"),
            EdgeValidationErrorCode::AlreadyConnected => write!(f, "AlreadyConnected"),
            EdgeValidationErrorCode::Custom(ty) => write!(f, "Custom({})", ty),
        }
    }
}

/// Permissive default: requires one output and one input on different nodes; ignores
/// `port_type` and does not check for duplicate edges.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultEdgeValidator;

impl EdgeValidator for DefaultEdgeValidator {
    fn validate(
        &self,
        from: &Port,
        to: &Port,
        _ctx: &PluginContext,
    ) -> Result<(), EdgeValidationError> {
        if from.node_id() == to.node_id() {
            return Err(EdgeValidationError::same_node(
                "Cannot connect two ports on the same node.".into(),
            ));
        }

        let one_output_one_input = matches!(
            (from.kind(), to.kind()),
            (PortKind::Output, PortKind::Input) | (PortKind::Input, PortKind::Output)
        );
        if !one_output_one_input {
            return Err(EdgeValidationError::kind_mismatch(
                "A connection must be between an output port and an input port.".into(),
            ));
        }

        Ok(())
    }
}
