use std::fmt::Display;

use crate::{Graph, NodeId, PluginContext, Port, PortKind, PortScope};

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
    /// A [`PortScope::Local`] port would connect across hierarchy levels.
    ScopeMismatch,
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

    pub fn scope_mismatch(message: String) -> Self {
        Self::new(EdgeValidationErrorCode::ScopeMismatch, message)
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
            EdgeValidationErrorCode::ScopeMismatch => write!(f, "ScopeMismatch"),
            EdgeValidationErrorCode::TypeMismatch => write!(f, "TypeMismatch"),
            EdgeValidationErrorCode::AlreadyConnected => write!(f, "AlreadyConnected"),
            EdgeValidationErrorCode::Custom(ty) => write!(f, "Custom({})", ty),
        }
    }
}

/// Permissive default: requires one output and one input on different nodes; enforces
/// [`PortScope::Local`] (same direct parent only). Ignores `port_type` and duplicate edges.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultEdgeValidator;

impl EdgeValidator for DefaultEdgeValidator {
    fn validate(
        &self,
        from: &Port,
        to: &Port,
        ctx: &PluginContext,
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

        if from.scope() == PortScope::Local || to.scope() == PortScope::Local {
            let graph = &ctx.graph;
            if !nodes_share_direct_parent(graph, from.node_id(), to.node_id()) {
                return Err(EdgeValidationError::scope_mismatch(
                    "Local ports may only connect to nodes with the same parent.".into(),
                ));
            }
        }

        Ok(())
    }
}

/// Same hierarchy level: identical direct parent (including both root-level).
fn nodes_share_direct_parent(graph: &Graph, a: NodeId, b: NodeId) -> bool {
    let parent_a = graph.get_node(&a).and_then(|n| n.parent());
    let parent_b = graph.get_node(&b).and_then(|n| n.parent());
    parent_a == parent_b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;

    #[test]
    fn nodes_share_direct_parent_siblings_and_roots() {
        let mut g = Graph::new();
        let parent = g.create_node("default").build();
        let (a, _, _) = g.create_node("default").build_with_ports();
        let (b, _, _) = g.create_node("default").build_with_ports();
        g.add_child(parent, a).unwrap();
        g.add_child(parent, b).unwrap();
        assert!(nodes_share_direct_parent(&g, a, b));

        let r1 = g.create_node("default").build();
        let r2 = g.create_node("default").build();
        assert!(nodes_share_direct_parent(&g, r1, r2));
        assert!(!nodes_share_direct_parent(&g, a, r1));
    }
}
