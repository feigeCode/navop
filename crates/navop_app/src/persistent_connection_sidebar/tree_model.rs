use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeInput {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// 同级分组的排序权重；None 表示未显式排序。
    /// 与 `home_tab::data::sort_workspaces`、`WorkspaceRepository::list()` 的
    /// `ORDER BY sort_order ASC` 保持一致：None 排在同级最前（SQLite 升序里
    /// NULL 在最前），再按名称自然顺序。
    pub sort_order: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionNodeInput {
    pub id: i64,
    pub workspace_id: Option<i64>,
    pub name: String,
    /// 最近使用时间戳，用于按配置的排序方式（如 LRU）排列连接树
    pub last_used_at: Option<i64>,
    /// 更新时间与创建时间，LRU 排序回退字段，与数据库 COALESCE 顺序保持一致
    pub updated_at: Option<i64>,
    pub created_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionTreeRow {
    RecentHeader {
        count: usize,
        expanded: bool,
    },
    RecentConnection {
        id: i64,
        name: String,
    },
    /// “分组”分区标题：把最近连接区和下方文件夹区分开，本身不可折叠。
    GroupsHeader {
        count: usize,
    },
    /// “未分组”分区标题：把散落在根部的未分组连接和上方文件夹区分开。
    UnassignedHeader {
        count: usize,
        expanded: bool,
    },
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
        workspace_id: Option<i64>,
    },
}

/// 连接树各分区的渲染选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TreeSectionOptions {
    /// 是否渲染“分组”分区标题；搜索或没有任何分组时为 false。
    pub show_groups_header: bool,
    /// 是否渲染“未分组”分区标题；搜索状态下为 false。
    pub show_unassigned_header: bool,
    /// “未分组”分区是否展开；折叠时只保留标题行。
    pub unassigned_expanded: bool,
}

impl Default for TreeSectionOptions {
    fn default() -> Self {
        Self {
            show_groups_header: true,
            show_unassigned_header: true,
            unassigned_expanded: true,
        }
    }
}

pub(crate) fn build_connection_tree_rows(
    workspaces: &[WorkspaceNodeInput],
    connections: &[ConnectionNodeInput],
    collapsed_workspaces: &HashSet<i64>,
    sections: TreeSectionOptions,
) -> Vec<ConnectionTreeRow> {
    let workspace_by_id: HashMap<_, _> = workspaces.iter().map(|item| (item.id, item)).collect();
    let mut children: HashMap<Option<i64>, Vec<i64>> = HashMap::new();
    for workspace in workspaces {
        let parent = workspace
            .parent_id
            .filter(|parent| *parent != workspace.id && workspace_by_id.contains_key(parent));
        children.entry(parent).or_default().push(workspace.id);
    }
    // 同级分组显式排序，不依赖上游切片的顺序（数据库返回顺序、云同步合并顺序等）。
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| {
            match (workspace_by_id.get(left), workspace_by_id.get(right)) {
                (Some(left), Some(right)) => workspace_order_cmp(left, right),
                _ => std::cmp::Ordering::Equal,
            }
        });
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
    // 兜底：父分组缺失时归为根分组已在上面处理；这里只兜住因环状 parent 引用
    // 永远进不了根列表的分组，按同样的同级规则排序后从根开始追加，保证可达。
    let mut fallback_workspaces = workspaces
        .iter()
        .filter(|workspace| !builder.visited.contains(&workspace.id))
        .collect::<Vec<_>>();
    fallback_workspaces.sort_by(|left, right| workspace_order_cmp(left, right));
    for workspace in fallback_workspaces {
        if !builder.visited.contains(&workspace.id) {
            builder.append_workspace(workspace.id, 0);
        }
    }
    append_unassigned_connections(
        connections,
        &builder.workspace_by_id,
        &mut builder.rows,
        sections,
    );
    // “分组”标题固定在最近连接区之后、第一个分组之前：把实际渲染的分组
    // （已被折叠/空分组过滤影响）数量作为计数，没有分组时不出空标题。
    let group_count = builder
        .rows
        .iter()
        .filter(|row| matches!(row, ConnectionTreeRow::Workspace { .. }))
        .count();
    if sections.show_groups_header && group_count > 0 {
        builder
            .rows
            .insert(0, ConnectionTreeRow::GroupsHeader { count: group_count });
    }
    builder.rows
}

