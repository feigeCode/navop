//! Rendering for [`Block`] via GPUI's high-level [`Render`] trait.
//!
//! Each block kind produces a distinct visual style: H1 has a bottom border,
//! list items render a marker column (bullet / ordinal), and raw Markdown
//! fallback renders as plain text.

use std::sync::Arc;

use gpui::*;
use gpui_component::{
    Icon, IconName, Sizable as _, Size,
    button::{Button as UiButton, ButtonVariants as _},
    highlighter::LanguageRegistry,
    menu::{DropdownMenu as _, PopupMenuItem},
    popover::Popover,
    spinner::Spinner,
    text::{TextView, TextViewStyle},
    tooltip::Tooltip,
};
use palette::IntoColor;

mod host_artifact;

const BLOCK_EDITOR_CONTEXT: &str = "BlockEditor";
const TABLE_TOOLBAR_HEIGHT: f32 = 28.0;
const TABLE_TOOLBAR_BUTTON_SIZE: f32 = 24.0;
const TABLE_TOOLBAR_GAP: f32 = 2.0;
const TABLE_SIZE_PICKER_COLUMNS: usize = 6;
const TABLE_SIZE_PICKER_ROWS: usize = 10;
const TABLE_SIZE_PICKER_CELL_SIZE: f32 = 20.0;
const TABLE_SIZE_PICKER_CELL_GAP: f32 = 4.0;

use self::host_artifact::{
    HostArtifactSize, contained_block_size, inline_size, render_host_svg, scrollable_block_size,
};
use super::element::BlockTextElement;
use super::{
    Block, BlockEvent, BlockKind, EnlargedBlockKind, HostRenderedArtifact, ImageResolvedSource,
    ImageRuntime,
};
use crate::BlockRenderKind;
use crate::components::{
    Editor, HtmlCssColor, HtmlDocument, HtmlNode, HtmlNodeKind, InlineScript,
    TableCellInlineImageSegment, TableColumnAlignment, TableColumnLayout, attr_value,
    display_math_font_size, inline_math_font_size, parse_display_math_source,
    parse_html_image_block, parse_mermaid_fence_source, parse_table_cell_inline_images,
    render_display_math_svg, render_inline_math_svg, resolve_image_source, style_for_node,
};
use crate::icons::{callout as callout_icons, indicators};
use crate::theme::{Theme, ThemeDimensions};
use rust_i18n::t;

// Unicode bullet glyphs for nested list depths.
const BULLET_FILLED: &str = "\u{2022}";
const BULLET_HOLLOW: &str = "\u{25E6}";
const BULLET_SQUARE: &str = "\u{25A1}";
fn bulleted_list_marker(depth: usize) -> &'static str {
    match depth {
        0 => BULLET_FILLED,
        1 => BULLET_HOLLOW,
        _ => BULLET_SQUARE,
    }
}

#[derive(Clone, Copy)]
enum TableToolbarAction {
    Align {
        column: usize,
        alignment: TableColumnAlignment,
    },
    Delete,
}

struct TableToolbarButton {
    id: String,
    icon: IconName,
    tooltip: SharedString,
    selected: bool,
    danger: bool,
    action: TableToolbarAction,
}

fn render_table_toolbar_button(
    theme: &Theme,
    table_block: WeakEntity<Block>,
    button: TableToolbarButton,
) -> AnyElement {
    let c = &theme.colors;
    let transparent = hsla(0.0, 0.0, 0.0, 0.0);
    let debug_selector = button.id.clone();
    let tooltip_text = button.tooltip.clone();
    div()
        .id(ElementId::Name(button.id.into()))
        .debug_selector(move || debug_selector.clone())
        .size(px(TABLE_TOOLBAR_BUTTON_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(if button.selected {
            c.table_axis_selected_bg
        } else {
            transparent
        })
        .text_color(if button.danger {
            c.dialog_danger_button_bg
        } else {
            c.dialog_secondary_button_text
        })
        .hover(|this| this.bg(c.dialog_secondary_button_hover))
        .active(|this| this.opacity(0.86))
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .block_mouse_except_scroll()
        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
            cx.stop_propagation();
        })
        .on_click(move |_event, _window, cx| {
            cx.stop_propagation();
            let _ = table_block.update(cx, |_block, cx| match button.action {
                TableToolbarAction::Align { column, alignment } => {
                    cx.emit(BlockEvent::RequestAlignTableColumn { column, alignment });
                }
                TableToolbarAction::Delete => cx.emit(BlockEvent::RequestDeleteTable),
            });
        })
        .child(Icon::new(button.icon).with_size(Size::XSmall))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_table_size_picker(
    theme: Arc<Theme>,
    table_block: WeakEntity<Block>,
    table_id: String,
    active_focus_handle: FocusHandle,
    current_columns: usize,
    current_visual_rows: usize,
    resize_tooltip: SharedString,
    columns_tooltip: SharedString,
    rows_tooltip: SharedString,
) -> AnyElement {
    let close_block = table_block.clone();
    let trigger_id = SharedString::from(format!("table-size-picker-trigger-{table_id}"));
    let popover_id = SharedString::from(format!("table-size-picker-{table_id}"));
    let trigger_cell = || {
        div()
            .size(px(4.0))
            .border(px(1.0))
            .border_color(theme.colors.dialog_secondary_button_text)
    };
    let trigger_grid = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .gap(px(2.0))
                .child(trigger_cell())
                .child(trigger_cell()),
        )
        .child(
            div()
                .flex()
                .gap(px(2.0))
                .child(trigger_cell())
                .child(trigger_cell()),
        );

    Popover::new(popover_id)
        .anchor(Anchor::TopLeft)
        .appearance(false)
        // The table toolbar is rendered only while a cell is focused. Keep
        // focus on that cell while the popover is open so the toolbar and its
        // keyed popover state remain mounted.
        .track_focus(&active_focus_handle)
        .on_open_change(move |open, _window, cx| {
            if !*open {
                let _ = close_block.update(cx, |block, cx| {
                    if block.table_size_picker_hover.take().is_some() {
                        cx.notify();
                    }
                });
            }
        })
        .trigger(
            UiButton::new(trigger_id)
                .child(trigger_grid)
                .tooltip(resize_tooltip)
                .ghost()
                .small()
                .px_0()
                .w(px(TABLE_TOOLBAR_BUTTON_SIZE))
                .h(px(TABLE_TOOLBAR_BUTTON_SIZE)),
        )
        .content(move |_popover, _window, cx| {
            let popover_entity = cx.entity().downgrade();
            let preview = table_block
                .upgrade()
                .and_then(|block| block.read(cx).table_size_picker_hover)
                .unwrap_or((current_columns, current_visual_rows));
            let preview_columns = preview.0.min(TABLE_SIZE_PICKER_COLUMNS);
            let preview_rows = preview.1.min(TABLE_SIZE_PICKER_ROWS);
            let c = &theme.colors;
            let grid = div()
                .flex()
                .flex_col()
                .gap(px(TABLE_SIZE_PICKER_CELL_GAP))
                .children((1..=TABLE_SIZE_PICKER_ROWS).map(|visual_row| {
                    div().flex().gap(px(TABLE_SIZE_PICKER_CELL_GAP)).children(
                        (1..=TABLE_SIZE_PICKER_COLUMNS).map(|column| {
                            let selected = column <= preview_columns && visual_row <= preview_rows;
                            let hover_block = table_block.clone();
                            let resize_block = table_block.clone();
                            let popover_entity = popover_entity.clone();
                            div()
                                .id(ElementId::Name(
                                    format!(
                                        "table-size-picker-cell-{table_id}-{column}-{visual_row}"
                                    )
                                    .into(),
                                ))
                                .size(px(TABLE_SIZE_PICKER_CELL_SIZE))
                                .rounded(px(3.0))
                                .border(px(1.0))
                                .border_color(if selected {
                                    c.dialog_primary_button_bg
                                } else {
                                    c.dialog_border
                                })
                                .bg(if selected {
                                    c.table_axis_selected_bg
                                } else {
                                    c.dialog_surface
                                })
                                .cursor_pointer()
                                .on_hover(move |hovered, _window, cx| {
                                    if *hovered {
                                        let _ = hover_block.update(cx, |block, cx| {
                                            let next = Some((column, visual_row));
                                            if block.table_size_picker_hover != next {
                                                block.table_size_picker_hover = next;
                                                cx.notify();
                                            }
                                        });
                                    }
                                })
                                .on_click(move |_event, window, cx| {
                                    cx.stop_propagation();
                                    let _ = resize_block.update(cx, |block, cx| {
                                        block.table_size_picker_hover = None;
                                        cx.emit(BlockEvent::RequestResizeTable {
                                            visual_rows: visual_row,
                                            columns: column,
                                        });
                                    });
                                    let _ = popover_entity.update(cx, |popover, cx| {
                                        popover.dismiss(window, cx);
                                    });
                                })
                        }),
                    )
                }));
            let columns_tooltip = columns_tooltip.clone();
            let rows_tooltip = rows_tooltip.clone();
            let hover_clear_block = table_block.clone();

            div()
                // TopLeft anchors to the trigger itself. Reserve the toolbar
                // height so the visible panel opens directly below the button.
                .pt(px(TABLE_TOOLBAR_HEIGHT + TABLE_TOOLBAR_GAP))
                .child(
                    div()
                        .id(ElementId::Name(
                            format!("table-size-picker-panel-{table_id}").into(),
                        ))
                        .p(px(10.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .occlude()
                        .bg(c.dialog_surface)
                        .border(px(1.0))
                        .border_color(c.dialog_border)
                        .rounded(px(6.0))
                        .shadow_lg()
                        .on_hover(move |hovered, _window, cx| {
                            if !*hovered {
                                let _ = hover_clear_block.update(cx, |block, cx| {
                                    if block.table_size_picker_hover.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .child(grid)
                        .child(div().w_full().h(px(1.0)).bg(c.dialog_border))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(7.0))
                                .child(
                                    div()
                                        .id(ElementId::Name(
                                            format!("table-size-picker-columns-{table_id}").into(),
                                        ))
                                        .min_w(px(36.0))
                                        .h(px(24.0))
                                        .px(px(8.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.0))
                                        .border(px(1.0))
                                        .border_color(c.dialog_border)
                                        .bg(c.dialog_surface)
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(columns_tooltip.clone()).build(window, cx)
                                        })
                                        .child(preview_columns.to_string()),
                                )
                                .child("×")
                                .child(
                                    div()
                                        .id(ElementId::Name(
                                            format!("table-size-picker-rows-{table_id}").into(),
                                        ))
                                        .min_w(px(36.0))
                                        .h(px(24.0))
                                        .px(px(8.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.0))
                                        .border(px(1.0))
                                        .border_color(c.dialog_border)
                                        .bg(c.dialog_surface)
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(rows_tooltip.clone()).build(window, cx)
                                        })
                                        .child(preview_rows.to_string()),
                                ),
                        ),
                )
        })
        .into_any_element()
}

fn fallback_image_label(alt: &str) -> SharedString {
    if alt.trim().is_empty() {
        SharedString::from(t!("MarkdownEditor.image_placeholder").to_string())
    } else {
        SharedString::from(alt.to_string())
    }
}

fn render_image_placeholder(
    runtime: &ImageRuntime,
    width: Length,
    height: Pixels,
    theme: &Theme,
) -> AnyElement {
    let c = &theme.colors;
    let d = &theme.dimensions;
    let t = &theme.typography;
    div()
        .w(width)
        .h(height)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(d.image_radius))
        .border(px(1.0))
        .border_color(c.image_placeholder_border)
        .bg(c.image_placeholder_bg)
        .px(px(d.block_padding_x))
        .text_center()
        .text_size(px(t.text_size))
        .text_color(c.image_placeholder_text)
        .child(fallback_image_label(&runtime.alt))
        .into_any_element()
}

