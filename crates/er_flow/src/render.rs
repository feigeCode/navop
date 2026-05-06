use ferrum_flow::{Node, NodeCardVariant, NodeRenderer, Port, RenderContext};
use gpui::{
    AnyElement, Element as _, Hsla, ParentElement as _, Styled as _, div, prelude::FluentBuilder,
    px, rgb,
};
use gpui_component::Theme;

/// ER 实体卡片的主题色配置，映射自应用级 GPUI 主题。
#[derive(Clone, Copy, PartialEq)]
pub struct ErCardTheme {
    pub card_background: Hsla,
    pub card_border: Hsla,
    pub card_border_selected: Hsla,
    pub stripe: Hsla,
    pub header_background: Hsla,
    pub header_text: Hsla,
    pub header_border: Hsla,
    pub row_odd: Hsla,
    pub row_even: Hsla,
    pub row_border: Hsla,
    pub column_text: Hsla,
    pub type_text: Hsla,
    pub null_text: Hsla,
    pub badge_primary: Hsla,
    pub badge_secondary: Hsla,
    pub field_dot: Hsla,
}

impl Default for ErCardTheme {
    fn default() -> Self {
        Self {
            card_background: rgb(0xffffff).into(),
            card_border: rgb(0x60a5fa).into(),
            card_border_selected: rgb(0x2563eb).into(),
            stripe: rgb(0xe6f4ff).into(),
            header_background: rgb(0xe4e4e7).into(),
            header_text: rgb(0x111827).into(),
            header_border: rgb(0xbfdbfe).into(),
            row_odd: rgb(0xf9fafb).into(),
            row_even: rgb(0xffffff).into(),
            row_border: rgb(0xe5e7eb).into(),
            column_text: rgb(0x111827).into(),
            type_text: rgb(0x6b7280).into(),
            null_text: rgb(0x9ca3af).into(),
            badge_primary: rgb(0x3b82f6).into(),
            badge_secondary: rgb(0x22c55e).into(),
            field_dot: rgb(0x9ca3af).into(),
        }
    }
}

