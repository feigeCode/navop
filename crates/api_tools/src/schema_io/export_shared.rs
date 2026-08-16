use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use super::schema_shared::operation_id;
use crate::http::RequestMethod;

#[derive(Default)]
pub struct OperationIds {
    counts: BTreeMap<String, usize>,
}

impl OperationIds {
    pub fn next(&mut self, name: &str, method: RequestMethod) -> String {
        let base = operation_id(name, method);
        let count = self.counts.entry(base.clone()).or_default();
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{base}_{count}")
        }
    }
}

pub struct OperationTarget {
    pub path: String,
    pub method: String,
}

pub fn insert_operation(
    paths: &mut Map<String, Value>,
    target: OperationTarget,
    operation: Map<String, Value>,
) -> Result<()> {
    let path_item = paths
        .entry(target.path.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(item) = path_item.as_object_mut() else {
        return Err(anyhow!("invalid path item for {}", target.path));
    };
    if item.contains_key(&target.method) {
        return Err(anyhow!(
            "duplicate operation {} {}",
            target.method.to_ascii_uppercase(),
            target.path
        ));
    }
    item.insert(target.method, Value::Object(operation));
    Ok(())
}