/// 同级分组的比较规则：sort_order 升序（None 排同级最前，与 SQLite 的
/// `ORDER BY sort_order ASC` 及 `home_tab::data::sort_workspaces` 一致），
/// 再按名称的自然顺序。
fn workspace_order_cmp(
    left: &WorkspaceNodeInput,
    right: &WorkspaceNodeInput,
) -> std::cmp::Ordering {
    left.sort_order
        .cmp(&right.sort_order)
        .then_with(|| crate::connection_sort::connection_name_cmp(&left.name, &right.name))
}

pub(crate) fn filter_connection_tree_inputs(
    workspaces: &mut Vec<WorkspaceNodeInput>,
    connections: &mut Vec<ConnectionNodeInput>,
    query: &str,
    connection_matches: impl Fn(&ConnectionNodeInput) -> bool,
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
        connection_matches(connection)
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

pub(crate) fn hide_empty_workspace_inputs(
    workspaces: &mut Vec<WorkspaceNodeInput>,
    connections: &[ConnectionNodeInput],
) {
    let workspace_parent = workspaces
        .iter()
        .map(|workspace| (workspace.id, workspace.parent_id))
        .collect::<HashMap<_, _>>();
    let mut visible_workspace_ids = connections
        .iter()
        .filter_map(|connection| connection.workspace_id)
        .collect::<HashSet<_>>();
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
                    workspace_id: Some(id),
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

fn append_unassigned_connections(
    connections: &[ConnectionNodeInput],
    workspace_by_id: &HashMap<i64, &WorkspaceNodeInput>,
    rows: &mut Vec<ConnectionTreeRow>,
    sections: TreeSectionOptions,
) {
    // 除了真正的「未分组」，分组已经不在列表里的连接也归到这里：它们既不属于
    // 任何现存分组，又不会出现在下面的分组行里，否则会从侧栏里彻底消失
    // （分组被别处删除、同步中分组尚未落地时都能碰到）。
    let unassigned = connections
        .iter()
        .filter(|connection| {
            connection
                .workspace_id
                .is_none_or(|workspace_id| !workspace_by_id.contains_key(&workspace_id))
        })
        .collect::<Vec<_>>();
    if unassigned.is_empty() {
        return;
    }
    // 未分组连接自成独立分区：有标题时先插入标题行，折叠状态下只保留标题行，
    // 与上方文件夹区、上方（或顶部的）最近连接区形成明确的分区界限。
    if sections.show_unassigned_header {
        rows.push(ConnectionTreeRow::UnassignedHeader {
            count: unassigned.len(),
            expanded: sections.unassigned_expanded,
        });
        if !sections.unassigned_expanded {
            return;
        }
    }
    rows.extend(
        unassigned
            .into_iter()
            .map(|connection| ConnectionTreeRow::Connection {
                id: connection.id,
                name: connection.name.clone(),
                depth: 0,
                workspace_id: None,
            }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: i64, parent_id: Option<i64>, name: &str) -> WorkspaceNodeInput {
        WorkspaceNodeInput {
            id,
            parent_id,
            name: name.to_string(),
            sort_order: None,
        }
    }

    fn ordered_workspace(
        id: i64,
        parent_id: Option<i64>,
        name: &str,
        sort_order: i32,
    ) -> WorkspaceNodeInput {
        WorkspaceNodeInput {
            sort_order: Some(sort_order),
            ..workspace(id, parent_id, name)
        }
    }

    /// 大多数用例关注层级本身，默认关掉“分组”标题避免干扰；
    /// 分区标题由专门的用例覆盖。
    fn plain_section() -> TreeSectionOptions {
        TreeSectionOptions {
            show_groups_header: false,
            ..TreeSectionOptions::default()
        }
    }

    fn sectioned(
        show_groups_header: bool,
        show_unassigned_header: bool,
        unassigned_expanded: bool,
    ) -> TreeSectionOptions {
        TreeSectionOptions {
            show_groups_header,
            show_unassigned_header,
            unassigned_expanded,
        }
    }

    fn connection(id: i64, workspace_id: Option<i64>, name: &str) -> ConnectionNodeInput {
        ConnectionNodeInput {
            id,
            workspace_id,
            name: name.to_string(),
            last_used_at: None,
            updated_at: None,
            created_at: None,
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
            plain_section(),
        );

        assert_eq!(
            vec![
                ("workspace", 0),
                ("workspace", 1),
                ("connection", 2),
                ("connection", 1),
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
            plain_section(),
        );

        assert_eq!(vec![("workspace", 0)], row_shape(&rows));
    }

    #[test]
    fn unassigned_connections_get_their_own_section_header() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[
                connection(10, Some(1), "Workspace connection"),
                connection(20, None, "Loose connection"),
            ],
            &HashSet::new(),
            plain_section(),
        );

        // 标题插在文件夹区之后、未分组连接之前，计数只统计未分组连接。
        assert_eq!(
            vec![
                ("workspace", 0),
                ("connection", 1),
                ("unassigned-header", 0),
                ("connection", 0),
            ],
            row_shape(&rows)
        );
        assert!(matches!(
            rows.get(2),
            Some(ConnectionTreeRow::UnassignedHeader {
                count: 1,
                expanded: true,
            })
        ));
        assert!(matches!(
            rows.last(),
            Some(ConnectionTreeRow::Connection {
                id: 20,
                depth: 0,
                workspace_id: None,
                ..
            })
        ));
    }

    #[test]
    fn unassigned_header_counts_only_loose_connections() {
        let rows = build_connection_tree_rows(
            &[
                workspace(1, None, "Root"),
                workspace(2, None, "Another root"),
            ],
            &[
                connection(10, Some(1), "Workspace connection"),
                connection(20, None, "Loose A"),
                connection(30, None, "Loose B"),
            ],
            &HashSet::new(),
            plain_section(),
        );

        assert!(matches!(
            rows.iter()
                .find(|row| matches!(row, ConnectionTreeRow::UnassignedHeader { .. })),
            Some(ConnectionTreeRow::UnassignedHeader {
                count: 2,
                expanded: true,
            })
        ));
    }

    #[test]
    fn connection_pointing_at_a_missing_workspace_falls_back_to_unassigned() {
        // 分组已被删掉（或同步中尚未落地）时，连接既不在任何分组行里，
        // 也曾经不算「未分组」⇒ 会从侧栏彻底消失，这类连接必须落到未分组区。
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[
                connection(10, Some(1), "Workspace connection"),
                connection(20, Some(99), "Orphan connection"),
            ],
            &HashSet::new(),
            sectioned(false, true, true),
        );

        assert_eq!(
            vec![
                ("workspace", 0),
                ("connection", 1),
                ("unassigned-header", 0),
                ("connection", 0),
            ],
            row_shape(&rows)
        );
        assert!(matches!(
            rows.last(),
            Some(ConnectionTreeRow::Connection {
                id: 20,
                workspace_id: None,
                ..
            })
        ));
    }

    #[test]
    fn no_unassigned_connections_adds_no_section_header() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[connection(10, Some(1), "Connection")],
            &HashSet::new(),
            plain_section(),
        );

        assert_eq!(vec![("workspace", 0), ("connection", 1)], row_shape(&rows));
    }

    #[test]
    fn collapsed_unassigned_section_keeps_only_its_header() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[connection(20, None, "Loose connection")],
            &HashSet::new(),
            sectioned(false, true, false),
        );

        assert_eq!(
            vec![("workspace", 0), ("unassigned-header", 0)],
            row_shape(&rows)
        );
        assert!(matches!(
            rows.last(),
            Some(ConnectionTreeRow::UnassignedHeader {
                count: 1,
                expanded: false,
            })
        ));
    }

    #[test]
    fn search_results_skip_the_unassigned_section_header() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[connection(20, None, "Loose connection")],
            &HashSet::new(),
            sectioned(false, false, false),
        );

        // 搜索时分区标题不重复出现，但命中的未分组连接仍然可见。
        assert_eq!(vec![("workspace", 0), ("connection", 0)], row_shape(&rows));
    }

    #[test]
    fn sibling_groups_follow_sort_order_then_natural_name() {
        let rows = build_connection_tree_rows(
            &[
                // 故意打乱输入顺序，验证排序不依赖上游切片。
                ordered_workspace(1, None, "Group 10", 10),
                ordered_workspace(2, None, "Group 2", 2),
                ordered_workspace(3, None, "Beta", 1),
                workspace(4, None, "Untitled"),
                workspace(5, None, "Alpha"),
            ],
            &[],
            &HashSet::new(),
            plain_section(),
        );

        // sort_order 升序；None 排在同级最前（SQLite 升序里 NULL 在最前），
        // 同为 None 再按名称自然排序。
        assert_eq!(
            vec![5, 4, 3, 2, 1],
            rows.iter()
                .filter_map(|row| match row {
                    ConnectionTreeRow::Workspace { id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(vec![("workspace", 0); 5], row_shape(&rows));
    }

    #[test]
    fn groups_header_counts_rendered_groups_and_sits_before_them() {
        let workspaces = [
            workspace(1, None, "Root"),
            workspace(2, Some(1), "Child"),
            workspace(3, None, "Another root"),
        ];
        let rows = build_connection_tree_rows(
            &workspaces,
            &[connection(10, Some(2), "Child connection")],
            &HashSet::new(),
            sectioned(true, false, true),
        );

        // “分组”标题固定在最前，计数是实际渲染的分组数。
        // 两个根分组都没有 sort_order，按名称自然序 "Another root" < "Root"。
        assert_eq!(
            vec![
                ("groups-header", 0),
                ("workspace", 0),
                ("workspace", 0),
                ("workspace", 1),
                ("connection", 2),
            ],
            row_shape(&rows)
        );
        assert!(matches!(
            rows.first(),
            Some(ConnectionTreeRow::GroupsHeader { count: 3 })
        ));

        // 折叠父分组后子分组不再渲染，计数随之减少。
        let collapsed_rows = build_connection_tree_rows(
            &workspaces,
            &[],
            &HashSet::from([1]),
            sectioned(true, false, true),
        );
        assert!(matches!(
            collapsed_rows.first(),
            Some(ConnectionTreeRow::GroupsHeader { count: 2 })
        ));
    }

    #[test]
    fn groups_header_is_skipped_when_no_group_is_rendered() {
        let rows = build_connection_tree_rows(
            &[],
            &[connection(20, None, "Loose connection")],
            &HashSet::new(),
            sectioned(true, false, true),
        );

        assert_eq!(vec![("connection", 0)], row_shape(&rows));
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, ConnectionTreeRow::GroupsHeader { .. }))
        );
    }

    #[test]
    fn groups_header_is_skipped_while_searching() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root")],
            &[connection(10, Some(1), "Connection")],
            &HashSet::new(),
            sectioned(false, false, true),
        );

        assert_eq!(vec![("workspace", 0), ("connection", 1)], row_shape(&rows));
    }

    #[test]
    fn none_sort_order_precedes_explicit_ones_like_database_reload() {
        // 与 WorkspaceRepository::list() 的 `ORDER BY sort_order ASC`
        // （NULL 在最前）以及 home_tab::data::sort_workspaces 保持一致，
        // 否则“新建分组后”和“重新加载后”的同级顺序会不一样。
        let rows = build_connection_tree_rows(
            &[
                ordered_workspace(1, None, "Explicit", 1),
                workspace(2, None, "Implicit B"),
                workspace(3, None, "Implicit A"),
            ],
            &[],
            &HashSet::new(),
            plain_section(),
        );

        assert_eq!(
            vec![3, 2, 1],
            rows.iter()
                .filter_map(|row| match row {
                    ConnectionTreeRow::Workspace { id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn nested_siblings_sort_within_their_own_parent() {
        let rows = build_connection_tree_rows(
            &[
                ordered_workspace(1, None, "Root", 0),
                ordered_workspace(3, Some(1), "Child B", 2),
                ordered_workspace(4, Some(1), "Child A", 1),
                ordered_workspace(2, None, "Root sibling", 1),
            ],
            &[],
            &HashSet::new(),
            plain_section(),
        );

        assert_eq!(
            vec![(1, 0), (4, 1), (3, 1), (2, 0)],
            rows.iter()
                .filter_map(|row| match row {
                    ConnectionTreeRow::Workspace { id, depth, .. } => Some((*id, *depth)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn orphaned_group_joins_root_siblings_in_sorted_order() {
        let rows = build_connection_tree_rows(
            &[
                ordered_workspace(1, None, "Regular root", 5),
                // 父分组已不存在：兜底为根分组，并按自己的 sort_order 参与排序。
                ordered_workspace(2, Some(99), "Orphan", 1),
                ordered_workspace(3, None, "Later root", 9),
            ],
            &[],
            &HashSet::new(),
            plain_section(),
        );

        assert_eq!(
            vec![2, 1, 3],
            rows.iter()
                .filter_map(|row| match row {
                    ConnectionTreeRow::Workspace { id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn connection_rows_keep_their_workspace_drop_target() {
        let rows = build_connection_tree_rows(
            &[workspace(1, None, "Root"), workspace(2, Some(1), "Child")],
            &[
                connection(10, Some(1), "Root connection"),
                connection(20, Some(2), "Child connection"),
                connection(30, None, "Unassigned connection"),
            ],
            &HashSet::new(),
            plain_section(),
        );

        let drop_targets = rows
            .iter()
            .filter_map(|row| match row {
                ConnectionTreeRow::Connection {
                    id, workspace_id, ..
                } => Some((*id, *workspace_id)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(vec![(20, Some(2)), (10, Some(1)), (30, None)], drop_targets);
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
            plain_section(),
        );

        assert_eq!(3, rows.len());
        assert_eq!(
            vec![("workspace", 0), ("workspace", 0), ("workspace", 1),],
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

        filter_connection_tree_inputs(&mut workspaces, &mut connections, "mysql", |connection| {
            connection.name.to_lowercase().contains("mysql")
        });

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

        filter_connection_tree_inputs(
            &mut workspaces,
            &mut connections,
            "production",
            |connection| connection.name.to_lowercase().contains("production"),
        );

        assert_eq!(
            vec![1],
            workspaces.iter().map(|item| item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            vec![10],
            connections.iter().map(|item| item.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_keeps_connection_matched_by_non_name_field() {
        let mut workspaces = vec![workspace(1, None, "Production")];
        let mut connections = vec![
            connection(10, Some(1), "Primary database"),
            connection(20, Some(1), "Replica database"),
        ];

        filter_connection_tree_inputs(
            &mut workspaces,
            &mut connections,
            "192.168.10.24",
            |connection| connection.id == 10,
        );

        assert_eq!(
            vec![1],
            workspaces.iter().map(|item| item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            vec![10],
            connections.iter().map(|item| item.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hide_empty_workspace_removes_empty_subtrees() {
        let mut workspaces = vec![
            workspace(1, None, "Empty root"),
            workspace(2, Some(1), "Empty child"),
        ];

        hide_empty_workspace_inputs(&mut workspaces, &[]);

        assert!(workspaces.is_empty());
    }

    #[test]
    fn hide_empty_workspace_keeps_ancestors_of_connected_descendants() {
        let mut workspaces = vec![
            workspace(1, None, "Root"),
            workspace(2, Some(1), "Child"),
            workspace(3, Some(2), "Grandchild"),
        ];
        let connections = vec![connection(10, Some(3), "Connection")];

        hide_empty_workspace_inputs(&mut workspaces, &connections);

        assert_eq!(
            vec![1, 2, 3],
            workspaces
                .iter()
                .map(|workspace| workspace.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn hide_empty_workspace_removes_empty_sibling_branches() {
        let mut workspaces = vec![
            workspace(1, None, "Root"),
            workspace(2, Some(1), "Connected"),
            workspace(3, Some(1), "Empty"),
        ];
        let connections = vec![connection(10, Some(2), "Connection")];

        hide_empty_workspace_inputs(&mut workspaces, &connections);

        assert_eq!(
            vec![1, 2],
            workspaces
                .iter()
                .map(|workspace| workspace.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn hide_empty_workspace_removes_workspace_matched_only_by_name() {
        let mut workspaces = vec![workspace(1, None, "Empty production")];
        let mut connections = Vec::new();

        filter_connection_tree_inputs(&mut workspaces, &mut connections, "production", |_| false);
        hide_empty_workspace_inputs(&mut workspaces, &connections);

        assert!(workspaces.is_empty());
    }

    fn row_shape(rows: &[ConnectionTreeRow]) -> Vec<(&'static str, usize)> {
        rows.iter()
            .map(|row| match row {
                ConnectionTreeRow::RecentHeader { .. } => ("recent-header", 0),
                ConnectionTreeRow::RecentConnection { .. } => ("recent-connection", 0),
                ConnectionTreeRow::GroupsHeader { .. } => ("groups-header", 0),
                ConnectionTreeRow::UnassignedHeader { .. } => ("unassigned-header", 0),
                ConnectionTreeRow::Workspace { depth, .. } => ("workspace", *depth),
                ConnectionTreeRow::Connection { depth, .. } => ("connection", *depth),
            })
            .collect()
    }
}
