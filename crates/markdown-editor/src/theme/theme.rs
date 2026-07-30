//! Theme data structures and defaults.
//!
//! The theme layer keeps visual tokens out of editor logic so rendering and
//! interaction code can depend on stable semantic names instead of hard-coded
//! values.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use gpui::{App, FontWeight, Global, Hsla, hsla, rgba};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::config::{
    VelotypeConfigDirs, merge_non_empty_json_values, object_without_empty_values,
    prune_empty_json_values, read_json_or_jsonc, sanitize_config_file_stem,
};
use crate::host_services::EditorHostTheme;

/// Serializable font weight that maps to GPUI's [`FontWeight`] constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontWeightDef {
    /// Thin font weight.
    Thin,
    /// Light font weight.
    Light,
    /// Normal font weight.
    Normal,
    /// Medium font weight.
    Medium,
    /// Semibold font weight.
    Semibold,
    /// Bold font weight.
    Bold,
    /// Extra-bold font weight.
    Extrabold,
    /// Black font weight.
    Black,
}

impl FontWeightDef {
    /// Converts the serialized theme value into GPUI's runtime font weight.
    pub fn to_font_weight(&self) -> FontWeight {
        match self {
            FontWeightDef::Thin => FontWeight::THIN,
            FontWeightDef::Light => FontWeight::LIGHT,
            FontWeightDef::Normal => FontWeight::NORMAL,
            FontWeightDef::Medium => FontWeight::MEDIUM,
            FontWeightDef::Semibold => FontWeight::SEMIBOLD,
            FontWeightDef::Bold => FontWeight::BOLD,
            FontWeightDef::Extrabold => FontWeight::EXTRA_BOLD,
            FontWeightDef::Black => FontWeight::BLACK,
        }
    }
}

/// All configurable colors for the editor UI.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeColors {
    /// Background of the editor scroll area (behind all blocks).
    pub editor_background: Hsla,
    /// Background of the focused raw block in source-editing mode.
    pub source_mode_block_bg: Hsla,
    /// Background used for visible Markdown comment blocks.
    pub comment_bg: Hsla,
    /// Default paragraph / body text colour.
    pub text_default: Hsla,
    /// Inline link text colour in rendered mode.
    pub text_link: Hsla,
    /// Placeholder text shown in empty focused blocks.
    pub text_placeholder: Hsla,
    /// H1 heading text colour.
    pub text_h1: Hsla,
    /// H2 heading text colour.
    pub text_h2: Hsla,
    /// H3 heading text colour.
    pub text_h3: Hsla,
    /// H4 heading text colour.
    pub text_h4: Hsla,
    /// H5 heading text colour.
    pub text_h5: Hsla,
    /// H6 heading text colour.
    pub text_h6: Hsla,
    /// H1 bottom-border colour.
    pub border_h1: Hsla,
    /// H2 bottom-border colour.
    pub border_h2: Hsla,
    /// Quote block text colour.
    pub text_quote: Hsla,
    /// Quote block left-border colour.
    pub border_quote: Hsla,
    /// Note callout background.
    pub callout_note_bg: Hsla,
    /// Note callout accent border/text colour.
    pub callout_note_border: Hsla,
    /// Tip callout background.
    pub callout_tip_bg: Hsla,
    /// Tip callout accent border/text colour.
    pub callout_tip_border: Hsla,
    /// Important callout background.
    pub callout_important_bg: Hsla,
    /// Important callout accent border/text colour.
    pub callout_important_border: Hsla,
    /// Warning callout background.
    pub callout_warning_bg: Hsla,
    /// Warning callout accent border/text colour.
    pub callout_warning_border: Hsla,
    /// Caution callout background.
    pub callout_caution_bg: Hsla,
    /// Caution callout accent border/text colour.
    pub callout_caution_border: Hsla,
    /// Background of footnote definition grouping shells.
    pub footnote_bg: Hsla,
    /// Border colour of footnote definition grouping shells.
    pub footnote_border: Hsla,
    /// Background of the footnote ordinal badge.
    pub footnote_badge_bg: Hsla,
    /// Text colour of the footnote ordinal badge.
    pub footnote_badge_text: Hsla,
    /// Back-reference colour inside footnote headers.
    pub footnote_backref: Hsla,
    /// Border colour of interactive task-list checkboxes.
    pub task_checkbox_border: Hsla,
    /// Background of unchecked task-list checkboxes.
    pub task_checkbox_bg: Hsla,
    /// Background of checked task-list checkboxes.
    pub task_checkbox_checked_bg: Hsla,
    /// Checkmark colour inside checked task-list checkboxes.
    pub task_checkbox_check: Hsla,
    /// Colour of the separator block line.
    pub separator_color: Hsla,
    /// Background of inline code and code-block quads.
    pub code_bg: Hsla,
    /// Text colour inside code blocks.
    pub code_text: Hsla,
    /// Background of the focused code-block language input.
    pub code_language_input_bg: Hsla,
    /// Border colour of the focused code-block language input.
    pub code_language_input_border: Hsla,
    /// Text colour of the focused code-block language input.
    pub code_language_input_text: Hsla,
    /// Placeholder colour of the focused code-block language input.
    pub code_language_input_placeholder: Hsla,
    /// Syntax colour for comments inside code blocks.
    pub code_syntax_comment: Hsla,
    /// Syntax colour for keywords inside code blocks.
    pub code_syntax_keyword: Hsla,
    /// Syntax colour for strings inside code blocks.
    pub code_syntax_string: Hsla,
    /// Syntax colour for numbers inside code blocks.
    pub code_syntax_number: Hsla,
    /// Syntax colour for types and modules inside code blocks.
    pub code_syntax_type: Hsla,
    /// Syntax colour for functions and constructors inside code blocks.
    pub code_syntax_function: Hsla,
    /// Syntax colour for constants inside code blocks.
    pub code_syntax_constant: Hsla,
    /// Syntax colour for variables and parameters inside code blocks.
    pub code_syntax_variable: Hsla,
    /// Syntax colour for properties and attributes inside code blocks.
    pub code_syntax_property: Hsla,
    /// Syntax colour for operators inside code blocks.
    pub code_syntax_operator: Hsla,
    /// Syntax colour for punctuation inside code blocks.
    pub code_syntax_punctuation: Hsla,
    /// Border colour of native table cells.
    pub table_border: Hsla,
    /// Background of native table header cells.
    pub table_header_bg: Hsla,
    /// Background of native table body cells.
    pub table_cell_bg: Hsla,
    /// Outline colour of the active native table cell.
    pub table_cell_active_outline: Hsla,
    /// Preview highlight colour for row/column table-axis selection bands.
    pub table_axis_preview_bg: Hsla,
    /// Selected highlight colour for row/column table-axis selection bands.
    pub table_axis_selected_bg: Hsla,
    /// Background of rendered-mode native table append controls.
    pub table_append_button_bg: Hsla,
    /// Hover background of rendered-mode native table append controls.
    pub table_append_button_hover: Hsla,
    /// Text colour of rendered-mode native table append controls.
    pub table_append_button_text: Hsla,
    /// Background of image placeholders in rendered mode.
    pub image_placeholder_bg: Hsla,
    /// Border colour of image placeholders in rendered mode.
    pub image_placeholder_border: Hsla,
    /// Text colour of image placeholders in rendered mode.
    pub image_placeholder_text: Hsla,
    /// Caption text colour shown below rendered images.
    pub image_caption_text: Hsla,
    /// Scrollbar thumb colour (auto-fading overlay).
    pub scrollbar_thumb: Hsla,
    /// Text-editing cursor (caret) colour.
    pub cursor: Hsla,
    /// Text-selection highlight colour.
    pub selection: Hsla,
    /// Semi-transparent backdrop behind the unsaved-changes dialog.
    pub dialog_backdrop: Hsla,
    /// Background of the unsaved-changes dialog.
    pub dialog_surface: Hsla,
    /// Border colour of the unsaved-changes dialog.
    pub dialog_border: Hsla,
    /// Title text colour in the unsaved-changes dialog.
    pub dialog_title: Hsla,
    /// Body text colour in the unsaved-changes dialog.
    pub dialog_body: Hsla,
    /// Muted / hint text colour in the unsaved-changes dialog.
    pub dialog_muted: Hsla,
    /// Primary (save-and-close) button background.
    pub dialog_primary_button_bg: Hsla,
    /// Primary button hover background.
    pub dialog_primary_button_hover: Hsla,
    /// Primary button text colour.
    pub dialog_primary_button_text: Hsla,
    /// Secondary (cancel) button background.
    pub dialog_secondary_button_bg: Hsla,
    /// Secondary button hover background.
    pub dialog_secondary_button_hover: Hsla,
    /// Secondary button text colour.
    pub dialog_secondary_button_text: Hsla,
    /// Danger (discard-and-close) button background.
    pub dialog_danger_button_bg: Hsla,
    /// Danger button hover background.
    pub dialog_danger_button_hover: Hsla,
    /// Danger button text colour.
    pub dialog_danger_button_text: Hsla,
    /// Background of the editor status bar.
    pub status_bar_background: Hsla,
    /// Primary text colour in the status bar.
    pub status_bar_text: Hsla,
    /// Dimmed/secondary text colour in the status bar.
    pub status_bar_text_dim: Hsla,
    /// Hover background for clickable status bar items.
    pub status_bar_button_hover: Hsla,
}