impl ErCardTheme {
    /// 从应用的 GPUI 主题映射创建 ER 卡片主题色。
    pub fn from_ui_theme(ui: &Theme) -> Self {
        Self {
            card_background: ui.background,
            card_border: ui.primary,
            card_border_selected: ui.primary,
            stripe: ui.primary.opacity(0.08),
            header_background: ui.muted,
            header_text: ui.foreground,
            header_border: ui.border,
            row_odd: ui.table,
            row_even: ui.background,
            row_border: ui.border,
            column_text: ui.foreground,
            type_text: ui.muted_foreground,
            null_text: ui.muted_foreground,
            badge_primary: ui.primary,
            badge_secondary: ui.success,
            field_dot: ui.muted_foreground,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct ErEntityRenderer {
    pub card_theme: ErCardTheme,
}

impl ErEntityRenderer {
    pub fn new(theme: ErCardTheme) -> Self {
        Self { card_theme: theme }
    }

    pub fn from_ui_theme(ui: &Theme) -> Self {
        Self {
            card_theme: ErCardTheme::from_ui_theme(ui),
        }
    }
}

impl NodeRenderer for ErEntityRenderer {
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement {
        let selected = ctx.graph.selected_node_iter().any(|id| *id == node.id());

        ctx.node_card_shell(node, selected, NodeCardVariant::Custom)
            .bg(self.card_theme.card_background)
            .border_color(if selected {
                self.card_theme.card_border_selected
            } else {
                self.card_theme.card_border
            })
            .shadow_md()
            .overflow_hidden()
            .child(render_table(node, &self.card_theme))
            .into_any()
    }

    fn port_render(&self, node: &Node, port: &Port, ctx: &mut RenderContext) -> Option<AnyElement> {
        let frame = ctx.port_screen_frame(node, port)?;

        Some(
            frame
                .anchor_div()
                .rounded_full()
                .bg(rgb(ctx.theme.default_port_fill))
                .into_any(),
        )
    }
}

fn render_table(node: &Node, theme: &ErCardTheme) -> AnyElement {
    let columns = node
        .data_ref()
        .get("fields")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(div().w_full().h(px(10.0)).bg(theme.stripe))
        .child(render_header(node, theme))
        .children(
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| render_column(column, index, theme)),
        )
        .into_any()
}

fn render_header(node: &Node, theme: &ErCardTheme) -> AnyElement {
    let table = text_value(node, "name").unwrap_or_else(|| "table".to_string());
    let schema = text_value(node, "schema").or_else(|| text_value(node, "comment"));

    div()
        .w_full()
        .h(px(48.0))
        .px(px(8.0))
        .bg(theme.header_background)
        .border_b_1()
        .border_color(theme.header_border)
        .flex()
        .items_center()
        .justify_between()
        .child(header_title(table, theme))
        .when_some(schema, |this, schema| {
            this.child(header_schema(schema, theme))
        })
        .into_any()
}

fn header_title(table: String, theme: &ErCardTheme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(database_placeholder(theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_lg()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.header_text)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(table),
        )
        .into_any()
}

fn database_placeholder(theme: &ErCardTheme) -> AnyElement {
    div()
        .w(px(18.0))
        .h(px(18.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.badge_primary)
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(theme.badge_primary)
        .child("D")
        .into_any()
}

fn header_schema(schema: String, theme: &ErCardTheme) -> AnyElement {
    div()
        .text_xs()
        .text_color(theme.type_text)
        .max_w(px(92.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(schema)
        .into_any()
}

fn render_column(column: &serde_json::Value, index: usize, theme: &ErCardTheme) -> AnyElement {
    let name = json_text(column, "name");
    let ty = json_text(column, "data_type");
    let is_pk = column
        .get("primary_key")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let is_fk = column
        .get("foreign_key")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let nullable = column
        .get("nullable")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let background = if index.is_multiple_of(2) {
        theme.row_odd
    } else {
        theme.row_even
    };

    div()
        .w_full()
        .min_h(px(34.0))
        .px(px(8.0))
        .py(px(8.0))
        .gap_2()
        .flex()
        .items_center()
        .justify_between()
        .bg(background)
        .border_b_1()
        .border_color(theme.row_border)
        .child(render_column_name(name, is_pk, is_fk, theme))
        .child(render_column_type(&ty, is_pk, is_fk, nullable, theme))
        .into_any()
}

fn render_column_name(name: String, is_pk: bool, is_fk: bool, theme: &ErCardTheme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_1()
        .min_w_0()
        .child(status_dot(is_pk, is_fk, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.column_text)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name),
        )
        .into_any()
}

fn status_dot(is_pk: bool, is_fk: bool, theme: &ErCardTheme) -> AnyElement {
    let color = if is_pk {
        theme.badge_primary
    } else if is_fk {
        theme.badge_secondary
    } else {
        theme.field_dot
    };

    div()
        .w(px(10.0))
        .h(px(10.0))
        .rounded_full()
        .flex_shrink_0()
        .bg(color)
        .into_any()
}

fn render_column_type(
    ty: &str,
    is_pk: bool,
    is_fk: bool,
    nullable: bool,
    theme: &ErCardTheme,
) -> AnyElement {
    div()
        .max_w(px(132.0))
        .ml_2()
        .flex()
        .items_center()
        .justify_end()
        .gap_1()
        .flex_shrink_0()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.type_text)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .when(is_pk, |this| {
            this.child(type_badge("PK", theme.badge_primary))
        })
        .when(is_fk && !is_pk, |this| {
            this.child(type_badge("FK", theme.badge_secondary))
        })
        .child(ty.to_string())
        .when(nullable, |this| {
            this.child(div().text_color(theme.null_text).child("null"))
        })
        .into_any()
}

fn type_badge(label: &'static str, color: Hsla) -> AnyElement {
    div()
        .px(px(3.0))
        .rounded(px(3.0))
        .text_xs()
        .text_color(color)
        .border_1()
        .border_color(color)
        .child(label)
        .into_any()
}

fn text_value(node: &Node, key: &str) -> Option<String> {
    node.data_ref()
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn json_text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}