fn render_loading_placeholder(
    runtime: &ImageRuntime,
    width: Length,
    height: Pixels,
    theme: &Theme,
) -> AnyElement {
    let c = &theme.colors;
    let d = &theme.dimensions;
    let t = &theme.typography;
    div()
        .w(width)
        .h(height)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(d.image_radius))
        .border(px(1.0))
        .border_color(c.image_placeholder_border)
        .bg(c.image_placeholder_bg)
        .px(px(d.block_padding_x))
        .text_center()
        .text_size(px(t.code_size))
        .text_color(c.image_placeholder_text)
        .child(if runtime.alt.trim().is_empty() {
            SharedString::from(t!("MarkdownEditor.image_loading_without_alt").to_string())
        } else {
            SharedString::from(
                t!(
                    "MarkdownEditor.image_loading_with_alt_template",
                    alt = runtime.alt.clone()
                )
                .to_string(),
            )
        })
        .into_any_element()
}

fn render_host_block_loading_placeholder(raw: &str, theme: &Theme, font_size: f32) -> AnyElement {
    let c = &theme.colors;
    let d = &theme.dimensions;
    let t = &theme.typography;
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(px(8.0))
        .rounded_sm()
        .bg(c.source_mode_block_bg)
        .px(px(d.block_padding_x))
        .py(px(d.block_padding_y))
        .text_size(px(font_size))
        .line_height(rems(t.text_line_height))
        .text_color(c.text_default)
        .child(
            div()
                .flex_none()
                .pt(px(2.0))
                .text_color(c.dialog_muted)
                .child(Spinner::new().with_size(Size::Small).color(c.dialog_muted)),
        )
        .child(
            div()
                .min_w(px(0.0))
                .flex_1()
                .child(SharedString::from(raw.to_string())),
        )
        .into_any_element()
}

fn wrap_with_quote_guides(content: AnyElement, quote_depth: usize, theme: &Theme) -> AnyElement {
    if quote_depth == 0 {
        return content;
    }

    let c = &theme.colors;
    let d = &theme.dimensions;
    let guide_offset = d.quote_padding_left;
    let total_padding = guide_offset * quote_depth as f32;

    div()
        .w_full()
        .relative()
        .pl(px(total_padding))
        .child(content)
        .children((0..quote_depth).map(|level| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(guide_offset * level as f32))
                .w(px(d.quote_border_width))
                .bg(c.border_quote)
        }))
        .into_any_element()
}

fn callout_accent_and_background(variant: super::CalloutVariant, theme: &Theme) -> (Hsla, Hsla) {
    let c = &theme.colors;
    match variant {
        super::CalloutVariant::Note => (c.callout_note_border, c.callout_note_bg),
        super::CalloutVariant::Tip => (c.callout_tip_border, c.callout_tip_bg),
        super::CalloutVariant::Important => (c.callout_important_border, c.callout_important_bg),
        super::CalloutVariant::Warning => (c.callout_warning_border, c.callout_warning_bg),
        super::CalloutVariant::Caution => (c.callout_caution_border, c.callout_caution_bg),
    }
}

fn visible_quote_guides(block: &Block) -> usize {
    block.visible_quote_depth
}

fn effective_table_width(block: &Block, viewport_width: f32, d: &ThemeDimensions) -> f32 {
    let centered_width = Editor::centered_column_width(viewport_width, d);
    let visible_quote_guides = visible_quote_guides(block);
    let quote_inset = d.quote_padding_left * visible_quote_guides as f32;
    let callout_inset = if block.callout_depth > 0 {
        d.callout_padding_x * 2.0 + d.callout_border_width
    } else {
        0.0
    };

    (centered_width - quote_inset - callout_inset)
        .max((d.table_cell_padding_x * 2.0 + 80.0).max(120.0))
}

fn container_image_width_budget(block: &Block, viewport_width: f32, d: &ThemeDimensions) -> f32 {
    let centered_width = Editor::centered_column_width(viewport_width, d);
    let visible_quote_guides = visible_quote_guides(block);
    let quote_inset = d.quote_padding_left * visible_quote_guides as f32;
    let callout_inset = if block.callout_depth > 0 {
        d.callout_padding_x * 2.0 + d.callout_border_width
    } else {
        0.0
    };

    centered_width - quote_inset - callout_inset
}

fn effective_image_width(block: &Block, viewport_width: f32, d: &ThemeDimensions) -> f32 {
    let list_inset = d.nested_block_indent * block.render_depth as f32;
    (container_image_width_budget(block, viewport_width, d) - d.block_padding_x * 2.0 - list_inset)
        .max(160.0)
}

fn effective_list_item_image_width(block: &Block, viewport_width: f32, d: &ThemeDimensions) -> f32 {
    let marker_width = match block.kind() {
        BlockKind::BulletedListItem => d.list_marker_width,
        BlockKind::TaskListItem { .. } => d.list_marker_width.max(d.task_checkbox_size),
        BlockKind::NumberedListItem => d.ordered_list_marker_width,
        _ => 0.0,
    };
    let list_inset = d.nested_block_indent * block.render_depth as f32;

    (container_image_width_budget(block, viewport_width, d)
        - d.block_padding_x * 2.0
        - list_inset
        - marker_width
        - d.list_marker_gap)
        .max(160.0)
}

/// Returns a human-readable list ordinal: numbers at depth 0, lowercase
/// letters at depth 1, and unicode roman numerals at depth 2+.
fn numbered_list_marker(depth: usize, ordinal: usize) -> String {
    match depth {
        0 => format!("{ordinal}."),
        1 => format!("{}.", alphabetic_list_marker(ordinal)),
        _ => format!("{}.", roman_list_marker(ordinal)),
    }
}

/// Expands beyond 26 by wrapping: a...z, a1...z1, a2...z2, ...
fn alphabetic_list_marker(ordinal: usize) -> String {
    const ALPHABET: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";

    let ordinal = ordinal.max(1);
    if ordinal <= ALPHABET.len() {
        return char::from(ALPHABET[ordinal - 1]).to_string();
    }

    let wrapped = ordinal - (ALPHABET.len() + 1);
    let letter = char::from(ALPHABET[wrapped % ALPHABET.len()]);
    let suffix = wrapped + 1;
    format!("{letter}{suffix}")
}

/// Converts an ASCII roman numeral string to its unicode ligature equivalents
/// where possible (for example, "III" to a single roman numeral glyph).
fn roman_list_marker(ordinal: usize) -> String {
    let ascii = ascii_roman_numeral(ordinal.max(1));
    let mut index = 0;
    let mut marker = String::new();

    while index < ascii.len() {
        let remaining = &ascii[index..];
        if let Some((token_len, token)) = roman_unicode_token(remaining) {
            marker.push_str(token);
            index += token_len;
        } else {
            break;
        }
    }

    marker
}

