use super::*;

#[test]
fn context_removed_and_added_lines_align_into_rows() {
    let diff = "\
diff --git a/main.rs b/main.rs
index 8a1218a..f00c965 100644
--- a/main.rs
+++ b/main.rs
@@ -1,5 +1,4 @@
 fn main() {
-    old_call();
-    legacy();
+    new_call();
     tail();
 }
";
    let parsed = parse_side_by_side(diff);

    assert_eq!(5, parsed.rows.len());
    assert_eq!(5, parsed.old_line_count);
    assert_eq!(4, parsed.new_line_count);

    let context = &parsed.rows[0];
    assert_eq!(Some(1), context.left.as_ref().map(|line| line.number));
    assert_eq!(Some(1), context.right.as_ref().map(|line| line.number));
    assert_eq!(DiffLineKind::Context, context.left.as_ref().unwrap().kind);

    let paired = &parsed.rows[1];
    assert_eq!("    old_call();", paired.left.as_ref().unwrap().text);
    assert_eq!(DiffLineKind::Removed, paired.left.as_ref().unwrap().kind);
    assert_eq!("    new_call();", paired.right.as_ref().unwrap().text);
    assert_eq!(DiffLineKind::Added, paired.right.as_ref().unwrap().kind);

    let removed_only = &parsed.rows[2];
    assert_eq!("    legacy();", removed_only.left.as_ref().unwrap().text);
    assert!(removed_only.right.is_none());

    let trailing = &parsed.rows[3];
    assert_eq!(Some(4), trailing.left.as_ref().map(|line| line.number));
    assert_eq!(Some(3), trailing.right.as_ref().map(|line| line.number));
    assert_eq!("    tail();", trailing.right.as_ref().unwrap().text);

    let closing = &parsed.rows[4];
    assert_eq!(Some(5), closing.left.as_ref().map(|line| line.number));
    assert_eq!(Some(4), closing.right.as_ref().map(|line| line.number));
}

#[test]
fn pure_addition_leaves_left_side_empty() {
    let diff = "\
diff --git a/f.txt b/f.txt
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,4 @@
 a
+b
+c
 d
";
    let parsed = parse_side_by_side(diff);

    assert_eq!(4, parsed.rows.len());
    assert!(parsed.rows[1].left.is_none());
    assert!(parsed.rows[2].left.is_none());
    assert_eq!(
        Some(2),
        parsed.rows[1].right.as_ref().map(|line| line.number)
    );
    assert_eq!(
        Some(3),
        parsed.rows[2].right.as_ref().map(|line| line.number)
    );
    assert_eq!(2, parsed.old_line_count);
    assert_eq!(4, parsed.new_line_count);
}

#[test]
fn new_file_diff_only_fills_right_side() {
    let diff = "\
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+one
+two
\\ No newline at end of file
";
    let parsed = parse_side_by_side(diff);

    assert_eq!(2, parsed.rows.len());
    assert!(parsed.rows.iter().all(|row| row.left.is_none()));
    assert_eq!(0, parsed.old_line_count);
    assert_eq!(2, parsed.new_line_count);
}

#[test]
fn only_first_file_section_is_parsed() {
    let diff = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-a
+b
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-a
+c
";
    let parsed = parse_side_by_side(diff);

    assert_eq!(1, parsed.rows.len());
    assert_eq!("b", parsed.rows[0].right.as_ref().unwrap().text);
}

#[test]
fn missing_hunk_count_defaults_are_handled() {
    assert_eq!(Some((12, 30)), parse_hunk_header("-12 +30,2 @@ fn x()"));
    assert_eq!(Some((1, 1)), parse_hunk_header("-1 +1 @@"));
    assert_eq!(None, parse_hunk_header("not a header"));
}

#[test]
fn aligned_sides_keep_original_line_numbers() {
    let parsed = parse_side_by_side(
        "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,3 @@
 context
-removed
+added
 tail
",
    );
    let (left, right) = aligned_side_by_side(&parsed);

    assert_eq!("context\nremoved\ntail", left.text);
    assert_eq!("context\nadded\ntail", right.text);
    assert_eq!(vec![Some(1), Some(2), Some(3)], left.line_numbers);
    assert_eq!(vec![Some(1), Some(2), Some(3)], right.line_numbers);
    assert_eq!(vec![false, true, false], left.changed);
    assert_eq!(vec![false, true, false], right.changed);
}

