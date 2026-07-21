use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeInput {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionNodeInput {
    pub id: i64,
    pub workspace_id: Option<i64>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionTreeRow {
    Workspace {
        id: i64,
        name: String,
        depth: usize,
        direct_connection_count: usize,
        has_children: bool,
        expanded: bool,
    },
    Connection {
        id: i64,
        name: String,
        depth: usize,
    },
    Unassigned {
        connection_count: usize,
        expanded: bool,
    },
}

pub(crate) fn build_connection_tree_rows(
    workspaces: &[WorkspaceNodeInput],
    connections: &[ConnectionNodeInput],
    collapsed_workspaces: &HashSet<i64>,
    unassigned_collapsed: bool,
) -> Vec<ConnectionTreeRow> {
    let workspace_by_id: HashMap<_, _> = workspaces.iter().map(|item| (item.id, item)).collect();
    let mut children: HashMap<Option<i64>, Vec<i64>> = HashMap::new();
    for workspace in workspaces {
        let parent = workspace
            .parent_id
            .filter(|parent| *parent != workspace.id && workspace_by_id.contains_key(parent));
        children.entry(parent).or_default().push(workspace.id);
    }

    let root_ids = children.get(&None).cloned().unwrap_or_default();
    let mut builder = TreeBuilder {
        workspace_by_id,
        children,
        connections,
        collapsed: collapsed_workspaces,
        visited: HashSet::new(),
        rows: Vec::new(),
    };
    for id in root_ids {
        builder.append_workspace(id, 0);
    }
    for workspace in workspaces {
        if !builder.visited.contains(&workspace.id) {
            builder.append_workspace(workspace.id, 0);
        }
    }
    append_unassigned_rows(connections, unassigned_collapsed, &mut builder.rows);
    builder.rows
}

pub(crate) fn filter_connection_tree_inputs(
    workspaces: &mut Vec<WorkspaceNodeInput>,
    connections: &mut Vec<ConnectionNodeInput>,
    query: &str,
) {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return;
    }
    let workspace_parent = workspaces
        .iter()
        .map(|workspace| (workspace.id, workspace.parent_id))
        .collect::<HashMap<_, _>>();
    let matched_workspace_ids = workspaces
        .iter()
        .filter(|workspace| workspace.name.to_lowercase().contains(&query))
        .map(|workspace| workspace.id)
        .collect::<HashSet<_>>();
    let mut visible_workspace_ids = matched_workspace_ids.clone();
    connections.retain(|connection| {
        connection.name.to_lowercase().contains(&query)
            || connection
                .workspace_id
                .is_some_and(|workspace_id| matched_workspace_ids.contains(&workspace_id))
    });
    visible_workspace_ids.extend(
        connections
            .iter()
            .filter_map(|connection| connection.workspace_id),
    );
    let mut pending = visible_workspace_ids.iter().copied().collect::<Vec<_>>();
    while let Some(workspace_id) = pending.pop() {
        if let Some(Some(parent_id)) = workspace_parent.get(&workspace_id)
            && visible_workspace_ids.insert(*parent_id)
        {
            pending.push(*parent_id);
        }
    }
    workspaces.retain(|workspace| visible_workspace_ids.contains(&workspace.id));
}

struct TreeBuilder<'a> {
    workspace_by_id: HashMap<i64, &'a WorkspaceNodeInput>,
    children: HashMap<Option<i64>, Vec<i64>>,
    connections: &'a [ConnectionNodeInput],
    collapsed: &'a HashSet<i64>,
    visited: HashSet<i64>,
    rows: Vec<ConnectionTreeRow>,
}

impl TreeBuilder<'_> {
    fn append_workspace(&mut self, id: i64, depth: usize) {
        let Some(workspace) = self.workspace_by_id.get(&id) else {
            return;
        };
        if !self.visited.insert(id) {
            return;
        }
        let child_ids = self.children.get(&Some(id)).cloned().unwrap_or_default();
        let direct_connections: Vec<_> = self
            .connections
            .iter()
            .filter(|connection| connection.workspace_id == Some(id))
            .collect();
        let expanded = !self.collapsed.contains(&id);
        self.rows.push(ConnectionTreeRow::Workspace {
            id,
            name: workspace.name.clone(),
            depth,
            direct_connection_count: direct_connections.len(),
            has_children: !child_ids.is_empty() || !direct_connections.is_empty(),
            expanded,
        });
        if !expanded {
            self.mark_descendants_visited(&child_ids);
            return;
        }
        for child_id in child_ids {
            self.append_workspace(child_id, depth + 1);
        }
        self.rows
            .extend(direct_connections.into_iter().map(|connection| {
                ConnectionTreeRow::Connection {
                    id: connection.id,
                    name: connection.name.clone(),
                    depth: depth + 1,
                }
            }));
    }

    fn mark_descendants_visited(&mut self, child_ids: &[i64]) {
        for child_id in child_ids {
            if !self.visited.insert(*child_id) {
                continue;
            }
            let grandchildren = self
                .children
                .get(&Some(*child_id))
                .cloned()
                .unwrap_or_default();
            self.mark_descendants_visited(&grandchildren);
        }
    }
}

