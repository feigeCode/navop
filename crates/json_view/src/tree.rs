//! JSON 可折叠树渲染的共享实现。
//!
//! `JsonFormatterView`(工具页)与 `JsonValueView`(通用值视图)复用同一套
//! 扁平行模型与行渲染;交互(折叠/复制)通过 `TreeCallbacks` 回调注入,
//! 由各自的宿主 View 决定如何更新自身状态。

use std::collections::HashSet;
use std::sync::Arc;

use gpui::*;
use gpui_component::h_flex;

pub type NodePath = Vec<u32>;

/// 超过该阈值(非根节点)默认折叠,避免打开超大文档时一次性展开。
pub const COLLAPSE_THRESHOLD: usize = 200;

#[derive(Clone)]
pub(crate) struct FlatRow {
    pub depth: usize,
    pub path: NodePath,
    pub kind: RowKind,
}

#[derive(Clone)]
pub(crate) enum RowKind {
    Object {
        key: String,
        count: usize,
        expanded: bool,
    },
    Array {
        key: String,
        count: usize,
        expanded: bool,
    },
    Primitive {
        key: String,
        value: String,
        raw: String,
        ty: ValueTy,
        needs_comma: bool,
    },
    Close {
        bracket: char,
        needs_comma: bool,
    },
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ValueTy {
    String,
    Number,
    Bool,
    Null,
}

/// 行交互回调:宿主 View 通过这两个闭包接住折叠与复制动作。
#[derive(Clone)]
pub(crate) struct TreeCallbacks {
    pub toggle: Arc<dyn Fn(&NodePath, &mut Window, &mut App)>,
    pub copy_value: Arc<dyn Fn(String, &mut Window, &mut App)>,
}

pub(crate) fn flatten(
    value: &serde_json::Value,
    key: &str,
    depth: usize,
    is_last: bool,
    expanded: &HashSet<NodePath>,
    path: &mut Vec<u32>,
    out: &mut Vec<FlatRow>,
) {
    let key_prefix = if key.is_empty() {
        String::new()
    } else {
        format!("\"{}\": ", key)
    };

    match value {
        serde_json::Value::Object(map) => {
            let is_expanded = depth == 0 || expanded.contains(path);
            out.push(FlatRow {
                depth,
                path: path.clone(),
                kind: RowKind::Object {
                    key: key_prefix.clone(),
                    count: map.len(),
                    expanded: is_expanded,
                },
            });
            if is_expanded {
                let count = map.len();
                for (i, (k, v)) in map.iter().enumerate() {
                    path.push(i as u32);
                    flatten(v, k, depth + 1, i + 1 == count, expanded, path, out);
                    path.pop();
                }
                out.push(FlatRow {
                    depth,
                    path: {
                        let mut p = path.clone();
                        p.push(u32::MAX);
                        p
                    },
                    kind: RowKind::Close {
                        bracket: '}',
                        needs_comma: !is_last,
                    },
                });
            }
        }
        serde_json::Value::Array(arr) => {
            let is_expanded = depth == 0 || expanded.contains(path);
            out.push(FlatRow {
                depth,
                path: path.clone(),
                kind: RowKind::Array {
                    key: key_prefix.clone(),
                    count: arr.len(),
                    expanded: is_expanded,
                },
            });
            if is_expanded {
                let count = arr.len();
                for (i, v) in arr.iter().enumerate() {
                    path.push(i as u32);
                    flatten(v, "", depth + 1, i + 1 == count, expanded, path, out);
                    path.pop();
                }
                out.push(FlatRow {
                    depth,
                    path: {
                        let mut p = path.clone();
                        p.push(u32::MAX);
                        p
                    },
                    kind: RowKind::Close {
                        bracket: ']',
                        needs_comma: !is_last,
                    },
                });
            }
        }
        serde_json::Value::String(s) => out.push(FlatRow {
            depth,
            path: path.clone(),
            kind: RowKind::Primitive {
                key: key_prefix,
                value: serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\"")),
                raw: s.clone(),
                ty: ValueTy::String,
                needs_comma: !is_last,
            },
        }),
        serde_json::Value::Number(n) => out.push(FlatRow {
            depth,
            path: path.clone(),
            kind: RowKind::Primitive {
                key: key_prefix,
                value: n.to_string(),
                raw: n.to_string(),
                ty: ValueTy::Number,
                needs_comma: !is_last,
            },
        }),
        serde_json::Value::Bool(b) => out.push(FlatRow {
            depth,
            path: path.clone(),
            kind: RowKind::Primitive {
                key: key_prefix,
                value: b.to_string(),
                raw: b.to_string(),
                ty: ValueTy::Bool,
                needs_comma: !is_last,
            },
        }),
        serde_json::Value::Null => out.push(FlatRow {
            depth,
            path: path.clone(),
            kind: RowKind::Primitive {
                key: key_prefix,
                value: "null".to_string(),
                raw: "null".to_string(),
                ty: ValueTy::Null,
                needs_comma: !is_last,
            },
        }),
    }
}