#[test]
fn aligned_sides_add_empty_placeholder_rows() {
    let parsed = parse_side_by_side(
        "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,2 @@
 context
-removed
 tail
",
    );
    let (left, right) = aligned_side_by_side(&parsed);

    assert_eq!("context\nremoved\ntail", left.text);
    assert_eq!("context\n\ntail", right.text);
    assert_eq!(vec![Some(1), Some(2), Some(3)], left.line_numbers);
    assert_eq!(vec![Some(1), None, Some(2)], right.line_numbers);
    assert_eq!(vec![false, false, false], left.placeholders);
    assert_eq!(vec![false, true, false], right.placeholders);
}

/// 装饰范围必须连行尾换行一起算。这条断言是给"看起来多余的那个 +1"站岗的：
/// 组件的 `Fill` 对空 range 是直接丢弃的（`normalize` 拒收、`layout_range_corners`
/// 返回 `None`），而占位行的文本本身就是空的——一旦把换行省掉，最该被指出来的
/// 占位行反而整块不画，且不报任何错。
#[test]
fn aligned_spans_include_the_newline_so_placeholder_rows_still_paint() {
    let parsed = parse_side_by_side(
        "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,2 @@
 context
-removed
 tail
",
    );
    let (left, right) = aligned_side_by_side(&parsed);

    assert_eq!(vec![0..8, 8..16, 16..20], ranges_of(&left));
    // 右栏中间那行是占位，文本为空：`8..9` 覆盖的正是那个换行。
    assert_eq!("context\n\ntail", right.text);
    assert_eq!(vec![0..8, 8..9, 9..13], ranges_of(&right));

    for (range, _) in aligned_span_ranges(&right) {
        assert!(
            !range.is_empty(),
            "占位行也必须拿到非空范围，否则背景画不出来：{range:?}"
        );
    }
    // 逐行首尾相接，并且完整覆盖文本。
    let left_ranges = ranges_of(&left);
    assert_eq!(0, left_ranges[0].start);
    assert_eq!(left.text.len(), left_ranges.last().expect("三行").end);
}

#[test]
fn aligned_spans_tell_changes_apart_from_placeholders() {
    let parsed = parse_side_by_side(
        "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,2 @@
 context
-removed
 tail
",
    );
    let (left, right) = aligned_side_by_side(&parsed);

    // 左侧是"这行被改了"，右侧是"这一侧这里没有对应行"——两者都要上色，
    // 但颜色语义不同，所以必须区分开。
    assert_eq!(
        vec![
            AlignedSpanKind::Context,
            AlignedSpanKind::Changed,
            AlignedSpanKind::Context
        ],
        kinds_of(&left)
    );
    assert_eq!(
        vec![
            AlignedSpanKind::Context,
            AlignedSpanKind::Placeholder,
            AlignedSpanKind::Context
        ],
        kinds_of(&right)
    );
}

fn ranges_of(side: &AlignedDiffSide) -> Vec<std::ops::Range<usize>> {
    aligned_span_ranges(side)
        .into_iter()
        .map(|(range, _)| range)
        .collect()
}

fn kinds_of(side: &AlignedDiffSide) -> Vec<AlignedSpanKind> {
    aligned_span_ranges(side)
        .into_iter()
        .map(|(_, kind)| kind)
        .collect()
}

#[test]
fn change_starts_group_contiguous_changed_rows() {
    let parsed = parse_side_by_side(
        "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1,7 +1,7 @@
 context
-removed one
-removed two
+added one
+added two
 separator
-removed three
+added three
 tail
",
    );

    assert_eq!(vec![1, 4], change_starts(&parsed));
}

/// 单栏 diff 的着色区间：把原文按行切开，逐行断言它被切到哪一段、染什么色。
fn text_spans(diff: &str) -> Vec<(&str, DiffTextSpanKind)> {
    diff_text_spans(diff)
        .into_iter()
        .map(|(range, kind)| (&diff[range], kind))
        .collect()
}

#[test]
fn single_pane_spans_cover_the_whole_line_including_its_newline() {
    let diff = "\
diff --git a/main.rs b/main.rs
index 8a1218a..f00c965 100644
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    old_call();
+    new_call();
}
";
    assert_eq!(
        vec![
            ("diff --git a/main.rs b/main.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1,3 +1,3 @@\n", DiffTextSpanKind::Marker),
            ("-    old_call();\n", DiffTextSpanKind::Removed),
            ("+    new_call();\n", DiffTextSpanKind::Added),
        ],
        text_spans(diff)
    );
}

