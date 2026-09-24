//! 纵向「列：值」显示方式的契约测试
//!
//! 纵向形态与网格共用同一份数据、同一套列可见性，但渲染入口完全不同。
//! 这里锁住几处不能漂移的接线：
//!
//! 1. 扁平行号 ↔「记录头 / 字段行」的映射（`uniform_list` 要求等高，
//!    记录头与字段行必须共用同一个行号空间）；
//! 2. 工具栏必须有显示方式入口，且当前形态要打勾；
//! 3. 渲染入口按模式分支，网格形态仍然走 `EditTable`；
//! 4. 纵向列表必须虚拟化，并且复用表格行高设置；
//! 5. 单元格值只经 delegate 的换算接口取，视图层不碰原始行/列坐标；
//! 6. 数据字体在渲染入口解析一次（字体枚举是系统级全量调用），不按行解析；
//! 7. 网格专属的交互入口（编辑、表内查找、大文本编辑器）在纵向形态下收起；
//! 8. 显示方式是跨结果集保留的偏好，不能只存在 `DataGrid` 实例上。

use super::data_grid::{VerticalLine, vertical_line_at, vertical_line_count};
use rust_i18n::t;

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

fn slice_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source
        .find(start)
        .unwrap_or_else(|| panic!("找不到起点 `{start}`"));
    let body = &source[start..];
    let end = body
        .find(end)
        .unwrap_or_else(|| panic!("找不到终点 `{end}`"));
    &body[..end]
}

