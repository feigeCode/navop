//! 底部状态栏「复制 SQL」的契约测试（#290）。
//!
//! 状态栏里那条 SQL 是用户排查「这次到底发了什么查询」的唯一凭据，
//! 而宽表场景下它总被省略号截断，所以复制按钮必须复制**完整文本**
//! 而不是屏幕上可见的那一截；空 SQL 也不该把剪贴板悄悄清掉。

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

/// 截出 `render_status_bar` 的实现体。
///
/// 一律先切出实现区再断言，避免断言文本命中不该命中的位置。
fn status_bar_body(source: &str) -> &str {
    let start = source
        .find("fn render_status_bar(")
        .expect("render_status_bar implementation");
    let body = &source[start..];
    let end = body.find("\n    fn ").expect("next method");
    &body[..end]
}

/// 截出 `handle_copy_current_sql` 的实现体。
fn copy_handler_body(source: &str) -> &str {
    let start = source
        .find("fn handle_copy_current_sql(")
        .expect("handle_copy_current_sql implementation");
    let body = &source[start..];
    let end = body.find("\n    fn ").expect("next method");
    &body[..end]
}

#[test]
fn status_bar_offers_a_copy_button_for_the_executed_sql() {
    let body = status_bar_body(data_grid_source());

    assert!(
        body.contains("Button::new(\"copy-current-sql\")"),
        "状态栏应给执行过的 SQL 配一个复制按钮"
    );
    assert!(
        body.contains(".on_click(cx.listener(Self::handle_copy_current_sql))"),
        "复制按钮必须接到 handle_copy_current_sql 上"
    );
    assert!(
        body.contains("TableDataGrid.copy_sql"),
        "复制按钮要有 tooltip 词条，否则用户看不出它是什么"
    );
    assert!(
        body.contains("table_data_info.current_sql"),
        "按钮必须挂在 SQL 文本旁边，而不是别的位置"
    );
}

#[test]
fn copy_handler_writes_the_full_sql_and_skips_blank_text() {
    let body = copy_handler_body(data_grid_source());

    assert!(
        body.contains("ClipboardItem::new_string(sql)"),
        "复制的是完整 SQL 文本"
    );
    assert!(body.contains("current_sql"), "取的是当前执行过的 SQL");
    assert!(
        body.contains("sql.trim().is_empty()"),
        "空 SQL 不应写进剪贴板"
    );
}
