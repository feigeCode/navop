//! 打开中的 DB 树注册表。
//!
//! 扩展页面拿不到 `Entity<DbTreeView>`（那属于宿主内部），但有时需要让
//! 宿主把树刷新一遍——例如 `navop.db.execute` 跑完 DDL 之后。这里维护一份
//! 弱引用清单，宿主侧的 `navop.db.refreshTree` 通过
//! [`refresh_open_db_trees`] 找到活着的树并逐个刷新。
//!
//! 引用是弱引用：窗口/标签关掉之后树会被回收，下一轮登记或刷新时自然剔除。

use gpui::{App, BorrowAppContext, Entity, Global, WeakEntity};

use crate::db_tree_view::DbTreeView;

/// 当前进程里打开着的 DB 树（弱引用，按创建顺序）。
#[derive(Default)]
pub struct GlobalDbTreeViews {
    trees: Vec<WeakEntity<DbTreeView>>,
}

impl Global for GlobalDbTreeViews {}

/// 登记一棵新打开的树（`DbTreeView` 构造点各调一次，重复登记会去重）。
pub fn register_db_tree_view(tree: Entity<DbTreeView>, cx: &mut impl BorrowAppContext) {
    cx.update_default_global::<GlobalDbTreeViews, _>(|global, _| {
        global.trees.retain(|tree| tree.upgrade().is_some());
        if !global
            .trees
            .iter()
            .any(|known| known.entity_id() == tree.entity_id())
        {
            global.trees.push(tree.downgrade());
        }
    });
}

/// 刷新所有打开着的树；`connection_id` 给定时只刷新该连接的根节点。
///
/// 返回实际发起的刷新次数（0 表示没有匹配的已打开树——调用方可以据此提示
/// “树没打开”而不是假装成功）。必须在主线程调用。
pub fn refresh_open_db_trees(cx: &mut App, connection_id: Option<&str>) -> usize {
    let trees: Vec<Entity<DbTreeView>> = cx
        .try_global::<GlobalDbTreeViews>()
        .map(|global| {
            global
                .trees
                .iter()
                .filter_map(|tree| tree.upgrade())
                .collect()
        })
        .unwrap_or_default();

    let mut refreshed = 0;
    for tree in trees {
        let roots = tree.read(cx).connection_root_ids(connection_id);
        refreshed += roots.len();
        for root in roots {
            tree.update(cx, |view, cx| view.refresh_tree(root, cx));
        }
    }
    refreshed
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::db_tree_view::DbTreeView;

    /// 同一棵树被重复登记时只留一条弱引用（构造点可能在 tab 重建时再跑一次）。
    #[gpui::test]
    fn registering_the_same_tree_twice_keeps_one_entry(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        // 构造树会读全局数据库状态；测试里给默认值就够了。
        cx.update(|cx| cx.set_global(db::GlobalDbState::default()));
        let connections: Vec<one_core::storage::StoredConnection> = Vec::new();
        let (tree, _window) =
            cx.add_window_view(|window, cx| DbTreeView::new(&connections, window, cx));

        cx.update(|cx| {
            register_db_tree_view(tree.clone(), cx);
            register_db_tree_view(tree, cx);
            let count = cx
                .try_global::<GlobalDbTreeViews>()
                .map(|global| global.trees.len())
                .unwrap_or(0);
            assert_eq!(count, 1);
            // 过滤到没有匹配连接时是空操作（不会发起任何加载）。
            assert_eq!(refresh_open_db_trees(cx, Some("no-such-connection")), 0);
        });
    }
}