fn ascii_roman_numeral(mut ordinal: usize) -> String {
    const MAP: &[(usize, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];

    let mut result = String::new();
    for (value, symbol) in MAP {
        while ordinal >= *value {
            result.push_str(symbol);
            ordinal -= *value;
        }
    }
    result
}

fn roman_unicode_token(remaining: &str) -> Option<(usize, &'static str)> {
    const TOKENS: &[(&str, &str)] = &[
        ("XII", "\u{216B}"),
        ("XI", "\u{216A}"),
        ("IX", "\u{2168}"),
        ("VIII", "\u{2167}"),
        ("VII", "\u{2166}"),
        ("VI", "\u{2165}"),
        ("IV", "\u{2163}"),
        ("III", "\u{2162}"),
        ("II", "\u{2161}"),
        ("I", "\u{2160}"),
        ("V", "\u{2164}"),
        ("X", "\u{2169}"),
        ("L", "\u{216C}"),
        ("C", "\u{216D}"),
        ("D", "\u{216E}"),
        ("M", "\u{216F}"),
    ];

    TOKENS.iter().find_map(|(ascii, unicode)| {
        remaining
            .starts_with(ascii)
            .then_some((ascii.len(), *unicode))
    })
}

fn html_children_text(node: &HtmlNode) -> String {
    if node.children.is_empty() {
        return node.raw_source.clone();
    }

    let mut text = String::new();
    for child in &node.children {
        if child.tag_name == "br" {
            text.push('\n');
        } else {
            text.push_str(&html_children_text(child));
        }
    }
    text
}

#[derive(Clone, Copy, Debug)]
struct HtmlComputedStyle {
    color: Hsla,
    font_size: f32,
    root_font_size: f32,
}

#[derive(Clone, Copy, Debug)]
struct HtmlNodeVisualStyle {
    computed: HtmlComputedStyle,
    background: Option<Hsla>,
}

impl HtmlComputedStyle {
    fn root(theme: &Theme) -> Self {
        Self {
            color: theme.colors.text_default,
            font_size: theme.typography.text_size,
            root_font_size: theme.typography.text_size,
        }
    }
}

fn html_css_color_to_hsla(color: HtmlCssColor, current_color: Hsla) -> Hsla {
    match color {
        HtmlCssColor::CurrentColor => current_color,
        HtmlCssColor::Rgba(color) => Rgba::new(
            color.red as f32 / 255.0,
            color.green as f32 / 255.0,
            color.blue as f32 / 255.0,
            color.alpha.clamp(0.0, 1.0),
        )
        .into_color(),
    }
}

fn html_node_visual_style(
    node: &HtmlNode,
    parent: HtmlComputedStyle,
    theme: &Theme,
) -> HtmlNodeVisualStyle {
    let c = &theme.colors;
    let t = &theme.typography;
    let mut computed = parent;
    let mut background = None;

    match node.tag_name.as_str() {
        "a" => computed.color = c.text_link,
        "blockquote" => computed.color = c.text_quote,
        "code" | "kbd" | "pre" => {
            computed.color = c.code_text;
            computed.font_size = t.code_size;
            background = Some(c.code_bg);
        }
        "mark" => background = Some(c.comment_bg),
        "figcaption" => {
            computed.color = c.image_caption_text;
            computed.font_size = t.code_size;
        }
        "small" | "sup" | "sub" => computed.font_size = (computed.font_size * 0.8).max(6.0),
        "th" => background = Some(c.table_header_bg),
        "td" => background = Some(c.table_cell_bg),
        _ => {}
    }

    let inline_style = style_for_node(node);
    if let Some(color) = inline_style.color {
        computed.color = html_css_color_to_hsla(color, computed.color);
    }
    if let Some(font_size) = inline_style.font_size {
        computed.font_size = font_size.resolve(computed.font_size, computed.root_font_size);
    }
    if let Some(color) = inline_style.background_color {
        background = Some(html_css_color_to_hsla(color, computed.color));
    }

    HtmlNodeVisualStyle {
        computed,
        background,
    }
}

fn html_text_view_style(theme: &Theme) -> TextViewStyle {
    let c = &theme.colors;
    let t = &theme.typography;

    let mut code_block = StyleRefinement::default();
    code_block.background = Some(c.code_bg.into());
    code_block.text.color = Some(c.code_text);
    let mut table_head = StyleRefinement::default();
    table_head.background = Some(c.table_header_bg.into());
    table_head.text.color = Some(c.text_default);
    let mut table_cell = StyleRefinement::default();
    table_cell.background = Some(c.table_cell_bg.into());
    let mut style = TextViewStyle::default()
        .paragraph_gap(rems(theme.dimensions.block_gap / t.text_size.max(1.0)))
        .code_block(code_block)
        .inline_code(HighlightStyle {
            color: Some(c.code_text),
            background_color: Some(c.code_bg),
            ..Default::default()
        })
        .table_head(table_head)
        .table_cell(table_cell);
    style.heading_base_font_size = px(t.text_size);
    style.is_dark = c.editor_background.lightness < 0.5;
    style
}

impl Block {
    fn on_html_details_toggle_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.html_details_open = !self.html_details_open;
        cx.stop_propagation();
        cx.notify();
    }

    fn render_image_content(
        &self,
        runtime: &ImageRuntime,
        max_width: Length,
        max_height: Pixels,
        placeholder_height: Pixels,
        theme: &Theme,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let source = runtime.resolved_source.clone();
        let placeholder_theme = theme.clone();
        let loading_theme = theme.clone();
        let runtime_for_fallback = runtime.clone();
        let runtime_for_loading = runtime.clone();

        let image = match source {
            ImageResolvedSource::Local(path) => img(path),
            ImageResolvedSource::Remote(uri) => img(uri),
        }
        .max_w(max_width)
        .max_h(max_height)
        .object_fit(ObjectFit::Contain)
        .with_fallback(move || {
            render_image_placeholder(
                &runtime_for_fallback,
                max_width,
                placeholder_height,
                &placeholder_theme,
            )
        })
        .with_loading(move || {
            render_loading_placeholder(
                &runtime_for_loading,
                max_width,
                placeholder_height,
                &loading_theme,
            )
        });

        let mut container = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(d.image_caption_gap))
            .child(image);

        if let Some(title) = runtime
            .title
            .as_ref()
            .filter(|title| !title.trim().is_empty())
        {
            container = container.child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(t.code_size))
                    .text_color(c.image_caption_text)
                    .child(SharedString::from(title.clone())),
            );
        }

        container.into_any_element()
    }

    /// Renders a host-rendered Mermaid/Math SVG with a click-to-enlarge handler.
    /// The click asks the editor to open the enlarged preview overlay.
    fn render_clickable_host_svg(
        &self,
        rendered: &HostRenderedArtifact,
        size: HostArtifactSize,
        kind: EnlargedBlockKind,
        source: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let artifact = rendered.clone();
        // A click must reach this image, so swallow the mouse-down instead of
        // letting it focus the block: Math/Mermaid blocks swap their rendered
        // image for editable source text on focus, which would destroy this
        // element before the click fires and leave the enlarged view unopened.
        // The id keeps on_click's pending-mouse-down state alive across the
        // redraw scheduled by the mousedown.
        img(rendered.image.clone())
            .id(ElementId::Name(
                format!("enlargable-host-svg-{}", self.record.id).into(),
            ))
            .debug_selector(|| "enlargable-host-svg".to_string())
            .w(px(size.width))
            .h(px(size.height))
            .object_fit(ObjectFit::Contain)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |_block, _event, _window, cx| {
                cx.emit(BlockEvent::RequestEnlargeRenderedBlock {
                    kind,
                    source: source.clone(),
                    artifact: artifact.clone(),
                });
            }))
            .into_any_element()
    }

    fn render_rendered_block_source_button(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let source_label =
            SharedString::from(t!("MarkdownEditor.enlarged_view_source").to_string());

        div()
            .id(ElementId::Name(
                format!("rendered-block-source-{}", self.record.id).into(),
            ))
            .debug_selector(|| "rendered-block-source".to_string())
            .absolute()
            .top(px(4.0))
            .right(px(4.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .bg(c.dialog_secondary_button_bg)
            .text_color(c.dialog_secondary_button_text)
            .text_size(px(theme.typography.code_size))
            .hover(|this| this.bg(c.dialog_secondary_button_hover))
            .active(|this| this.opacity(0.86))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(source_label.clone()).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(|_block, _event, _window, cx| {
                cx.stop_propagation();
                cx.emit(BlockEvent::RequestFocus);
            }))
            .child(SharedString::from(
                t!("MarkdownEditor.enlarged_view_source").to_string(),
            ))
            .into_any_element()
    }

    fn render_rendered_block(
        &self,
        content: AnyElement,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .relative()
            .w_full()
            .child(content)
            .child(self.render_rendered_block_source_button(theme, cx))
            .into_any_element()
    }

    fn render_math_content(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let raw = self
            .record
            .raw_fallback
            .as_deref()
            .unwrap_or_else(|| self.display_text());

        let Some(source) = parse_display_math_source(raw) else {
            return div()
                .w_full()
                .text_size(px(t.text_size))
                .line_height(rems(t.text_line_height))
                .text_color(c.text_default)
                .child(SharedString::from(raw.to_string()))
                .into_any_element();
        };

        let request = self.host_render_request(BlockRenderKind::Math, source.body.clone());
        if let Some(rendered) = self.resolve_host_render(request.clone(), cx) {
            let size = contained_block_size(
                &rendered,
                self.host_render_available_width(),
                d.image_root_max_height,
                96.0,
            );
            return div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(d.block_padding_y.max(6.0)))
                .child(self.render_clickable_host_svg(
                    &rendered,
                    size,
                    EnlargedBlockKind::Math,
                    source.body.clone(),
                    cx,
                ))
                .into_any_element();
        }

        if self.host_render_is_pending(&request) {
            return render_host_block_loading_placeholder(raw, theme, t.text_size);
        }

        // Navop's host renderer is backed by the extension WASM runtime. While
        // that background request is pending (or if it fails), never run the
        // local RaTeX renderer from GPUI's synchronous render pass. Doing so
        // repeats parse/layout/SVG generation on every repaint and blocks
        // typing and scrolling.
        if self.has_host_render_provider() {
            return div()
                .w_full()
                .rounded_sm()
                .bg(c.source_mode_block_bg)
                .px(px(d.block_padding_x))
                .py(px(d.block_padding_y))
                .text_size(px(t.text_size))
                .line_height(rems(t.text_line_height))
                .text_color(c.text_default)
                .child(SharedString::from(raw.to_string()))
                .into_any_element();
        }

        match render_display_math_svg(&source, c.text_default, display_math_font_size(t.text_size))
        {
            Ok(rendered) => div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(d.block_padding_y.max(6.0)))
                .child(
                    img(rendered.path)
                        .max_w(Length::Definite(relative(1.0)))
                        .max_h(px(d.image_root_max_height))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Err(err) => div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded_sm()
                .bg(c.source_mode_block_bg)
                .px(px(d.block_padding_x))
                .py(px(d.block_padding_y))
                .text_size(px(t.text_size))
                .line_height(rems(t.text_line_height))
                .text_color(c.text_default)
                .child(SharedString::from(raw.to_string()))
                .child(
                    div()
                        .text_size(px(t.code_size))
                        .text_color(c.dialog_muted)
                        .child(SharedString::from(format!("LaTeX render error: {err}"))),
                )
                .into_any_element(),
        }
    }

    fn render_mermaid_content(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let raw = self
            .record
            .raw_fallback
            .as_deref()
            .unwrap_or_else(|| self.display_text());

        let Some(source) = parse_mermaid_fence_source(raw) else {
            return div()
                .w_full()
                .text_size(px(t.text_size))
                .line_height(rems(t.text_line_height))
                .text_color(c.text_default)
                .child(SharedString::from(raw.to_string()))
                .into_any_element();
        };

        let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
        let available_width = effective_image_width(self, viewport_width, d);

        let request = self.host_render_request(BlockRenderKind::Mermaid, source.body.clone());
        if let Some(rendered) = self.resolve_host_render(request.clone(), cx) {
            let size =
                scrollable_block_size(&rendered, available_width, d.image_root_max_height, 260.0);
            let image = self.render_clickable_host_svg(
                &rendered,
                size,
                EnlargedBlockKind::Mermaid,
                source.body.clone(),
                cx,
            );
            let content = if size.width <= available_width + 0.5 {
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(image)
                    .into_any_element()
            } else {
                div()
                    .id(ElementId::Name(
                        format!("mermaid-scroll-{}", self.record.id).into(),
                    ))
                    .w_full()
                    .overflow_x_scroll()
                    .scrollbar_width(px(0.0))
                    .child(div().w(px(size.width)).child(image))
                    .into_any_element()
            };
            return div()
                .w_full()
                .py(px(d.block_padding_y.max(6.0)))
                .child(content)
                .into_any_element();
        }

        if self.host_render_is_pending(&request) {
            return render_host_block_loading_placeholder(raw, theme, t.code_size);
        }

        // Mermaid rendering belongs exclusively to Navop's host/WASM document
        // renderer. When no provider is installed, or it returned an error,
        // preserve an editable raw-source fallback rather than running the
        // Velotype native renderer on the UI thread.
        div()
            .w_full()
            .rounded_sm()
            .bg(c.source_mode_block_bg)
            .px(px(d.block_padding_x))
            .py(px(d.block_padding_y))
            .text_size(px(t.code_size))
            .line_height(rems(t.text_line_height))
            .text_color(c.text_default)
            .child(SharedString::from(raw.to_string()))
            .into_any_element()
    }

    fn render_text_or_mixed_inline_visuals(
        &self,
        theme: &Theme,
        focused: bool,
        is_placeholder: bool,
        placeholder_text: Option<SharedString>,
        placeholder_color: Option<Hsla>,
        text_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Mixed inline visuals are display-only. Once focused, the text element
        // takes over so caret movement, projection markers, and IME ranges stay
        // anchored to editable text rather than rendered SVG/script offsets.
        if focused || is_placeholder || !self.has_mixed_inline_visuals() {
            return match placeholder_text {
                Some(placeholder) => BlockTextElement::with_placeholder(
                    cx.entity(),
                    is_placeholder,
                    placeholder,
                    placeholder_color,
                )
                .into_any_element(),
                None => BlockTextElement::new(cx.entity(), is_placeholder).into_any_element(),
            };
        }

        self.render_mixed_inline_visual_runs(theme, text_color, font_size, font_weight, cx)
    }

    fn render_heading_content(
        &self,
        level: u8,
        theme: &Theme,
        focused: bool,
        is_placeholder: bool,
        text_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = self.render_text_or_mixed_inline_visuals(
            theme,
            focused,
            is_placeholder,
            None,
            None,
            text_color,
            font_size,
            font_weight,
            cx,
        );

        if !focused || self.heading_shortcut_marker() != Some(level) {
            return text;
        }

        // The prefix is a visual sibling, never part of BlockTextElement.
        // Editable offsets therefore remain anchored to the clean heading
        // title, while the just-typed Markdown syntax stays visible.
        div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_row()
            .items_start()
            .child(div().flex_none().child(SharedString::from(format!(
                "{} ",
                "#".repeat(level as usize)
            ))))
            .child(div().min_w(px(0.0)).flex_grow_1().child(text))
            .into_any_element()
    }

    fn render_mixed_inline_visual_runs(
        &self,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_inline_tree_runs(
            &self.record.title,
            theme,
            base_color,
            font_size,
            font_weight,
            cx,
        )
    }

    fn render_inline_tree_runs(
        &self,
        tree: &crate::components::InlineTextTree,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(0.0))
            .text_size(px(font_size))
            .line_height(rems(theme.typography.text_line_height))
            .children(self.render_inline_tree_children(
                tree,
                theme,
                base_color,
                font_size,
                font_weight,
                cx,
            ))
            .into_any_element()
    }

    fn render_inline_tree_children(
        &self,
        tree: &crate::components::InlineTextTree,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let cache = tree.render_cache();
        let text = cache.visible_text();
        let mut children = Vec::new();
        let mut cursor = 0usize;

        for span in cache.spans() {
            if cursor < span.range.start {
                let fallback_span = crate::components::InlineSpan {
                    range: cursor..span.range.start,
                    style: crate::components::InlineStyle::default(),
                    html_style: None,
                    link: None,
                    footnote: None,
                    math: None,
                };
                children.extend(self.render_inline_text_word_segments(
                    &text[cursor..span.range.start],
                    &fallback_span,
                    theme,
                    base_color,
                    font_size,
                    font_weight,
                    cx,
                ));
            }

            let span_text = &text[span.range.clone()];
            if let Some(math) = span.math.as_ref() {
                children.push(
                    self.render_inline_math_segment(math, span, theme, base_color, font_size, cx),
                );
            } else {
                children.extend(self.render_inline_text_word_segments(
                    span_text,
                    span,
                    theme,
                    base_color,
                    font_size,
                    font_weight,
                    cx,
                ));
            }
            cursor = span.range.end;
        }

        if cursor < text.len() {
            let fallback_span = crate::components::InlineSpan {
                range: cursor..text.len(),
                style: crate::components::InlineStyle::default(),
                html_style: None,
                link: None,
                footnote: None,
                math: None,
            };
            children.extend(self.render_inline_text_word_segments(
                &text[cursor..],
                &fallback_span,
                theme,
                base_color,
                font_size,
                font_weight,
                cx,
            ));
        }

        children
    }

    /// Split a styled text run into wrap-friendly word segments. The mixed
    /// inline-visual layout is a `flex_wrap` row, so a long run rendered as one
    /// element wraps internally and claims the full row width, pushing the next
    /// item (inline math, a script, ...) onto its own line. Emitting one element
    /// per whitespace-delimited word lets the row break between words and keeps
    /// adjacent visuals on the same visual line. Inline code and background
    /// highlights stay a single element so their pill/background is continuous.
    fn render_inline_text_word_segments(
        &self,
        text: &str,
        span: &crate::components::InlineSpan,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let has_background = span
            .html_style
            .is_some_and(|style| style.background_color.is_some());
        let mut segments = Vec::new();
        for word in inline_word_chunks(text, span.style.code, has_background) {
            segments.push(self.render_inline_text_segment(
                word,
                span,
                theme,
                base_color,
                font_size,
                font_weight,
                cx,
            ));
        }
        segments
    }

    fn render_inline_text_segment(
        &self,
        text: &str,
        span: &crate::components::InlineSpan,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if text.is_empty() {
            return div().into_any_element();
        }

        let mut color = if span.link.is_some() || span.footnote.is_some() {
            theme.colors.text_link
        } else {
            base_color
        };
        if let Some(style) = span.html_style
            && let Some(html_color) = style.color
        {
            color = html_css_color_to_hsla(html_color, color);
        }

        let script_offset = match span.style.script {
            InlineScript::Normal => 0.0,
            InlineScript::Superscript => -font_size * 0.28,
            InlineScript::Subscript => font_size * 0.22,
        };
        let display_font_size = if span.style.has_script() {
            (font_size * 0.72).max(6.0)
        } else {
            font_size
        };

        let mut element = div()
            .min_w(px(0.0))
            .text_size(px(display_font_size))
            .line_height(rems(theme.typography.text_line_height))
            .text_color(color)
            .font_weight(if span.style.bold {
                FontWeight::BOLD
            } else {
                font_weight
            })
            .child(SharedString::from(text.to_string()));

        if script_offset != 0.0 {
            element = element.relative().top(px(script_offset));
        }

        if span.style.underline || span.link.is_some() || span.footnote.is_some() {
            element = element.underline();
        }
        if span.style.code {
            element = element
                .rounded(px(theme.dimensions.code_bg_radius))
                .px(px(theme.dimensions.code_bg_pad_x))
                .py(px(theme.dimensions.code_bg_pad_y))
                .bg(theme.colors.code_bg);
        }
        if let Some(style) = span.html_style
            && let Some(background) = style.background_color
        {
            element = element
                .rounded(px(3.0))
                .px(px(2.0))
                .bg(html_css_color_to_hsla(background, color));
        }

        // This run renders as plain (non-interactive) text, so a link inside a
        // mixed inline-visual block (alongside math or a script) would otherwise
        // have no way to be followed. Attach the open-link handlers directly to
        // the segment; they act only on Cmd/Ctrl+click so a plain click still
        // falls through and focuses the block for editing. The wrapper element
        // gates the hand cursor on that same modifier, matching the normal-text
        // path where links render through `BlockTextElement`.
        if let Some(link) = span.link.clone() {
            let element = element
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_rendered_link_mouse_down),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |block, event: &MouseUpEvent, _window, cx| {
                        if event.modifiers.secondary() {
                            block.open_rendered_link(&link, cx);
                        }
                    }),
                );
            return LinkFollowCursor {
                child: element.into_any_element(),
            }
            .into_any_element();
        }

        element.into_any_element()
    }

    fn render_inline_math_segment(
        &self,
        math: &crate::components::InlineMath,
        span: &crate::components::InlineSpan,
        theme: &Theme,
        base_color: Hsla,
        font_size: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut color = base_color;
        if let Some(style) = span.html_style
            && let Some(html_color) = style.color
        {
            color = html_css_color_to_hsla(html_color, color);
        }
        let math_size = inline_math_font_size(font_size);
        let request = self.host_render_request(BlockRenderKind::InlineMath, math.body.clone());
        if let Some(rendered) = self.resolve_host_render(request.clone(), cx) {
            let line_height = math_size * 1.65;
            let size = inline_size(&rendered, line_height, self.host_render_available_width());
            return div()
                .flex()
                .items_center()
                .h(px(line_height))
                .child(render_host_svg(&rendered, size))
                .into_any_element();
        }
        if self.host_render_is_pending(&request) {
            return div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .h(px(math_size * 1.65))
                .text_color(theme.colors.dialog_muted)
                .child(
                    Spinner::new()
                        .with_size(Size::Small)
                        .color(theme.colors.dialog_muted),
                )
                .child(self.render_inline_text_segment(
                    &math.source,
                    span,
                    theme,
                    base_color,
                    font_size,
                    FontWeight::NORMAL,
                    cx,
                ))
                .into_any_element();
        }
        if self.has_host_render_provider() {
            return self.render_inline_text_segment(
                &math.source,
                span,
                theme,
                base_color,
                font_size,
                FontWeight::NORMAL,
                cx,
            );
        }
        match render_inline_math_svg(&math.body, color, math_size) {
            Ok(rendered) => div()
                .flex()
                .items_center()
                .h(px(math_size * 1.65))
                .child(
                    img(rendered.path)
                        .max_h(px(math_size * 1.65))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Err(_) => self.render_inline_text_segment(
                &math.source,
                span,
                theme,
                base_color,
                font_size,
                FontWeight::NORMAL,
                cx,
            ),
        }
    }

    fn render_inline_image_content(&self, runtime: &ImageRuntime, theme: &Theme) -> AnyElement {
        let d = &theme.dimensions;
        let source = runtime.resolved_source.clone();
        let max_height = px(d.image_cell_placeholder_height);
        let max_width =
            Length::Definite(px((d.image_cell_placeholder_height * 1.6).max(48.0)).into());
        let placeholder_theme = theme.clone();
        let loading_theme = theme.clone();
        let runtime_for_fallback = runtime.clone();
        let runtime_for_loading = runtime.clone();

        let image = match source {
            ImageResolvedSource::Local(path) => img(path),
            ImageResolvedSource::Remote(uri) => img(uri),
        }
        .max_w(max_width)
        .max_h(max_height)
        .object_fit(ObjectFit::Contain)
        .with_fallback(move || {
            render_image_placeholder(
                &runtime_for_fallback,
                max_width,
                max_height,
                &placeholder_theme,
            )
        })
        .with_loading(move || {
            render_loading_placeholder(&runtime_for_loading, max_width, max_height, &loading_theme)
        });

        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .child(image)
            .into_any_element()
    }

    fn render_table_cell_inline_images(
        &self,
        theme: &Theme,
        font_weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let segments = parse_table_cell_inline_images(&self.record.title.serialize_markdown());
        if !segments
            .iter()
            .any(|segment| matches!(segment, TableCellInlineImageSegment::Image { .. }))
        {
            return None;
        }

        let mut children = Vec::new();
        for segment in segments {
            match segment {
                TableCellInlineImageSegment::Text(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    let tree = self.inline_tree_from_markdown_with_context(&text);
                    children.extend(self.render_inline_tree_children(
                        &tree,
                        theme,
                        theme.colors.text_default,
                        theme.typography.text_size,
                        font_weight,
                        cx,
                    ));
                }
                TableCellInlineImageSegment::Image { markdown, syntax } => {
                    if let Some(runtime) = self.image_runtime_for_syntax(syntax) {
                        children.push(self.render_inline_image_content(&runtime, theme));
                    } else {
                        let tree = crate::components::InlineTextTree::plain(markdown);
                        children.extend(self.render_inline_tree_children(
                            &tree,
                            theme,
                            theme.colors.text_default,
                            theme.typography.text_size,
                            font_weight,
                            cx,
                        ));
                    }
                }
            }
        }

        Some(
            div()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .text_size(px(theme.typography.text_size))
                .line_height(rems(theme.typography.text_line_height))
                .children(children)
                .into_any_element(),
        )
    }

    fn render_html_document(
        &self,
        document: &HtmlDocument,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        if !document.is_semantic() {
            return div()
                .w_full()
                .rounded_sm()
                .bg(c.source_mode_block_bg)
                .px(px(d.block_padding_x))
                .py(px(d.block_padding_y))
                .text_size(px(t.code_size))
                .text_color(c.text_default)
                .child(SharedString::from(document.raw_source.clone()))
                .into_any_element();
        }

        div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(d.block_gap * 0.4))
            .children(
                document.nodes.iter().map(|node| {
                    self.render_html_node(node, theme, HtmlComputedStyle::root(theme), cx)
                }),
            )
            .into_any_element()
    }

    fn render_html_node(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        inherited_style: HtmlComputedStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;

        if node.kind == HtmlNodeKind::RawTextBlock {
            return div()
                .w_full()
                .rounded_sm()
                .bg(c.source_mode_block_bg)
                .px(px(d.block_padding_x * 0.6))
                .py(px(d.block_padding_y * 0.6))
                .text_size(px(t.code_size))
                .text_color(c.text_default)
                .child(SharedString::from(node.raw_source.clone()))
                .into_any_element();
        }

        if node.tag_name == "#text" {
            return div()
                .min_w(px(0.0))
                .text_size(px(inherited_style.font_size))
                .text_color(inherited_style.color)
                .child(SharedString::from(node.raw_source.clone()))
                .into_any_element();
        }

        let node_style = html_node_visual_style(node, inherited_style, theme);
        match node.tag_name.as_str() {
            "strong" | "b" => {
                self.render_html_inline_container(node, theme, node_style, FontWeight::BOLD, cx)
            }
            "em" | "i" | "span" | "abbr" | "dfn" | "time" | "u" | "ins" | "del" | "small"
            | "sup" | "sub" | "a" => {
                self.render_html_inline_container(node, theme, node_style, FontWeight::NORMAL, cx)
            }
            "mark" => {
                self.render_html_inline_container(node, theme, node_style, FontWeight::NORMAL, cx)
            }
            "code" | "kbd" => {
                let mut element =
                    div()
                        .flex()
                        .rounded(px(4.0))
                        .px(px(4.0))
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "q" => {
                let mut element = div()
                    .flex()
                    .text_size(px(node_style.computed.font_size))
                    .text_color(node_style.computed.color)
                    .children([
                        div().child("\u{201C}").into_any_element(),
                        div()
                            .children(node.children.iter().map(|child| {
                                self.render_html_node(child, theme, node_style.computed, cx)
                            }))
                            .into_any_element(),
                        div().child("\u{201D}").into_any_element(),
                    ]);
                if let Some(bg) = node_style.background {
                    element = element.bg(bg).rounded(px(3.0)).px(px(2.0));
                }
                element.into_any_element()
            }
            "br" => div().child("\n").into_any_element(),
            "hr" => div()
                .w_full()
                .h(px(d.separator_thickness))
                .my(px(d.separator_margin_y))
                .bg(c.separator_color)
                .rounded(px(999.0))
                .into_any_element(),
            "blockquote" => {
                let mut element =
                    div()
                        .w_full()
                        .pl(px(d.quote_padding_left))
                        .border_l(px(d.quote_border_width))
                        .border_color(c.border_quote)
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "pre" => {
                let mut element = div()
                    .w_full()
                    .rounded_sm()
                    .px(px(d.code_block_padding_x))
                    .py(px(d.code_block_padding_y))
                    .text_size(px(node_style.computed.font_size))
                    .text_color(node_style.computed.color)
                    .child(SharedString::from(html_children_text(node)));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "img" => self.render_html_image(node, theme, node_style),
            "table" => self.render_html_table(node, theme, node_style, cx),
            "thead" | "tbody" | "tfoot" => {
                let mut element =
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "tr" => self.render_html_table_row(node, theme, node_style, cx),
            "th" | "td" => {
                let mut element =
                    div()
                        .min_w(px(0.0))
                        .flex_grow_1()
                        .border(px(1.0))
                        .border_color(c.table_border)
                        .px(px(d.table_cell_padding_x))
                        .py(px(d.table_cell_padding_y))
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .font_weight(if node.tag_name == "th" {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "details" => self.render_html_details(node, theme, node_style, cx),
            "summary" => {
                let mut element =
                    div()
                        .w_full()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "figure" => {
                let mut element =
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(d.image_caption_gap))
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            "figcaption" => {
                let mut element =
                    div()
                        .w_full()
                        .text_center()
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
            _ => {
                let mut element =
                    div()
                        .w_full()
                        .text_size(px(node_style.computed.font_size))
                        .text_color(node_style.computed.color)
                        .children(node.children.iter().map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        }));
                if let Some(bg) = node_style.background {
                    element = element.bg(bg);
                }
                element.into_any_element()
            }
        }
    }

    fn render_html_inline_container(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        node_style: HtmlNodeVisualStyle,
        weight: FontWeight,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .flex()
            .min_w(px(0.0))
            .text_size(px(node_style.computed.font_size))
            .text_color(node_style.computed.color)
            .font_weight(weight)
            .children(
                node.children
                    .iter()
                    .map(|child| self.render_html_node(child, theme, node_style.computed, cx)),
            );
        if let Some(bg) = node_style.background {
            element = element.bg(bg).rounded(px(3.0)).px(px(2.0));
        }
        match node.tag_name.as_str() {
            "sup" => {
                element = element
                    .relative()
                    .top(px(-node_style.computed.font_size * 0.28))
            }
            "sub" => {
                element = element
                    .relative()
                    .top(px(node_style.computed.font_size * 0.22))
            }
            _ => {}
        }
        element.into_any_element()
    }

    fn render_html_image(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        node_style: HtmlNodeVisualStyle,
    ) -> AnyElement {
        let parsed_image = parse_html_image_block(&node.raw_source);
        let src = parsed_image
            .as_ref()
            .map(|image| image.src.as_str())
            .or_else(|| attr_value(node, "src"))
            .filter(|src| !src.trim().is_empty());
        let Some(src) = src else {
            let mut element = div()
                .text_size(px(node_style.computed.font_size))
                .text_color(node_style.computed.color)
                .child(SharedString::from(node.raw_source.clone()));
            if let Some(bg) = node_style.background {
                element = element.bg(bg);
            }
            return element.into_any_element();
        };
        let alt = parsed_image
            .as_ref()
            .map(|image| image.alt.clone())
            .unwrap_or_else(|| attr_value(node, "alt").unwrap_or_default().to_string());
        let zoom = parsed_image
            .as_ref()
            .map(|image| image.zoom_factor())
            .unwrap_or(1.0);
        let runtime = ImageRuntime {
            alt,
            src: src.to_string(),
            title: None,
            resolved_source: resolve_image_source(src, self.image_base_dir()),
        };
        let content = self.render_image_content(
            &runtime,
            Length::Definite(relative(zoom)),
            px(theme.dimensions.image_root_max_height * zoom),
            px(theme.dimensions.image_root_placeholder_height * zoom),
            theme,
        );
        if let Some(bg) = node_style.background {
            div().w_full().bg(bg).child(content).into_any_element()
        } else {
            content
        }
    }

    fn render_html_table(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        node_style: HtmlNodeVisualStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .w_full()
            .border(px(1.0))
            .border_color(theme.colors.table_border)
            .text_size(px(node_style.computed.font_size))
            .text_color(node_style.computed.color)
            .children(
                node.children
                    .iter()
                    .map(|child| self.render_html_node(child, theme, node_style.computed, cx)),
            );
        if let Some(bg) = node_style.background {
            element = element.bg(bg);
        }
        element.into_any_element()
    }

    fn render_html_table_row(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        node_style: HtmlNodeVisualStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .w_full()
            .flex()
            .text_size(px(node_style.computed.font_size))
            .text_color(node_style.computed.color)
            .children(
                node.children
                    .iter()
                    .map(|child| self.render_html_node(child, theme, node_style.computed, cx)),
            );
        if let Some(bg) = node_style.background {
            element = element.bg(bg);
        }
        element.into_any_element()
    }

    fn render_html_details(
        &self,
        node: &HtmlNode,
        theme: &Theme,
        node_style: HtmlNodeVisualStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_open = attr_value(node, "open").is_some() || self.html_details_open;
        let summary = node
            .children
            .iter()
            .find(|child| child.tag_name == "summary");
        let body = node
            .children
            .iter()
            .filter(|child| child.tag_name != "summary");

        let mut container = div()
            .w_full()
            .rounded_sm()
            .border(px(1.0))
            .border_color(theme.colors.table_border)
            .px(px(theme.dimensions.block_padding_x))
            .py(px(theme.dimensions.block_padding_y))
            .text_size(px(node_style.computed.font_size))
            .text_color(node_style.computed.color)
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(theme.dimensions.list_marker_gap))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(Self::on_html_details_toggle_mouse_down),
                    )
                    .child(if is_open { "\u{25BE}" } else { "\u{25B8}" })
                    .children(summary.into_iter().map(|summary| {
                        self.render_html_node(summary, theme, node_style.computed, cx)
                    })),
            );
        if let Some(bg) = node_style.background {
            container = container.bg(bg);
        }

        if is_open {
            container =
                container.child(
                    div()
                        .w_full()
                        .pt(px(theme.dimensions.block_padding_y))
                        .children(body.map(|child| {
                            self.render_html_node(child, theme, node_style.computed, cx)
                        })),
                );
        }

        container.into_any_element()
    }

    fn render_shell(
        &self,
        block_id: ElementId,
        source_mode: bool,
        cursor_style: CursorStyle,
        padding_left: f32,
        padding_right: f32,
        dimensions: &ThemeDimensions,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let base = div()
            .id(block_id)
            .key_context(BLOCK_EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_newline))
            .on_action(cx.listener(Self::on_delete_back))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_word_delete_back))
            .on_action(cx.listener(Self::on_word_delete_forward))
            .on_action(cx.listener(Self::on_focus_prev))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_word_move_left))
            .on_action(cx.listener(Self::on_word_move_right))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_block_up))
            .on_action(cx.listener(Self::on_block_down))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_word_select_left))
            .on_action(cx.listener(Self::on_word_select_right))
            .on_action(cx.listener(Self::on_select_home))
            .on_action(cx.listener(Self::on_select_end))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_bold_selection))
            .on_action(cx.listener(Self::on_italic_selection))
            .on_action(cx.listener(Self::on_underline_selection))
            .on_action(cx.listener(Self::on_strikethrough_selection))
            .on_action(cx.listener(Self::on_code_selection))
            .on_action(cx.listener(Self::on_indent_block))
            .on_action(cx.listener(Self::on_outdent_block))
            .on_action(cx.listener(Self::on_exit_code_block))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .min_w(px(0.0))
            .flex_shrink_0()
            .min_h(px(dimensions.block_min_height))
            .py(px(dimensions.block_padding_y))
            .pl(px(padding_left))
            .pr(px(padding_right))
            .cursor(cursor_style);

        if source_mode {
            base
        } else {
            base.on_action(cx.listener(Self::on_set_paragraph))
                .on_action(cx.listener(Self::on_set_heading_1))
                .on_action(cx.listener(Self::on_set_heading_2))
                .on_action(cx.listener(Self::on_set_heading_3))
                .on_action(cx.listener(Self::on_set_heading_4))
                .on_action(cx.listener(Self::on_set_heading_5))
                .on_action(cx.listener(Self::on_set_heading_6))
                .on_action(cx.listener(Self::on_toggle_bullet_list))
                .on_action(cx.listener(Self::on_toggle_ordered_list))
                .on_action(cx.listener(Self::on_toggle_task_list))
                .on_action(cx.listener(Self::on_toggle_quote))
                .on_action(cx.listener(Self::on_toggle_code_block))
                .on_action(cx.listener(Self::on_move_block_up))
                .on_action(cx.listener(Self::on_move_block_down))
                .on_action(cx.listener(Self::on_duplicate_block))
                .on_action(cx.listener(Self::on_delete_block))
        }
    }
}