/// All configurable dimensions (paddings, gaps, sizes) for the editor UI.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeDimensions {
    /// Padding around the editor content area.
    pub editor_padding: f32,
    /// Vertical gap between adjacent blocks.
    pub block_gap: f32,
    /// Minimum height of every block.
    pub block_min_height: f32,
    /// Vertical padding inside each block.
    pub block_padding_y: f32,
    /// Horizontal padding inside each block.
    pub block_padding_x: f32,
    /// Extra horizontal indent per nesting level (list items).
    pub nested_block_indent: f32,
    /// Gap between list marker and its text content.
    pub list_marker_gap: f32,
    /// Minimum width of the bullet list marker column.
    pub list_marker_width: f32,
    /// Minimum width of the ordered-list marker column.
    pub ordered_list_marker_width: f32,
    /// Width and height of the interactive task-list checkbox.
    pub task_checkbox_size: f32,
    /// Corner radius of the task-list checkbox.
    pub task_checkbox_radius: f32,
    /// Border width of the task-list checkbox.
    pub task_checkbox_border_width: f32,
    /// Checkmark font size inside the task-list checkbox.
    pub task_checkbox_check_size: f32,
    /// Extra padding below H1 text.
    pub h1_padding_bottom: f32,
    /// Margin below the H1 bottom border.
    pub h1_margin_bottom: f32,
    /// Width of the text-editing cursor (caret).
    pub cursor_width: f32,
    /// Thickness of the underline decoration.
    pub underline_thickness: f32,
    /// H1 bottom-border thickness.
    pub h1_border_width: f32,
    /// Quote block left-border thickness.
    pub quote_border_width: f32,
    /// Extra left padding between quote border and text.
    pub quote_padding_left: f32,
    /// Horizontal padding inside editor-level callout shells.
    pub callout_padding_x: f32,
    /// Vertical padding inside editor-level callout shells.
    pub callout_padding_y: f32,
    /// Vertical gap between callout body rows.
    pub callout_body_gap: f32,
    /// Corner radius of editor-level callout shells.
    pub callout_radius: f32,
    /// Accent border width of editor-level callout shells.
    pub callout_border_width: f32,
    /// Gap between callout icon and header text.
    pub callout_header_gap: f32,
    /// Vertical margin between the callout header row and the first body row.
    pub callout_header_margin_bottom: f32,
    /// Horizontal padding inside footnote grouping shells.
    pub footnote_padding_x: f32,
    /// Vertical padding inside footnote grouping shells.
    pub footnote_padding_y: f32,
    /// Corner radius of footnote grouping shells.
    pub footnote_radius: f32,
    /// Horizontal padding inside the footnote ordinal badge.
    pub footnote_badge_padding_x: f32,
    /// Vertical padding inside the footnote ordinal badge.
    pub footnote_badge_padding_y: f32,
    /// Thickness of the separator block line.
    pub separator_thickness: f32,
    /// Extra horizontal inset applied to separator blocks.
    pub separator_inset_x: f32,
    /// Vertical margin around separator blocks.
    pub separator_margin_y: f32,
    /// Vertical padding inside a code block.
    pub code_block_padding_y: f32,
    /// Horizontal padding inside a code block.
    pub code_block_padding_x: f32,
    /// Horizontal padding around inline code background quads.
    pub code_bg_pad_x: f32,
    /// Vertical padding around inline code background quads.
    pub code_bg_pad_y: f32,
    /// Corner radius for inline code background quads.
    pub code_bg_radius: f32,
    /// Width of the code-block language input.
    pub code_language_input_width: f32,
    /// Text layout height inside the code-block language input.
    pub code_language_input_height: f32,
    /// Horizontal padding inside the code-block language input.
    pub code_language_input_padding_x: f32,
    /// Vertical padding inside the code-block language input.
    pub code_language_input_padding_y: f32,
    /// Corner radius of the code-block language input.
    pub code_language_input_radius: f32,
    /// Border width of the code-block language input.
    pub code_language_input_border_width: f32,
    /// Gap between code text and the language input.
    pub code_language_input_gap: f32,
    /// Horizontal padding inside native table cells.
    pub table_cell_padding_x: f32,
    /// Vertical padding inside native table cells.
    pub table_cell_padding_y: f32,
    /// Minimum height of native table cells.
    pub table_cell_min_height: f32,
    /// Width of the append-column control and height of the append-row control.
    pub table_append_button_extent: f32,
    /// Inset padding around rendered-mode native table append controls.
    pub table_append_button_inset: f32,
    /// Invisible activation overlap that keeps append controls easy to hover.
    pub table_append_activation_band: f32,
    /// Corner radius of rendered images and image placeholders.
    pub image_radius: f32,
    /// Maximum height of rendered root-paragraph images.
    pub image_root_max_height: f32,
    /// Maximum height of rendered table-cell images.
    pub image_cell_max_height: f32,
    /// Default placeholder height for rendered root-paragraph images.
    pub image_root_placeholder_height: f32,
    /// Default placeholder height for rendered table-cell images.
    pub image_cell_placeholder_height: f32,
    /// Vertical gap between a rendered image and its caption.
    pub image_caption_gap: f32,
    /// Width of the custom scrollbar thumb.
    pub scrollbar_width: f32,
    /// Distance of the scrollbar thumb from the right edge.
    pub scrollbar_right: f32,
    /// Viewport width at which the content column starts shrinking.
    pub centered_shrink_start: f32,
    /// Viewport width at which the content column reaches minimum ratio.
    pub centered_shrink_end: f32,
    /// Minimum content-column width as a fraction of available width.
    pub centered_min_ratio: f32,
    /// Width of the unsaved-changes dialog.
    pub dialog_width: f32,
    /// Padding inside the unsaved-changes dialog.
    pub dialog_padding: f32,
    /// Gap between dialog sections.
    pub dialog_gap: f32,
    /// Corner radius of the unsaved-changes dialog.
    pub dialog_radius: f32,
    /// Border width of the unsaved-changes dialog.
    pub dialog_border_width: f32,
    /// Height of dialog action buttons.
    pub dialog_button_height: f32,
    /// Gap between dialog action buttons.
    pub dialog_button_gap: f32,
    /// Horizontal padding inside dialog action buttons.
    pub dialog_button_padding_x: f32,
    /// Height reserved for the in-window fallback menu bar.
    pub menu_bar_height: f32,
    /// Horizontal padding inside the in-window fallback menu bar.
    pub menu_bar_padding_x: f32,
    /// Vertical padding inside the in-window fallback menu bar.
    pub menu_bar_padding_y: f32,
    /// Gap between top-level menu buttons.
    pub menu_bar_gap: f32,
    /// Minimum width of each top-level menu button.
    pub menu_bar_button_width: f32,
    /// Height of each top-level menu button.
    pub menu_bar_button_height: f32,
    /// Horizontal padding inside top-level menu buttons.
    pub menu_bar_button_padding_x: f32,
    /// Corner radius of top-level menu buttons.
    pub menu_bar_button_radius: f32,
    /// Text size used by menu labels.
    pub menu_text_size: f32,
    /// Top position of the in-window fallback floating menu panel.
    pub menu_panel_top: f32,
    /// Width of the in-window fallback floating menu panel.
    pub menu_panel_width: f32,
    /// Padding inside floating menu panels.
    pub menu_panel_padding: f32,
    /// Gap between items inside floating menu panels.
    pub menu_panel_gap: f32,
    /// Corner radius of floating menu panels.
    pub menu_panel_radius: f32,
    /// Height of each floating menu item.
    pub menu_item_height: f32,
    /// Horizontal padding inside floating menu items.
    pub menu_item_padding_x: f32,
    /// Corner radius of floating menu items.
    pub menu_item_radius: f32,
    /// Horizontal margin around menu separators.
    pub menu_separator_margin_x: f32,
    /// Vertical margin around menu separators.
    pub menu_separator_margin_y: f32,
    /// Height of menu separators.
    pub menu_separator_height: f32,
    /// Width of the root insert context menu panel.
    pub context_menu_panel_width: f32,
    /// Width of the insert-submenu panel.
    pub context_menu_submenu_width: f32,
    /// Horizontal gap between a context menu and its submenu.
    pub context_menu_submenu_gap: f32,
    /// Width of the table-axis context menu panel.
    pub context_menu_axis_panel_width: f32,
    /// Maximum width of the table-insert dialog.
    pub table_insert_dialog_width: f32,
    /// Gap between table-insert stepper label and controls.
    pub table_insert_stepper_gap: f32,
    /// Size of table-insert stepper buttons.
    pub table_insert_stepper_button_size: f32,
    /// Minimum width of the table-insert stepper value pill.
    pub table_insert_stepper_value_min_width: f32,
    /// Horizontal padding inside the table-insert stepper value pill.
    pub table_insert_stepper_value_padding_x: f32,
    /// Corner radius of table-insert stepper controls.
    pub table_insert_stepper_radius: f32,
    /// Left inset of the view-mode toggle.
    pub view_mode_toggle_left: f32,
    /// Bottom inset of the view-mode toggle.
    pub view_mode_toggle_bottom: f32,
    /// Horizontal padding inside the view-mode toggle.
    pub view_mode_toggle_padding_x: f32,
    /// Vertical padding inside the view-mode toggle.
    pub view_mode_toggle_padding_y: f32,
    /// Minimum width of the view-mode toggle.
    pub view_mode_toggle_min_width: f32,
    /// Corner radius of the view-mode toggle.
    pub view_mode_toggle_radius: f32,
    /// Border width of the view-mode toggle.
    pub view_mode_toggle_border_width: f32,
    /// Text size of the view-mode toggle.
    pub view_mode_toggle_text_size: f32,
    /// Height of the status bar.
    pub status_bar_height: f32,
    /// Horizontal padding inside the status bar.
    pub status_bar_padding_x: f32,
    /// Gap between items in the status bar.
    pub status_bar_item_gap: f32,
    /// Font size for status bar text.
    pub status_bar_text_size: f32,
}

/// All configurable typography settings (font sizes, weights, line heights).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeTypography {
    /// Default body text font size.
    pub text_size: f32,
    /// Default body text line height as a ratio of font size.
    pub text_line_height: f32,
    /// H1 heading font size.
    pub h1_size: f32,
    /// H1 heading font weight.
    pub h1_weight: FontWeightDef,
    /// H2 heading font size.
    pub h2_size: f32,
    /// H2 heading font weight.
    pub h2_weight: FontWeightDef,
    /// H3 heading font size.
    pub h3_size: f32,
    /// H3 heading font weight.
    pub h3_weight: FontWeightDef,
    /// H4 heading font size.
    pub h4_size: f32,
    /// H4 heading font weight.
    pub h4_weight: FontWeightDef,
    /// H5 heading font size.
    pub h5_size: f32,
    /// H5 heading font weight.
    pub h5_weight: FontWeightDef,
    /// H6 heading font size.
    pub h6_size: f32,
    /// H6 heading font weight.
    pub h6_weight: FontWeightDef,
    /// Code-block text font size.
    pub code_size: f32,
    /// Dialog title font size.
    pub dialog_title_size: f32,
    /// Dialog title font weight.
    pub dialog_title_weight: FontWeightDef,
    /// Dialog body font size.
    pub dialog_body_size: f32,
    /// Dialog body font weight.
    pub dialog_body_weight: FontWeightDef,
    /// Dialog button font size.
    pub dialog_button_size: f32,
    /// Dialog button font weight.
    pub dialog_button_weight: FontWeightDef,
}

/// Placeholder text shown in empty interactive elements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placeholders {
    /// Text shown in an empty focused block.
    pub empty_editing: String,
}

/// Deserialization adapter for `ThemeColors` with backward-compatible defaults.
#[derive(Deserialize)]
struct ThemeColorsDe {
    editor_background: Hsla,
    source_mode_block_bg: Option<Hsla>,
    block_focused_bg: Option<Hsla>,
    comment_bg: Option<Hsla>,
    text_default: Hsla,
    text_link: Option<Hsla>,
    text_placeholder: Hsla,
    text_h1: Hsla,
    text_h2: Hsla,
    text_h3: Hsla,
    text_h4: Hsla,
    text_h5: Hsla,
    text_h6: Hsla,
    border_h1: Hsla,
    border_h2: Option<Hsla>,
    text_quote: Hsla,
    border_quote: Hsla,
    callout_note_bg: Option<Hsla>,
    callout_note_border: Option<Hsla>,
    callout_tip_bg: Option<Hsla>,
    callout_tip_border: Option<Hsla>,
    callout_important_bg: Option<Hsla>,
    callout_important_border: Option<Hsla>,
    callout_warning_bg: Option<Hsla>,
    callout_warning_border: Option<Hsla>,
    callout_caution_bg: Option<Hsla>,
    callout_caution_border: Option<Hsla>,
    footnote_bg: Option<Hsla>,
    footnote_border: Option<Hsla>,
    footnote_badge_bg: Option<Hsla>,
    footnote_badge_text: Option<Hsla>,
    footnote_backref: Option<Hsla>,
    task_checkbox_border: Option<Hsla>,
    task_checkbox_bg: Option<Hsla>,
    task_checkbox_checked_bg: Option<Hsla>,
    task_checkbox_check: Option<Hsla>,
    separator_color: Option<Hsla>,
    code_bg: Option<Hsla>,
    code_text: Hsla,
    code_language_input_bg: Option<Hsla>,
    code_language_input_border: Option<Hsla>,
    code_language_input_text: Option<Hsla>,
    code_language_input_placeholder: Option<Hsla>,
    code_syntax_comment: Option<Hsla>,
    code_syntax_keyword: Option<Hsla>,
    code_syntax_string: Option<Hsla>,
    code_syntax_number: Option<Hsla>,
    code_syntax_type: Option<Hsla>,
    code_syntax_function: Option<Hsla>,
    code_syntax_constant: Option<Hsla>,
    code_syntax_variable: Option<Hsla>,
    code_syntax_property: Option<Hsla>,
    code_syntax_operator: Option<Hsla>,
    code_syntax_punctuation: Option<Hsla>,
    table_border: Option<Hsla>,
    table_header_bg: Option<Hsla>,
    table_cell_bg: Option<Hsla>,
    table_cell_active_outline: Option<Hsla>,
    table_axis_preview_bg: Option<Hsla>,
    table_axis_selected_bg: Option<Hsla>,
    table_append_button_bg: Option<Hsla>,
    table_append_button_hover: Option<Hsla>,
    table_append_button_text: Option<Hsla>,
    image_placeholder_bg: Option<Hsla>,
    image_placeholder_border: Option<Hsla>,
    image_placeholder_text: Option<Hsla>,
    image_caption_text: Option<Hsla>,
    scrollbar_thumb: Hsla,
    cursor: Hsla,
    selection: Hsla,
    dialog_backdrop: Hsla,
    dialog_surface: Hsla,
    dialog_border: Hsla,
    dialog_title: Hsla,
    dialog_body: Hsla,
    dialog_muted: Hsla,
    dialog_primary_button_bg: Hsla,
    dialog_primary_button_hover: Hsla,
    dialog_primary_button_text: Hsla,
    dialog_secondary_button_bg: Hsla,
    dialog_secondary_button_hover: Hsla,
    dialog_secondary_button_text: Hsla,
    dialog_danger_button_bg: Hsla,
    dialog_danger_button_hover: Hsla,
    dialog_danger_button_text: Hsla,
    status_bar_background: Option<Hsla>,
    status_bar_text: Option<Hsla>,
    status_bar_text_dim: Option<Hsla>,
    status_bar_button_hover: Option<Hsla>,
}

