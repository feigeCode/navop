use std::collections::HashSet;

use crate::request_store::StoredFolder;

pub(crate) fn descendant_folder_ids(folders: &[StoredFolder], root_id: &str) -> HashSet<String> {
    let mut descendants = HashSet::from([root_id.to_string()]);
    let mut pending = vec![root_id.to_string()];
    while let Some(parent_id) = pending.pop() {
        for child in folders
            .iter()
            .filter(|folder| folder.parent_id.as_deref() == Some(parent_id.as_str()))
        {
            if descendants.insert(child.id.clone()) {
                pending.push(child.id.clone());
            }
        }
    }
    descendants
}

/// 返回从根目录到指定目录的目录 ID 链。
///
/// 目录数据来自用户可编辑的本地 JSON，因此这里必须防御找不到父目录、
/// 自环和多节点环，不能使用无界递归。
pub(crate) fn ancestor_folder_ids(
    folders: &[StoredFolder],
    folder_id: Option<&str>,
) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = folder_id.map(str::to_string);
    let mut visited = HashSet::new();

    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some(folder) = folders.iter().find(|folder| folder.id == id) else {
            break;
        };
        chain.push(folder.id.clone());
        current = folder.parent_id.clone();
    }

    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, parent_id: Option<&str>) -> StoredFolder {
        StoredFolder {
            id: id.to_string(),
            name: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            description: String::new(),
            base_url: None,
            params: Vec::new(),
            headers: Vec::new(),
            variables: Vec::new(),
        }
    }

    #[test]
    fn descendant_folder_ids_contains_root_and_nested_children() {
        let folders = vec![
            folder("root", None),
            folder("child", Some("root")),
            folder("grandchild", Some("child")),
            folder("other", None),
        ];

        assert_eq!(
            descendant_folder_ids(&folders, "root"),
            HashSet::from([
                "root".to_string(),
                "child".to_string(),
                "grandchild".to_string(),
            ])
        );
    }

    #[test]
    fn ancestor_folder_ids_returns_root_to_leaf_and_stops_on_cycles() {
        let folders = vec![
            folder("root", None),
            folder("child", Some("root")),
            folder("leaf", Some("child")),
        ];
        assert_eq!(
            ancestor_folder_ids(&folders, Some("leaf")),
            vec!["root", "child", "leaf"]
        );

        let cycle = vec![
            folder("a", Some("b")),
            folder("b", Some("a")),
            folder("self", Some("self")),
        ];
        // 环没有真实根节点；这里保证 parent-first 的稳定顺序并及时终止。
        assert_eq!(ancestor_folder_ids(&cycle, Some("a")), vec!["b", "a"]);
        assert_eq!(ancestor_folder_ids(&cycle, Some("self")), vec!["self"]);
        assert!(ancestor_folder_ids(&cycle, Some("missing")).is_empty());
    }
}
