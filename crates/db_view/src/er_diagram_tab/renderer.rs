use er_flow::{Node, NodeCardVariant, NodeRenderer, Port, RenderContext};
use gpui::{
    AnyElement, Element as _, ParentElement as _, Styled as _, div, prelude::FluentBuilder, px, rgb,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _};

const CARD_BACKGROUND: u32 = 0xffffff;
const CARD_BORDER: u32 = 0x60a5fa;
const CARD_BORDER_SELECTED: u32 = 0x2563eb;
const STRIPE_BACKGROUND: u32 = 0xe6f4ff;
const HEADER_BACKGROUND: u32 = 0xe4e4e7;
const HEADER_TEXT: u32 = 0x111827;
const HEADER_BORDER: u32 = 0xbfdbfe;
const ROW_BACKGROUND_ODD: u32 = 0xf9fafb;
const ROW_BACKGROUND_EVEN: u32 = 0xffffff;
const ROW_BORDER: u32 = 0xe5e7eb;
const COLUMN_TEXT: u32 = 0x111827;
const TYPE_TEXT: u32 = 0x6b7280;
const NULL_TEXT: u32 = 0x9ca3af;
const BLUE_ACCENT: u32 = 0x3b82f6;
const GREEN_ACCENT: u32 = 0x22c55e;
const FIELD_DOT: u32 = 0x9ca3af;
const PORT_FILL: u32 = 0x2563eb;

pub(crate) struct ErTableRenderer;

impl NodeRenderer for ErTableRenderer {
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement {
        let selected = ctx.graph.selected_node.contains(&node.id);
        ctx.node_card_shell(node, selected, NodeCardVariant::Custom)
            .bg(rgb(CARD_BACKGROUND))
            .border_color(rgb(if selected {
                CARD_BORDER_SELECTED
            } else {
                CARD_BORDER
            }))
            .shadow_md()
            .overflow_hidden()
            .child(render_table(node))
            .into_any()
    }

    fn port_render(&self, node: &Node, port: &Port, ctx: &mut RenderContext) -> Option<AnyElement> {
        let frame = ctx.port_screen_frame(node, port)?;
        Some(
            frame
                .anchor_div()
                .rounded_full()
                .bg(rgb(PORT_FILL))
                .into_any(),
        )
    }
}

fn render_table(node: &Node) -> AnyElement {
    let columns = node
        .data
        .get("columns")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(div().w_full().h(px(10.0)).bg(rgb(STRIPE_BACKGROUND)))
        .child(render_header(node))
        .children(
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| render_column(column, index)),
        )
        .into_any()
}

fn render_header(node: &Node) -> AnyElement {
    let table = text_value(node, "table").unwrap_or_else(|| "table".to_string());
    let schema = text_value(node, "schema");

    div()
        .w_full()
        .h(px(48.0))
        .px(px(8.0))
        .bg(rgb(HEADER_BACKGROUND))
        .border_b_1()
        .border_color(rgb(HEADER_BORDER))
        .flex()
        .items_center()
        .justify_between()
        .child(header_title(table))
        .when_some(schema, |this, schema| this.child(header_schema(schema)))
        .into_any()
}

fn header_title(table: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(
            Icon::new(IconName::Database)
                .with_size(Size::Small)
                .text_color(rgb(BLUE_ACCENT)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_lg()
                .font_medium()
                .text_color(rgb(HEADER_TEXT))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(table),
        )
        .into_any()
}

fn header_schema(schema: String) -> AnyElement {
    div()
        .text_xs()
        .text_color(rgb(TYPE_TEXT))
        .max_w(px(92.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(schema)
        .into_any()
}

fn render_column(column: &serde_json::Value, index: usize) -> AnyElement {
    let name = json_text(column, "name");
    let ty = json_text(column, "ty");
    let is_pk = column.get("pk").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_fk = column.get("fk").and_then(|v| v.as_bool()).unwrap_or(false);
    let nullable = column
        .get("nullable")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let background = if index % 2 == 0 {
        ROW_BACKGROUND_ODD
    } else {
        ROW_BACKGROUND_EVEN
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
        .bg(rgb(background))
        .border_b_1()
        .border_color(rgb(ROW_BORDER))
        .child(render_column_name(name, is_pk, is_fk))
        .child(render_column_type(&ty, is_pk, is_fk, nullable))
        .into_any()
}

fn render_column_name(name: String, is_pk: bool, is_fk: bool) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_1()
        .min_w_0()
        .child(status_dot(is_pk, is_fk))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .font_medium()
                .text_color(rgb(COLUMN_TEXT))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name),
        )
        .into_any()
}

fn status_dot(is_pk: bool, is_fk: bool) -> AnyElement {
    let color = if is_pk {
        BLUE_ACCENT
    } else if is_fk {
        GREEN_ACCENT
    } else {
        FIELD_DOT
    };
    div()
        .w(px(10.0))
        .h(px(10.0))
        .rounded_full()
        .flex_shrink_0()
        .bg(rgb(color))
        .into_any()
}

fn render_column_type(ty: &str, is_pk: bool, is_fk: bool, nullable: bool) -> AnyElement {
    div()
        .max_w(px(132.0))
        .ml_2()
        .flex()
        .items_center()
        .justify_end()
        .gap_1()
        .flex_shrink_0()
        .text_xs()
        .font_medium()
        .text_color(rgb(TYPE_TEXT))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .when(is_pk, |this| {
            this.child(
                Icon::new(IconName::Key)
                    .with_size(Size::XSmall)
                    .text_color(rgb(BLUE_ACCENT)),
            )
        })
        .when(is_fk && !is_pk, |this| {
            this.child(
                Icon::new(IconName::Network)
                    .with_size(Size::XSmall)
                    .text_color(rgb(GREEN_ACCENT)),
            )
        })
        .child(ty.to_string())
        .when(nullable, |this| {
            this.child(div().text_color(rgb(NULL_TEXT)).child("null"))
        })
        .into_any()
}

fn text_value(node: &Node, key: &str) -> Option<String> {
    node.data
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
