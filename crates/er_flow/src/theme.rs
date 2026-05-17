use ferrum_flow::FlowTheme;
use gpui::Hsla;
use gpui_component::{Colorize as _, Theme};

pub fn er_flow_theme() -> FlowTheme {
    let mut theme = FlowTheme::light();
    theme.node_card_background = 0x00ffffff;
    theme.node_card_border = 0x0031485f;
    theme.node_caption_text = 0x00182739;
    theme.default_port_fill = 0x000f766e;
    theme.background = 0x00f8fafc;
    theme.background_grid_dot = 0x00cbd5e1;
    theme.edge_stroke = 0x0064748b;
    theme.minimap_node_stroke = 0x0031485f;
    theme
}

/// 从应用的 GPUI 主题映射创建 FlowTheme，使画布自适应亮/暗模式。
pub fn er_flow_theme_from_ui(ui: &Theme) -> FlowTheme {
    let grid_dot = blend_to_u32(ui.background, ui.muted_foreground, 0.78);
    let muted_line = blend_to_u32(ui.background, ui.muted_foreground, 0.45);
    let minimap_node_fill = blend_to_u32(ui.background, ui.muted, 0.5);

    let mut theme = FlowTheme::light();
    theme.node_card_background = hsla_to_u32(ui.background);
    theme.node_card_border = hsla_to_u32(ui.border);
    theme.node_card_border_selected = hsla_to_u32(ui.primary);
    theme.undefined_node_background = hsla_to_u32(ui.muted);
    theme.undefined_node_border = hsla_to_u32(ui.border);
    theme.node_caption_text = hsla_to_u32(ui.foreground);
    theme.undefined_node_caption_text = hsla_to_u32(ui.muted_foreground);
    theme.default_port_fill = hsla_to_u32(ui.primary);
    theme.background = hsla_to_u32(ui.background);
    theme.background_grid_dot = grid_dot;
    theme.edge_stroke = muted_line;
    theme.edge_stroke_selected = hsla_to_u32(ui.primary);
    theme.selection_rect_border = hsla_to_u32(ui.primary);
    theme.selection_rect_fill_rgba = hsla_to_u32_rgba(Hsla {
        a: 0.3,
        ..ui.primary
    });
    theme.snap_guide_stroke = hsla_to_u32(ui.primary);
    theme.port_preview_line = hsla_to_u32(ui.primary);
    theme.port_preview_dot = hsla_to_u32(ui.muted_foreground);
    theme.minimap_background = hsla_to_u32(ui.popover);
    theme.minimap_border = hsla_to_u32(ui.border);
    theme.minimap_edge = muted_line;
    theme.minimap_node_fill = minimap_node_fill;
    theme.minimap_node_stroke = hsla_to_u32(ui.border);
    theme.minimap_viewport_stroke = hsla_to_u32(ui.primary);
    theme.zoom_controls_background = hsla_to_u32(ui.popover);
    theme.zoom_controls_border = hsla_to_u32(ui.border);
    theme.zoom_controls_text = hsla_to_u32(ui.foreground);
    theme.context_menu_background = hsla_to_u32(ui.popover);
    theme.context_menu_border = hsla_to_u32(ui.border);
    theme.context_menu_text = hsla_to_u32(ui.foreground);
    theme.context_menu_shortcut_text = hsla_to_u32(ui.muted_foreground);
    theme.context_menu_separator = hsla_to_u32(ui.border);
    theme.error = hsla_to_u32(ui.danger);
    theme.info = hsla_to_u32(ui.info);
    theme.success = hsla_to_u32(ui.success);
    theme.warning = hsla_to_u32(ui.warning);
    theme
}

fn blend_to_u32(background: Hsla, foreground: Hsla, background_weight: f32) -> u32 {
    hsla_to_u32(background.mix(foreground, background_weight))
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
    use gpui_component::ThemeColor;

    #[test]
    fn creates_er_theme() {
        let theme = er_flow_theme();

        assert_eq!(theme.default_port_fill, 0x000f766e);
        assert_eq!(theme.background, 0x00f8fafc);
    }

    #[test]
    fn ui_theme_mapping_uses_current_background_and_semantic_colors() {
        let ui = Theme::from(ThemeColor::dark().as_ref());
        let theme = er_flow_theme_from_ui(&ui);

        assert_eq!(theme.background, hsla_to_u32(ui.background));
        assert_eq!(theme.error, hsla_to_u32(ui.danger));
        assert_eq!(theme.info, hsla_to_u32(ui.info));
        assert_eq!(theme.success, hsla_to_u32(ui.success));
        assert_eq!(theme.warning, hsla_to_u32(ui.warning));
        assert_ne!(theme.background_grid_dot, hsla_to_u32(ui.muted_foreground));
    }
}
