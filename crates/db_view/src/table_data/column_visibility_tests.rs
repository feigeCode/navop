//! 「字段过滤」面板的契约测试
//!
//! 表数据页的字段过滤原先是下拉菜单：高度由条目数决定，字段多的表（几十列）
//! 会一路顶到窗口底部，最后几个字段根本点不到。现在改成固定尺寸的弹出面板
//! （标题计数 + 字段搜索 + 限高滚动列表 + 底部操作条），这里锁住几件不能
//! 漂移的事：
//!
//! 1. 入口是弹出面板而不是下拉菜单，列表高度按行数算出来再封顶；
//! 2. 滚动区必须有确定高度，否则内容会把容器撑开、滚动条永远不出现；
//! 3. 搜索框在 `new()` 里创建（面板内容闭包每次渲染都会重跑）；
//! 4. 一次点击只翻转一次：整行挂处理器，勾选框只负责显示；
//! 5. 最后一个可见字段不能隐藏，「显示全部字段」在全可见时禁用。

use super::data_grid::{
    COLUMN_VISIBILITY_PANEL_MAX_HEIGHT, COLUMN_VISIBILITY_PANEL_WIDTH,
    COLUMN_VISIBILITY_ROW_HEIGHT, filter_column_visibility_entries,
};
use gpui::{SharedString, px};
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

fn panel_source() -> &'static str {
    slice_between(
        data_grid_source(),
        "fn build_column_visibility_panel(",
        "/// 构建「显示方式」菜单",
    )
}

fn entries() -> Vec<(usize, SharedString, bool)> {
    vec![
        (0, "id".into(), true),
        (3, "TENANT_ID".into(), true),
        (5, "biz_type".into(), false),
    ]
}

#[test]
fn the_field_search_filters_case_insensitively() {
    let all = entries();

    // 空串与纯空白都不算过滤条件：面板一打开就是全量字段。
    assert_eq!(3, filter_column_visibility_entries(&all, "").len());
    assert_eq!(3, filter_column_visibility_entries(&all, "   ").len());

    // 子串匹配、忽略大小写：字段名多是 snake_case，不该逼用户记大小写。
    let hits = filter_column_visibility_entries(&all, "id");
    assert_eq!(
        vec![0, 3],
        hits.iter().map(|(ix, _, _)| *ix).collect::<Vec<_>>()
    );
    let upper = filter_column_visibility_entries(&all, "tenant");
    assert_eq!(
        vec![3],
        upper.iter().map(|(ix, _, _)| *ix).collect::<Vec<_>>()
    );

    // 过滤只筛行，不动可见性：隐藏的字段被搜出来时仍然是隐藏状态。
    let hidden = filter_column_visibility_entries(&all, "biz");
    assert_eq!(Some(&(5, "biz_type".into(), false)), hidden.first());

    assert!(filter_column_visibility_entries(&all, "nope").is_empty());
}

#[test]
fn the_field_filter_is_a_bounded_popover_instead_of_a_dropdown_menu() {
    let grid = data_grid_source();
    let button = slice_between(
        grid,
        "fn render_column_visibility_button(",
        "\n    /// 工具栏里的查找框",
    );

    // 面板才能限高：下拉菜单的高度由条目数决定。
    assert!(button.contains("Popover::new(\"column-visibility\")"));
    assert!(button.contains("build_column_visibility_panel("));
    assert!(!button.contains("dropdown_menu("));
    // 面板宽度在这里定，字段列表跟着面板走。
    assert!(button.contains(".w(COLUMN_VISIBILITY_PANEL_WIDTH)"));
    assert_eq!(px(240.), COLUMN_VISIBILITY_PANEL_WIDTH);
}