impl Focusable for Block {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The render method builds the full element tree for a block:
/// - Common wrapper: key_context, track_focus, action handlers, mouse events.
/// - Kind-specific styling: headings get size/weight/border, list items get
///   a flex row with marker + content, everything else renders as plain text.
/// - The [`BlockTextElement`] handles text layout, selection, and cursor.
impl Render for Block {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.install_heading_shortcut_blur_subscription(window, cx);
        self.sync_code_highlight_registry_revision();

        let focused = self.focus_handle.is_focused(window);
        // Keep the marker strictly scoped to the uninterrupted focus session
        // in which the heading shortcut was typed. The blur observer handles
        // normal window focus transitions, while this render-time guard also
        // covers programmatic block focus changes before deferred blur
        // callbacks are drained.
        if !focused {
            self.clear_heading_shortcut_marker(cx);
        }
        if self.sync_image_focus_state(focused) {
            cx.notify();
        }

        let showing_rendered_image = self.showing_rendered_image();
        // Inline math stays in the projected view while focused (its `$...$`
        // source shows as editable text), so links and other styling in the same
        // block keep their attributes instead of collapsing to raw Markdown, the
        // same way script spans already behave.
        self.sync_inline_projection_for_focus(focused && !showing_rendered_image);

        if focused && self.cursor_blink_task.is_none() {
            self.start_cursor_blink(cx);
        } else if !focused && self.cursor_blink_task.is_some() {
            self.cursor_blink_task = None;
        }

