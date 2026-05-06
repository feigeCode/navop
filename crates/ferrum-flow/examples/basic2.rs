use ferrum_flow::*;
use gpui::{AppContext as _, Application, Size, WindowOptions, px};
use serde_json::json;

fn main() {
    Application::new().run(|cx| {
        let mut graph = Graph::new();

        graph
            .create_node("")
            .position(100.0, 100.0)
            .output()
            .output()
            .output_with(PortPosition::Bottom, Size::new(px(20.0), px(20.0)))
            .output_at(PortPosition::Bottom)
            .data(json!({ "label": "Node 1" }))
            .build();

        graph
            .create_node("")
            .position(300.0, 400.0)
            .input()
            .input_at(PortPosition::Top)
            .input_at(PortPosition::Top)
            .output()
            .output_at(PortPosition::Bottom)
            .output_at(PortPosition::Bottom)
            .data(json!({ "label": "Node 2" }))
            .build();

        graph
            .create_node("")
            .position(500.0, 500.0)
            .input()
            .output()
            .data(json!({ "label": "Node 3" }))
            .build();

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .default_plugins()
                    .plugin(MinimapPlugin::new())
                    .plugin(ZoomControlsPlugin::new())
                    .plugin(ClipboardPlugin::new())
                    .plugin(ContextMenuPlugin::new())
                    .plugin(SelectAllViewportPlugin::new())
                    .plugin(AlignPlugin::new())
                    .plugin(FocusSelectionPlugin::new())
                    .plugin(FitAllGraphPlugin::new())
                    .plugin(SnapGuidesPlugin::new())
                    .plugin(ToastPlugin::new())
                    .build()
            })
        })
        .unwrap();
    });
}