#[test]
fn file_headers_before_the_first_hunk_are_not_painted_as_changes() {
    // `--- a/x` / `+++ b/x` 长得像增删行，但它们在 hunk 之外：染错了远比不染刺眼。
    let diff = "\
diff --git a/main.rs b/main.rs
--- a/main.rs
+++ b/main.rs
@@ -1 +1 @@
-old
+new
";
    assert_eq!(
        vec![
            ("diff --git a/main.rs b/main.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1 +1 @@\n", DiffTextSpanKind::Marker),
            ("-old\n", DiffTextSpanKind::Removed),
            ("+new\n", DiffTextSpanKind::Added),
        ],
        text_spans(diff)
    );
}

#[test]
fn a_second_file_restarts_the_hunk_state() {
    // 多文件 diff：第二个文件的文件头不能被当成上一个 hunk 里的新增行。
    let diff = "\
diff --git a/a.rs b/a.rs
@@ -1 +1 @@
-old
+new
diff --git b/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-before
+after
";
    assert_eq!(
        vec![
            ("diff --git a/a.rs b/a.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1 +1 @@\n", DiffTextSpanKind::Marker),
            ("-old\n", DiffTextSpanKind::Removed),
            ("+new\n", DiffTextSpanKind::Added),
            ("diff --git b/b.rs b/b.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1 +1 @@\n", DiffTextSpanKind::Marker),
            ("-before\n", DiffTextSpanKind::Removed),
            ("+after\n", DiffTextSpanKind::Added),
        ],
        text_spans(diff)
    );
}

#[test]
fn added_lines_keeping_a_plus_prefix_are_still_additions() {
    // `++x` 在 hunk 内是新增加了一行 `+x`，不是文件头。
    let diff = "\
diff --git a/main.rs b/main.rs
@@ -1,1 +1,2 @@
 keep
++plus
";
    assert_eq!(
        vec![
            ("diff --git a/main.rs b/main.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1,1 +1,2 @@\n", DiffTextSpanKind::Marker),
            ("++plus\n", DiffTextSpanKind::Added),
        ],
        text_spans(diff)
    );
}

#[test]
fn no_newline_marker_is_skipped_and_a_missing_final_newline_still_paints() {
    // 原文末尾没有换行（最后一行的区间就落在字符串末尾），且中间夹了一行
    // `\ No newline at end of file`：那一行不是 diff 内容，不染。
    let diff = "\
diff --git a/main.rs b/main.rs
@@ -1 +1 @@
-old
\\ No newline at end of file
+new";

    let spans = text_spans(diff);
    assert_eq!(
        vec![
            ("diff --git a/main.rs b/main.rs\n", DiffTextSpanKind::Marker),
            ("@@ -1 +1 @@\n", DiffTextSpanKind::Marker),
            ("-old\n", DiffTextSpanKind::Removed),
            ("+new", DiffTextSpanKind::Added),
        ],
        spans
    );
    // 区间不能为空：空区间会被渲染层整个丢掉。
    assert!(spans.iter().all(|(text, _)| !text.is_empty()));
}

#[test]
fn a_diff_without_hunks_paints_nothing() {
    // 只改了文件模式 / 只重命名：没有 hunk，单栏视图不该凭文件头染色。
    let diff = "\
diff --git a/main.rs b/main.rs
old mode 100644
new mode 100755
similarity index 100%
rename from main.rs
rename to main2.rs
";
    assert_eq!(
        vec![("diff --git a/main.rs b/main.rs\n", DiffTextSpanKind::Marker)],
        text_spans(diff)
    );
}

#[test]
fn change_starts_are_empty_without_changed_rows() {
    let parsed = SideBySideDiff {
        rows: vec![DiffRow {
            left: Some(DiffLine {
                number: 1,
                text: "context".to_string(),
                kind: DiffLineKind::Context,
            }),
            right: Some(DiffLine {
                number: 1,
                text: "context".to_string(),
                kind: DiffLineKind::Context,
            }),
        }],
        old_line_count: 1,
        new_line_count: 1,
    };

    assert!(change_starts(&parsed).is_empty());
}
