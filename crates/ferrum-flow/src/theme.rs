//! Canvas-wide visual tokens. Plugins can replace or tweak values in
//! [`crate::plugin::InitPluginContext::theme`] / [`crate::plugin::PluginContext::theme`].
//!
//! Colors are `u32` in **GPUI `rgb` / `rgba` layout**: `0x00RRGGBB` for opaque colors
//! (first byte unused by [`gpui::rgb`]), and `0xRRGGBBAA` for [`gpui::rgba`] fills.

/// Default canvas chrome: node cards, grid, edges, selection marquee.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowTheme {
    /// Default node card background ([`gpui::rgb`]).
    pub node_card_background: u32,
    /// Default node card border when not selected.
    pub node_card_border: u32,
    /// Default node card border when selected.
    pub node_card_border_selected: u32,

    /// Unknown node type card background.
    pub undefined_node_background: u32,
    /// Unknown node type card border.
    pub undefined_node_border: u32,

    /// Primary label on default node cards.
    pub node_caption_text: u32,
    /// Label on undefined-type node cards.
    pub undefined_node_caption_text: u32,

    /// Default circular port fill ([`NodeRenderer::port_render`](crate::NodeRenderer::port_render);
    /// layout via [`crate::plugin::RenderContext::port_screen_frame`]).
    pub default_port_fill: u32,

    /// Main surface color behind the dot grid.
    pub background: u32,
    /// Dot color for the background grid.
    pub background_grid_dot: u32,

    /// Edge curve when not selected.
    pub edge_stroke: u32,
    /// Edge curve when selected.
    pub edge_stroke_selected: u32,

    /// Marquee / move-preview rectangle outline ([`gpui::rgb`]).
    pub selection_rect_border: u32,
    /// Marquee / move-preview fill ([`gpui::rgba`], e.g. `0x78A0FF4c`).
    pub selection_rect_fill_rgba: u32,

    /// Temporary line while dragging a link from a port.
    pub port_preview_line: u32,
    /// Endpoint disc while dragging a link from a port (muted so it does not overpower the canvas).
    pub port_preview_dot: u32,

    /// Minimap inner panel fill ([`crate::MinimapPlugin`]).
    pub minimap_background: u32,
    /// Minimap inner panel outline.
    pub minimap_border: u32,
    /// Minimap graph edges (straight segments between node centers).
    pub minimap_edge: u32,
    /// Minimap node rectangle fill.
    pub minimap_node_fill: u32,
    /// Minimap node rectangle outline.
    pub minimap_node_stroke: u32,
    /// Minimap viewport / visible-area frame.
    pub minimap_viewport_stroke: u32,

    /// Zoom bar button fill ([`crate::ZoomControlsPlugin`]).
    pub zoom_controls_background: u32,
    /// Zoom bar button border.
    pub zoom_controls_border: u32,
    /// Zoom bar glyph color.
    pub zoom_controls_text: u32,

    /// Context menu panel fill ([`crate::ContextMenuPlugin`]).
    pub context_menu_background: u32,
    /// Context menu panel outline.
    pub context_menu_border: u32,
    /// Context menu row label.
    pub context_menu_text: u32,
    /// Context menu shortcut hint (muted).
    pub context_menu_shortcut_text: u32,
    /// Context menu separator rule between rows.
    pub context_menu_separator: u32,
}

impl Default for FlowTheme {
    #[allow(clippy::mixed_case_hex_literals)]
    fn default() -> Self {
        Self {
            node_card_background: 0x00FFFFFF,
            node_card_border: 0x001A192B,
            node_card_border_selected: 0x00FF7800,
            undefined_node_background: 0x00F5F5F5,
            undefined_node_border: 0x00FF9800,
            node_caption_text: 0x001A192B,
            undefined_node_caption_text: 0x005F6368,
            default_port_fill: 0x001A192B,
            background: 0x00f8f9fb,
            background_grid_dot: 0x009F9FA7,
            edge_stroke: 0x00b1b1b8,
            edge_stroke_selected: 0x00FF7800,
            selection_rect_border: 0x0078A0FF,
            selection_rect_fill_rgba: 0x78A0FF4c,
            port_preview_line: 0x00b1b1b8,
            port_preview_dot: 0x007189a3,
            minimap_background: 0x00f8f9fb,
            minimap_border: 0x00b1b1b8,
            minimap_edge: 0x00b1b1b8,
            minimap_node_fill: 0x00FFFFFF,
            minimap_node_stroke: 0x001a192b,
            minimap_viewport_stroke: 0x0078a0ff,
            zoom_controls_background: 0x00fcfcfc,
            zoom_controls_border: 0x00c8c8d0,
            zoom_controls_text: 0x001a192b,
            context_menu_background: 0x00fcfcfc,
            context_menu_border: 0x00c8c8d0,
            context_menu_text: 0x001a192b,
            context_menu_shortcut_text: 0x007a7a88,
            context_menu_separator: 0x00e0e0e8,
        }
    }
}

impl FlowTheme {
    /// Same as [`Default::default`]; kept for explicit call sites.
    pub fn light() -> Self {
        Self::default()
    }
}
