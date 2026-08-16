use std::collections::BTreeMap;

use serde_json::Value;

use super::schema_shared::folder_for_tag;
use crate::request_store::{StoredFolder, StoredRequest};

pub struct ImportState<'a> {
    pub root: &'a Value,
    pub has_server: bool,
    pub folders: Vec<StoredFolder>,
    pub requests: Vec<StoredRequest>,
    folder_ids: BTreeMap<String, String>,
}

impl<'a> ImportState<'a> {
    pub fn new(root: &'a Value, has_server: bool) -> Self {
        Self {
            root,
            has_server,
            folders: Vec::new(),
            requests: Vec::new(),
            folder_ids: BTreeMap::new(),
        }
    }

    pub fn folder_id(&mut self, tag: Option<&str>) -> Option<String> {
        folder_for_tag(&mut self.folders, &mut self.folder_ids, tag)
    }
}
