//! 工作空间（分组）层级同步辅助
//!
//! 分组之间的父子关系跨设备传递时必须使用云端 ID 引用：载荷里只携带
//! `parent_cloud_id`，落到目标设备后再解析成本地 `parent_id`——本地整数 ID
//! 只在本机有效，直接上传会让目标设备误绑定到同号的其它分组。
//!
//! 因此这里集中提供两类共享能力：
//!
//! - 处理顺序：父分组必须先于子分组处理，否则子分组拿不到父分组的云端 ID；
//! - 层级对齐：一次同步的所有写入结束后，按云端记录把本地 `parent_id` 重新对齐，
//!   使下载顺序、父分组缺失等情况都能自愈。

use std::collections::{HashMap, HashSet};

use crate::cloud_sync::models::{CloudSyncData, WorkspaceParentLink};
use crate::cloud_sync::service::SyncError;
use crate::storage::{Workspace, WorkspaceRepository};

/// 按「祖先先于后代」稳定排序
///
/// 上传与下载都依赖这个顺序：父分组先落地并取得云端 ID，子分组随后才能
/// 把 `parent_id` 解析成稳定引用。
pub(crate) fn sort_ancestors_first(mut workspaces: Vec<Workspace>) -> Vec<Workspace> {
    let parent_by_id: HashMap<i64, Option<i64>> = workspaces
        .iter()
        .filter_map(|workspace| workspace.id.map(|id| (id, workspace.parent_id)))
        .collect();

    workspaces.sort_by_key(|workspace| hierarchy_depth(workspace.id, &parent_by_id));
    workspaces
}

/// 计算分组在本地树中的深度（对环形父子关系做保护）
fn hierarchy_depth(id: Option<i64>, parent_by_id: &HashMap<i64, Option<i64>>) -> usize {
    let mut depth = 0;
    let mut current = id;
    let mut visited = HashSet::new();

    while let Some(id) = current {
        if !visited.insert(id) {
            break;
        }
        current = parent_by_id.get(&id).copied().flatten();
        if current.is_some() {
            depth += 1;
        }
    }

    depth
}

/// 本地已落地分组的「云端 ID → 本地 ID」映射
pub(crate) fn local_ids_by_cloud_id(workspaces: &[Workspace]) -> HashMap<String, i64> {
    workspaces
        .iter()
        .filter_map(|workspace| workspace.cloud_id.clone().zip(workspace.id))
        .collect()
}

/// 收集本次同步看到的云端分组及其父分组关系
pub(crate) fn cloud_parent_links(
    records: &[CloudSyncData],
    decrypt_link: impl Fn(&CloudSyncData) -> Option<WorkspaceParentLink>,
) -> Vec<(String, WorkspaceParentLink)> {
    records
        .iter()
        .filter_map(|record| decrypt_link(record).map(|link| (record.id.clone(), link)))
        .collect()
}

/// 把云端父分组关系解析为本地 `parent_id`
///
/// 返回 `None` 表示无法判定（旧载荷，或父分组尚未落地到本机），
/// 调用方必须保留本地现有层级，不能当成根分组处理。
pub(crate) fn resolve_local_parent_id(
    link: &WorkspaceParentLink,
    local_ids: &HashMap<String, i64>,
) -> Option<Option<i64>> {
    match link {
        WorkspaceParentLink::Unknown => None,
        WorkspaceParentLink::Root => Some(None),
        WorkspaceParentLink::Cloud(cloud_id) => local_ids.get(cloud_id).copied().map(Some),
    }
}

