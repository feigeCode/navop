use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use super::SKILL_FILE;

const MAX_SCAN_DEPTH: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillLoadError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillCatalog {
    pub skills: Vec<SkillMetadata>,
    pub errors: Vec<SkillLoadError>,
}

impl SkillCatalog {
    pub fn load_from_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut catalog = Self::default();
        for root in roots {
            discover_root(&root, &mut catalog);
        }
        catalog
            .skills
            .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
        catalog.skills.dedup_by(|a, b| a.path == b.path);
        catalog
    }
}

#[derive(Debug, Error)]
pub(super) enum SkillParseError {
    #[error("failed to read skill file: {0}")]
    Read(#[from] io::Error),
    #[error("missing YAML frontmatter delimited by ---")]
    MissingFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    metadata: SkillFrontmatterMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatterMetadata {
    #[serde(default, rename = "short-description")]
    short_description: Option<String>,
}

pub(super) fn parse_skill_file(path: &Path) -> Result<SkillMetadata, SkillParseError> {
    let contents = fs::read_to_string(path)?;
    let frontmatter = extract_frontmatter(&contents).ok_or(SkillParseError::MissingFrontmatter)?;
    let parsed: SkillFrontmatter = serde_yaml::from_str(&frontmatter)?;
    Ok(SkillMetadata {
        name: parsed
            .name
            .as_deref()
            .map(sanitize_single_line)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_skill_name(path)),
        description: parsed
            .description
            .as_deref()
            .map(sanitize_single_line)
            .unwrap_or_default(),
        short_description: parsed
            .metadata
            .short_description
            .as_deref()
            .map(sanitize_single_line)
            .filter(|value| !value.is_empty()),
        path: path.to_path_buf(),
    })
}

fn discover_root(root: &Path, catalog: &mut SkillCatalog) {
    if !root.is_dir() {
        return;
    }
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth > MAX_SCAN_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visit_skill_entry(entry.path(), depth, &mut queue, catalog);
        }
    }
}

fn visit_skill_entry(
    path: PathBuf,
    depth: usize,
    queue: &mut VecDeque<(PathBuf, usize)>,
    catalog: &mut SkillCatalog,
) {
    if hidden_path(&path) {
        return;
    }
    if path.is_dir() {
        queue.push_back((path, depth + 1));
    } else if path.file_name().and_then(|name| name.to_str()) == Some(SKILL_FILE) {
        match parse_skill_file(&path) {
            Ok(skill) => catalog.skills.push(skill),
            Err(error) => catalog.errors.push(SkillLoadError {
                path,
                message: error.to_string(),
            }),
        }
    }
}

fn hidden_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn extract_frontmatter(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        if line == "---" {
            return Some(out.join("\n"));
        }
        out.push(line);
    }
    None
}

fn sanitize_single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn default_skill_name(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(sanitize_single_line)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "skill".to_string())
}
