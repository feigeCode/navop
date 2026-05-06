//! Custom canvas chrome via [`InitPluginContext::theme`] in [`Plugin::setup`].
use ferrum_flow::*;
use gpui::{AppContext as _, Application, WindowOptions};
use serde_json::json;

struct DarkGridThemePlugin;

impl Plugin for DarkGridThemePlugin {
    fn name(&self) -> &'static str {
        "dark_grid_theme"
    }

    fn setup(&mut self, ctx: &mut InitPluginContext) {
        ctx.theme.background = 0x001a1d2a;
        ctx.theme.background_grid_dot = 0x003d4559;
        ctx.theme.node_card_background = 0x0024283a;
        ctx.theme.node_card_border = 0x004a5568;
        ctx.theme.node_card_border_selected = 0x00f5a524;
        ctx.theme.node_caption_text = 0x00e8eaef;
        ctx.theme.default_port_fill = 0x004a5568;
        ctx.theme.undefined_node_background = 0x00303845;
        ctx.theme.undefined_node_border = 0x00f5a524;
        ctx.theme.undefined_node_caption_text = 0x00b8bcc8;
        ctx.theme.edge_stroke = 0x0050586b;
        ctx.theme.edge_stroke_selected = 0x00f5a524;
        ctx.theme.selection_rect_border = 0x006b8cff;
        ctx.theme.selection_rect_fill_rgba = 0x6b8cff33;
        ctx.theme.port_preview_line = 0x0050586b;
        ctx.theme.port_preview_dot = 0x0060809e;
        ctx.theme.minimap_background = 0x0018202e;
        ctx.theme.minimap_border = 0x004a5568;
        ctx.theme.minimap_edge = 0x0050586b;
        ctx.theme.minimap_node_fill = 0x0024283a;
        ctx.theme.minimap_node_stroke = 0x00607080;
        ctx.theme.minimap_viewport_stroke = 0x006b8cff;
        ctx.theme.zoom_controls_background = 0x0024283a;
        ctx.theme.zoom_controls_border = 0x004a5568;
        ctx.theme.zoom_controls_text = 0x00e8eaef;
        ctx.theme.context_menu_background = 0x0024283a;
        ctx.theme.context_menu_border = 0x004a5568;
        ctx.theme.context_menu_text = 0x00e8eaef;
        ctx.theme.context_menu_shortcut_text = 0x009098a8;
        ctx.theme.context_menu_separator = 0x003d4559;
    }
}

fn main() {
    Application::new().run(|cx| {
        let mut graph = Graph::new();

        graph
            .create_node("")
            .position(100.0, 100.0)
            .output()
            .output()
            .data(json!({ "label": "Themed" }))
            .build();

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .plugin(DarkGridThemePlugin)
                    .plugin(MinimapPlugin::new())
                    .plugin(SelectionPlugin::new())
                    .plugin(NodeInteractionPlugin::new())
                    .plugin(ViewportPlugin::new())
                    .plugin(ZoomControlsPlugin::new())
                    .plugin(BackgroundPlugin::new())
                    .plugin(NodePlugin::new())
                    .plugin(PortInteractionPlugin::new())
                    .plugin(EdgePlugin::new())
                    .plugin(ClipboardPlugin::new())
                    .plugin(ContextMenuPlugin::new())
                    .plugin(SelectAllViewportPlugin::new())
                    .plugin(AlignPlugin::new())
                    .plugin(FocusSelectionPlugin::new())
                    .plugin(FitAllGraphPlugin::new())
                    .plugin(DeletePlugin::new())
                    .plugin(HistoryPlugin::new())
                    .build()
            })
        })
        .unwrap();
    });
}
