//! 表格内查找的单元测试与结构契约

use super::{
    FindMatch, NULL_TEXT, char_index_to_byte_index, char_range_to_byte_range,
    normalize_find_query, resolve_find, row_highlight_ranges, row_matches, row_text,
    scroll_target_for_match,
};
use crate::edit_table::TableKeybindings;

fn cells(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|value| Some(value.to_string())).collect()
}

fn ranges(matches: &[FindMatch]) -> Vec<(usize, usize)> {
    matches
        .iter()
        .map(|m| (m.row_offset + m.char_range.start, m.char_range.end))
        .collect()
}

#[test]
fn query_is_normalized_to_trimmed_lowercase() {
    assert_eq!("alice", normalize_find_query("  ALIce  "));
    assert_eq!("", normalize_find_query("   "));
}

#[test]
fn matches_are_case_insensitive_and_include_null_cells() {
    let cells = vec![Some("Alice".to_string()), None];
    let matches = row_matches(&cells, "alice");
    assert_eq!(1, matches.len());
    assert_eq!(0, matches[0].char_range.start);

    // 用户看到的是 NULL，查找也必须能命中它。
    let null_matches = row_matches(&cells, "null");
    assert_eq!(1, null_matches.len());
    assert_eq!("alice  null", row_text(&cells));
    assert_eq!(NULL_TEXT, "NULL");
}

#[test]
fn row_offsets_are_relative_to_the_whole_row_text() {
    let matches = row_matches(&cells(&["ab", "cd"]), "cd");

    assert_eq!(1, matches.len());
    // "ab" + 两个空格分隔符 => 第二个单元格从第 4 个字符开始。
    assert_eq!(4, matches[0].row_offset);
    assert_eq!(vec![(4, 2)], ranges(&matches));
}

#[test]
fn matches_remember_the_column_they_were_found_in() {
    // 命中所在列必须随命中一起保留：导航到命中时靠它横向滚列，
    // 否则命中在视口右侧以外的列上时，用户只看到行在动、高亮在屏幕外。
    let second_column = row_matches(&cells(&["aa", "bb"]), "b");
    assert_eq!(2, second_column.len());
    assert!(
        second_column
            .iter()
            .all(|cell_match| cell_match.col_ix == 1)
    );

    let first_column = row_matches(&cells(&["aa", "bb"]), "aa");
    assert_eq!(0, first_column[0].col_ix);

    // NULL 单元格的命中同样要带上它所在的列。
    let null_cell = row_matches(&[Some("x".to_string()), None], "null");
    assert_eq!(1, null_cell.len());
    assert_eq!(1, null_cell[0].col_ix);
}

#[test]
fn overlapping_needles_do_not_produce_overlapping_matches() {
    // "aa" 在 "aaa" 里只算一次命中，避免高亮区间交叠。
    assert_eq!(1, row_matches(&cells(&["aaa"]), "aa").len());
}

#[test]
fn adjacent_matches_merge_into_one_highlight_range() {
    let mut matches = row_matches(&cells(&["aaa"]), "a");
    assert_eq!(3, matches.len());
    matches[1].is_current = true;

    let merged = row_highlight_ranges(&matches, 3);
    assert_eq!(vec![(0..3, true)], merged);
}

#[test]
fn separated_matches_stay_as_separate_highlight_ranges() {
    let mut combined = row_matches(&cells(&["a b"]), "a");
    combined.extend(row_matches(&cells(&["a b"]), "b"));
    combined.sort_by_key(|m| m.row_offset + m.char_range.start);

    let merged = row_highlight_ranges(&combined, 3);

    assert_eq!(2, merged.len());
    assert_eq!(0..1, merged[0].0);
    assert_eq!(2..3, merged[1].0);
}

#[test]
fn empty_query_yields_no_outcome_and_no_matches() {
    let outcome = resolve_find(5, "", |_| 1);
    assert_eq!(0, outcome.total);
    assert!(outcome.rows.is_empty());
    assert!(row_matches(&cells(&["a"]), "").is_empty());
}

#[test]
fn resolve_find_collects_hit_rows_and_totals() {
    // 行 1 有 2 个命中，行 3 有 1 个命中。
    let outcome = resolve_find(4, "a", |row| match row {
        1 => 2,
        3 => 1,
        _ => 0,
    });

    assert_eq!(3, outcome.total);
    assert_eq!(vec![1, 3], outcome.rows);
}