impl<'de> Deserialize<'de> for ThemeColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ThemeColorsDe::deserialize(deserializer)?;
        Ok(Self {
            editor_background: raw.editor_background,
            source_mode_block_bg: raw
                .source_mode_block_bg
                .or(raw.block_focused_bg)
                .unwrap_or_else(|| Hsla::from(rgba(0x313131ff))),
            comment_bg: raw
                .comment_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xfbbf2426))),
            text_default: raw.text_default,
            text_link: raw
                .text_link
                .unwrap_or_else(|| Hsla::from(rgba(0x60a5faff))),
            text_placeholder: raw.text_placeholder,
            text_h1: raw.text_h1,
            text_h2: raw.text_h2,
            text_h3: raw.text_h3,
            text_h4: raw.text_h4,
            text_h5: raw.text_h5,
            text_h6: raw.text_h6,
            border_h1: raw.border_h1,
            border_h2: raw
                .border_h2
                .unwrap_or_else(|| Hsla::from(rgba(0xe0e0e0cc))),
            text_quote: raw.text_quote,
            border_quote: raw.border_quote,
            callout_note_bg: raw
                .callout_note_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x94a3b81f))),
            callout_note_border: raw
                .callout_note_border
                .unwrap_or_else(|| Hsla::from(rgba(0x94a3b4ff))),
            callout_tip_bg: raw
                .callout_tip_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x1d4ed81f))),
            callout_tip_border: raw
                .callout_tip_border
                .unwrap_or_else(|| Hsla::from(rgba(0x60a5faff))),
            callout_important_bg: raw
                .callout_important_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xca8a041f))),
            callout_important_border: raw
                .callout_important_border
                .unwrap_or_else(|| Hsla::from(rgba(0xfbbf24ff))),
            callout_warning_bg: raw
                .callout_warning_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xfb71851f))),
            callout_warning_border: raw
                .callout_warning_border
                .unwrap_or_else(|| Hsla::from(rgba(0xfb7185ff))),
            callout_caution_bg: raw
                .callout_caution_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xdc26261f))),
            callout_caution_border: raw
                .callout_caution_border
                .unwrap_or_else(|| Hsla::from(rgba(0xf87171ff))),
            footnote_bg: raw
                .footnote_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x212124ff))),
            footnote_border: raw
                .footnote_border
                .unwrap_or_else(|| Hsla::from(rgba(0x71717a52))),
            footnote_badge_bg: raw
                .footnote_badge_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xa1a1aa24))),
            footnote_badge_text: raw
                .footnote_badge_text
                .unwrap_or_else(|| Hsla::from(rgba(0xd4d4d8cc))),
            footnote_backref: raw
                .footnote_backref
                .unwrap_or_else(|| Hsla::from(rgba(0xa1a1aaff))),
            task_checkbox_border: raw
                .task_checkbox_border
                .unwrap_or_else(|| Hsla::from(rgba(0x71717aff))),
            task_checkbox_bg: raw
                .task_checkbox_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x00000000))),
            task_checkbox_checked_bg: raw
                .task_checkbox_checked_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xf0efedff))),
            task_checkbox_check: raw
                .task_checkbox_check
                .unwrap_or_else(|| Hsla::from(rgba(0x18181bff))),
            separator_color: raw
                .separator_color
                .unwrap_or_else(|| Hsla::from(rgba(0x71717aff))),
            code_bg: raw.code_bg.unwrap_or_else(|| Hsla::from(rgba(0x111827ff))),
            code_text: raw.code_text,
            code_language_input_bg: raw
                .code_language_input_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x343941ff))),
            code_language_input_border: raw
                .code_language_input_border
                .unwrap_or_else(|| Hsla::from(rgba(0x4b5563cc))),
            code_language_input_text: raw
                .code_language_input_text
                .unwrap_or_else(|| Hsla::from(rgba(0xe5e7ebff))),
            code_language_input_placeholder: raw
                .code_language_input_placeholder
                .unwrap_or_else(|| Hsla::from(rgba(0x9ca3afcc))),
            code_syntax_comment: raw
                .code_syntax_comment
                .unwrap_or_else(|| Hsla::from(rgba(0x565f89ff))),
            code_syntax_keyword: raw
                .code_syntax_keyword
                .unwrap_or_else(|| Hsla::from(rgba(0xbb9af7ff))),
            code_syntax_string: raw
                .code_syntax_string
                .unwrap_or_else(|| Hsla::from(rgba(0x9ece6aff))),
            code_syntax_number: raw
                .code_syntax_number
                .unwrap_or_else(|| Hsla::from(rgba(0xff9e64ff))),
            code_syntax_type: raw
                .code_syntax_type
                .unwrap_or_else(|| Hsla::from(rgba(0x2ac3deff))),
            code_syntax_function: raw
                .code_syntax_function
                .unwrap_or_else(|| Hsla::from(rgba(0x7aa2f7ff))),
            code_syntax_constant: raw
                .code_syntax_constant
                .unwrap_or_else(|| Hsla::from(rgba(0xffd166ff))),
            code_syntax_variable: raw
                .code_syntax_variable
                .unwrap_or_else(|| Hsla::from(rgba(0xe5e9f0ff))),
            code_syntax_property: raw
                .code_syntax_property
                .unwrap_or_else(|| Hsla::from(rgba(0x7dcfffcc))),
            code_syntax_operator: raw
                .code_syntax_operator
                .unwrap_or_else(|| Hsla::from(rgba(0x89ddffff))),
            code_syntax_punctuation: raw
                .code_syntax_punctuation
                .unwrap_or_else(|| Hsla::from(rgba(0x9aa5ceff))),
            table_border: raw
                .table_border
                .unwrap_or_else(|| Hsla::from(rgba(0x3f3f46ff))),
            table_header_bg: raw
                .table_header_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x232326ff))),
            table_cell_bg: raw
                .table_cell_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x1d1d20ff))),
            table_cell_active_outline: raw
                .table_cell_active_outline
                .unwrap_or_else(|| Hsla::from(rgba(0x60a5faff))),
            table_axis_preview_bg: raw
                .table_axis_preview_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xf4f4f51a))),
            table_axis_selected_bg: raw
                .table_axis_selected_bg
                .unwrap_or_else(|| Hsla::from(rgba(0xf4f4f533))),
            table_append_button_bg: raw
                .table_append_button_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x27272aff))),
            table_append_button_hover: raw
                .table_append_button_hover
                .unwrap_or_else(|| Hsla::from(rgba(0x3f3f46ff))),
            table_append_button_text: raw
                .table_append_button_text
                .unwrap_or_else(|| Hsla::from(rgba(0xf4f4f5ff))),
            image_placeholder_bg: raw
                .image_placeholder_bg
                .unwrap_or_else(|| Hsla::from(rgba(0x202024ff))),
            image_placeholder_border: raw
                .image_placeholder_border
                .unwrap_or_else(|| Hsla::from(rgba(0x52525bff))),
            image_placeholder_text: raw
                .image_placeholder_text
                .unwrap_or_else(|| Hsla::from(rgba(0xd4d4d8ff))),
            image_caption_text: raw
                .image_caption_text
                .unwrap_or_else(|| Hsla::from(rgba(0xa1a1aaff))),
            scrollbar_thumb: raw.scrollbar_thumb,
            cursor: raw.cursor,
            selection: raw.selection,
            dialog_backdrop: raw.dialog_backdrop,
            dialog_surface: raw.dialog_surface,
            dialog_border: raw.dialog_border,
            dialog_title: raw.dialog_title,
            dialog_body: raw.dialog_body,
            dialog_muted: raw.dialog_muted,
            dialog_primary_button_bg: raw.dialog_primary_button_bg,
            dialog_primary_button_hover: raw.dialog_primary_button_hover,
            dialog_primary_button_text: raw.dialog_primary_button_text,
            dialog_secondary_button_bg: raw.dialog_secondary_button_bg,
            dialog_secondary_button_hover: raw.dialog_secondary_button_hover,
            dialog_secondary_button_text: raw.dialog_secondary_button_text,
            dialog_danger_button_bg: raw.dialog_danger_button_bg,
            dialog_danger_button_hover: raw.dialog_danger_button_hover,
            dialog_danger_button_text: raw.dialog_danger_button_text,
            status_bar_background: raw
                .status_bar_background
                .unwrap_or_else(|| Hsla::from(rgba(0x1c1c1fff))),
            status_bar_text: raw
                .status_bar_text
                .unwrap_or_else(|| Hsla::from(rgba(0xd4d4d8cc))),
            status_bar_text_dim: raw
                .status_bar_text_dim
                .unwrap_or_else(|| Hsla::from(rgba(0x71717aff))),
            status_bar_button_hover: raw
                .status_bar_button_hover
                .unwrap_or_else(|| Hsla::from(rgba(0x3f3f46ff))),
        })
    }
}

/// Deserialization adapter for `ThemeDimensions` with backward-compatible defaults.
#[derive(Deserialize)]
struct ThemeDimensionsDe {
    editor_padding: f32,
    block_gap: f32,
    block_min_height: f32,
    block_padding_y: f32,
    block_padding_x: f32,
    nested_block_indent: f32,
    list_marker_gap: f32,
    list_marker_width: f32,
    ordered_list_marker_width: f32,
    task_checkbox_size: Option<f32>,
    task_checkbox_radius: Option<f32>,
    task_checkbox_border_width: Option<f32>,
    task_checkbox_check_size: Option<f32>,
    h1_padding_bottom: f32,
    h1_margin_bottom: f32,
    cursor_width: f32,
    underline_thickness: f32,
    h1_border_width: f32,
    quote_border_width: f32,
    quote_padding_left: f32,
    callout_padding_x: Option<f32>,
    callout_padding_y: Option<f32>,
    callout_body_gap: Option<f32>,
    callout_radius: Option<f32>,
    callout_border_width: Option<f32>,
    callout_header_gap: Option<f32>,
    callout_header_margin_bottom: Option<f32>,
    footnote_padding_x: Option<f32>,
    footnote_padding_y: Option<f32>,
    footnote_radius: Option<f32>,
    footnote_badge_padding_x: Option<f32>,
    footnote_badge_padding_y: Option<f32>,
    separator_thickness: Option<f32>,
    separator_inset_x: Option<f32>,
    separator_margin_y: Option<f32>,
    code_block_padding_y: f32,
    code_block_padding_x: f32,
    code_bg_pad_x: f32,
    code_bg_pad_y: f32,
    code_bg_radius: f32,
    code_language_input_width: Option<f32>,
    code_language_input_height: Option<f32>,
    code_language_input_padding_x: Option<f32>,
    code_language_input_padding_y: Option<f32>,
    code_language_input_radius: Option<f32>,
    code_language_input_border_width: Option<f32>,
    code_language_input_gap: Option<f32>,
    table_cell_padding_x: Option<f32>,
    table_cell_padding_y: Option<f32>,
    table_cell_min_height: Option<f32>,
    table_append_button_extent: Option<f32>,
    table_append_button_inset: Option<f32>,
    table_append_activation_band: Option<f32>,
    image_radius: Option<f32>,
    image_root_max_height: Option<f32>,
    image_cell_max_height: Option<f32>,
    image_root_placeholder_height: Option<f32>,
    image_cell_placeholder_height: Option<f32>,
    image_caption_gap: Option<f32>,
    scrollbar_width: f32,
    scrollbar_right: f32,
    centered_shrink_start: f32,
    centered_shrink_end: f32,
    centered_min_ratio: f32,
    dialog_width: f32,
    dialog_padding: f32,
    dialog_gap: f32,
    dialog_radius: f32,
    dialog_border_width: f32,
    dialog_button_height: f32,
    dialog_button_gap: f32,
    dialog_button_padding_x: f32,
    menu_bar_height: Option<f32>,
    menu_bar_padding_x: Option<f32>,
    menu_bar_padding_y: Option<f32>,
    menu_bar_gap: Option<f32>,
    menu_bar_button_width: Option<f32>,
    menu_bar_button_height: Option<f32>,
    menu_bar_button_padding_x: Option<f32>,
    menu_bar_button_radius: Option<f32>,
    menu_text_size: Option<f32>,
    menu_panel_top: Option<f32>,
    menu_panel_width: Option<f32>,
    menu_panel_padding: Option<f32>,
    menu_panel_gap: Option<f32>,
    menu_panel_radius: Option<f32>,
    menu_item_height: Option<f32>,
    menu_item_padding_x: Option<f32>,
    menu_item_radius: Option<f32>,
    menu_separator_margin_x: Option<f32>,
    menu_separator_margin_y: Option<f32>,
    menu_separator_height: Option<f32>,
    context_menu_panel_width: Option<f32>,
    context_menu_submenu_width: Option<f32>,
    context_menu_submenu_gap: Option<f32>,
    context_menu_axis_panel_width: Option<f32>,
    table_insert_dialog_width: Option<f32>,
    table_insert_stepper_gap: Option<f32>,
    table_insert_stepper_button_size: Option<f32>,
    table_insert_stepper_value_min_width: Option<f32>,
    table_insert_stepper_value_padding_x: Option<f32>,
    table_insert_stepper_radius: Option<f32>,
    view_mode_toggle_left: Option<f32>,
    view_mode_toggle_bottom: Option<f32>,
    view_mode_toggle_padding_x: Option<f32>,
    view_mode_toggle_padding_y: Option<f32>,
    view_mode_toggle_min_width: Option<f32>,
    view_mode_toggle_radius: Option<f32>,
    view_mode_toggle_border_width: Option<f32>,
    view_mode_toggle_text_size: Option<f32>,
    status_bar_height: Option<f32>,
    status_bar_padding_x: Option<f32>,
    status_bar_item_gap: Option<f32>,
    status_bar_text_size: Option<f32>,
}