        let block_id = ElementId::Name(format!("block-{}", self.record.id).into());
        let show_heading_shortcut_marker = focused && self.heading_shortcut_marker().is_some();
        let is_placeholder = focused
            && !show_heading_shortcut_marker
            && self.display_text().is_empty()
            && self.marked_range.is_none();

        let theme = self.effective_theme(cx);
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
        self.set_host_render_environment(
            effective_image_width(self, viewport_width, d),
            window.scale_factor(),
        );
        let depth_padding = d.block_padding_x + d.nested_block_indent * self.render_depth as f32;

        if self.is_table_cell() {
            let table_cell_position = self.table_cell_position();
            let is_header = table_cell_position
                .map(|position| position.is_header())
                .unwrap_or(false);
            // The header row is only styled distinctly (shaded background, medium
            // weight) when the show-table-headers preference is enabled.
            let style_as_header =
                is_header && crate::config::EditorSettings::show_table_headers(cx);
            let bg = if style_as_header {
                c.table_header_bg
            } else {
                c.table_cell_bg
            };
            let border_color = if focused {
                c.table_cell_active_outline
            } else {
                c.table_border
            };
            let cell_debug_selector = format!("table-cell-{}", self.record.id);
            let cell_base = self
                .render_shell(
                    block_id,
                    false,
                    if showing_rendered_image {
                        CursorStyle::PointingHand
                    } else {
                        CursorStyle::IBeam
                    },
                    0.0,
                    0.0,
                    d,
                    cx,
                )
                .debug_selector(move || cell_debug_selector.clone())
                .w_full()
                .h_full()
                .min_h(px(d.table_cell_min_height))
                .px(px(d.table_cell_padding_x))
                .py(px(d.table_cell_padding_y))
                .rounded(px(2.0))
                .border(px(1.0))
                .border_color(border_color)
                .bg(bg)
                .text_size(px(t.text_size))
                .text_color(c.text_default)
                .line_height(rems(t.text_line_height))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(Self::on_table_cell_context_menu_mouse_down),
                );

