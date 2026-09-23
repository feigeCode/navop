//! 表格内查找的契约测试
//!
//! 查找依赖几处不能漂移的接线：结果网格开启 delegate 开关、
//! 二进制列按「可见描述」参与匹配、网格内的 Cmd/Ctrl+F 归
//! 查找所有（而不是被外层对象搜索抢走），以及表数据页只有
//! 工具栏那一个搜索框（表格不再浮出第二个输入框）。

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

fn results_delegate_source() -> &'static str {
    include_str!("results_delegate.rs")
}

fn table_data_tab_source() -> &'static str {
    include_str!("../table_data_tab.rs")
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

#[test]
fn table_data_tab_hands_focus_to_the_table_inside_the_grid() {
    let tab = table_data_tab_source();
    let start = tab
        .find("impl Focusable for TableDataTabContent {")
        .expect("TableDataTabContent 的可聚焦实现");
    let body = &tab[start..];
    let end = body.find("\n}").expect("impl 结束");
    let body = &body[..end];

    // 页签容器激活页签时聚焦的是「页签内容」的 focus handle。如果这里返回
    // 页签外壳自己的句柄，焦点路径里就没有 `EditTable` 键盘上下文，
    // 表内查找（Cmd/Ctrl+F）会完全没反应——必须指向网格内部的表格。
    assert!(body.contains("self.data_grid.read(cx).table_focus_handle(cx)"));
}

#[test]
fn the_toolbar_search_box_is_the_only_find_input() {
    let grid = data_grid_source();

    // 表数据页只允许一个搜索输入框：工具栏那个。表格侧被置为「宿主渲染输入框」，
    // 否则又会出现「上面一个框、浮层又一个框」的重复入口。
    assert!(grid.contains("state.set_find_hosted(true, cx)"));
    assert!(grid.contains("Input::new(&self.find_input)"));
    assert!(!grid.contains("Input::new(&self.search_input)"));
}

#[test]
fn the_toolbar_find_bar_carries_the_counter_and_navigation() {
    let grid = data_grid_source();
    let start = grid.find("fn render_find_bar(").expect("find bar renderer");
    let body = &grid[start..];
    let end = body
        .find("\n    pub fn render_toolbar(")
        .expect("next method");
    let body = &body[..end];

    // 输入框、计数与上下跳必须在同一行，否则又变成分开的两处入口。
    assert!(body.contains("table.find_total()"));
    assert!(body.contains("table.find_current()"));
    assert!(body.contains("Self::handle_find_previous"));
    assert!(body.contains("Self::handle_find_next"));
}

#[test]
fn handing_focus_to_the_toolbar_input_comes_from_the_table_event() {
    let grid = data_grid_source();

    // 在表格里按 Cmd/Ctrl+F 时输入框在工具栏，表格只能发事件请宿主聚焦。
    assert!(grid.contains("EditTableEvent::FindRequested"));
    assert!(grid.contains("focus_search_input(&this.find_input, window, cx)"));
    // 计数变化后宿主必须重新渲染工具栏，否则 17/92 永远停在旧值。
    assert!(grid.contains("EditTableEvent::FindStateChanged => cx.notify()"));
}

#[test]
fn replacing_the_page_data_recomputes_the_matches() {
    let grid = data_grid_source();
    let start = grid
        .find("state.refresh(cx);")
        .expect("refresh after swapping the table data");
    let tail = &grid[start..];

    // 命中缓存按「显示行」存，换页/刷新后整批失效；查询框会跨页保留，
    // 不重扫就会把高亮留在错误的行上。
    assert!(tail.contains("state.refresh_find(cx);"));
}

#[test]
fn local_row_search_is_gone_because_find_replaced_it() {
    let delegate = results_delegate_source();

    // 行过滤的入口（工具栏搜索框）已经改成表内查找，留下的空查询词分支
    // 只会让 filtered_row_count 看起来像在过滤。
    assert!(!delegate.contains("row_search_query"));
    assert!(!delegate.contains("row_matches_search_query"));
}

#[test]
fn the_toolbar_find_bar_keeps_the_table_keyboard_navigation() {
    let grid = data_grid_source();
    let start = grid.find("fn render_find_bar(").expect("find bar renderer");
    let body = &grid[start..];
    let end = body
        .find("\n    pub fn render_toolbar(")
        .expect("next method");
    let body = &body[..end];

    // 搜索框在工具栏，已经不在表格的 `EditTable` 上下文下；不挂这个上下文，
    // 焦点在框里按 Cmd/Ctrl+G 会静默落空（表格那套绑定够不到它）。
    assert!(body.contains(".key_context(HOSTED_FIND_CONTEXT)"));
    assert!(body.contains("Self::on_action_find_next"));
    assert!(body.contains("Self::on_action_find_previous"));
}
