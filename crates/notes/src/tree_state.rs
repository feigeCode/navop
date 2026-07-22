use crate::{DocumentFormat, FileNode, MarkdownViewMode, NodeKind, NotebookUiState};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub relative_path: PathBuf,
    pub display_name: String,
    pub kind: NodeKind,
    pub format: Option<DocumentFormat>,
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TreeState {
    pub selected_document: Option<PathBuf>,
    pub expanded_directories: BTreeSet<PathBuf>,
    pub markdown_view_modes: BTreeMap<String, MarkdownViewMode>,
    pub last_created_format: DocumentFormat,
}

impl TreeState {
    pub fn from_ui_state(state: NotebookUiState) -> Self {
        Self {
            selected_document: state.selected_document,
            expanded_directories: state.expanded_directories,
            markdown_view_modes: state.markdown_view_modes,
            last_created_format: state.last_created_format,
        }
    }

    pub fn to_ui_state(&self) -> NotebookUiState {
        NotebookUiState {
            selected_document: self.selected_document.clone(),
            expanded_directories: self.expanded_directories.clone(),
            markdown_view_modes: self.markdown_view_modes.clone(),
            last_created_format: self.last_created_format,
        }
    }

    pub fn toggle_directory(&mut self, path: &Path) {
        if !self.expanded_directories.remove(path) {
            self.expanded_directories.insert(path.to_path_buf());
        }
    }

    pub fn project(&self, nodes: &[FileNode]) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        project_nodes(nodes, 0, &self.expanded_directories, &mut rows);
        rows
    }

    pub fn select_fallback(&mut self, rows: &[TreeRow]) {
        let selected_exists = self
            .selected_document
            .as_ref()
            .is_some_and(|selected| rows.iter().any(|row| row.relative_path == *selected));
        if !selected_exists {
            self.selected_document = rows
                .iter()
                .find(|row| row.kind == NodeKind::Document)
                .map(|row| row.relative_path.clone());
        }
    }
}

fn project_nodes(
    nodes: &[FileNode],
    depth: usize,
    expanded: &BTreeSet<PathBuf>,
    rows: &mut Vec<TreeRow>,
) {
    for node in nodes {
        let is_expanded = expanded.contains(&node.relative_path);
        rows.push(TreeRow {
            relative_path: node.relative_path.clone(),
            display_name: node.display_name.clone(),
            kind: node.kind,
            format: node.format,
            depth,
            expanded: is_expanded,
        });
        if node.kind == NodeKind::Directory && is_expanded {
            project_nodes(&node.children, depth + 1, expanded, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_empty_and_expanded_directories() {
        let nodes = vec![FileNode {
            relative_path: "work".into(),
            display_name: "work".into(),
            kind: NodeKind::Directory,
            format: None,
            children: vec![FileNode {
                relative_path: "work/note.md".into(),
                display_name: "note".into(),
                kind: NodeKind::Document,
                format: Some(DocumentFormat::Markdown),
                children: Vec::new(),
            }],
        }];
        let mut state = TreeState::default();
        assert_eq!(1, state.project(&nodes).len());
        state.toggle_directory(Path::new("work"));
        let rows = state.project(&nodes);
        assert_eq!(2, rows.len());
        assert_eq!(1, rows[1].depth);
    }
}