            let cell_base = if style_as_header {
                cell_base.font_weight(FontWeight::MEDIUM)
            } else {
                cell_base
            };

            if showing_rendered_image && let Some(runtime) = self.image_runtime() {
                return cell_base
                    .child(self.render_image_content(
                        runtime,
                        Length::Definite(relative(1.0)),
                        px(d.image_cell_max_height),
                        px(d.image_cell_placeholder_height),
                        &theme,
                    ))
                    .into_any_element();
            }

            if !focused
                && let Some(inline_images) = self.render_table_cell_inline_images(
                    &theme,
                    if style_as_header {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    },
                    cx,
                )
            {
                return cell_base.child(inline_images).into_any_element();
            }

            return cell_base
                .child(self.render_text_or_mixed_inline_visuals(
                    &theme,
                    focused,
                    is_placeholder,
                    None,
                    None,
                    c.text_default,
                    t.text_size,
                    if style_as_header {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    },
                    cx,
                ))
                .into_any_element();
        }

        // Source-mode rendering: raw text with no formatting.
        if self.is_source_raw_mode()
            && (focused
                || !matches!(
                    self.kind(),
                    BlockKind::HtmlBlock | BlockKind::MathBlock | BlockKind::MermaidBlock
                ))
        {
            if focused && self.cursor_blink_task.is_none() {
                self.start_cursor_blink(cx);
            } else if !focused && self.cursor_blink_task.is_some() {
                self.cursor_blink_task = None;
            }
            let source_base = self
                .render_shell(
                    block_id.clone(),
                    true,
                    CursorStyle::IBeam,
                    d.block_padding_x,
                    d.block_padding_x,
                    d,
                    cx,
                )
                .text_size(px(t.text_size))
                .text_color(c.text_default)
                .line_height(rems(t.text_line_height));

            let source_base = if self.kind() == BlockKind::Comment {
                source_base.bg(c.comment_bg).rounded_sm()
            } else if focused {
                source_base.bg(c.source_mode_block_bg).rounded_sm()
            } else {
                source_base
            };

            return source_base
                .child(BlockTextElement::new(cx.entity(), is_placeholder))
                .into_any_element();
        }

        let focused_base = self.render_shell(
            block_id.clone(),
            false,
            if showing_rendered_image {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            },
            if self.kind().is_separator() {
                depth_padding + d.separator_inset_x
            } else {
                depth_padding
            },
            if self.kind().is_separator() {
                d.block_padding_x + d.separator_inset_x
            } else {
                d.block_padding_x
            },
            d,
            cx,
        );

        if showing_rendered_image && self.kind() == BlockKind::Paragraph {
            let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
            let max_width = px(effective_image_width(self, viewport_width, d));
            if let Some(runtime) = self.image_runtime() {
                return focused_base
                    .child(self.render_image_content(
                        runtime,
                        max_width.into(),
                        px(d.image_root_max_height),
                        px(d.image_root_placeholder_height),
                        &theme,
                    ))
                    .into_any_element();
            }
        }