pub(crate) fn seed_default_expanded(
    value: &serde_json::Value,
    path: &mut Vec<u32>,
    expanded: &mut HashSet<NodePath>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if path.is_empty() || map.len() <= COLLAPSE_THRESHOLD {
                expanded.insert(path.clone());
            }
            for (i, (_, v)) in map.iter().enumerate() {
                path.push(i as u32);
                seed_default_expanded(v, path, expanded);
                path.pop();
            }
        }
        serde_json::Value::Array(arr) => {
            if path.is_empty() || arr.len() <= COLLAPSE_THRESHOLD {
                expanded.insert(path.clone());
            }
            for (i, v) in arr.iter().enumerate() {
                path.push(i as u32);
                seed_default_expanded(v, path, expanded);
                path.pop();
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_all_paths(
    value: &serde_json::Value,
    path: &mut Vec<u32>,
    out: &mut HashSet<NodePath>,
) {
    match value {
        serde_json::Value::Object(map) => {
            out.insert(path.clone());
            for (i, (_, v)) in map.iter().enumerate() {
                path.push(i as u32);
                collect_all_paths(v, path, out);
                path.pop();
            }
        }
        serde_json::Value::Array(arr) => {
            out.insert(path.clone());
            for (i, v) in arr.iter().enumerate() {
                path.push(i as u32);
                collect_all_paths(v, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// 精简:每个数组只保留第一个元素(递归)。结果仍是合法 JSON。
pub(crate) fn simplify(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), simplify(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            if let Some(first) = arr.first() {
                serde_json::Value::Array(vec![simplify(first)])
            } else {
                serde_json::Value::Array(vec![])
            }
        }
        _ => value.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_flat_row(
    ix: usize,
    row: &FlatRow,
    mono_font: &SharedString,
    fg: Hsla,
    warn: Hsla,
    muted_fg: Hsla,
    success: Hsla,
    info: Hsla,
    callbacks: &TreeCallbacks,
) -> AnyElement {
    let indent_width = 18.;
    let chevron_size = 18.;
    let row_h = 22.;

    let base = div()
        .id(ix)
        .pl(px(indent_width) * row.depth)
        .w_full()
        .font_family(mono_font.clone())
        .text_sm();

    match &row.kind {
        RowKind::Object {
            key,
            count,
            expanded,
        } => {
            let label = if *expanded {
                format!("{key}{{")
            } else {
                format!("{key}{{...{count}}}")
            };
            let path = row.path.clone();
            let toggle = callbacks.clone();
            base.h(px(row_h))
                .child(row_open(
                    *expanded,
                    chevron_size,
                    muted_fg,
                    fg,
                    label,
                    mono_font,
                ))
                .on_click(move |_ev, window, cx| (toggle.toggle)(&path, window, cx))
                .into_any_element()
        }
        RowKind::Array {
            key,
            count,
            expanded,
        } => {
            let label = if *expanded {
                format!("{key}[")
            } else {
                format!("{key}[...{count}]")
            };
            let path = row.path.clone();
            let toggle = callbacks.clone();
            base.h(px(row_h))
                .child(row_open(
                    *expanded,
                    chevron_size,
                    muted_fg,
                    fg,
                    label,
                    mono_font,
                ))
                .on_click(move |_ev, window, cx| (toggle.toggle)(&path, window, cx))
                .into_any_element()
        }
        RowKind::Primitive {
            key,
            value,
            raw,
            ty,
            needs_comma,
        } => {
            let color = match ty {
                ValueTy::String => success,
                ValueTy::Number => info,
                ValueTy::Bool => warn,
                ValueTy::Null => muted_fg,
            };
            let raw_owned = raw.clone();
            let copy = callbacks.clone();
            base.min_h(px(row_h))
                .child(
                    h_flex()
                        .gap_0()
                        .items_start()
                        .w_full()
                        .min_w_0()
                        .pl(px(chevron_size))
                        .child(
                            div()
                                .flex_none()
                                .min_w_0()
                                .text_color(fg)
                                .child(key.clone()),
                        )
                        .child(
                            div()
                                .id(("json-value", ix))
                                .flex_1()
                                .min_w_0()
                                .text_color(color)
                                .cursor_pointer()
                                .hover(|s| s.bg(muted_fg.opacity(0.08)))
                                .child(if *needs_comma {
                                    format!("{value},")
                                } else {
                                    value.clone()
                                })
                                .on_click(move |_ev, window, cx| {
                                    (copy.copy_value)(raw_owned.clone(), window, cx)
                                }),
                        ),
                )
                .into_any_element()
        }
        RowKind::Close {
            bracket,
            needs_comma,
        } => {
            let text = if *needs_comma {
                format!("{bracket},")
            } else {
                bracket.to_string()
            };
            base.h(px(row_h))
                .child(
                    h_flex()
                        .gap_0()
                        .items_center()
                        .h_full()
                        .pl(px(chevron_size))
                        .text_color(fg)
                        .child(text),
                )
                .into_any_element()
        }
    }
}

fn row_open(
    expanded: bool,
    chevron_size: f32,
    muted_fg: Hsla,
    fg: Hsla,
    label: String,
    mono_font: &SharedString,
) -> Div {
    h_flex()
        .gap_0()
        .items_center()
        .h_full()
        .w_full()
        .child(
            div()
                .w(px(chevron_size))
                .h(px(chevron_size))
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted_fg)
                .font_family(mono_font.clone())
                .text_xs()
                .child(if expanded { "▼" } else { "▶" }),
        )
        .child(div().text_color(fg).child(label))
}

#[cfg(test)]
mod tests {
    use super::{COLLAPSE_THRESHOLD, collect_all_paths, flatten, seed_default_expanded, simplify};
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn simplify_truncates_arrays_recursively() {
        let v = json!({"a": [1, 2, 3], "b": "x", "c": {"d": [9, 8]}});
        assert_eq!(simplify(&v), json!({"a": [1], "b": "x", "c": {"d": [9]}}));
    }

    #[test]
    fn simplify_keeps_empty_array_empty() {
        assert_eq!(simplify(&json!([])), json!([]));
    }

    #[test]
    fn seed_default_collapses_large_containers() {
        let big: Vec<usize> = (0..(COLLAPSE_THRESHOLD + 1)).collect();
        let v = json!({"small": [1, 2], "big": big});
        let mut expanded = HashSet::new();
        seed_default_expanded(&v, &mut Vec::new(), &mut expanded);
        assert!(expanded.contains(&vec![]));
        assert!(expanded.contains(&vec![0u32]));
        assert!(!expanded.contains(&vec![1u32]));
    }

    #[test]
    fn collect_all_paths_includes_every_container() {
        let v = json!({"a": [1, {"b": 2}]});
        let mut paths = HashSet::new();
        collect_all_paths(&v, &mut Vec::new(), &mut paths);
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn flatten_collapsed_array_shows_summary_row() {
        let v = json!({"items": [1, 2, 3]});
        let expanded = HashSet::new();
        let mut rows = Vec::new();
        flatten(&v, "", 0, true, &expanded, &mut Vec::new(), &mut rows);
        assert_eq!(rows.len(), 3);
    }
}