/// 按云端记录把本地分组的父分组对齐，返回实际写入的行数
///
/// 幂等：只有解析结果与本地现有值不同才写库，并且不改动 `updated_at`，
/// 避免把一次纯层级对齐变成一轮多余的上传。
pub(crate) fn reconcile_parent_links(
    repo: &WorkspaceRepository,
    workspaces: &[Workspace],
    links: &[(String, WorkspaceParentLink)],
) -> Result<usize, SyncError> {
    let local_ids = local_ids_by_cloud_id(workspaces);
    let parent_by_cloud_id: HashMap<&str, Option<i64>> = workspaces
        .iter()
        .filter_map(|workspace| {
            workspace
                .cloud_id
                .as_deref()
                .map(|cloud_id| (cloud_id, workspace.parent_id))
        })
        .collect();

    let mut updated = 0;
    for (cloud_id, link) in links {
        let Some(target_parent) = resolve_local_parent_id(link, &local_ids) else {
            continue;
        };
        let Some(&local_id) = local_ids.get(cloud_id) else {
            continue;
        };
        let Some(current_parent) = parent_by_cloud_id.get(cloud_id.as_str()).copied() else {
            continue;
        };
        if current_parent == target_parent {
            continue;
        }

        repo.update_parent_id(local_id, target_parent)
            .map_err(|error| SyncError::StorageError(error.to_string()))?;
        tracing::info!(
            "[同步] 对齐分组层级: {} {:?} -> {:?}",
            cloud_id,
            current_parent,
            target_parent
        );
        updated += 1;
    }

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::traits::Repository;

    fn stored_workspace(
        id: i64,
        name: &str,
        parent_id: Option<i64>,
        cloud_id: Option<&str>,
    ) -> Workspace {
        let mut workspace = Workspace::new(name.to_string());
        workspace.id = Some(id);
        workspace.parent_id = parent_id;
        workspace.cloud_id = cloud_id.map(str::to_string);
        workspace
    }

    fn repository() -> (tempfile::TempDir, WorkspaceRepository) {
        let temp = tempfile::tempdir().expect("temp directory");
        let connection =
            SqliteConnection::open(temp.path().join("workspaces.db")).expect("open sqlite");
        connection
            .with_connection(run_migrations)
            .expect("migrations run");
        (temp, WorkspaceRepository::new(connection))
    }

    fn names(workspaces: &[Workspace]) -> Vec<&str> {
        workspaces
            .iter()
            .map(|workspace| workspace.name.as_str())
            .collect()
    }

    #[test]
    fn sorting_puts_parents_before_descendants() {
        let workspaces = vec![
            stored_workspace(3, "grandchild", Some(2), None),
            stored_workspace(1, "root", None, None),
            stored_workspace(2, "child", Some(1), None),
        ];

        let sorted = sort_ancestors_first(workspaces);

        assert_eq!(vec!["root", "child", "grandchild"], names(&sorted));
    }

    #[test]
    fn sorting_survives_parent_cycles() {
        let workspaces = vec![
            stored_workspace(1, "first", Some(2), None),
            stored_workspace(2, "second", Some(1), None),
        ];

        let sorted = sort_ancestors_first(workspaces);

        assert_eq!(2, sorted.len());
    }

    #[test]
    fn parent_link_distinguishes_unknown_root_and_unresolved_parent() {
        let local_ids = HashMap::from([("cloud-parent".to_string(), 7_i64)]);

        assert_eq!(
            None,
            resolve_local_parent_id(&WorkspaceParentLink::Unknown, &local_ids)
        );
        assert_eq!(
            Some(None),
            resolve_local_parent_id(&WorkspaceParentLink::Root, &local_ids)
        );
        assert_eq!(
            Some(Some(7)),
            resolve_local_parent_id(
                &WorkspaceParentLink::Cloud("cloud-parent".to_string()),
                &local_ids
            )
        );
        assert_eq!(
            None,
            resolve_local_parent_id(
                &WorkspaceParentLink::Cloud("cloud-missing".to_string()),
                &local_ids
            )
        );
    }

    #[test]
    fn reconciliation_links_child_when_parent_landed_earlier() {
        let (_temp, repo) = repository();
        let mut parent = Workspace::new("parent".to_string());
        let parent_id = repo.insert(&mut parent).expect("parent insert");
        let mut child = Workspace::new("child".to_string());
        let child_id = repo.insert(&mut child).expect("child insert");
        repo.update_cloud_id(parent_id, Some("cloud-parent".to_string()))
            .expect("parent cloud id");
        repo.update_cloud_id(child_id, Some("cloud-child".to_string()))
            .expect("child cloud id");
        let before = repo
            .get(child_id)
            .expect("child load")
            .expect("child exists");

        let workspaces = repo.list().expect("workspaces");
        let links = vec![(
            "cloud-child".to_string(),
            WorkspaceParentLink::Cloud("cloud-parent".to_string()),
        )];
        let updated =
            reconcile_parent_links(&repo, &workspaces, &links).expect("reconcile succeeds");

        let stored = repo
            .get(child_id)
            .expect("child load")
            .expect("child exists");
        assert_eq!(1, updated);
        assert_eq!(Some(parent_id), stored.parent_id);
        // 层级对齐属于同步收敛动作，不应该被记成一次本地修改。
        assert_eq!(before.updated_at, stored.updated_at);
    }

    #[test]
    fn reconciliation_is_idempotent() {
        let (_temp, repo) = repository();
        let mut parent = Workspace::new("parent".to_string());
        let parent_id = repo.insert(&mut parent).expect("parent insert");
        let mut child = Workspace::new("child".to_string());
        let child_id = repo.insert(&mut child).expect("child insert");
        repo.update_cloud_id(parent_id, Some("cloud-parent".to_string()))
            .expect("parent cloud id");
        repo.update_cloud_id(child_id, Some("cloud-child".to_string()))
            .expect("child cloud id");

        let links = vec![(
            "cloud-child".to_string(),
            WorkspaceParentLink::Cloud("cloud-parent".to_string()),
        )];
        let first = repo.list().expect("workspaces");
        reconcile_parent_links(&repo, &first, &links).expect("first reconciliation");
        let second = repo.list().expect("workspaces");
        let updated =
            reconcile_parent_links(&repo, &second, &links).expect("second reconciliation");

        assert_eq!(0, updated);
    }

    #[test]
    fn reconciliation_keeps_local_hierarchy_for_legacy_payloads() {
        let (_temp, repo) = repository();
        let mut parent = Workspace::new("parent".to_string());
        let parent_id = repo.insert(&mut parent).expect("parent insert");
        let mut child = Workspace::new("child".to_string());
        child.parent_id = Some(parent_id);
        let child_id = repo.insert(&mut child).expect("child insert");
        repo.update_cloud_id(child_id, Some("cloud-child".to_string()))
            .expect("child cloud id");

        let workspaces = repo.list().expect("workspaces");
        let links = vec![("cloud-child".to_string(), WorkspaceParentLink::Unknown)];
        let updated =
            reconcile_parent_links(&repo, &workspaces, &links).expect("reconcile succeeds");

        let stored = repo
            .get(child_id)
            .expect("child load")
            .expect("child exists");
        assert_eq!(0, updated);
        assert_eq!(Some(parent_id), stored.parent_id);
    }

    #[test]
    fn reconciliation_keeps_local_hierarchy_when_parent_not_landed() {
        let (_temp, repo) = repository();
        let mut parent = Workspace::new("parent".to_string());
        let parent_id = repo.insert(&mut parent).expect("parent insert");
        let mut child = Workspace::new("child".to_string());
        child.parent_id = Some(parent_id);
        let child_id = repo.insert(&mut child).expect("child insert");
        repo.update_cloud_id(child_id, Some("cloud-child".to_string()))
            .expect("child cloud id");

        let workspaces = repo.list().expect("workspaces");
        let links = vec![(
            "cloud-child".to_string(),
            WorkspaceParentLink::Cloud("cloud-parent-not-downloaded".to_string()),
        )];
        let updated =
            reconcile_parent_links(&repo, &workspaces, &links).expect("reconcile succeeds");

        let stored = repo
            .get(child_id)
            .expect("child load")
            .expect("child exists");
        assert_eq!(0, updated);
        assert_eq!(Some(parent_id), stored.parent_id);
    }

    #[test]
    fn reconciliation_clears_parent_when_cloud_says_root() {
        let (_temp, repo) = repository();
        let mut parent = Workspace::new("parent".to_string());
        let parent_id = repo.insert(&mut parent).expect("parent insert");
        let mut child = Workspace::new("child".to_string());
        child.parent_id = Some(parent_id);
        let child_id = repo.insert(&mut child).expect("child insert");
        repo.update_cloud_id(child_id, Some("cloud-child".to_string()))
            .expect("child cloud id");

        let workspaces = repo.list().expect("workspaces");
        let links = vec![("cloud-child".to_string(), WorkspaceParentLink::Root)];
        let updated =
            reconcile_parent_links(&repo, &workspaces, &links).expect("reconcile succeeds");

        let stored = repo
            .get(child_id)
            .expect("child load")
            .expect("child exists");
        assert_eq!(1, updated);
        assert_eq!(None, stored.parent_id);
    }

    #[test]
    fn reconciliation_ignores_workspaces_without_cloud_id() {
        let (_temp, repo) = repository();
        let mut local_only = Workspace::new("local only".to_string());
        let local_only_id = repo.insert(&mut local_only).expect("workspace insert");

        let workspaces = repo.list().expect("workspaces");
        let links = vec![(
            "cloud-child".to_string(),
            WorkspaceParentLink::Cloud("cloud-parent".to_string()),
        )];
        let updated =
            reconcile_parent_links(&repo, &workspaces, &links).expect("reconcile succeeds");

        let stored = repo
            .get(local_only_id)
            .expect("workspace load")
            .expect("workspace exists");
        assert_eq!(0, updated);
        assert_eq!(None, stored.parent_id);
    }
}