impl<'de> Deserialize<'de> for ThemeDimensions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ThemeDimensionsDe::deserialize(deserializer)?;
        Ok(Self {
            editor_padding: raw.editor_padding,
            block_gap: raw.block_gap,
            block_min_height: raw.block_min_height,
            block_padding_y: raw.block_padding_y,
            block_padding_x: raw.block_padding_x,
            nested_block_indent: raw.nested_block_indent,
            list_marker_gap: raw.list_marker_gap,
            list_marker_width: raw.list_marker_width,
            ordered_list_marker_width: raw.ordered_list_marker_width,
            task_checkbox_size: raw.task_checkbox_size.unwrap_or(14.0),
            task_checkbox_radius: raw.task_checkbox_radius.unwrap_or(4.0),
            task_checkbox_border_width: raw.task_checkbox_border_width.unwrap_or(1.0),
            task_checkbox_check_size: raw.task_checkbox_check_size.unwrap_or(10.0),
            h1_padding_bottom: raw.h1_padding_bottom,
            h1_margin_bottom: raw.h1_margin_bottom,
            cursor_width: raw.cursor_width,
            underline_thickness: raw.underline_thickness,
            h1_border_width: raw.h1_border_width,
            quote_border_width: raw.quote_border_width,
            quote_padding_left: raw.quote_padding_left,
            callout_padding_x: raw.callout_padding_x.unwrap_or(14.0),
            callout_padding_y: raw.callout_padding_y.unwrap_or(10.0),
            callout_body_gap: raw.callout_body_gap.unwrap_or(8.0),
            callout_radius: raw.callout_radius.unwrap_or(10.0),
            callout_border_width: raw.callout_border_width.unwrap_or(4.0),
            callout_header_gap: raw.callout_header_gap.unwrap_or(6.0),
            callout_header_margin_bottom: raw.callout_header_margin_bottom.unwrap_or(6.0),
            footnote_padding_x: raw.footnote_padding_x.unwrap_or(10.0),
            footnote_padding_y: raw.footnote_padding_y.unwrap_or(6.0),
            footnote_radius: raw.footnote_radius.unwrap_or(6.0),
            footnote_badge_padding_x: raw.footnote_badge_padding_x.unwrap_or(4.0),
            footnote_badge_padding_y: raw.footnote_badge_padding_y.unwrap_or(1.0),
            separator_thickness: raw.separator_thickness.unwrap_or(1.0),
            separator_inset_x: raw.separator_inset_x.unwrap_or(40.0),
            separator_margin_y: raw.separator_margin_y.unwrap_or(10.0),
            code_block_padding_y: raw.code_block_padding_y,
            code_block_padding_x: raw.code_block_padding_x,
            code_bg_pad_x: raw.code_bg_pad_x,
            code_bg_pad_y: raw.code_bg_pad_y,
            code_bg_radius: raw.code_bg_radius,
            code_language_input_width: raw.code_language_input_width.unwrap_or(156.0),
            code_language_input_height: raw.code_language_input_height.unwrap_or(18.0),
            code_language_input_padding_x: raw.code_language_input_padding_x.unwrap_or(8.0),
            code_language_input_padding_y: raw.code_language_input_padding_y.unwrap_or(3.0),
            code_language_input_radius: raw.code_language_input_radius.unwrap_or(6.0),
            code_language_input_border_width: raw.code_language_input_border_width.unwrap_or(1.0),
            code_language_input_gap: raw.code_language_input_gap.unwrap_or(8.0),
            table_cell_padding_x: raw.table_cell_padding_x.unwrap_or(10.0),
            table_cell_padding_y: raw.table_cell_padding_y.unwrap_or(8.0),
            table_cell_min_height: raw.table_cell_min_height.unwrap_or(42.0),
            table_append_button_extent: raw.table_append_button_extent.unwrap_or(16.0),
            table_append_button_inset: raw.table_append_button_inset.unwrap_or(8.0),
            table_append_activation_band: raw.table_append_activation_band.unwrap_or(18.0),
            image_radius: raw.image_radius.unwrap_or(12.0),
            image_root_max_height: raw.image_root_max_height.unwrap_or(420.0),
            image_cell_max_height: raw.image_cell_max_height.unwrap_or(180.0),
            image_root_placeholder_height: raw.image_root_placeholder_height.unwrap_or(260.0),
            image_cell_placeholder_height: raw.image_cell_placeholder_height.unwrap_or(120.0),
            image_caption_gap: raw.image_caption_gap.unwrap_or(8.0),
            scrollbar_width: raw.scrollbar_width,
            scrollbar_right: raw.scrollbar_right,
            centered_shrink_start: raw.centered_shrink_start,
            centered_shrink_end: raw.centered_shrink_end,
            centered_min_ratio: raw.centered_min_ratio,
            dialog_width: raw.dialog_width,
            dialog_padding: raw.dialog_padding,
            dialog_gap: raw.dialog_gap,
            dialog_radius: raw.dialog_radius,
            dialog_border_width: raw.dialog_border_width,
            dialog_button_height: raw.dialog_button_height,
            dialog_button_gap: raw.dialog_button_gap,
            dialog_button_padding_x: raw.dialog_button_padding_x,
            menu_bar_height: raw.menu_bar_height.unwrap_or(32.0),
            menu_bar_padding_x: raw.menu_bar_padding_x.unwrap_or(10.0),
            menu_bar_padding_y: raw.menu_bar_padding_y.unwrap_or(4.0),
            menu_bar_gap: raw.menu_bar_gap.unwrap_or(2.0),
            menu_bar_button_width: raw.menu_bar_button_width.unwrap_or(48.0),
            menu_bar_button_height: raw.menu_bar_button_height.unwrap_or(24.0),
            menu_bar_button_padding_x: raw.menu_bar_button_padding_x.unwrap_or(8.0),
            menu_bar_button_radius: raw.menu_bar_button_radius.unwrap_or(5.0),
            menu_text_size: raw.menu_text_size.unwrap_or(12.0),
            menu_panel_top: raw.menu_panel_top.unwrap_or(30.0),
            menu_panel_width: raw.menu_panel_width.unwrap_or(180.0),
            menu_panel_padding: raw.menu_panel_padding.unwrap_or(4.0),
            menu_panel_gap: raw.menu_panel_gap.unwrap_or(1.0),
            menu_panel_radius: raw.menu_panel_radius.unwrap_or(8.0),
            menu_item_height: raw.menu_item_height.unwrap_or(28.0),
            menu_item_padding_x: raw.menu_item_padding_x.unwrap_or(8.0),
            menu_item_radius: raw.menu_item_radius.unwrap_or(5.0),
            menu_separator_margin_x: raw.menu_separator_margin_x.unwrap_or(6.0),
            menu_separator_margin_y: raw.menu_separator_margin_y.unwrap_or(3.0),
            menu_separator_height: raw.menu_separator_height.unwrap_or(1.0),
            context_menu_panel_width: raw.context_menu_panel_width.unwrap_or(132.0),
            context_menu_submenu_width: raw.context_menu_submenu_width.unwrap_or(148.0),
            context_menu_submenu_gap: raw.context_menu_submenu_gap.unwrap_or(2.0),
            context_menu_axis_panel_width: raw.context_menu_axis_panel_width.unwrap_or(164.0),
            table_insert_dialog_width: raw.table_insert_dialog_width.unwrap_or(380.0),
            table_insert_stepper_gap: raw.table_insert_stepper_gap.unwrap_or(8.0),
            table_insert_stepper_button_size: raw.table_insert_stepper_button_size.unwrap_or(32.0),
            table_insert_stepper_value_min_width: raw
                .table_insert_stepper_value_min_width
                .unwrap_or(56.0),
            table_insert_stepper_value_padding_x: raw
                .table_insert_stepper_value_padding_x
                .unwrap_or(10.0),
            table_insert_stepper_radius: raw.table_insert_stepper_radius.unwrap_or(8.0),
            view_mode_toggle_left: raw.view_mode_toggle_left.unwrap_or(12.0),
            view_mode_toggle_bottom: raw.view_mode_toggle_bottom.unwrap_or(12.0),
            view_mode_toggle_padding_x: raw.view_mode_toggle_padding_x.unwrap_or(8.0),
            view_mode_toggle_padding_y: raw.view_mode_toggle_padding_y.unwrap_or(4.0),
            view_mode_toggle_min_width: raw.view_mode_toggle_min_width.unwrap_or(88.0),
            view_mode_toggle_radius: raw.view_mode_toggle_radius.unwrap_or(999.0),
            view_mode_toggle_border_width: raw.view_mode_toggle_border_width.unwrap_or(1.0),
            view_mode_toggle_text_size: raw.view_mode_toggle_text_size.unwrap_or(11.0),
            status_bar_height: raw.status_bar_height.unwrap_or(28.0),
            status_bar_padding_x: raw.status_bar_padding_x.unwrap_or(12.0),
            status_bar_item_gap: raw.status_bar_item_gap.unwrap_or(12.0),
            status_bar_text_size: raw.status_bar_text_size.unwrap_or(11.0),
        })
    }
}

/// Top-level theme combining colors, dimensions, typography and placeholders.
///
/// Can be deserialized from JSON, allowing users to ship custom theme files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
    pub dimensions: ThemeDimensions,
    pub typography: ThemeTypography,
    pub placeholders: Placeholders,
}

fn host_surface(background: Hsla, amount: f32) -> Hsla {
    let direction = if background.l < 0.5 { 1.0 } else { -1.0 };
    Hsla {
        l: (background.l + direction * amount).clamp(0.0, 1.0),
        ..background
    }
}

fn host_translucent(color: Hsla, opacity: f32) -> Hsla {
    Hsla {
        a: (color.a * opacity).clamp(0.0, 1.0),
        ..color
    }
}

#[allow(unused)]
impl Theme {
    /// Applies the compact palette exposed by an embedding application to the
    /// editor's semantic color tokens while preserving Velotype typography,
    /// dimensions, placeholders, and syntax-highlight categories.
    ///
    /// The host deliberately supplies only five stable colors. Derived
    /// surfaces and translucent interaction states are calculated here so the
    /// rest of the editor never falls back to Velotype's built-in black/gray
    /// chrome when embedded in a differently themed application.
    pub(crate) fn with_host_palette(base: &Theme, host: &EditorHostTheme) -> Self {
        let mut theme = base.clone();
        let colors = &mut theme.colors;
        let background = host.background;
        let foreground = host.foreground;
        let border = host.border;
        let muted = host.muted;
        let accent = host.accent;

        let surface_low = host_surface(background, 0.025);
        let surface = host_surface(background, 0.05);
        let surface_high = host_surface(background, 0.085);
        let accent_soft = host_translucent(accent, 0.14);
        let accent_selected = host_translucent(accent, 0.26);
        let primary_text = if accent.l >= 0.58 {
            background
        } else {
            foreground
        };

        colors.editor_background = background;
        colors.source_mode_block_bg = surface;
        colors.comment_bg = accent_soft;

        colors.text_default = foreground;
        colors.text_link = accent;
        colors.text_placeholder = muted;
        colors.text_h1 = foreground;
        colors.text_h2 = foreground;
        colors.text_h3 = foreground;
        colors.text_h4 = foreground;
        colors.text_h5 = foreground;
        colors.text_h6 = foreground;
        colors.border_h1 = border;
        colors.border_h2 = border;
        colors.text_quote = muted;
        colors.border_quote = border;

        colors.callout_note_bg = accent_soft;
        colors.callout_note_border = accent;
        colors.callout_tip_bg = accent_soft;
        colors.callout_tip_border = accent;
        colors.callout_important_bg = accent_soft;
        colors.callout_important_border = accent;

        colors.footnote_bg = surface_low;
        colors.footnote_border = border;
        colors.footnote_badge_bg = surface_high;
        colors.footnote_badge_text = foreground;
        colors.footnote_backref = accent;

        colors.task_checkbox_border = border;
        colors.task_checkbox_bg = background;
        colors.task_checkbox_checked_bg = accent;
        colors.task_checkbox_check = primary_text;
        colors.separator_color = border;

        colors.code_bg = surface_low;
        colors.code_text = foreground;
        colors.code_language_input_bg = surface_high;
        colors.code_language_input_border = border;
        colors.code_language_input_text = foreground;
        colors.code_language_input_placeholder = muted;

        colors.table_border = border;
        colors.table_header_bg = surface;
        colors.table_cell_bg = surface_low;
        colors.table_cell_active_outline = accent;
        colors.table_axis_preview_bg = accent_soft;
        colors.table_axis_selected_bg = accent_selected;
        colors.table_append_button_bg = surface;
        colors.table_append_button_hover = surface_high;
        colors.table_append_button_text = muted;

        colors.image_placeholder_bg = surface_low;
        colors.image_placeholder_border = border;
        colors.image_placeholder_text = muted;
        colors.image_caption_text = muted;
        colors.scrollbar_thumb = host_translucent(muted, 0.78);
        colors.cursor = accent;
        colors.selection = accent_selected;

        colors.dialog_backdrop = host_translucent(background, 0.78);
        colors.dialog_surface = surface;
        colors.dialog_border = border;
        colors.dialog_title = foreground;
        colors.dialog_body = foreground;
        colors.dialog_muted = muted;
        colors.dialog_primary_button_bg = accent;
        colors.dialog_primary_button_hover = host_surface(accent, 0.07);
        colors.dialog_primary_button_text = primary_text;
        colors.dialog_secondary_button_bg = surface_high;
        colors.dialog_secondary_button_hover = host_surface(background, 0.12);
        colors.dialog_secondary_button_text = foreground;

        colors.status_bar_background = surface_low;
        colors.status_bar_text = foreground;
        colors.status_bar_text_dim = muted;
        colors.status_bar_button_hover = surface_high;

        theme
    }

