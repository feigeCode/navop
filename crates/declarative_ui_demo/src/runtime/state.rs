use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateStore {
    values: BTreeMap<String, String>,
}

impl StateStore {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> bool {
        let key = key.into();
        let value = value.into();
        if self.values.get(&key) == Some(&value) {
            return false;
        }
        self.values.insert(key, value);
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }

    pub(super) fn changed_keys(&self, next: &Self) -> BTreeSet<String> {
        self.values
            .keys()
            .chain(next.values.keys())
            .filter(|key| self.values.get(*key) != next.values.get(*key))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateChangeOrigin {
    External,
    Action { name: String, source_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateChange {
    pub revision: u64,
    pub changed_keys: BTreeSet<String>,
    pub origin: StateChangeOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    pub state_changed: bool,
    pub revision: u64,
}