#[test]
fn resolve_find_ignores_rows_outside_the_display_range() {
    let outcome = resolve_find(2, "a", |_| 1);
    assert_eq!(2, outcome.total);
    assert_eq!(vec![0, 1], outcome.rows);
}

#[test]
fn scroll_keeps_the_viewport_when_the_match_is_visible() {
    assert_eq!(None, scroll_target_for_match(5, &(3..10), 3, 100));
}

#[test]
fn scroll_moves_up_when_the_match_is_above_the_viewport() {
    assert_eq!(Some(2), scroll_target_for_match(2, &(5..15), 5, 100));
}

#[test]
fn scroll_clamps_the_last_page_at_the_end_of_the_table() {
    // 命中最靠后时不能滚过表格末尾，否则视口会被拉出空白。
    assert_eq!(Some(90), scroll_target_for_match(99, &(5..15), 5, 100));
}

#[test]
fn scroll_returns_none_for_an_empty_table() {
    assert_eq!(None, scroll_target_for_match(0, &(0..0), 0, 0));
}

#[test]
fn find_shortcuts_are_bound_in_the_edit_table_context() {
    let source = include_str!("../mod.rs");

    assert!(source.contains("KeyBinding::new(key, Find, Some(CONTEXT))"));
    assert!(source.contains("KeyBinding::new(key, FindNext, Some(CONTEXT))"));
    assert!(source.contains("KeyBinding::new(key, FindPrevious, Some(CONTEXT))"));
}

#[test]
fn hosted_find_bar_binds_the_same_navigation_shortcuts() {
    let source = include_str!("../mod.rs");

    // 宿主渲染的输入框要能复用同一份（可重绑的）上下跳键位。
    assert!(source.contains("pub const HOSTED_FIND_CONTEXT: &str = \"EditTableHostedFind\";"));
    assert!(source.contains("KeyBinding::new(key, FindNext, Some(HOSTED_FIND_CONTEXT))"));
    assert!(source.contains("KeyBinding::new(key, FindPrevious, Some(HOSTED_FIND_CONTEXT))"));
    assert!(source.contains("Some(HOSTED_FIND_CONTEXT),\n        FindNext,"));
    assert!(source.contains("Some(HOSTED_FIND_CONTEXT),\n        FindPrevious,"));
}

#[test]
fn default_find_shortcuts_follow_the_platform_convention() {
    let bindings = TableKeybindings::default();
    let expected = if cfg!(target_os = "macos") {
        ("cmd-f", "cmd-g", "cmd-shift-g")
    } else {
        ("ctrl-f", "ctrl-g", "ctrl-shift-g")
    };

    assert_eq!(vec![expected.0.to_string()], bindings.find_shortcuts());
    assert_eq!(vec![expected.1.to_string()], bindings.find_next_shortcuts());
    assert_eq!(
        vec![expected.2.to_string()],
        bindings.find_previous_shortcuts()
    );
}

#[test]
fn highlight_shares_the_cell_content_box_with_the_td_content() {
    let state = include_str!("../state.rs");
    let start = state
        .find("fn render_interactive_cell(")
        .expect("interactive cell renderer");
    let body = &state[start..];
    let end = body.find("\n    fn render_find_highlight(").expect("next method");
    let body = &body[..end];

    // 高亮与 td 内容必须挂在同一个「内容区」盒子里（相对 `relative` 容器绝对定位）：
    // 单元格 padding 会把内容推离 padding box，而高亮的横向原点取自自身 bounds，
    // 不放在内容区里就会整条左移一个 padding 的宽度。
    assert!(body.contains("self.render_find_highlight(row_ix, col_ix, window, cx)"));
    assert!(body.contains("self.measure_render_td(row_ix, col_ix, window, cx)"));
    assert!(body.contains(".relative()"));
    assert!(body.contains(".inset_0()"));
}

