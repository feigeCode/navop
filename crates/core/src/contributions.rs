#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionProvenance {
    pub extension_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotItem {
    pub command: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub group: Option<String>,
    pub when: Option<String>,
    pub args: serde_json::Value,
    pub provenance: ContributionProvenance,
}

#[derive(Debug, Clone, Default)]
pub struct SlotRegistry {
    entries: Vec<RegisteredSlotItem>,
}

impl SlotRegistry {
    pub fn add(&mut self, position: impl Into<String>, item: SlotItem) {
        self.entries.push(RegisteredSlotItem {
            position: position.into(),
            item,
        });
    }

    pub fn items(&self, position: &str) -> Vec<SlotItem> {
        let mut items = self
            .entries
            .iter()
            .filter(|entry| entry.position == position)
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>();
        items.sort_by_key(slot_sort_key);
        items
    }

    pub fn unregister_extension(&mut self, extension_id: &str) {
        self.entries
            .retain(|entry| entry.item.provenance.extension_id != extension_id);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RegisteredSlotItem {
    position: String,
    item: SlotItem,
}

fn slot_sort_key(item: &SlotItem) -> (String, i32, String) {
    let label = item.label.clone().unwrap_or_else(|| item.command.clone());
    let Some(group) = &item.group else {
        return (String::new(), 0, label);
    };
    let (name, order) = group
        .split_once('@')
        .map(|(name, order)| (name, order.parse::<i32>().unwrap_or(0)))
        .unwrap_or((group.as_str(), 0));
    (name.to_string(), order, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(command: &str, group: &str, extension: &str) -> SlotItem {
        SlotItem {
            command: command.to_string(),
            label: Some(command.to_string()),
            icon: None,
            group: Some(group.to_string()),
            when: None,
            args: serde_json::Value::Null,
            provenance: ContributionProvenance {
                extension_id: extension.to_string(),
            },
        }
    }

    #[test]
    fn registry_returns_sorted_items_for_position() {
        let mut registry = SlotRegistry::default();
        registry.add("db.tree.table", item("late", "extension@20", "ext.a"));
        registry.add("db.tree.table", item("early", "extension@10", "ext.a"));
        let items = registry.items("db.tree.table");
        assert_eq!(
            vec!["early", "late"],
            items.iter().map(|i| i.command.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unregister_extension_removes_only_that_extension() {
        let mut registry = SlotRegistry::default();
        registry.add("db.tree.table", item("a", "extension@10", "ext.a"));
        registry.add("db.tree.table", item("b", "extension@10", "ext.b"));
        registry.unregister_extension("ext.a");
        let items = registry.items("db.tree.table");
        assert_eq!(1, items.len());
        assert_eq!("b", items[0].command);
    }
}
