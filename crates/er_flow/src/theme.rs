use ferrum_flow::FlowTheme;

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
