use crate::{
    Diagnostic, DiagnosticCode, DiagnosticPhase, Diagnostics, NodePath, StateStore, VElement, VNode,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingResolution {
    pub root: VNode,
    pub diagnostics: Diagnostics,
}

pub fn resolve_bindings(node: &VNode, state: &StateStore) -> VNode {
    resolve_bindings_checked(node, state).root
}

pub fn resolve_bindings_checked(node: &VNode, state: &StateStore) -> BindingResolution {
    BindingResolver::new(state).resolve(node)
}

struct BindingResolver<'a> {
    state: &'a StateStore,
    diagnostics: Diagnostics,
}

impl<'a> BindingResolver<'a> {
    fn new(state: &'a StateStore) -> Self {
        Self {
            state,
            diagnostics: Diagnostics::default(),
        }
    }

    fn resolve(mut self, node: &VNode) -> BindingResolution {
        let root = self.resolve_node(node, &NodePath::root());
        BindingResolution {
            root,
            diagnostics: self.diagnostics,
        }
    }

    fn resolve_node(&mut self, node: &VNode, path: &NodePath) -> VNode {
        match node {
            VNode::Element(element) => self.resolve_element(element, path),
            VNode::Text(text) => VNode::Text(text.clone()),
            VNode::Fragment(children) => VNode::Fragment(self.resolve_children(children, path)),
        }
    }

    fn resolve_element(&mut self, element: &VElement, path: &NodePath) -> VNode {
        let mut resolved = element.clone();
        if let Some(key) = element.attr("bind") {
            self.resolve_bound_element(&mut resolved, key, path);
        } else {
            resolved.children = self.resolve_children(&element.children, path);
        }
        VNode::Element(resolved)
    }

    fn resolve_bound_element(&mut self, resolved: &mut VElement, key: &str, path: &NodePath) {
        let value = self.binding_value(key, path);
        if is_input(&resolved.tag) {
            resolved.attrs.insert("value".to_owned(), value);
            resolved.children = self.resolve_children(&resolved.children, path);
        } else {
            resolved.children = vec![VNode::Text(value)];
        }
    }

    fn resolve_children(&mut self, children: &[VNode], path: &NodePath) -> Vec<VNode> {
        children
            .iter()
            .enumerate()
            .map(|(index, child)| self.resolve_node(child, &path.child(index)))
            .collect()
    }

    fn binding_value(&mut self, key: &str, path: &NodePath) -> String {
        if let Some(value) = self.state.get(key) {
            return value.to_owned();
        }
        self.diagnostics.push(
            Diagnostic::warning(
                DiagnosticPhase::Binding,
                DiagnosticCode::MissingBinding,
                format!("state key `{key}` is not defined"),
            )
            .at_path(path.clone()),
        );
        String::new()
    }
}

fn is_input(tag: &str) -> bool {
    tag.eq_ignore_ascii_case("input") || tag.eq_ignore_ascii_case("textarea")
}