    /// Returns the built-in fallback theme used when no custom theme is loaded.
    pub fn default_theme() -> Self {
        Self {
            name: "Velotype".into(),
            colors: ThemeColors {
                editor_background: Hsla::from(rgba(0x191919ff)),
                source_mode_block_bg: Hsla::from(rgba(0x313131ff)),
                comment_bg: Hsla::from(rgba(0xfbbf2426)),
                text_default: Hsla::from(rgba(0xf0efedff)),
                text_link: Hsla::from(rgba(0x60a5faff)),
                text_placeholder: hsla(0., 0., 0.6, 1.0),
                text_h1: Hsla::from(rgba(0xf0efedff)),
                text_h2: Hsla::from(rgba(0xf0efedff)),
                text_h3: Hsla::from(rgba(0xf0efedff)),
                text_h4: Hsla::from(rgba(0xf0efedff)),
                text_h5: Hsla::from(rgba(0xf0efedff)),
                text_h6: Hsla::from(rgba(0xf0efedff)),
                border_h1: Hsla::from(rgba(0xe0e0e0ff)),
                border_h2: Hsla::from(rgba(0xe0e0e0cc)),
                text_quote: Hsla::from(rgba(0xd1d5dbff)),
                border_quote: Hsla::from(rgba(0x6b7280ff)),
                callout_note_bg: Hsla::from(rgba(0x94a3b81f)),
                callout_note_border: Hsla::from(rgba(0x94a3b4ff)),
                callout_tip_bg: Hsla::from(rgba(0x1d4ed81f)),
                callout_tip_border: Hsla::from(rgba(0x60a5faff)),
                callout_important_bg: Hsla::from(rgba(0xa78bfa1f)),
                callout_important_border: Hsla::from(rgba(0xa78bfaff)),
                callout_warning_bg: Hsla::from(rgba(0xfb71851f)),
                callout_warning_border: Hsla::from(rgba(0xfb7185ff)),
                callout_caution_bg: Hsla::from(rgba(0xdc26261f)),
                callout_caution_border: Hsla::from(rgba(0xf87171ff)),
                footnote_bg: Hsla::from(rgba(0x212124ff)),
                footnote_border: Hsla::from(rgba(0x71717a52)),
                footnote_badge_bg: Hsla::from(rgba(0xa1a1aa24)),
                footnote_badge_text: Hsla::from(rgba(0xd4d4d8cc)),
                footnote_backref: Hsla::from(rgba(0xa1a1aaff)),
                task_checkbox_border: Hsla::from(rgba(0x71717aff)),
                task_checkbox_bg: Hsla::from(rgba(0x00000000)),
                task_checkbox_checked_bg: Hsla::from(rgba(0xf0efedff)),
                task_checkbox_check: Hsla::from(rgba(0x18181bff)),
                separator_color: Hsla::from(rgba(0x71717aff)),
                code_bg: Hsla::from(rgba(0x23272eff)),
                code_text: Hsla::from(rgba(0xe5e7ebff)),
                code_language_input_bg: Hsla::from(rgba(0x343941ff)),
                code_language_input_border: Hsla::from(rgba(0x4b5563cc)),
                code_language_input_text: Hsla::from(rgba(0xe5e7ebff)),
                code_language_input_placeholder: Hsla::from(rgba(0x9ca3afcc)),
                code_syntax_comment: Hsla::from(rgba(0x565f89ff)),
                code_syntax_keyword: Hsla::from(rgba(0xbb9af7ff)),
                code_syntax_string: Hsla::from(rgba(0x9ece6aff)),
                code_syntax_number: Hsla::from(rgba(0xff9e64ff)),
                code_syntax_type: Hsla::from(rgba(0x2ac3deff)),
                code_syntax_function: Hsla::from(rgba(0x7aa2f7ff)),
                code_syntax_constant: Hsla::from(rgba(0xffd166ff)),
                code_syntax_variable: Hsla::from(rgba(0xe5e9f0ff)),
                code_syntax_property: Hsla::from(rgba(0x7dcfffcc)),
                code_syntax_operator: Hsla::from(rgba(0x89ddffff)),
                code_syntax_punctuation: Hsla::from(rgba(0x9aa5ceff)),
                table_border: Hsla::from(rgba(0x3f3f46ff)),
                table_header_bg: Hsla::from(rgba(0x232326ff)),
                table_cell_bg: Hsla::from(rgba(0x1d1d20ff)),
                table_cell_active_outline: Hsla::from(rgba(0x60a5faff)),
                table_axis_preview_bg: Hsla::from(rgba(0xf4f4f51a)),
                table_axis_selected_bg: Hsla::from(rgba(0xf4f4f533)),
                table_append_button_bg: Hsla::from(rgba(0x27272aff)),
                table_append_button_hover: Hsla::from(rgba(0x3f3f46ff)),
                table_append_button_text: Hsla::from(rgba(0xf4f4f5ff)),
                image_placeholder_bg: Hsla::from(rgba(0x202024ff)),
                image_placeholder_border: Hsla::from(rgba(0x52525bff)),
                image_placeholder_text: Hsla::from(rgba(0xd4d4d8ff)),
                image_caption_text: Hsla::from(rgba(0xa1a1aaff)),
                scrollbar_thumb: Hsla::from(rgba(0xd1d5dbd8)),
                cursor: Hsla::from(rgba(0xf0efedff)),
                selection: Hsla::from(rgba(0x1c3651ff)),
                dialog_backdrop: Hsla::from(rgba(0x09090bcc)),
                dialog_surface: Hsla::from(rgba(0x18181bff)),
                dialog_border: Hsla::from(rgba(0x27272aff)),
                dialog_title: Hsla::from(rgba(0xf4f4f5ff)),
                dialog_body: Hsla::from(rgba(0xd4d4d8ff)),
                dialog_muted: Hsla::from(rgba(0xa1a1aaff)),
                dialog_primary_button_bg: Hsla::from(rgba(0xf4f4f5ff)),
                dialog_primary_button_hover: Hsla::from(rgba(0xe4e4e7ff)),
                dialog_primary_button_text: Hsla::from(rgba(0x18181bff)),
                dialog_secondary_button_bg: Hsla::from(rgba(0x27272aff)),
                dialog_secondary_button_hover: Hsla::from(rgba(0x3f3f46ff)),
                dialog_secondary_button_text: Hsla::from(rgba(0xf4f4f5ff)),
                // Doubles as the destructive menu-item text color (e.g. Delete
                // Row/Column), so it must stay legible on the dark menu surface
                // rather than the muted red used previously.
                dialog_danger_button_bg: Hsla::from(rgba(0xef4444ff)),
                dialog_danger_button_hover: Hsla::from(rgba(0xdc2626ff)),
                dialog_danger_button_text: Hsla::from(rgba(0xfef2f2ff)),
                status_bar_background: Hsla::from(rgba(0x1c1c1fff)),
                status_bar_text: Hsla::from(rgba(0xd4d4d8cc)),
                status_bar_text_dim: Hsla::from(rgba(0x71717aff)),
                status_bar_button_hover: Hsla::from(rgba(0x3f3f46ff)),
            },
            dimensions: ThemeDimensions {
                editor_padding: 24.0,
                block_gap: 6.0,
                block_min_height: 28.0,
                block_padding_y: 4.0,
                block_padding_x: 12.0,
                nested_block_indent: 20.0,
                list_marker_gap: 8.0,
                list_marker_width: 12.0,
                ordered_list_marker_width: 20.0,
                task_checkbox_size: 14.0,
                task_checkbox_radius: 4.0,
                task_checkbox_border_width: 1.0,
                task_checkbox_check_size: 10.0,
                h1_padding_bottom: 4.0,
                h1_margin_bottom: 4.0,
                cursor_width: 2.0,
                underline_thickness: 1.0,
                h1_border_width: 1.0,
                quote_border_width: 3.0,
                quote_padding_left: 12.0,
                callout_padding_x: 14.0,
                callout_padding_y: 10.0,
                callout_body_gap: 8.0,
                callout_radius: 10.0,
                callout_border_width: 4.0,
                callout_header_gap: 6.0,
                callout_header_margin_bottom: 6.0,
                footnote_padding_x: 10.0,
                footnote_padding_y: 6.0,
                footnote_radius: 6.0,
                footnote_badge_padding_x: 4.0,
                footnote_badge_padding_y: 1.0,
                separator_thickness: 1.0,
                separator_inset_x: 40.0,
                separator_margin_y: 10.0,
                code_block_padding_y: 8.0,
                code_block_padding_x: 12.0,
                code_bg_pad_x: 3.0,
                code_bg_pad_y: 1.0,
                code_bg_radius: 4.0,
                code_language_input_width: 156.0,
                code_language_input_height: 18.0,
                code_language_input_padding_x: 8.0,
                code_language_input_padding_y: 3.0,
                code_language_input_radius: 6.0,
                code_language_input_border_width: 1.0,
                code_language_input_gap: 8.0,
                table_cell_padding_x: 10.0,
                table_cell_padding_y: 8.0,
                table_cell_min_height: 42.0,
                table_append_button_extent: 16.0,
                table_append_button_inset: 8.0,
                table_append_activation_band: 18.0,
                image_radius: 12.0,
                image_root_max_height: 420.0,
                image_cell_max_height: 180.0,
                image_root_placeholder_height: 260.0,
                image_cell_placeholder_height: 120.0,
                image_caption_gap: 8.0,
                scrollbar_width: 6.0,
                scrollbar_right: 6.0,
                centered_shrink_start: 1100.0,
                centered_shrink_end: 2200.0,
                centered_min_ratio: 0.58,
                dialog_width: 460.0,
                dialog_padding: 20.0,
                dialog_gap: 14.0,
                dialog_radius: 14.0,
                dialog_border_width: 1.0,
                dialog_button_height: 36.0,
                dialog_button_gap: 10.0,
                dialog_button_padding_x: 14.0,
                menu_bar_height: 32.0,
                menu_bar_padding_x: 10.0,
                menu_bar_padding_y: 4.0,
                menu_bar_gap: 2.0,
                menu_bar_button_width: 48.0,
                menu_bar_button_height: 24.0,
                menu_bar_button_padding_x: 8.0,
                menu_bar_button_radius: 5.0,
                menu_text_size: 12.0,
                menu_panel_top: 30.0,
                menu_panel_width: 180.0,
                menu_panel_padding: 4.0,
                menu_panel_gap: 1.0,
                menu_panel_radius: 8.0,
                menu_item_height: 28.0,
                menu_item_padding_x: 8.0,
                menu_item_radius: 5.0,
                menu_separator_margin_x: 6.0,
                menu_separator_margin_y: 3.0,
                menu_separator_height: 1.0,
                context_menu_panel_width: 132.0,
                context_menu_submenu_width: 148.0,
                context_menu_submenu_gap: 2.0,
                context_menu_axis_panel_width: 164.0,
                table_insert_dialog_width: 380.0,
                table_insert_stepper_gap: 8.0,
                table_insert_stepper_button_size: 32.0,
                table_insert_stepper_value_min_width: 56.0,
                table_insert_stepper_value_padding_x: 10.0,
                table_insert_stepper_radius: 8.0,
                view_mode_toggle_left: 12.0,
                view_mode_toggle_bottom: 12.0,
                view_mode_toggle_padding_x: 8.0,
                view_mode_toggle_padding_y: 4.0,
                view_mode_toggle_min_width: 88.0,
                view_mode_toggle_radius: 999.0,
                view_mode_toggle_border_width: 1.0,
                view_mode_toggle_text_size: 11.0,
                status_bar_height: 28.0,
                status_bar_padding_x: 12.0,
                status_bar_item_gap: 12.0,
                status_bar_text_size: 11.0,
            },
            typography: ThemeTypography {
                text_size: 17.0,
                text_line_height: 1.6,
                h1_size: 32.0,
                h1_weight: FontWeightDef::Bold,
                h2_size: 24.0,
                h2_weight: FontWeightDef::Bold,
                h3_size: 20.0,
                h3_weight: FontWeightDef::Semibold,
                h4_size: 18.0,
                h4_weight: FontWeightDef::Semibold,
                h5_size: 16.0,
                h5_weight: FontWeightDef::Semibold,
                h6_size: 14.0,
                h6_weight: FontWeightDef::Semibold,
                code_size: 15.0,
                dialog_title_size: 20.0,
                dialog_title_weight: FontWeightDef::Semibold,
                dialog_body_size: 14.0,
                dialog_body_weight: FontWeightDef::Normal,
                dialog_button_size: 14.0,
                dialog_button_weight: FontWeightDef::Medium,
            },
            placeholders: Placeholders {
                empty_editing: String::new(),
            },
        }
    }