        let content = match self.kind() {
            BlockKind::Separator => focused_base
                .py(px(d.separator_margin_y))
                .child(
                    div()
                        .w_full()
                        .h(px(d.separator_thickness))
                        .bg(c.separator_color)
                        .rounded(px(999.0)),
                )
                .into_any_element(),
            BlockKind::Heading { level: 1 } => focused_base
                .text_size(px(t.h1_size))
                .font_weight(t.h1_weight.to_font_weight())
                .text_color(c.text_h1)
                .pb(px(d.h1_padding_bottom))
                .mb(px(d.h1_margin_bottom))
                .border_b(px(d.h1_border_width))
                .border_color(c.border_h1)
                .child(self.render_heading_content(
                    1,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h1,
                    t.h1_size,
                    t.h1_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::Heading { level: 2 } => focused_base
                .text_size(px(t.h2_size))
                .font_weight(t.h2_weight.to_font_weight())
                .text_color(c.text_h2)
                .pb(px(d.h1_padding_bottom))
                .mb(px(d.h1_margin_bottom))
                .border_b(px(d.h1_border_width))
                .border_color(c.border_h2)
                .child(self.render_heading_content(
                    2,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h2,
                    t.h2_size,
                    t.h2_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::Heading { level: 3 } => focused_base
                .text_size(px(t.h3_size))
                .font_weight(t.h3_weight.to_font_weight())
                .text_color(c.text_h3)
                .child(self.render_heading_content(
                    3,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h3,
                    t.h3_size,
                    t.h3_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::Heading { level: 4 } => focused_base
                .text_size(px(t.h4_size))
                .font_weight(t.h4_weight.to_font_weight())
                .text_color(c.text_h4)
                .child(self.render_heading_content(
                    4,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h4,
                    t.h4_size,
                    t.h4_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::Heading { level: 5 } => focused_base
                .text_size(px(t.h5_size))
                .font_weight(t.h5_weight.to_font_weight())
                .text_color(c.text_h5)
                .child(self.render_heading_content(
                    5,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h5,
                    t.h5_size,
                    t.h5_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::Heading { level: 6 } => focused_base
                .text_size(px(t.h6_size))
                .font_weight(t.h6_weight.to_font_weight())
                .text_color(c.text_h6)
                .child(self.render_heading_content(
                    6,
                    &theme,
                    focused,
                    is_placeholder,
                    c.text_h6,
                    t.h6_size,
                    t.h6_weight.to_font_weight(),
                    cx,
                ))
                .into_any_element(),
            BlockKind::BulletedListItem => focused_base
                .text_size(px(t.text_size))
                .text_color(c.text_default)
                .line_height(rems(t.text_line_height))
                .w_full()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(d.list_marker_gap))
                .children([
                    div()
                        .min_w(px(d.list_marker_width))
                        .child(SharedString::new(bulleted_list_marker(self.render_depth))),
                    if showing_rendered_image {
                        let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
                        let max_width =
                            px(effective_list_item_image_width(self, viewport_width, d));
                        if let Some(runtime) = self.image_runtime() {
                            div().flex_grow_1().child(self.render_image_content(
                                runtime,
                                max_width.into(),
                                px(d.image_root_max_height),
                                px(d.image_root_placeholder_height),
                                &theme,
                            ))
                        } else {
                            div().min_w(px(0.0)).flex_grow_1().child(
                                self.render_text_or_mixed_inline_visuals(
                                    &theme,
                                    focused,
                                    is_placeholder,
                                    None,
                                    None,
                                    c.text_default,
                                    t.text_size,
                                    FontWeight::NORMAL,
                                    cx,
                                ),
                            )
                        }
                    } else {
                        div().min_w(px(0.0)).flex_grow_1().child(
                            self.render_text_or_mixed_inline_visuals(
                                &theme,
                                focused,
                                is_placeholder,
                                None,
                                None,
                                c.text_default,
                                t.text_size,
                                FontWeight::NORMAL,
                                cx,
                            ),
                        )
                    },
                ])
                .into_any_element(),
            BlockKind::TaskListItem { checked } => {
                let marker_width = d.list_marker_width.max(d.task_checkbox_size);
                let first_line_height = t.text_size * t.text_line_height;
                focused_base
                    .text_size(px(t.text_size))
                    .text_color(c.text_default)
                    .line_height(rems(t.text_line_height))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(d.list_marker_gap))
                    .children([
                        div()
                            .min_w(px(marker_width))
                            .h(px(first_line_height))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .size(px(d.task_checkbox_size))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(d.task_checkbox_radius))
                                    .border(px(d.task_checkbox_border_width))
                                    .border_color(c.task_checkbox_border)
                                    .bg(if checked {
                                        c.task_checkbox_checked_bg
                                    } else {
                                        c.task_checkbox_bg
                                    })
                                    .text_size(px(d.task_checkbox_check_size))
                                    .text_color(c.task_checkbox_check)
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(Self::on_task_checkbox_mouse_down),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(Self::on_task_checkbox_mouse_up),
                                    )
                                    .children(checked.then(|| {
                                        Icon::new(indicators::CHECKED)
                                            .with_size(px(d.task_checkbox_check_size))
                                            .text_color(c.task_checkbox_check)
                                    })),
                            ),
                        if showing_rendered_image {
                            let viewport_width =
                                f32::from(window.viewport_size().width.max(px(1.0)));
                            let max_width =
                                px(effective_list_item_image_width(self, viewport_width, d));
                            if let Some(runtime) = self.image_runtime() {
                                div().flex_grow_1().child(self.render_image_content(
                                    runtime,
                                    max_width.into(),
                                    px(d.image_root_max_height),
                                    px(d.image_root_placeholder_height),
                                    &theme,
                                ))
                            } else {
                                div().min_w(px(0.0)).flex_grow_1().child(
                                    self.render_text_or_mixed_inline_visuals(
                                        &theme,
                                        focused,
                                        is_placeholder,
                                        None,
                                        None,
                                        c.text_default,
                                        t.text_size,
                                        FontWeight::NORMAL,
                                        cx,
                                    ),
                                )
                            }
                        } else {
                            div().min_w(px(0.0)).flex_grow_1().child(
                                self.render_text_or_mixed_inline_visuals(
                                    &theme,
                                    focused,
                                    is_placeholder,
                                    None,
                                    None,
                                    c.text_default,
                                    t.text_size,
                                    FontWeight::NORMAL,
                                    cx,
                                ),
                            )
                        },
                    ])
                    .into_any_element()
            }
            BlockKind::NumberedListItem => focused_base
                .text_size(px(t.text_size))
                .text_color(c.text_default)
                .line_height(rems(t.text_line_height))
                .w_full()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(d.list_marker_gap))
                .children([
                    div()
                        .min_w(px(d.ordered_list_marker_width))
                        .child(SharedString::from(numbered_list_marker(
                            self.render_depth,
                            self.list_ordinal.unwrap_or(1),
                        ))),
                    if showing_rendered_image {
                        let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
                        let max_width =
                            px(effective_list_item_image_width(self, viewport_width, d));
                        if let Some(runtime) = self.image_runtime() {
                            div().flex_grow_1().child(self.render_image_content(
                                runtime,
                                max_width.into(),
                                px(d.image_root_max_height),
                                px(d.image_root_placeholder_height),
                                &theme,
                            ))
                        } else {
                            div().min_w(px(0.0)).flex_grow_1().child(
                                self.render_text_or_mixed_inline_visuals(
                                    &theme,
                                    focused,
                                    is_placeholder,
                                    None,
                                    None,
                                    c.text_default,
                                    t.text_size,
                                    FontWeight::NORMAL,
                                    cx,
                                ),
                            )
                        }
                    } else {
                        div().min_w(px(0.0)).flex_grow_1().child(
                            self.render_text_or_mixed_inline_visuals(
                                &theme,
                                focused,
                                is_placeholder,
                                None,
                                None,
                                c.text_default,
                                t.text_size,
                                FontWeight::NORMAL,
                                cx,
                            ),
                        )
                    },
                ])
                .into_any_element(),
            BlockKind::Quote => focused_base
                .text_size(px(t.text_size))
                .text_color(c.text_quote)
                .line_height(rems(t.text_line_height))
                .child(self.render_text_or_mixed_inline_visuals(
                    &theme,
                    focused,
                    is_placeholder,
                    None,
                    None,
                    c.text_quote,
                    t.text_size,
                    FontWeight::NORMAL,
                    cx,
                ))
                .into_any_element(),
            BlockKind::Callout(variant) => {
                let (accent, _) = callout_accent_and_background(variant, &theme);
                let title_is_empty = self.record.title.visible_text().is_empty();
                let show_static_default_label = title_is_empty && !focused;
                let header_label = SharedString::from(variant.label());
                let header_text = if show_static_default_label {
                    div()
                        .text_size(px(t.text_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child(header_label.clone())
                        .into_any_element()
                } else {
                    div()
                        .min_w(px(0.0))
                        .flex_grow_1()
                        .text_size(px(t.text_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child(self.render_text_or_mixed_inline_visuals(
                            &theme,
                            focused,
                            is_placeholder,
                            Some(header_label),
                            Some(accent),
                            accent,
                            t.text_size,
                            FontWeight::SEMIBOLD,
                            cx,
                        ))
                        .into_any_element()
                };

                focused_base
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(d.callout_header_gap))
                    .child(
                        div().text_color(accent).child(
                            Icon::new(callout_icons::icon(variant))
                                .with_size(px(t.text_size))
                                .text_color(accent),
                        ),
                    )
                    .child(header_text)
                    .into_any_element()
            }
            BlockKind::FootnoteDefinition => {
                let ordinal = self.footnote_definition_ordinal();
                let badge = ordinal
                    .map(|ordinal| ordinal.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let badge_text_size = px((t.code_size - 1.0).max(10.0));
                let header = focused_base
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(d.list_marker_gap))
                    .text_size(px(t.code_size))
                    .text_color(c.text_quote)
                    .child(
                        div()
                            .px(px(d.footnote_badge_padding_x))
                            .py(px(d.footnote_badge_padding_y))
                            .rounded(px(999.0))
                            .bg(c.footnote_badge_bg)
                            .text_size(badge_text_size)
                            .text_color(c.footnote_badge_text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(badge)),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_grow_1()
                            .text_color(c.text_quote)
                            .child(self.render_text_or_mixed_inline_visuals(
                                &theme,
                                focused,
                                is_placeholder,
                                None,
                                None,
                                c.text_quote,
                                t.code_size,
                                FontWeight::NORMAL,
                                cx,
                            )),
                    );

                if self.footnote_definition_has_backref() {
                    header
                        .child(
                            div()
                                .text_color(c.footnote_backref)
                                .hover(|this| this.text_color(c.text_link))
                                .cursor_pointer()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(Self::on_footnote_backref_mouse_down),
                                )
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(Self::on_footnote_backref_mouse_up),
                                )
                                .child("\u{21A9}"),
                        )
                        .into_any_element()
                } else {
                    header.into_any_element()
                }
            }
            BlockKind::CodeBlock { .. } => {
                let current_language = self
                    .code_language_text()
                    .trim()
                    .is_empty()
                    .then_some("text")
                    .unwrap_or_else(|| self.code_language_text())
                    .to_owned();
                let selected_language = LanguageRegistry::singleton()
                    .language(&current_language)
                    .map(|language| language.name.to_string())
                    .unwrap_or_else(|| current_language.clone());
                let mut language_options = LanguageRegistry::singleton().languages();
                if !language_options
                    .iter()
                    .any(|language| language.eq_ignore_ascii_case(&selected_language))
                {
                    language_options.push(SharedString::from(selected_language.clone()));
                    language_options.sort();
                }
                let language_block = cx.entity();
                let language_menu_options = language_options.clone();
                let language_menu_selected = selected_language.clone();
                let language_button = UiButton::new(SharedString::from(format!(
                    "code-language-button-{}",
                    self.record.id
                )))
                .label(current_language)
                .dropdown_caret(true)
                .ghost()
                .small()
                .dropdown_menu_with_anchor(
                    Anchor::TopRight,
                    move |mut menu, _, _| {
                        for language in &language_menu_options {
                            let checked = language.eq_ignore_ascii_case(&language_menu_selected);
                            let label = language.clone();
                            let next_language = language.clone();
                            let block = language_block.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div().w_full().child(label.clone())
                                })
                                .checked(checked)
                                .on_click(
                                    move |_, _window, cx| {
                                        if checked {
                                            return;
                                        }
                                        block.update(cx, |block, cx| {
                                            block.set_code_language(next_language.as_ref(), cx);
                                        });
                                    },
                                ),
                            );
                        }
                        menu.max_h(px(240.0)).scrollable(true)
                    },
                );

                // Keep the source surface and its language control in one
                // stable panel. This is the original Navop source-editor
                // composition: a 28px right-aligned ghost language dropdown
                // above a permanently mounted source surface.
                focused_base
                    .overflow_hidden()
                    .rounded(px(d.code_language_input_radius))
                    .border(px(1.0))
                    .border_color(c.code_language_input_border)
                    .bg(c.code_bg)
                    .p(px(d.code_block_padding_y))
                    .child(
                        div()
                            .w_full()
                            .h(px(28.0))
                            .min_h(px(28.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(language_button),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .w_full()
                            .text_size(px(t.code_size))
                            .text_color(c.code_text)
                            .line_height(rems(t.text_line_height))
                            .child(BlockTextElement::new(cx.entity(), is_placeholder)),
                    )
                    .into_any_element()
            }
            BlockKind::Table => {
                let Some(runtime) = self.table_runtime.clone() else {
                    return focused_base
                        .text_size(px(t.text_size))
                        .text_color(c.text_default)
                        .line_height(rems(t.text_line_height))
                        .child(self.render_text_or_mixed_inline_visuals(
                            &theme,
                            focused,
                            is_placeholder,
                            None,
                            None,
                            c.text_default,
                            t.text_size,
                            FontWeight::NORMAL,
                            cx,
                        ))
                        .into_any_element();
                };

                let viewport_width = f32::from(window.viewport_size().width.max(px(1.0)));
                let table_width = effective_table_width(self, viewport_width, d);
                let column_layout = self
                    .record
                    .table
                    .as_ref()
                    .map(|table| TableColumnLayout::measure(table, table_width, window, &theme))
                    .unwrap_or_else(|| TableColumnLayout::equal(runtime.header.len()));
                let body_row_count = runtime.rows.len();
                let current_visual_rows = body_row_count + 1;
                let current_columns = runtime.header.len().max(1);
                let weak_table_block = cx.entity().downgrade();
                let active_cell = runtime
                    .header
                    .iter()
                    .enumerate()
                    .find_map(|(column, cell)| {
                        let focus_handle = cell.read(cx).focus_handle.clone();
                        focus_handle
                            .is_focused(window)
                            .then_some((column, focus_handle))
                    })
                    .or_else(|| {
                        runtime.rows.iter().find_map(|row| {
                            row.iter().enumerate().find_map(|(column, cell)| {
                                let focus_handle = cell.read(cx).focus_handle.clone();
                                focus_handle
                                    .is_focused(window)
                                    .then_some((column, focus_handle))
                            })
                        })
                    });
                let active_column = active_cell.as_ref().map(|(column, _)| *column);
                let active_alignment = active_column.and_then(|column| {
                    self.record
                        .table
                        .as_ref()
                        .and_then(|table| table.alignments.get(column))
                        .copied()
                });

                let header_cells = runtime.header;
                let header_row = div().relative().w_full().flex().gap(px(0.0)).children(
                    header_cells.into_iter().enumerate().map(|(column, cell)| {
                        div()
                            .flex_none()
                            .flex_basis(relative(column_layout.fraction(column)))
                            .w(relative(column_layout.fraction(column)))
                            .h_full()
                            .min_w(px(0.0))
                            .child(cell)
                    }),
                );

                let body_rows = runtime.rows.into_iter().map(|row| {
                    div().relative().w_full().flex().gap(px(0.0)).children(
                        row.into_iter().enumerate().map(|(column, cell)| {
                            div()
                                .flex_none()
                                .flex_basis(relative(column_layout.fraction(column)))
                                .w(relative(column_layout.fraction(column)))
                                .h_full()
                                .min_w(px(0.0))
                                .child(cell)
                        }),
                    )
                });

                {
                    let mut rows = Vec::with_capacity(1 + body_row_count);
                    rows.push(header_row.into_any_element());
                    rows.extend(body_rows.map(|row| row.into_any_element()));

                    let table_toolbar = active_cell.map(|(column, active_focus_handle)| {
                        let alignment = active_alignment.unwrap_or(TableColumnAlignment::Default);
                        let left_button_block = weak_table_block.clone();
                        let center_button_block = weak_table_block.clone();
                        let right_button_block = weak_table_block.clone();
                        let delete_button_block = weak_table_block.clone();
                        let table_id = self.record.id;
                        let size_picker = render_table_size_picker(
                            theme.clone(),
                            weak_table_block.clone(),
                            table_id.to_string(),
                            active_focus_handle,
                            current_columns,
                            current_visual_rows,
                            SharedString::from(
                                t!("MarkdownEditor.table_toolbar_resize").to_string(),
                            ),
                            SharedString::from(
                                t!("MarkdownEditor.table_size_picker_columns").to_string(),
                            ),
                            SharedString::from(
                                t!("MarkdownEditor.table_size_picker_rows").to_string(),
                            ),
                        );
                        div()
                            .id(ElementId::Name(format!("table-toolbar-{table_id}").into()))
                            .debug_selector(|| "table-toolbar".to_string())
                            .absolute()
                            .top(px(-TABLE_TOOLBAR_HEIGHT))
                            .left_0()
                            .right_0()
                            .h(px(TABLE_TOOLBAR_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(TABLE_TOOLBAR_GAP))
                                    .child(size_picker)
                                    .child(render_table_toolbar_button(
                                        &theme,
                                        left_button_block,
                                        TableToolbarButton {
                                            id: format!("table-toolbar-align-left-{table_id}"),
                                            icon: crate::icons::alignment::LEFT,
                                            tooltip: SharedString::from(
                                                t!("MarkdownEditor.table_toolbar_align_left")
                                                    .to_string(),
                                            ),
                                            selected: matches!(
                                                alignment,
                                                TableColumnAlignment::Default
                                                    | TableColumnAlignment::Left
                                            ),
                                            danger: false,
                                            action: TableToolbarAction::Align {
                                                column,
                                                alignment: TableColumnAlignment::Left,
                                            },
                                        },
                                    ))
                                    .child(render_table_toolbar_button(
                                        &theme,
                                        center_button_block,
                                        TableToolbarButton {
                                            id: format!("table-toolbar-align-center-{table_id}"),
                                            icon: crate::icons::alignment::CENTER,
                                            tooltip: SharedString::from(
                                                t!("MarkdownEditor.table_toolbar_align_center")
                                                    .to_string(),
                                            ),
                                            selected: alignment == TableColumnAlignment::Center,
                                            danger: false,
                                            action: TableToolbarAction::Align {
                                                column,
                                                alignment: TableColumnAlignment::Center,
                                            },
                                        },
                                    ))
                                    .child(render_table_toolbar_button(
                                        &theme,
                                        right_button_block,
                                        TableToolbarButton {
                                            id: format!("table-toolbar-align-right-{table_id}"),
                                            icon: crate::icons::alignment::RIGHT,
                                            tooltip: SharedString::from(
                                                t!("MarkdownEditor.table_toolbar_align_right")
                                                    .to_string(),
                                            ),
                                            selected: alignment == TableColumnAlignment::Right,
                                            danger: false,
                                            action: TableToolbarAction::Align {
                                                column,
                                                alignment: TableColumnAlignment::Right,
                                            },
                                        },
                                    )),
                            )
                            .child(render_table_toolbar_button(
                                &theme,
                                delete_button_block,
                                TableToolbarButton {
                                    id: format!("table-toolbar-delete-{table_id}"),
                                    icon: IconName::Delete,
                                    tooltip: SharedString::from(
                                        t!("MarkdownEditor.table_toolbar_delete").to_string(),
                                    ),
                                    selected: false,
                                    danger: true,
                                    action: TableToolbarAction::Delete,
                                },
                            ))
                    });

                    div()
                        .id(block_id)
                        .debug_selector(|| "table-root".to_string())
                        .w_full()
                        // Keep enough of the normal inter-block gap available for the
                        // Typora-style floating toolbar. The toolbar remains absolutely
                        // positioned, so focusing a cell never resizes or shifts the table.
                        .mt(px((TABLE_TOOLBAR_HEIGHT - d.block_gap).max(0.0)))
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(0.0))
                        .children(rows)
                        .children(table_toolbar)
                        .into_any_element()
                }
            }
            BlockKind::HtmlBlock => {
                let html = self.record.html.as_ref().cloned().unwrap_or_else(|| {
                    crate::components::parse_html_document(
                        self.record
                            .raw_fallback
                            .as_deref()
                            .unwrap_or_else(|| self.display_text()),
                    )
                });
                let html_preview = if html.is_semantic() {
                    TextView::html(
                        SharedString::from(format!("velotype-html-preview-{}", self.record.id)),
                        SharedString::from(html.raw_source.clone()),
                    )
                    .style(html_text_view_style(&theme))
                    .selectable(false)
                    .w_full()
                    .min_w(px(0.0))
                    .into_any_element()
                } else {
                    // Keep unsafe, malformed, or unsupported HTML visible as
                    // source text instead of handing it to the semantic HTML
                    // parser. The preserved raw source remains the editing and
                    // serialization truth in either mode.
                    self.render_html_document(&html, &theme, cx)
                };
                focused_base
                    .text_size(px(t.text_size))
                    .text_color(c.text_default)
                    .line_height(rems(t.text_line_height))
                    .child(html_preview)
                    .into_any_element()
            }
            BlockKind::MathBlock => {
                if !focused {
                    self.last_layout = None;
                    self.last_bounds = None;
                }
                let child = if focused {
                    BlockTextElement::new(cx.entity(), is_placeholder).into_any_element()
                } else {
                    let preview = self.render_math_content(&theme, cx);
                    self.render_rendered_block(preview, &theme, cx)
                };
                focused_base.w_full().child(child).into_any_element()
            }
            BlockKind::MermaidBlock => {
                if !focused {
                    self.last_layout = None;
                    self.last_bounds = None;
                }
                let child = if focused {
                    BlockTextElement::new(cx.entity(), is_placeholder).into_any_element()
                } else {
                    let preview = self.render_mermaid_content(&theme, window, cx);
                    self.render_rendered_block(preview, &theme, cx)
                };
                focused_base.w_full().child(child).into_any_element()
            }
            BlockKind::Paragraph
            | BlockKind::Comment
            | BlockKind::RawMarkdown
            | BlockKind::Heading { .. } => focused_base
                .text_size(px(t.text_size))
                .text_color(c.text_default)
                .line_height(rems(t.text_line_height))
                .child(self.render_text_or_mixed_inline_visuals(
                    &theme,
                    focused,
                    is_placeholder,
                    None,
                    None,
                    c.text_default,
                    t.text_size,
                    FontWeight::NORMAL,
                    cx,
                ))
                .into_any_element(),
        };

        wrap_with_quote_guides(content, visible_quote_guides(self), &theme)
    }
}

/// Break a styled inline text run into wrap-friendly chunks for the mixed
/// inline-visual layout. Runs that carry their own box (inline code, background
/// highlight) stay a single chunk so their padding/background is continuous;
/// everything else is split on whitespace with each word keeping its trailing
/// space, so the `flex_wrap` row can break between words instead of pushing the
/// next inline visual onto its own line.
/// Wraps a rendered inline link run so the hand cursor only appears while the
/// Cmd/Ctrl follow modifier is held. Links in mixed inline-visual blocks (math,
/// scripts, inline images) render as plain divs, so this sets `PointingHand`
/// when its hitbox is hovered and the modifier is down, like `BlockTextElement`
/// does for normal text. The editor root repaints on follow-modifier toggles,
/// so the cursor re-evaluates without the pointer moving. Layout and painting
/// are delegated to the child.
struct LinkFollowCursor {
    child: AnyElement,
}

impl IntoElement for LinkFollowCursor {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LinkFollowCursor {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if hitbox.is_hovered(window) && window.modifiers().secondary() {
            // The editor root repaints on follow-modifier toggles, so the hand
            // cursor re-evaluates here even while the pointer stays still.
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        self.child.paint(window, cx);
    }
}

fn inline_word_chunks(text: &str, code: bool, has_background: bool) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    if code || has_background {
        return vec![text];
    }
    text.split_inclusive(char::is_whitespace).collect()
}

#[cfg(test)]
mod tests {
    use super::{HtmlComputedStyle, html_node_visual_style, inline_word_chunks};
    use crate::components::parse_html_document;
    use crate::theme::Theme;
    use gpui::{Hsla, Rgba};
    use palette::IntoColor;

    fn assert_color_near(color: Hsla, red: u8, green: u8, blue: u8, alpha: u8) {
        let color: Rgba = color.into_color();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as i16;
        assert!((channel(color.red) - red as i16).abs() <= 1);
        assert!((channel(color.green) - green as i16).abs() <= 1);
        assert!((channel(color.blue) - blue as i16).abs() <= 1);
        assert!((channel(color.alpha) - alpha as i16).abs() <= 1);
    }

    #[test]
    fn inline_word_chunks_split_text_runs_for_wrapping() {
        // Plain runs split per word so the flex-wrap row can break between
        // words and keep neighboring inline math on the same visual line.
        assert_eq!(
            inline_word_chunks("Fusce x malesuada", false, false),
            vec!["Fusce ", "x ", "malesuada"],
        );
        // Trailing whitespace stays attached so spacing survives the split.
        assert_eq!(inline_word_chunks("end ", false, false), vec!["end "]);
        assert!(inline_word_chunks("", false, false).is_empty());
    }

    #[test]
    fn inline_word_chunks_keep_boxed_runs_whole() {
        // Inline code and background highlights keep their box continuous.
        assert_eq!(
            inline_word_chunks("let x = 2", true, false),
            vec!["let x = 2"],
        );
        assert_eq!(
            inline_word_chunks("highlighted text", false, true),
            vec!["highlighted text"],
        );
    }

    #[test]
    fn html_render_style_inherits_color_and_font_size() {
        let theme = Theme::default_theme();
        let doc = parse_html_document(
            "<div style=\"color:blue; font-size:20px\"><span style=\"font-size:120%\">x</span></div>",
        );
        let root = HtmlComputedStyle::root(&theme);
        let parent = html_node_visual_style(&doc.nodes[0], root, &theme);
        let child = html_node_visual_style(&doc.nodes[0].children[0], parent.computed, &theme);

        assert_color_near(parent.computed.color, 0, 0, 255, 255);
        assert_color_near(child.computed.color, 0, 0, 255, 255);
        assert!((child.computed.font_size - 24.0).abs() < 0.01);
    }

    #[test]
    fn html_render_style_overrides_link_and_mark_defaults() {
        let theme = Theme::default_theme();
        let link_doc = parse_html_document("<a style=\"color:red\">x</a>");
        let link_style =
            html_node_visual_style(&link_doc.nodes[0], HtmlComputedStyle::root(&theme), &theme);
        assert_color_near(link_style.computed.color, 255, 0, 0, 255);

        let mark_doc = parse_html_document("<mark style=\"background-color:#123\">x</mark>");
        let mark_style =
            html_node_visual_style(&mark_doc.nodes[0], HtmlComputedStyle::root(&theme), &theme);
        assert_color_near(mark_style.background.unwrap(), 0x11, 0x22, 0x33, 0xff);
    }

    #[test]
    fn html_render_style_does_not_inherit_background_color() {
        let theme = Theme::default_theme();
        let doc =
            parse_html_document("<div style=\"background-color:#112233\"><span>child</span></div>");
        let root = HtmlComputedStyle::root(&theme);
        let parent = html_node_visual_style(&doc.nodes[0], root, &theme);
        let child = html_node_visual_style(&doc.nodes[0].children[0], parent.computed, &theme);

        assert_color_near(parent.background.unwrap(), 0x11, 0x22, 0x33, 0xff);
        assert!(child.background.is_none());
    }
}