#[test]
fn the_current_match_cell_is_outlined_because_long_text_clips_the_highlight() {
    let state = include_str!("../state.rs");
    let start = state
        .find("fn render_interactive_cell(")
        .expect("interactive cell renderer");
    let body = &state[start..];
    let end = body
        .find("\n    fn render_find_highlight(")
        .expect("next method");
    let body = &body[..end];

    // 单元格是单行截断显示的（`nowrap` + `text_ellipsis`），命中落在截断区之后时
    // 条带会被 `overflow_hidden` 整条裁掉，屏幕上再没有别的提示——所以必须给
    // 「当前命中的单元格」整体描边，用户才看得出命中在哪个格子里。
    assert!(body.contains("self.find_current_cell == Some((row_ix, col_ix))"));
    // 描边与当前命中的条带同色系。
    assert!(body.contains("find_highlight_color(cx.theme().selection, true)"));
    // 描边必须是叠加绘制：走 `border_2` + padding 补偿那条路会让单元格内容跳动。
    assert!(body.contains(".border_2()"));
}

#[test]
fn highlight_element_looks_up_pixel_positions_by_byte_index() {
    let element = include_str!("element.rs");

    // gpui 的 x_for_index 按字节下标定位，所以必须先把字符区间换算成字节区间；
    // 用固定字符宽度估算则会在中英文混排下错位。
    assert!(element.contains("x_for_index("));
    assert!(element.contains("char_range_to_byte_range("));
    assert!(!element.contains("char_width *"));
}

#[test]
fn highlight_shapes_text_with_the_style_active_at_paint_time() {
    let element = include_str!("element.rs");
    let state = include_str!("../state.rs");

    // 字体/字号必须在元素 paint 内从 `window.text_style()` 解析：gpui 的 Div
    // 只在 paint 子元素前才把 `.font()` / `.text_sm()` 推进 text_style_stack
    //（fork-0.3.111 div.rs `window.with_text_style(style.text_style()...)`）。
    // 若在 render 期（构建 cell div 之前）就烘焙死字体，td 用网格等宽字体渲染，
    // 高亮却按环境 UI 字体测宽度，两侧字形 advance 不同，条带会随命中前缀
    // 长度线性漂移（截图实测：URL 列第 19/32 字符处左偏 37/65px），高亮盖到
    // 错误的字符上。
    let paint = element
        .split("fn paint(")
        .nth(1)
        .expect("FindHighlightElement::paint");
    assert!(
        paint.contains("window.text_style()"),
        "paint 内必须现场解析 text_style（字体与字号），不能用 render 期烘焙的值"
    );

    // 构造函数不得再收字体/字号/text_runs：一旦收了，调用方就会在 render 期
    // 把环境样式传进来，paint 期的修正无从谈起。
    let signature = element
        .split("impl FindHighlightElement {")
        .nth(1)
        .expect("impl block");
    assert!(
        !signature.contains("font_size: Pixels,"),
        "构造函数不应接收 font_size"
    );
    assert!(!signature.contains("text_runs"), "构造函数不应接收 text_runs");

    // render 侧也不得再从环境 text_style 取字体去构造 TextRun。
    assert!(
        !state.contains("let font = window.text_style().font();"),
        "render_find_highlight 不得在 render 期取环境字体"
    );
}

#[test]
fn char_offsets_are_converted_to_byte_offsets_for_layout_lookup() {
    // "张三ab"：CJK 一字 3 字节，ASCII 一字 1 字节。
    assert_eq!(0, char_index_to_byte_index("张三ab", 0));
    assert_eq!(3, char_index_to_byte_index("张三ab", 1));
    assert_eq!(6, char_index_to_byte_index("张三ab", 2));
    assert_eq!(7, char_index_to_byte_index("张三ab", 3));
    assert_eq!(8, char_index_to_byte_index("张三ab", 4));
    // 越界（例如 to_lowercase 改变了字符数）退回文本末尾，不 panic。
    assert_eq!(8, char_index_to_byte_index("张三ab", 99));

    assert_eq!(6..7, char_range_to_byte_range("张三ab", 2..3));
    assert_eq!(6..8, char_range_to_byte_range("张三ab", 2..4));
}

#[test]
fn panel_is_hosted_by_the_table_itself() {
    let state = include_str!("../state.rs");

    assert!(state.contains("find_panel: Entity<SearchPanel>"));
    assert!(state.contains("SearchPanelEvent::QueryChanged"));
    assert!(state.contains("find_panel_visible(cx).then(|| self.find_panel.clone())"));
}