#[test]
fn the_field_list_scrolls_inside_a_height_computed_from_the_rows() {
    let panel = panel_source();

    // 高度按行数算出来再封顶。这里必须是确定高度（`h`）而不是上限（`max_h`）：
    // 上限之下内容会把容器撑到全部展开，滚动条就永远不出现了。
    assert!(panel.contains(".h(list_height)"));
    assert!(panel.contains(".min(COLUMN_VISIBILITY_PANEL_MAX_HEIGHT)"));
    assert!(panel.contains(".overflow_y_scroll()"));
    assert!(panel.contains(".track_scroll("));
    assert_eq!(px(320.), COLUMN_VISIBILITY_PANEL_MAX_HEIGHT);

    // 行高必须是固定值，列表高度就是它乘出来的。
    assert!(panel.contains(".h(COLUMN_VISIBILITY_ROW_HEIGHT)"));
    assert_eq!(px(28.), COLUMN_VISIBILITY_ROW_HEIGHT);

    // 常驻滚动条，并且挂在非滚动容器上：它是绝对定位的，放进滚动容器会跟着内容跑。
    assert!(panel.contains("Scrollbar::vertical("));
    assert!(panel.contains("ScrollbarMode::Always"));
    assert!(panel.contains(".viewport_from_layout()"));
    assert!(panel.contains("div().absolute().inset_0()"));

    // 长字段名截断，不能把面板撑宽。
    assert!(panel.contains(".text_ellipsis()"));
}

#[test]
fn the_field_search_input_is_created_once_outside_the_panel_content() {
    let grid = data_grid_source();
    let panel = panel_source();

    // 面板内容闭包每次渲染都会重跑：在里面建实体会每次换一个新的，光标也丢。
    assert!(!panel.contains("cx.new("));
    assert!(grid.contains("let column_visibility_search = cx.new(|cx| {"));
    assert!(grid.contains("TableDataGrid.column_visibility_search_placeholder"));
    // 与工具栏查找框同款：放大镜前缀 + 一键清空。
    assert!(panel.contains("Input::new(&search_input)"));
    assert!(panel.contains("Icon::new(IconName::Search)"));
    assert!(panel.contains(".cleanable(true)"));
    // 搜索词只影响面板列表，宿主重画即可。
    assert!(grid.contains("fn bind_column_visibility_search_event("));
    assert!(grid.contains("cx.subscribe_in(\n            &self.column_visibility_search,"));
}

#[test]
fn the_panel_shows_the_visible_count_and_a_keep_one_hint() {
    let panel = panel_source();

    // 计数用「显示几个 / 一共几列」，与查找框的计数写法一致；
    // 分母是全量字段数，搜索时也要能看出总共多少列。
    assert!(panel.contains("format!(\"{visible_total}/{total}\")"));
    assert!(panel.contains("TableDataGrid.column_visibility"));
    assert!(panel.contains("TableDataGrid.column_visibility_keep_one_hint"));
    assert!(panel.contains("TableDataGrid.search_no_match"));
    assert!(panel.contains("grid.show_all_columns(cx)"));

    // 词条缺失时 `t!` 会把 key 原样返回，界面上就会直接出现 `TableDataGrid.xxx`。
    for key in [
        "TableDataGrid.column_visibility",
        "TableDataGrid.column_visibility_search_placeholder",
        "TableDataGrid.column_visibility_keep_one_hint",
        "TableDataGrid.show_all_columns",
        "TableDataGrid.search_no_match",
    ] {
        assert_ne!(key, t!(key).as_ref(), "词条 `{key}` 缺失");
    }
}

#[test]
fn a_field_row_toggles_exactly_once_per_click() {
    let panel = panel_source();

    // 整行可点：勾选框与文字都算点击目标。
    assert!(panel.contains(".on_click(move |_, _window, cx| {"));
    assert!(panel.contains("grid.set_column_visibility(original_ix, !visible, cx)"));
    // 勾选框只显示状态：两个都挂处理器会把一次点击翻转两遍。
    let checkbox = slice_between(panel, "Checkbox::new((", ".child(label),");
    assert!(!checkbox.contains(".on_click("));
    assert!(checkbox.contains(".checked(visible)"));
}

#[test]
fn the_last_visible_field_cannot_be_hidden() {
    let panel = panel_source();

    // 整表无列的兜底：只剩一个可见字段时既不能点，勾选框也是禁用态。
    assert!(panel.contains("let is_last_visible = visible && visible_total <= 1;"));
    assert!(panel.contains(".when(!is_last_visible, |this| {"));
    assert!(panel.contains(".disabled(is_last_visible)"));
    // 「显示全部字段」在全可见时禁用，避免点了没反应。
    assert!(panel.contains(".disabled(visible_total == total)"));
}