    /// Returns the built-in light theme.
    ///
    /// The light theme intentionally reuses the default layout and typography
    /// tokens so it can focus on palette differences.
    pub fn light_theme() -> Self {
        let base = Self::default_theme();
        Self {
            name: BUILTIN_THEME_VELOTYPE_LIGHT_NAME.into(),
            colors: ThemeColors {
                editor_background: Hsla::from(rgba(0xf7f8fbff)),
                source_mode_block_bg: Hsla::from(rgba(0xeef2f7ff)),
                comment_bg: Hsla::from(rgba(0xfef3c766)),
                text_default: Hsla::from(rgba(0x1f2937ff)),
                text_link: Hsla::from(rgba(0x2563ebff)),
                text_placeholder: Hsla::from(rgba(0x6b7280cc)),
                text_h1: Hsla::from(rgba(0x111827ff)),
                text_h2: Hsla::from(rgba(0x111827ff)),
                text_h3: Hsla::from(rgba(0x111827ff)),
                text_h4: Hsla::from(rgba(0x111827ff)),
                text_h5: Hsla::from(rgba(0x111827ff)),
                text_h6: Hsla::from(rgba(0x111827ff)),
                border_h1: Hsla::from(rgba(0xcbd5e1ff)),
                border_h2: Hsla::from(rgba(0xdbe3efff)),
                text_quote: Hsla::from(rgba(0x475569ff)),
                border_quote: Hsla::from(rgba(0x94a3b8ff)),
                callout_note_bg: Hsla::from(rgba(0x2563eb14)),
                callout_note_border: Hsla::from(rgba(0x2563ebff)),
                callout_tip_bg: Hsla::from(rgba(0x16a34a14)),
                callout_tip_border: Hsla::from(rgba(0x16a34aff)),
                callout_important_bg: Hsla::from(rgba(0x7c3aed14)),
                callout_important_border: Hsla::from(rgba(0x7c3aedff)),
                callout_warning_bg: Hsla::from(rgba(0xf9731614)),
                callout_warning_border: Hsla::from(rgba(0xf97316ff)),
                callout_caution_bg: Hsla::from(rgba(0xdc262614)),
                callout_caution_border: Hsla::from(rgba(0xdc2626ff)),
                footnote_bg: Hsla::from(rgba(0xffffffff)),
                footnote_border: Hsla::from(rgba(0xcbd5e1ff)),
                footnote_badge_bg: Hsla::from(rgba(0xe2e8f0ff)),
                footnote_badge_text: Hsla::from(rgba(0x334155ff)),
                footnote_backref: Hsla::from(rgba(0x2563ebff)),
                task_checkbox_border: Hsla::from(rgba(0x94a3b8ff)),
                task_checkbox_bg: Hsla::from(rgba(0xffffffff)),
                task_checkbox_checked_bg: Hsla::from(rgba(0x2563ebff)),
                task_checkbox_check: Hsla::from(rgba(0xffffffff)),
                separator_color: Hsla::from(rgba(0xcbd5e1ff)),
                code_bg: Hsla::from(rgba(0xf1f5f9ff)),
                code_text: Hsla::from(rgba(0x111827ff)),
                code_language_input_bg: Hsla::from(rgba(0xffffffff)),
                code_language_input_border: Hsla::from(rgba(0xcbd5e1ff)),
                code_language_input_text: Hsla::from(rgba(0x1f2937ff)),
                code_language_input_placeholder: Hsla::from(rgba(0x64748bcc)),
                code_syntax_comment: Hsla::from(rgba(0x6b7280ff)),
                code_syntax_keyword: Hsla::from(rgba(0x7c3aedff)),
                code_syntax_string: Hsla::from(rgba(0x15803dff)),
                code_syntax_number: Hsla::from(rgba(0xc2410cff)),
                code_syntax_type: Hsla::from(rgba(0x0f766eff)),
                code_syntax_function: Hsla::from(rgba(0x2563ebff)),
                code_syntax_constant: Hsla::from(rgba(0xb45309ff)),
                code_syntax_variable: Hsla::from(rgba(0x1f2937ff)),
                code_syntax_property: Hsla::from(rgba(0x0891b2ff)),
                code_syntax_operator: Hsla::from(rgba(0x9333eaff)),
                code_syntax_punctuation: Hsla::from(rgba(0x64748bff)),
                table_border: Hsla::from(rgba(0xd1d5dbff)),
                table_header_bg: Hsla::from(rgba(0xf1f5f9ff)),
                table_cell_bg: Hsla::from(rgba(0xffffffff)),
                table_cell_active_outline: Hsla::from(rgba(0x2563ebff)),
                table_axis_preview_bg: Hsla::from(rgba(0x2563eb14)),
                table_axis_selected_bg: Hsla::from(rgba(0x2563eb29)),
                table_append_button_bg: Hsla::from(rgba(0xe2e8f0ff)),
                table_append_button_hover: Hsla::from(rgba(0xcbd5e1ff)),
                table_append_button_text: Hsla::from(rgba(0x334155ff)),
                image_placeholder_bg: Hsla::from(rgba(0xf8fafcff)),
                image_placeholder_border: Hsla::from(rgba(0xcbd5e1ff)),
                image_placeholder_text: Hsla::from(rgba(0x475569ff)),
                image_caption_text: Hsla::from(rgba(0x64748bff)),
                scrollbar_thumb: Hsla::from(rgba(0x64748bb8)),
                cursor: Hsla::from(rgba(0x111827ff)),
                selection: Hsla::from(rgba(0xbfdbfecc)),
                dialog_backdrop: Hsla::from(rgba(0x0f172a66)),
                dialog_surface: Hsla::from(rgba(0xffffffff)),
                dialog_border: Hsla::from(rgba(0xd1d5dbff)),
                dialog_title: Hsla::from(rgba(0x111827ff)),
                dialog_body: Hsla::from(rgba(0x374151ff)),
                dialog_muted: Hsla::from(rgba(0x6b7280ff)),
                dialog_primary_button_bg: Hsla::from(rgba(0x2563ebff)),
                dialog_primary_button_hover: Hsla::from(rgba(0x1d4ed8ff)),
                dialog_primary_button_text: Hsla::from(rgba(0xffffffff)),
                dialog_secondary_button_bg: Hsla::from(rgba(0xf1f5f9ff)),
                dialog_secondary_button_hover: Hsla::from(rgba(0xe2e8f0ff)),
                dialog_secondary_button_text: Hsla::from(rgba(0x1f2937ff)),
                dialog_danger_button_bg: Hsla::from(rgba(0xdc2626ff)),
                dialog_danger_button_hover: Hsla::from(rgba(0xb91c1cff)),
                dialog_danger_button_text: Hsla::from(rgba(0xffffffff)),
                status_bar_background: Hsla::from(rgba(0xe2e8f0ff)),
                status_bar_text: Hsla::from(rgba(0x334155ff)),
                status_bar_text_dim: Hsla::from(rgba(0x64748bff)),
                status_bar_button_hover: Hsla::from(rgba(0xcbd5e1ff)),
            },
            dimensions: base.dimensions,
            typography: base.typography,
            placeholders: base.placeholders,
        }
    }

    /// Parses a theme from JSON text.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    /// Loads a theme from a JSON file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Self::from_json(&json)
    }

    /// Serializes the theme into pretty-printed JSON.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// Metadata for a selectable theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeCatalogEntry {
    pub id: String,
    pub name: String,
}

const BUILTIN_THEME_VELOTYPE_ID: &str = "velotype";
const BUILTIN_THEME_VELOTYPE_NAME: &str = "Velotype";
const BUILTIN_THEME_VELOTYPE_LIGHT_ID: &str = "velotype-light";
const BUILTIN_THEME_VELOTYPE_LIGHT_NAME: &str = "Velotype Light";
const CUSTOM_THEME_ID: &str = "custom";

fn builtin_theme_catalog() -> Vec<ThemeCatalogEntry> {
    vec![
        ThemeCatalogEntry {
            id: BUILTIN_THEME_VELOTYPE_ID.into(),
            name: BUILTIN_THEME_VELOTYPE_NAME.into(),
        },
        ThemeCatalogEntry {
            id: BUILTIN_THEME_VELOTYPE_LIGHT_ID.into(),
            name: BUILTIN_THEME_VELOTYPE_LIGHT_NAME.into(),
        },
    ]
}

#[derive(Debug, Clone)]
struct CustomThemeEntry {
    id: String,
    name: String,
    creator: String,
    base_theme_id: String,
    theme: Theme,
}

/// Global singleton that holds the current [`Theme`].
///
/// Registered via [`Global`] so every component can access it through
/// `cx.global::<ThemeManager>().current()` without passing props.
pub struct ThemeManager {
    current: Arc<Theme>,
    current_theme_id: String,
    custom_themes: Vec<CustomThemeEntry>,
    theme_catalog: Vec<ThemeCatalogEntry>,
}

impl Global for ThemeManager {}

impl Default for ThemeManager {
    fn default() -> Self {
        Self {
            current: Arc::new(Theme::default_theme()),
            current_theme_id: BUILTIN_THEME_VELOTYPE_ID.into(),
            custom_themes: Vec::new(),
            theme_catalog: builtin_theme_catalog(),
        }
    }
}

#[allow(unused)]
impl ThemeManager {
    /// Installs the configured theme into GPUI's global state.
    pub fn init(cx: &mut App) {
        let theme_id = crate::config::read_app_preferences()
            .map(|preferences| preferences.default_theme_id)
            .unwrap_or_else(|_| BUILTIN_THEME_VELOTYPE_ID.into());
        Self::init_with_theme_id(cx, &theme_id);
    }

    /// Installs a specific theme into GPUI's global state.
    pub fn init_with_theme_id(cx: &mut App, theme_id: &str) {
        let mut manager = Self::default();
        if let Ok(dirs) = VelotypeConfigDirs::from_system()
            && let Err(err) = manager.load_custom_themes_from_dirs(&dirs)
        {
            eprintln!("failed to load custom themes: {err}");
        }
        let _ = manager.set_theme_by_id(theme_id);
        cx.set_global(manager);
    }

    /// Returns the currently active theme.
    pub fn current(&self) -> &Theme {
        &self.current
    }

    /// Returns an `Arc` clone of the currently active theme — O(1), no
    /// per-field copy. Use this in hot render paths instead of cloning the
    /// whole `Theme` struct (which has ~200 fields and a `String` name).
    pub fn current_arc(&self) -> Arc<Theme> {
        self.current.clone()
    }

    /// Returns the identifier of the currently active theme.
    pub fn current_theme_id(&self) -> &str {
        &self.current_theme_id
    }

    /// Returns all built-in and imported themes exposed in the native menu.
    pub fn available_themes(&self) -> &[ThemeCatalogEntry] {
        &self.theme_catalog
    }

