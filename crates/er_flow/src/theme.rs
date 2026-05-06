use ferrum_flow::FlowTheme;
use gpui::Hsla;
use gpui_component::Theme;

pub fn er_flow_theme() -> FlowTheme {
    FlowTheme {
        node_card_background: 0x00ffffff,
        node_card_border: 0x0031485f,
        node_caption_text: 0x00182739,
        default_port_fill: 0x000f766e,
        background: 0x00f8fafc,
        background_grid_dot: 0x00cbd5e1,
        edge_stroke: 0x0064748b,
        minimap_node_stroke: 0x0031485f,
        ..FlowTheme::light()
    }
}

/// 从应用的 GPUI 主题映射创建 FlowTheme，使画布自适应亮/暗模式。
pub fn er_flow_theme_from_ui(ui: &Theme) -> FlowTheme {
    FlowTheme {
        node_card_background: hsla_to_u32(ui.background),
        node_card_border: hsla_to_u32(ui.border),
        node_card_border_selected: hsla_to_u32(ui.primary),
        undefined_node_background: hsla_to_u32(ui.muted),
        undefined_node_border: hsla_to_u32(ui.border),
        node_caption_text: hsla_to_u32(ui.foreground),
        undefined_node_caption_text: hsla_to_u32(ui.muted_foreground),
        default_port_fill: hsla_to_u32(ui.primary),
        background: hsla_to_u32(ui.background),
        background_grid_dot: hsla_to_u32(ui.muted_foreground.opacity(0.25)),
        edge_stroke: hsla_to_u32(ui.border),
        edge_stroke_selected: hsla_to_u32(ui.primary),
        selection_rect_border: hsla_to_u32(ui.primary),
        selection_rect_fill_rgba: hsla_to_u32_rgba(Hsla {
            a: 0.3,
            ..ui.primary
        }),
        port_preview_line: hsla_to_u32(ui.border),
        port_preview_dot: hsla_to_u32(ui.muted_foreground),
        minimap_background: hsla_to_u32(ui.popover),
        minimap_border: hsla_to_u32(ui.border),
        minimap_edge: hsla_to_u32(ui.muted_foreground),
        minimap_node_fill: hsla_to_u32(ui.background),
        minimap_node_stroke: hsla_to_u32(ui.border),
        minimap_viewport_stroke: hsla_to_u32(ui.primary),
        zoom_controls_background: hsla_to_u32(ui.popover),
        zoom_controls_border: hsla_to_u32(ui.border),
        zoom_controls_text: hsla_to_u32(ui.foreground),
        context_menu_background: hsla_to_u32(ui.popover),
        context_menu_border: hsla_to_u32(ui.border),
        context_menu_text: hsla_to_u32(ui.foreground),
        context_menu_shortcut_text: hsla_to_u32(ui.muted_foreground),
        context_menu_separator: hsla_to_u32(ui.border),
    }
}

/// 将 GPUI Hsla 颜色转换为 FlowTheme 使用的 `0x00RRGGBB` 格式。
fn hsla_to_u32(color: Hsla) -> u32 {
    let rgb = color.to_rgb();
    let r = (rgb.r * 255.0) as u32;
    let g = (rgb.g * 255.0) as u32;
    let b = (rgb.b * 255.0) as u32;
    (r << 16) | (g << 8) | b
}

/// 将 GPUI Hsla 颜色转换为 FlowTheme 使用的 `0xRRGGBBAA` 格式（含 alpha）。
fn hsla_to_u32_rgba(color: Hsla) -> u32 {
    let rgb = color.to_rgb();
    let r = (rgb.r * 255.0) as u32;
    let g = (rgb.g * 255.0) as u32;
    let b = (rgb.b * 255.0) as u32;
    let a = (rgb.a * 255.0) as u32;
    (r << 24) | (g << 16) | (b << 8) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_er_theme() {
        let theme = er_flow_theme();

        assert_eq!(theme.default_port_fill, 0x000f766e);
        assert_eq!(theme.background, 0x00f8fafc);
    }
}
