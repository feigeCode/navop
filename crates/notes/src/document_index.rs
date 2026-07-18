use crate::DocumentFormat;
use crate::storage_support::write_json_atomic;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) const DOCUMENT_INDEX_FILE: &str = "documents.json";
const DOCUMENT_INDEX_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DocumentRecord {
    pub id: Uuid,
    pub format: DocumentFormat,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DocumentRecord {
    pub(crate) fn new(id: Uuid, format: DocumentFormat) -> Self {
        let now = Utc::now();
        Self {
            id,
            format,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PendingDocumentOperation {
    Rename { from: PathBuf, to: PathBuf },
    Delete { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DocumentIndex {
    pub schema_version: u32,
    pub documents: BTreeMap<PathBuf, DocumentRecord>,
    pub pending_operation: Option<PendingDocumentOperation>,
}

impl Default for DocumentIndex {
    fn default() -> Self {
        Self {
            schema_version: DOCUMENT_INDEX_SCHEMA_VERSION,
            documents: BTreeMap::new(),
            pending_operation: None,
        }
    }
}

impl DocumentIndex {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        write_json_atomic(path, self)
    }

    pub(crate) fn record(&mut self, path: PathBuf, id: Uuid, format: DocumentFormat) {
        self.documents
            .entry(path)
            .and_modify(|record| {
                record.id = id;
                record.format = format;
                record.updated_at = Utc::now();
            })
            .or_insert_with(|| DocumentRecord::new(id, format));
    }

    pub(crate) fn remap_prefix(&mut self, from: &Path, to: &Path) {
        let remapped = self
            .documents
            .iter()
            .filter_map(|(path, record)| {
                let suffix = path.strip_prefix(from).ok()?;
                Some((path.clone(), to.join(suffix), record.clone()))
            })
            .collect::<Vec<_>>();
        for (old, new, mut record) in remapped {
            self.documents.remove(&old);
            record.updated_at = Utc::now();
            self.documents.insert(new, record);
        }
    }

    pub(crate) fn remove_prefix(&mut self, prefix: &Path) {
        self.documents.retain(|path, _| !path.starts_with(prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_directory_records_without_changing_ids() {
        let id = Uuid::new_v4();
        let mut index = DocumentIndex::default();
        index.record(PathBuf::from("old/note.md"), id, DocumentFormat::Markdown);
        index.remap_prefix(Path::new("old"), Path::new("new"));
        assert_eq!(
            Some(&id),
            index.documents.get(Path::new("new/note.md")).map(|r| &r.id)
        );
    }
}