    /// Loads and activates a theme from a file.
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let theme = Theme::from_file(path)?;
        self.current_theme_id = self.theme_id_for_loaded_theme(&theme);
        self.current = Arc::new(theme);
        Ok(())
    }

    /// Loads and activates a theme from JSON text.
    pub fn load_json(&mut self, json: &str) -> anyhow::Result<()> {
        let theme = Theme::from_json(json)?;
        self.current_theme_id = self.theme_id_for_loaded_theme(&theme);
        self.current = Arc::new(theme);
        Ok(())
    }

    /// Replaces the active theme with a fully constructed value.
    pub fn set_theme(&mut self, theme: Theme) {
        self.current_theme_id = self.theme_id_for_loaded_theme(&theme);
        self.current = Arc::new(theme);
    }

    /// Restores the built-in default theme.
    pub fn reset(&mut self) {
        self.current = Arc::new(Theme::default_theme());
        self.current_theme_id = BUILTIN_THEME_VELOTYPE_ID.into();
    }

    /// Activates a theme by identifier.
    pub fn set_theme_by_id(&mut self, theme_id: &str) -> bool {
        match theme_id {
            id if id == BUILTIN_THEME_VELOTYPE_ID => {
                self.current = Arc::new(Theme::default_theme());
                self.current_theme_id = BUILTIN_THEME_VELOTYPE_ID.into();
                true
            }
            id if id == BUILTIN_THEME_VELOTYPE_LIGHT_ID => {
                self.current = Arc::new(Theme::light_theme());
                self.current_theme_id = BUILTIN_THEME_VELOTYPE_LIGHT_ID.into();
                true
            }
            id => {
                let Some(entry) = self.custom_themes.iter().find(|entry| entry.id == id) else {
                    return false;
                };
                self.current = Arc::new(entry.theme.clone());
                self.current_theme_id = entry.id.clone();
                true
            }
        }
    }

    /// Imports a user theme pack, persists a normalized copy, and activates it.
    pub fn import_theme_config(&mut self, path: impl AsRef<Path>) -> anyhow::Result<String> {
        let dirs = VelotypeConfigDirs::from_system()?;
        self.import_theme_config_with_dirs(path, &dirs)
    }

    fn import_theme_config_with_dirs(
        &mut self,
        path: impl AsRef<Path>,
        dirs: &VelotypeConfigDirs,
    ) -> anyhow::Result<String> {
        let raw = read_json_or_jsonc(path.as_ref())?;
        let default_base_theme_id = self.theme_import_base_theme_id();
        let (entry, normalized) =
            custom_theme_from_value_with_default_base(raw, default_base_theme_id.as_str())?;
        let file_name = format!(
            "{}_{}.json",
            sanitize_config_file_stem(&entry.name),
            sanitize_config_file_stem(&entry.creator)
        );
        let themes_dir = dirs.themes_dir();
        std::fs::create_dir_all(&themes_dir)?;
        std::fs::write(
            themes_dir.join(file_name),
            serde_json::to_string_pretty(&normalized)?,
        )?;
        let imported_id = entry.id.clone();
        self.upsert_custom_theme(entry);
        self.set_theme_by_id(&imported_id);
        Ok(imported_id)
    }

    fn load_custom_themes_from_dirs(&mut self, dirs: &VelotypeConfigDirs) -> anyhow::Result<()> {
        let themes_dir = dirs.themes_dir();
        if !themes_dir.exists() {
            return Ok(());
        }

        let mut loaded = Vec::new();
        for entry in std::fs::read_dir(&themes_dir)? {
            let path = entry?.path();
            if path.is_file() {
                match read_json_or_jsonc(&path)
                    .and_then(|value| custom_theme_from_value(value).map(|(entry, _)| entry))
                {
                    Ok(entry) => loaded.push(entry),
                    Err(err) => {
                        eprintln!("skipping custom theme config '{}': {err}", path.display())
                    }
                }
            }
        }
        loaded.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.creator.cmp(&right.creator))
        });
        for entry in loaded {
            self.upsert_custom_theme(entry);
        }
        Ok(())
    }

    fn upsert_custom_theme(&mut self, entry: CustomThemeEntry) {
        if let Some(existing) = self
            .custom_themes
            .iter_mut()
            .find(|existing| existing.id == entry.id)
        {
            *existing = entry;
        } else {
            self.custom_themes.push(entry);
        }
        self.rebuild_theme_catalog();
    }

    fn rebuild_theme_catalog(&mut self) {
        let mut catalog = builtin_theme_catalog();
        catalog.extend(self.custom_themes.iter().map(|entry| ThemeCatalogEntry {
            id: entry.id.clone(),
            name: format!("{} - {}", entry.name, entry.creator),
        }));
        self.theme_catalog = catalog;
    }

    fn theme_id_for_loaded_theme(&self, theme: &Theme) -> String {
        if theme.name == BUILTIN_THEME_VELOTYPE_NAME {
            BUILTIN_THEME_VELOTYPE_ID.into()
        } else if theme.name == BUILTIN_THEME_VELOTYPE_LIGHT_NAME {
            BUILTIN_THEME_VELOTYPE_LIGHT_ID.into()
        } else {
            CUSTOM_THEME_ID.into()
        }
    }

    fn theme_import_base_theme_id(&self) -> String {
        match self.current_theme_id.as_str() {
            BUILTIN_THEME_VELOTYPE_LIGHT_ID => BUILTIN_THEME_VELOTYPE_LIGHT_ID.into(),
            BUILTIN_THEME_VELOTYPE_ID => BUILTIN_THEME_VELOTYPE_ID.into(),
            id => self
                .custom_themes
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.base_theme_id.clone())
                .unwrap_or_else(|| BUILTIN_THEME_VELOTYPE_ID.into()),
        }
    }
}

fn custom_theme_from_value(value: Value) -> anyhow::Result<(CustomThemeEntry, Value)> {
    custom_theme_from_value_with_default_base(value, BUILTIN_THEME_VELOTYPE_ID)
}

fn custom_theme_from_value_with_default_base(
    mut value: Value,
    default_base_theme_id: &str,
) -> anyhow::Result<(CustomThemeEntry, Value)> {
    prune_empty_json_values(&mut value);
    let Value::Object(mut object) = value else {
        bail!("theme config must be a JSON object");
    };
    let object = object_without_empty_values(std::mem::take(&mut object));
    let name = required_string(&object, "name")?;
    let creator = required_string(&object, "creator")?;
    let base_theme_id = resolved_custom_theme_base_id(&object, default_base_theme_id);
    let raw_theme_patch = object
        .get("theme")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    if !raw_theme_patch.is_object() {
        bail!("field 'theme' must be a JSON object when present");
    }

    let base_theme = custom_theme_base_theme(&base_theme_id);
    let mut merged = serde_json::to_value(base_theme)?;
    let mut theme_patch = filter_json_by_schema(&raw_theme_patch, &merged);
    if let Value::Object(theme_patch_object) = &mut theme_patch {
        theme_patch_object.remove("name");
    }
    merge_non_empty_json_values(&mut merged, &theme_patch);
    if let Value::Object(merged_object) = &mut merged {
        merged_object.insert("name".into(), Value::String(name.clone()));
    }
    let theme: Theme = serde_json::from_value(merged)
        .with_context(|| format!("failed to construct custom theme '{name}'"))?;
    let id = format!(
        "custom:{}_{}",
        sanitize_config_file_stem(&name),
        sanitize_config_file_stem(&creator)
    );
    let mut normalized_object = Map::new();
    normalized_object.insert("name".into(), Value::String(name.clone()));
    normalized_object.insert("creator".into(), Value::String(creator.clone()));
    normalized_object.insert(
        "base_theme_id".into(),
        Value::String(base_theme_id.to_string()),
    );
    for key in ["description", "version", "homepage", "license"] {
        if let Some(value) = object.get(key) {
            normalized_object.insert(key.into(), value.clone());
        }
    }
    if !theme_patch
        .as_object()
        .map(|object| object.is_empty())
        .unwrap_or(false)
    {
        normalized_object.insert("theme".into(), theme_patch);
    }
    let normalized = Value::Object(normalized_object);

    Ok((
        CustomThemeEntry {
            id,
            name,
            creator,
            base_theme_id: base_theme_id.to_string(),
            theme,
        },
        normalized,
    ))
}

fn resolved_custom_theme_base_id<'a>(
    object: &'a Map<String, Value>,
    default_base_theme_id: &'a str,
) -> &'a str {
    object
        .get("base_theme_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_builtin_theme_id(value))
        .unwrap_or_else(|| {
            if is_builtin_theme_id(default_base_theme_id) {
                default_base_theme_id
            } else {
                BUILTIN_THEME_VELOTYPE_ID
            }
        })
}

fn is_builtin_theme_id(theme_id: &str) -> bool {
    theme_id == BUILTIN_THEME_VELOTYPE_ID || theme_id == BUILTIN_THEME_VELOTYPE_LIGHT_ID
}

fn custom_theme_base_theme(theme_id: &str) -> Theme {
    if theme_id == BUILTIN_THEME_VELOTYPE_LIGHT_ID {
        Theme::light_theme()
    } else {
        Theme::default_theme()
    }
}

fn filter_json_by_schema(value: &Value, schema: &Value) -> Value {
    match (value, schema) {
        (Value::Object(value_object), Value::Object(schema_object)) => {
            let mut filtered = Map::new();
            for (key, value) in value_object {
                if let Some(schema_value) = schema_object.get(key) {
                    filtered.insert(key.clone(), filter_json_by_schema(value, schema_value));
                }
            }
            Value::Object(filtered)
        }
        (value, _) => value.clone(),
    }
}

fn required_string(object: &Map<String, Value>, key: &str) -> anyhow::Result<String> {
    let Some(value) = object.get(key) else {
        bail!("missing required field '{key}'");
    };
    let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        bail!("field '{key}' must be a non-empty string");
    };
    Ok(text.to_string())
}
#[cfg(test)]
mod tests {
    use super::{Theme, ThemeManager};
    use crate::config::VelotypeConfigDirs;
    use crate::host_services::EditorHostTheme;
    use gpui::rgba;

    #[test]
    fn host_palette_overrides_editor_semantic_colors() {
        let host = EditorHostTheme {
            background: rgba(0x102030ff).into(),
            foreground: rgba(0xe1e2e3ff).into(),
            border: rgba(0x405060ff).into(),
            muted: rgba(0x708090ff).into(),
            accent: rgba(0x22aaffff).into(),
        };

        let theme = Theme::with_host_palette(&Theme::default_theme(), &host);

        assert_eq!(theme.colors.editor_background, host.background);
        assert_eq!(theme.colors.text_default, host.foreground);
        assert_eq!(theme.colors.text_h1, host.foreground);
        assert_eq!(theme.colors.text_placeholder, host.muted);
        assert_eq!(theme.colors.text_link, host.accent);
        assert_eq!(theme.colors.border_h1, host.border);
        assert_eq!(theme.colors.border_quote, host.border);
        assert_eq!(theme.colors.code_text, host.foreground);
        assert_eq!(theme.colors.code_language_input_border, host.border);
        assert_eq!(theme.colors.table_border, host.border);
        assert_eq!(theme.colors.table_cell_active_outline, host.accent);
        assert_eq!(theme.colors.cursor, host.accent);
    }

    #[test]
    fn deserializes_legacy_block_focused_bg_key() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let legacy_json = default_json.replace("source_mode_block_bg", "block_focused_bg");