fn append_unassigned_rows(
    connections: &[ConnectionNodeInput],
    collapsed: bool,
    rows: &mut Vec<ConnectionTreeRow>,
) {
    let unassigned: Vec<_> = connections
        .iter()
        .filter(|connection| connection.workspace_id.is_none())
        .collect();
    if unassigned.is_empty() {
        return;
    }
    rows.push(ConnectionTreeRow::Unassigned {
        connection_count: unassigned.len(),
        expanded: !collapsed,
    });
    if !collapsed {
        rows.extend(
            unassigned
                .into_iter()
                .map(|connection| ConnectionTreeRow::Connection {
                    id: connection.id,
                    name: connection.name.clone(),
                    depth: 1,
                }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: i64, parent_id: Option<i64>, name: &str) -> WorkspaceNodeInput {
        WorkspaceNodeInput {
            id,
            parent_id,
            name: name.to_string(),
        }
    }

    fn connection(id: i64, workspace_id: Option<i64>, name: &str) -> ConnectionNodeInput {
        ConnectionNodeInput {
            id,
            workspace_id,
            name: name.to_string(),
        }
    }

    #[test]
    fn nested_groups_render_as_a_tree_before_their_connections() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root"), workspace(2, Some(1), "Child")],
            &[
                connection(10, Some(1), "Root connection"),
                connection(20, Some(2), "Child connection"),
            ],
            &HashSet::new(),
            false,
        );

        assert_eq!(
            vec![
                ("workspace", 0),
                ("workspace", 1),
                ("connection", 2),
                ("connection", 1)
            ],
            row_shape(&rows)
        );
    }

    #[test]
    fn collapsed_group_hides_descendants_but_keeps_the_group_row() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root"), workspace(2, Some(1), "Child")],
            &[connection(10, Some(1), "Connection")],
            &HashSet::from([1]),
            false,
        );

        assert_eq!(vec![("workspace", 0)], row_shape(&rows));
    }

    #[test]
    fn orphaned_and_cyclic_groups_remain_reachable_from_the_root() {
        let rows = build_connection_tree_rows(
            &[
                workspace(1, Some(99), "Orphan"),
                workspace(2, Some(3), "Cycle A"),
                workspace(3, Some(2), "Cycle B"),
            ],
            &[],
            &HashSet::new(),
            false,
        );

        assert_eq!(3, rows.len());
        assert_eq!(
            vec![("workspace", 0), ("workspace", 0), ("workspace", 1)],
            row_shape(&rows)
        );
    }

    #[test]
    fn search_keeps_matching_connections_and_their_workspace_path() {
        let mut workspaces = vec![
            workspace(1, None, "Company"),
            workspace(2, Some(1), "Production"),
            workspace(3, None, "Personal"),
        ];
        let mut connections = vec![
            connection(10, Some(2), "Primary MySQL"),
            connection(20, Some(3), "Local Redis"),
        ];

        filter_connection_tree_inputs(&mut workspaces, &mut connections, "mysql");

        assert_eq!(
            vec![1, 2],
            workspaces.iter().map(|item| item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            vec![10],
            connections.iter().map(|item| item.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn workspace_search_keeps_direct_connections() {
        let mut workspaces = vec![workspace(1, None, "Production")];
        let mut connections = vec![connection(10, Some(1), "Primary MySQL")];

        filter_connection_tree_inputs(&mut workspaces, &mut connections, "production");

        assert_eq!(
            vec![1],
            workspaces.iter().map(|item| item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            vec![10],
            connections.iter().map(|item| item.id).collect::<Vec<_>>()
        );
    }

    fn row_shape(rows: &[ConnectionTreeRow]) -> Vec<(&'static str, usize)> {
        rows.iter()
            .map(|row| match row {
                ConnectionTreeRow::Workspace { depth, .. } => ("workspace", *depth),
                ConnectionTreeRow::Connection { depth, .. } => ("connection", *depth),
                ConnectionTreeRow::Unassigned { .. } => ("unassigned", 0),
            })
            .collect()
    }
}
