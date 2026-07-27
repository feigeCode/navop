use std::{collections::BTreeMap, fmt};

use thiserror::Error;

use crate::{VElement, VNode};

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct NodePath(pub Vec<usize>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub path: NodePath,
    pub kind: PatchKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchKind {
    Replace { node: VNode },
    SetText { text: String },
    UpdateAttributes { attrs: BTreeMap<String, String> },
    UpdateClasses { classes: Vec<String> },
    InsertChild { index: usize, node: VNode },
    RemoveChild { index: usize },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DiffError {
    #[error("node path does not exist: {0:?}")]
    InvalidPath(NodePath),
    #[error("patch does not apply to the node at path: {0:?}")]
    KindMismatch(NodePath),
}

pub fn diff(old: &VNode, new: &VNode) -> Vec<Patch> {
    DiffBuilder::default().build(old, new)
}

pub fn apply_patches(root: &mut VNode, patches: &[Patch]) -> Result<(), DiffError> {
    let mut candidate = root.clone();
    for patch in patches {
        apply_patch(&mut candidate, patch)?;
    }
    *root = candidate;
    Ok(())
}

#[derive(Default)]
struct DiffBuilder {
    patches: Vec<Patch>,
}

impl DiffBuilder {
    fn build(mut self, old: &VNode, new: &VNode) -> Vec<Patch> {
        self.diff_at(old, new, &NodePath::root());
        self.patches
    }

    fn diff_at(&mut self, old: &VNode, new: &VNode, path: &NodePath) {
        match (old, new) {
            (VNode::Text(old), VNode::Text(new)) => {
                if old != new {
                    self.push(path, PatchKind::SetText { text: new.clone() });
                }
            }
            (VNode::Element(old), VNode::Element(new))
                if old.tag == new.tag && old.key() == new.key() =>
            {
                self.diff_element(old, new, path);
            }
            (VNode::Fragment(old), VNode::Fragment(new)) => self.diff_children(old, new, path),
            _ => self.push(path, PatchKind::Replace { node: new.clone() }),
        }
    }

    fn diff_element(&mut self, old: &VElement, new: &VElement, path: &NodePath) {
        if old.attrs != new.attrs {
            self.push(
                path,
                PatchKind::UpdateAttributes {
                    attrs: new.attrs.clone(),
                },
            );
        }
        if old.classes != new.classes {
            self.push(
                path,
                PatchKind::UpdateClasses {
                    classes: new.classes.clone(),
                },
            );
        }
        self.diff_children(&old.children, &new.children, path);
    }

    fn diff_children(&mut self, old: &[VNode], new: &[VNode], path: &NodePath) {
        for index in 0..old.len().min(new.len()) {
            self.diff_at(&old[index], &new[index], &path.child(index));
        }
        for index in (new.len()..old.len()).rev() {
            self.push(path, PatchKind::RemoveChild { index });
        }
        for (index, node) in new.iter().enumerate().skip(old.len()) {
            self.push(
                path,
                PatchKind::InsertChild {
                    index,
                    node: node.clone(),
                },
            );
        }
    }

    fn push(&mut self, path: &NodePath, kind: PatchKind) {
        self.patches.push(Patch {
            path: path.clone(),
            kind,
        });
    }
}

fn apply_patch(root: &mut VNode, patch: &Patch) -> Result<(), DiffError> {
    let node = node_at_mut(root, &patch.path)?;
    match &patch.kind {
        PatchKind::Replace { node: replacement } => *node = replacement.clone(),
        PatchKind::SetText { text } => set_text(node, text, &patch.path)?,
        PatchKind::UpdateAttributes { attrs } => set_attrs(node, attrs, &patch.path)?,
        PatchKind::UpdateClasses { classes } => set_classes(node, classes, &patch.path)?,
        PatchKind::InsertChild { index, node: child } => insert_child(
            node,
            ChildInsertion {
                index: *index,
                node: child,
            },
            &patch.path,
        )?,
        PatchKind::RemoveChild { index } => remove_child(node, *index, &patch.path)?,
    }
    Ok(())
}

fn node_at_mut<'a>(root: &'a mut VNode, path: &NodePath) -> Result<&'a mut VNode, DiffError> {
    let mut current = root;
    for index in &path.0 {
        current = current
            .children_mut()
            .and_then(|children| children.get_mut(*index))
            .ok_or_else(|| DiffError::InvalidPath(path.clone()))?;
    }
    Ok(current)
}

fn set_text(node: &mut VNode, text: &str, path: &NodePath) -> Result<(), DiffError> {
    let VNode::Text(current) = node else {
        return Err(DiffError::KindMismatch(path.clone()));
    };
    *current = text.to_owned();
    Ok(())
}

fn set_attrs(
    node: &mut VNode,
    attrs: &BTreeMap<String, String>,
    path: &NodePath,
) -> Result<(), DiffError> {
    let VNode::Element(element) = node else {
        return Err(DiffError::KindMismatch(path.clone()));
    };
    element.attrs = attrs.clone();
    Ok(())
}

fn set_classes(node: &mut VNode, classes: &[String], path: &NodePath) -> Result<(), DiffError> {
    let VNode::Element(element) = node else {
        return Err(DiffError::KindMismatch(path.clone()));
    };
    element.classes = classes.to_vec();
    Ok(())
}

struct ChildInsertion<'a> {
    index: usize,
    node: &'a VNode,
}

fn insert_child(
    node: &mut VNode,
    insertion: ChildInsertion<'_>,
    path: &NodePath,
) -> Result<(), DiffError> {
    let children = node
        .children_mut()
        .ok_or_else(|| DiffError::KindMismatch(path.clone()))?;
    if insertion.index > children.len() {
        return Err(DiffError::InvalidPath(path.clone()));
    }
    children.insert(insertion.index, insertion.node.clone());
    Ok(())
}

fn remove_child(node: &mut VNode, index: usize, path: &NodePath) -> Result<(), DiffError> {
    let children = node
        .children_mut()
        .ok_or_else(|| DiffError::KindMismatch(path.clone()))?;
    if index >= children.len() {
        return Err(DiffError::InvalidPath(path.clone()));
    }
    children.remove(index);
    Ok(())
}

impl NodePath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn child(&self, index: usize) -> Self {
        let mut path = self.0.clone();
        path.push(index);
        Self(path)
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str("root");
        }
        for (index, segment) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(".")?;
            }
            write!(formatter, "{segment}")?;
        }
        Ok(())
    }
}