        let theme = Theme::from_json(&legacy_json).expect("legacy theme should deserialize");
        assert!(theme.colors.source_mode_block_bg.a > 0.0);
    }

    #[test]
    fn border_h2_falls_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();
        object
            .get_mut("colors")
            .and_then(|colors| colors.as_object_mut())
            .expect("theme should include colors")
            .remove("border_h2");
        let json = serde_json::to_string(&object).expect("theme json should serialize");

        let theme = Theme::from_json(&json).expect("theme without border_h2 should deserialize");
        assert_eq!(theme.colors.border_h2, rgba(0xe0e0e0cc).into());
    }

    #[test]
    fn comment_background_falls_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();
        object
            .get_mut("colors")
            .and_then(|colors| colors.as_object_mut())
            .expect("theme should include colors")
            .remove("comment_bg");
        let json = serde_json::to_string(&object).expect("theme json should serialize");

        let theme = Theme::from_json(&json).expect("theme without comment_bg should deserialize");
        assert_eq!(theme.colors.comment_bg, rgba(0xfbbf2426).into());
    }

    #[test]
    fn default_theme_json_omits_dialog_badge_and_strings_tokens() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");

        assert!(parsed.get("strings").is_none());

        let colors = parsed
            .get("colors")
            .and_then(|colors| colors.as_object())
            .expect("theme should include colors");
        assert!(!colors.contains_key(&format!("dialog_{}", "badge_bg")));
        assert!(!colors.contains_key(&format!("dialog_{}", "badge_text")));

        let dimensions = parsed
            .get("dimensions")
            .and_then(|dimensions| dimensions.as_object())
            .expect("theme should include dimensions");
        assert!(!dimensions.contains_key(&format!("dialog_{}", "badge_padding_x")));
        assert!(!dimensions.contains_key(&format!("dialog_{}", "badge_padding_y")));
    }

    #[test]
    fn legacy_theme_json_with_strings_still_loads() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();
        object.insert(
            "strings".into(),
            serde_json::json!({
                "menu_file": "Legacy File",
                "menu_language": "Legacy Language"
            }),
        );
        let json = serde_json::to_string(&object).expect("theme json should serialize");

        Theme::from_json(&json).expect("legacy theme strings should be ignored safely");
    }

    #[test]
    fn callout_dimensions_fall_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();
        let dimensions = object
            .get_mut("dimensions")
            .and_then(|dimensions| dimensions.as_object_mut())
            .expect("theme should include dimensions");
        dimensions.remove("callout_padding_x");
        dimensions.remove("callout_padding_y");
        dimensions.remove("callout_body_gap");
        dimensions.remove("callout_radius");
        dimensions.remove("callout_border_width");
        dimensions.remove("callout_header_gap");
        dimensions.remove("callout_header_margin_bottom");
        let json = serde_json::to_string(&object).expect("theme json should serialize");

        let theme = Theme::from_json(&json).expect("theme without callout dimensions should load");
        assert_eq!(theme.dimensions.callout_padding_x, 14.0);
        assert_eq!(theme.dimensions.callout_padding_y, 10.0);
        assert_eq!(theme.dimensions.callout_body_gap, 8.0);
        assert_eq!(theme.dimensions.callout_radius, 10.0);
        assert_eq!(theme.dimensions.callout_border_width, 4.0);
        assert_eq!(theme.dimensions.callout_header_gap, 6.0);
        assert_eq!(theme.dimensions.callout_header_margin_bottom, 6.0);
    }

    #[test]
    fn footnote_tokens_fall_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();

        let colors = object
            .get_mut("colors")
            .and_then(|colors| colors.as_object_mut())
            .expect("theme should include colors");
        colors.remove("footnote_bg");
        colors.remove("footnote_border");
        colors.remove("footnote_badge_bg");
        colors.remove("footnote_badge_text");
        colors.remove("footnote_backref");

        let dimensions = object
            .get_mut("dimensions")
            .and_then(|dimensions| dimensions.as_object_mut())
            .expect("theme should include dimensions");
        dimensions.remove("footnote_padding_x");
        dimensions.remove("footnote_padding_y");
        dimensions.remove("footnote_radius");
        dimensions.remove("footnote_badge_padding_x");
        dimensions.remove("footnote_badge_padding_y");

        let json = serde_json::to_string(&object).expect("theme json should serialize");
        let theme = Theme::from_json(&json).expect("theme without footnote tokens should load");

        assert_eq!(theme.colors.footnote_bg, rgba(0x212124ff).into());
        assert_eq!(theme.colors.footnote_border, rgba(0x71717a52).into());
        assert_eq!(theme.colors.footnote_badge_bg, rgba(0xa1a1aa24).into());
        assert_eq!(theme.colors.footnote_badge_text, rgba(0xd4d4d8cc).into());
        assert_eq!(theme.colors.footnote_backref, rgba(0xa1a1aaff).into());
        assert_eq!(theme.dimensions.footnote_padding_x, 10.0);
        assert_eq!(theme.dimensions.footnote_padding_y, 6.0);
        assert_eq!(theme.dimensions.footnote_radius, 6.0);
        assert_eq!(theme.dimensions.footnote_badge_padding_x, 4.0);
        assert_eq!(theme.dimensions.footnote_badge_padding_y, 1.0);
    }

    #[test]
    fn code_language_palette_tokens_fall_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();

        let colors = object
            .get_mut("colors")
            .and_then(|colors| colors.as_object_mut())
            .expect("theme should include colors");
        colors.remove("code_bg");
        colors.remove("code_language_input_bg");
        colors.remove("code_language_input_border");
        colors.remove("code_language_input_text");
        colors.remove("code_language_input_placeholder");

        let json = serde_json::to_string(&object).expect("theme json should serialize");
        let theme =
            Theme::from_json(&json).expect("theme without code language palette should load");

        assert_eq!(theme.colors.code_bg, rgba(0x111827ff).into());
        assert_eq!(theme.colors.code_language_input_bg, rgba(0x343941ff).into());
        assert_eq!(
            theme.colors.code_language_input_border,
            rgba(0x4b5563cc).into()
        );
        assert_eq!(
            theme.colors.code_language_input_text,
            rgba(0xe5e7ebff).into()
        );
        assert_eq!(
            theme.colors.code_language_input_placeholder,
            rgba(0x9ca3afcc).into()
        );
    }

    #[test]
    fn important_callout_defaults_use_purple_palette() {
        let theme = Theme::default_theme();
        assert_eq!(theme.colors.callout_important_bg, rgba(0xa78bfa1f).into());
        assert_eq!(
            theme.colors.callout_important_border,
            rgba(0xa78bfaff).into()
        );
        assert_eq!(theme.dimensions.block_gap, 6.0);
        assert_eq!(theme.colors.footnote_bg, rgba(0x212124ff).into());
        assert_eq!(theme.dimensions.footnote_padding_x, 10.0);
        assert_eq!(theme.colors.code_bg, rgba(0x23272eff).into());
        assert_eq!(theme.colors.code_language_input_bg, rgba(0x343941ff).into());
        assert_eq!(
            theme.colors.code_language_input_border,
            rgba(0x4b5563cc).into()
        );
    }

    #[test]
    fn light_theme_uses_light_palette_without_changing_layout_tokens() {
        let dark = Theme::default_theme();
        let light = Theme::light_theme();

        assert_eq!(light.name, "Velotype Light");
        assert_eq!(light.colors.editor_background, rgba(0xf7f8fbff).into());
        assert_eq!(light.colors.text_default, rgba(0x1f2937ff).into());
        assert_eq!(light.colors.text_link, rgba(0x2563ebff).into());
        assert_eq!(light.colors.code_bg, rgba(0xf1f5f9ff).into());
        assert_eq!(
            light.colors.code_language_input_border,
            rgba(0xcbd5e1ff).into()
        );
        assert_eq!(
            light.colors.table_cell_active_outline,
            rgba(0x2563ebff).into()
        );
        assert_eq!(light.dimensions.block_gap, dark.dimensions.block_gap);
        assert_eq!(light.typography.text_size, dark.typography.text_size);
    }

    #[test]
    fn menu_dimension_tokens_fall_back_when_omitted() {
        let default_json = Theme::default_theme()
            .to_json()
            .expect("default theme should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&default_json).expect("default theme json should parse");
        let mut object = parsed
            .as_object()
            .expect("theme should serialize to a json object")
            .clone();

        let dimensions = object
            .get_mut("dimensions")
            .and_then(|dimensions| dimensions.as_object_mut())
            .expect("theme should include dimensions");
        dimensions.remove("menu_bar_height");
        dimensions.remove("menu_item_height");
        dimensions.remove("context_menu_panel_width");
        dimensions.remove("table_insert_dialog_width");
        dimensions.remove("view_mode_toggle_min_width");
        dimensions.remove("view_mode_toggle_text_size");

        let json = serde_json::to_string(&object).expect("theme json should serialize");
        let theme = Theme::from_json(&json).expect("theme without menu tokens should load");

        assert_eq!(theme.dimensions.menu_bar_height, 32.0);
        assert_eq!(theme.dimensions.menu_item_height, 28.0);
        assert_eq!(theme.dimensions.context_menu_panel_width, 132.0);
        assert_eq!(theme.dimensions.table_insert_dialog_width, 380.0);
        assert_eq!(theme.dimensions.view_mode_toggle_min_width, 88.0);
        assert_eq!(theme.dimensions.view_mode_toggle_text_size, 11.0);
    }

    #[test]
    fn imports_partial_jsonc_theme_and_persists_normalized_json() {
        let root = std::env::temp_dir().join(format!("velotype-theme-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("temp root should be created");
        let source = root.join("theme.jsonc");
        std::fs::write(
            &source,
            r#"{
                // Required metadata.
                "name": "Night Writer",
                "creator": "Ada",
                "description": "",
                "theme": {
                    "dimensions": {
                        "block_gap": 12.0,
                        "menu_text_size": null
                    },
                    "placeholders": {
                        "empty_editing": ""
                    }
                }
            }"#,
        )
        .expect("theme config should be written");

        let dirs = VelotypeConfigDirs::from_root(&root);
        let mut manager = ThemeManager::default();
        let imported_id = manager
            .import_theme_config_with_dirs(&source, &dirs)
            .expect("theme config should import");

        assert_eq!(manager.current_theme_id(), imported_id);
        assert_eq!(manager.current().name, "Night Writer");
        assert_eq!(
            manager.current().colors.editor_background,
            Theme::default_theme().colors.editor_background
        );
        assert_eq!(manager.current().dimensions.block_gap, 12.0);
        assert_eq!(manager.current().dimensions.menu_text_size, 12.0);
        assert!(
            manager
                .available_themes()
                .iter()
                .any(|entry| { entry.id == imported_id && entry.name == "Night Writer - Ada" })
        );

        let normalized = std::fs::read_to_string(dirs.themes_dir().join("Night_Writer_Ada.json"))
            .expect("normalized theme config should exist");
        assert!(normalized.contains("\"name\": \"Night Writer\""));
        assert!(normalized.contains("\"creator\": \"Ada\""));
        assert!(normalized.contains("\"base_theme_id\": \"velotype\""));
        assert!(normalized.contains("\"block_gap\": 12.0"));
        assert!(!normalized.contains("menu_text_size"));
        assert!(!normalized.contains("empty_editing"));
        assert!(!normalized.contains("description"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn custom_theme_pack_can_inherit_light_base() {
        let value = serde_json::json!({
            "name": "Day Writer",
            "creator": "Ada",
            "base_theme_id": "velotype-light",
            "theme": {
                "dimensions": {
                    "menu_panel_radius": 12.0
                },
                "colors": {
                    "text_link": null
                }
            }
        });

        let (entry, normalized) =
            super::custom_theme_from_value(value).expect("theme should import");
        let light = Theme::light_theme();

        assert_eq!(entry.base_theme_id, "velotype-light");
        assert_eq!(
            entry.theme.colors.editor_background,
            light.colors.editor_background
        );
        assert_eq!(entry.theme.colors.text_default, light.colors.text_default);
        assert_eq!(entry.theme.colors.text_link, light.colors.text_link);
        assert_eq!(entry.theme.dimensions.menu_panel_radius, 12.0);
        assert_eq!(
            normalized
                .get("base_theme_id")
                .and_then(|value| value.as_str()),
            Some("velotype-light")
        );
        assert!(
            normalized
                .pointer("/theme/colors")
                .and_then(|value| value.as_object())
                .map(|colors| !colors.contains_key("text_link"))
                .unwrap_or(true)
        );
    }

    #[test]
    fn invalid_custom_theme_base_falls_back_to_dark() {
        let value = serde_json::json!({
            "name": "Broken Base",
            "creator": "Ada",
            "base_theme_id": "missing",
            "theme": {
                "dimensions": {
                    "block_gap": 10.0
                }
            }
        });

        let (entry, normalized) =
            super::custom_theme_from_value(value).expect("invalid base should not fail import");

        assert_eq!(entry.base_theme_id, "velotype");
        assert_eq!(
            entry.theme.colors.editor_background,
            Theme::default_theme().colors.editor_background
        );
        assert_eq!(
            normalized
                .get("base_theme_id")
                .and_then(|value| value.as_str()),
            Some("velotype")
        );
    }

    #[test]
    fn importing_without_base_uses_current_builtin_theme_as_base() {
        let root =
            std::env::temp_dir().join(format!("velotype-light-theme-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("temp root should be created");
        let source = root.join("theme.jsonc");
        std::fs::write(
            &source,
            r#"{
                "name": "Light Radius",
                "creator": "Ada",
                "theme": {
                    "dimensions": {
                        "menu_panel_radius": 14.0
                    }
                }
            }"#,
        )
        .expect("theme config should be written");

        let dirs = VelotypeConfigDirs::from_root(&root);
        let mut manager = ThemeManager::default();
        assert!(manager.set_theme_by_id("velotype-light"));
        let imported_id = manager
            .import_theme_config_with_dirs(&source, &dirs)
            .expect("theme config should import");

        assert_eq!(manager.current_theme_id(), imported_id);
        assert_eq!(
            manager.current().colors.editor_background,
            Theme::light_theme().colors.editor_background
        );
        assert_eq!(manager.current().dimensions.menu_panel_radius, 14.0);

        let normalized = std::fs::read_to_string(dirs.themes_dir().join("Light_Radius_Ada.json"))
            .expect("normalized theme config should exist");
        assert!(normalized.contains("\"base_theme_id\": \"velotype-light\""));

        let mut reloaded = ThemeManager::default();
        reloaded
            .load_custom_themes_from_dirs(&dirs)
            .expect("saved theme should reload");
        assert!(reloaded.set_theme_by_id(&imported_id));
        assert_eq!(
            reloaded.current().colors.editor_background,
            Theme::light_theme().colors.editor_background
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn theme_manager_switches_builtin_themes() {
        let mut manager = ThemeManager::default();
        assert_eq!(manager.current_theme_id(), "velotype");
        assert_eq!(manager.current().name, "Velotype");
        assert_eq!(
            manager
                .available_themes()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Velotype", "Velotype Light"]
        );

        assert!(manager.set_theme_by_id("velotype-light"));
        assert_eq!(manager.current_theme_id(), "velotype-light");
        assert_eq!(manager.current().name, "Velotype Light");
        assert_eq!(
            manager.current().colors.editor_background,
            rgba(0xf7f8fbff).into()
        );

        assert!(manager.set_theme_by_id("velotype"));
        assert_eq!(manager.current_theme_id(), "velotype");
        assert_eq!(manager.current().name, "Velotype");
        assert!(!manager.set_theme_by_id("missing"));
    }
}
