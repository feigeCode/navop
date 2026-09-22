//! 表格内查找的契约测试
//!
//! 查找依赖三处不能漂移的接线：结果网格开启 delegate 开关、
//! 二进制列按「可见描述」参与匹配、以及网格内的 Cmd/Ctrl+F 归
//! 查找面板所有（而不是被外层对象搜索抢走）。

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

fn results_delegate_source() -> &'static str {
    include_str!("results_delegate.rs")
}

#[test]
fn results_grid_enables_in_table_find() {
    let delegate = results_delegate_source();

    assert!(delegate.contains("fn find_in_table_enabled(&self, _cx: &App) -> bool {"));
    assert!(delegate.contains("fn find_cell_text("));
}

#[test]
fn binary_cells_are_matched_by_their_visible_description() {
    let delegate = results_delegate_source();
    let start = delegate
        .find("fn find_cell_text(")
        .expect("find_cell_text implementation");
    let body = &delegate[start..];
    let end = body.find("\n    fn get_cell_value(").expect("next method");
    let body = &body[..end];

    // 二进制列画的是大小描述，查找必须命中同一段文本，
    // 而不是把原始字节塞进搜索文本。
    assert!(body.contains("current_binary_arc"));
    assert!(body.contains("TableData.binary_value"));
    assert!(!body.contains("from_utf8_lossy"));
}

#[test]
fn table_search_owns_cmd_f_inside_the_results_grid() {
    let grid = data_grid_source();

    // 查找面板由 EditTable 自身承载，因此网格只需要保留
    // 「打开表格查询 / 设计器」这类工具栏动作，不再重复绑定 Cmd+F。
    assert!(grid.contains("EditTable::new(&self.table)"));
    assert!(grid.contains(".on_action(cx.listener(Self::on_action_open_table_designer))"));
}
