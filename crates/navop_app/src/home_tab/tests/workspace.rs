use super::*;

use one_core::storage::{StoredConnection, Workspace};

use crate::home_tab::workspace::promote_workspace_children_in_memory;

fn workspace(id: i64, parent_id: Option<i64>) -> Workspace {
    let mut workspace = Workspace::new(format!("workspace-{id}"));
    workspace.id = Some(id);
    workspace.parent_id = parent_id;
    workspace
}

fn connection(id: i64, workspace_id: Option<i64>) -> StoredConnection {
    let mut connection = stored_external_connection("sqlite");
    connection.id = Some(id);
    connection.workspace_id = workspace_id;
    connection
}

#[test]
fn deleting_middle_workspace_promotes_children_and_connections_to_parent() {
    let mut workspaces = vec![
        workspace(1, None),
        workspace(2, Some(1)),
        workspace(3, Some(2)),
        workspace(4, None),
    ];
    let mut connections = vec![
        connection(10, Some(2)),
        connection(11, Some(3)),
        connection(12, Some(4)),
    ];

    let promoted_to = promote_workspace_children_in_memory(&mut workspaces, &mut connections, 2);

    assert_eq!(Some(1), promoted_to);
    assert!(workspaces.iter().all(|item| item.id != Some(2)));
    let leaf = workspaces
        .iter()
        .find(|item| item.id == Some(3))
        .expect("子分组应保留在树里");
    assert_eq!(Some(1), leaf.parent_id);
    assert_eq!(
        Some(Some(1)),
        connections
            .iter()
            .find(|item| item.id == Some(10))
            .map(|item| item.workspace_id)
    );
    assert_eq!(
        Some(Some(3)),
        connections
            .iter()
            .find(|item| item.id == Some(11))
            .map(|item| item.workspace_id)
    );
}

#[test]
fn deleting_root_workspace_detaches_children_and_connections() {
    let mut workspaces = vec![workspace(1, None), workspace(2, Some(1))];
    let mut connections = vec![connection(10, Some(1))];

    let promoted_to = promote_workspace_children_in_memory(&mut workspaces, &mut connections, 1);

    assert_eq!(None, promoted_to);
    assert_eq!(None, workspaces[0].parent_id);
    assert_eq!(None, connections[0].workspace_id);
}

#[test]
fn creating_workspace_reuses_the_shared_workspace_sort_order() {
    let source = include_str!("../workspace.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);

    assert!(
        production.contains("sort_workspaces(&mut this.workspaces);"),
        "新建分组后应复用 data::sort_workspaces 排序"
    );
    assert!(
        !production.contains("connection_name_cmp(&left.name, &right.name)"),
        "新建分组后不应再按名字全局排序"
    );
}

#[test]
fn workspace_deleted_event_reloads_connections() {
    let source = include_str!("../lifecycle.rs");
    let deleted_arm = source
        .split("ConnectionDataEvent::WorkspaceDeleted { .. } => {")
        .nth(1)
        .expect("WorkspaceDeleted 分支应存在");
    let arm = deleted_arm
        .split("ConnectionDataEvent::CredentialCreated")
        .next()
        .unwrap_or(deleted_arm);

    assert!(arm.contains("this.load_workspaces(cx);"));
    assert!(
        arm.contains("this.load_connections(cx);"),
        "删除分组会改变连接归属，必须重载连接避免其在树里消失"
    );
}