fn count_matches(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn vertical_line_count_adds_one_header_line_per_record() {
    assert_eq!(0, vertical_line_count(0, 3));
    assert_eq!(4, vertical_line_count(1, 3));
    assert_eq!(8, vertical_line_count(2, 3));
    // 没有可见列就没有任何一行，而不是只剩记录头。
    assert_eq!(0, vertical_line_count(10, 0));
}

#[test]
fn vertical_lines_map_back_to_records_and_fields() {
    // 3 个可见列：记录头 + 3 条字段，共 4 行构成一条记录。
    let expected = [
        VerticalLine::Record { row: 0 },
        VerticalLine::Field { row: 0, col: 0 },
        VerticalLine::Field { row: 0, col: 1 },
        VerticalLine::Field { row: 0, col: 2 },
        VerticalLine::Record { row: 1 },
        VerticalLine::Field { row: 1, col: 0 },
        VerticalLine::Field { row: 1, col: 1 },
        VerticalLine::Field { row: 1, col: 2 },
    ];

    assert_eq!(expected.len(), vertical_line_count(2, 3));
    for (line_ix, line) in expected.into_iter().enumerate() {
        assert_eq!(Some(line), vertical_line_at(line_ix, 3), "第 {line_ix} 行");
    }
}

#[test]
fn vertical_lines_are_empty_without_visible_columns() {
    assert_eq!(None, vertical_line_at(0, 0));
    assert_eq!(None, vertical_line_at(7, 0));
}

#[test]
fn vertical_view_labels_resolve_to_translations() {
    // `t!` 在词条缺失时会把 key 原样返回 —— 界面上就会直接出现
    // `TableDataGrid.view_mode` 这种字样。key 打错必须在这里炸。
    for key in [
        "TableDataGrid.view_mode",
        "TableDataGrid.view_mode_grid",
        "TableDataGrid.view_mode_vertical",
        "TableDataGrid.vertical_record",
        "TableDataGrid.vertical_empty",
        "TableDataGrid.vertical_loading",
    ] {
        assert_ne!(key, t!(key).as_ref(), "词条 `{key}` 缺失");
    }
}

#[test]
fn the_toolbar_exposes_the_display_mode_switch() {
    let grid = data_grid_source();

    // 两个形态都要能选到，且当前形态要打勾，否则切换进得去出不来。
    assert!(grid.contains("fn render_view_mode_button("));
    assert!(grid.contains("Button::new(\"view-mode\")"));
    assert!(grid.contains("build_view_mode_menu("));
    assert!(grid.contains("TableDataGrid.view_mode_grid"));
    assert!(grid.contains("TableDataGrid.view_mode_vertical"));
    assert!(grid.contains(".checked(current == mode)"));
}

#[test]
fn the_render_entry_branches_on_the_display_mode() {
    let grid = data_grid_source();
    let body = slice_between(
        grid,
        "pub fn render_table_area(",
        "\n    fn render_status_bar(",
    );

    assert!(body.contains("self.is_vertical_view(cx)"));
    assert!(body.contains("self.render_vertical_view(&font, cx)"));
    // 网格形态仍然是 `EditTable`（`find_contract_tests` 依赖这一行）。
    assert!(body.contains("EditTable::new(&self.table)"));
    // 纵向形态先解析数据字体再渲染，解析动作不允许下放到按行渲染里。
    assert!(body.contains("let font = self.vertical_view_font(cx);"));
}

#[test]
fn the_vertical_list_is_virtualized_with_the_table_row_height() {
    let grid = data_grid_source();
    let body = slice_between(
        grid,
        "fn render_vertical_view(",
        "\n    /// 「字段过滤」入口",
    );

    // 一页可能上千条记录、每列一行：不虚拟化会在每帧重建上万个文本元素。
    assert!(body.contains("uniform_list("));
    assert!(body.contains("\"table-vertical-list\""));
    assert!(body.contains("vertical_line_at(line_ix, column_count)"));
    // 行高沿用表格行高设置，纵向视图与网格保持同一节奏。
    assert!(body.contains("one_ui::table_row_height(cx)"));
    assert!(body.contains("render_vertical_line(line, delegate, &font, line_height, cx)"));
    assert!(body.contains(".track_scroll(&self.vertical_scroll_handle)"));
}

#[test]
fn the_vertical_view_reads_cells_through_the_delegate_boundary() {
    let grid = data_grid_source();
    let body = slice_between(
        grid,
        "fn render_vertical_line(",
        "/// 纵向视图的数据字体缓存。",
    );

    // 显示行 → 实际行、展示列 → 原始列只能由 delegate 完成换算；
    // 视图层直接读 `delegate.rows[...]` 会把两套坐标混在一起。
    assert!(body.contains("delegate.vertical_field(row, col)"));
    assert!(!body.contains("delegate.rows"));
    assert!(!body.contains("original_column_index"));
}

#[test]
fn the_vertical_data_font_is_resolved_once_per_render_and_cached() {
    let grid = data_grid_source();
    let font = slice_between(grid, "fn vertical_view_font(", "\n    /// 切换显示方式");
    let vertical = slice_between(
        grid,
        "fn render_vertical_view(",
        "\n    /// 「字段过滤」入口",
    );

    // 字体族解析要走系统字体枚举，必须命中缓存，且不能在按行渲染里再解析一次。
    assert!(font.contains("cache.requested_family == requested_family"));
    assert!(font.contains("installed_grid_monospace_font("));
    assert!(font.contains("cx.text_system().all_font_names()"));
    assert!(!vertical.contains("all_font_names"));
    assert!(!vertical.contains("installed_grid_monospace_font"));
}

#[test]
fn grid_only_affordances_are_hidden_in_the_vertical_view() {
    let grid = data_grid_source();
    let body = slice_between(
        grid,
        "pub fn render_toolbar(",
        "\n    pub fn render_table_area(",
    );

    // 纵向形态没有「第几行第几列」这个落点：行编辑、表内查找、大文本编辑器
    // 全部收起来，不留点了没反应的按钮。
    assert!(body.contains("let grid_affordances = !self.is_vertical_view(cx);"));
    assert_eq!(
        5,
        count_matches(body, ".when(editable && grid_affordances, |this| {")
    );
    assert_eq!(1, count_matches(body, ".when(grid_affordances, |this| {"));
    assert!(
        body.contains("(self.config.usage == DataGridUsage::TableData && grid_affordances)"),
        "表内查找框只在网格形态下出现"
    );

    // 与显示方式无关的入口必须保留：刷新、字段过滤、导出、表设计器。
    assert!(body.contains("Button::new(\"refresh-data\")"));
    assert!(body.contains("self.render_view_mode_button(cx)"));
    assert!(body.contains("self.render_column_visibility_button(cx)"));
    assert!(body.contains("Button::new(\"open-table-designer\")"));
    assert!(body.contains("Button::new(\"export-data\")"));
}

#[test]
fn the_display_mode_is_persisted_beyond_the_data_grid_instance() {
    let grid = data_grid_source();
    let body = slice_between(grid, "fn switch_view_mode(", "\n    /// 「显示方式」入口");

    // SQL 结果页每执行一次查询都会新建一个 `DataGrid`，只改实例状态会让用户
    // 每次查询都被打回网格形态，所以必须写回 `AppSettings`。
    assert!(body.contains("AppSettings::update_and_save(cx,"));
    assert!(body.contains("settings.table_view_mode = mode;"));
    assert!(body.contains("cx.notify();"));
}
