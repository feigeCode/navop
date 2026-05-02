use ferrum_flow::*;
use gpui::{AppContext as _, Application, WindowOptions};
use serde_json::json;

fn main() {
    Application::new().run(|cx| {
        let mut graph = Graph::new();

        for j in 0..100 {
            for i in 0..100 {
                graph
                    .create_node("")
                    .position(200.0 * i as f32, 200.0 * j as f32)
                    .input()
                    .output()
                    .data(json!({ "label": format!("Node {}", i * 100 + j) }))
                    .build();
            }
        }

        let node_ids = graph.nodes().keys().copied().collect::<Vec<_>>();

        generate_chain_edges(&mut graph, node_ids);

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .plugin(BackgroundPlugin::new())
                    .plugin(SelectionPlugin::new())
                    .plugin(NodeInteractionPlugin::new())
                    .plugin(ViewportPlugin::new())
                    .plugin(NodePlugin::new())
                    .plugin(PortInteractionPlugin::new())
                    .plugin(EdgePlugin::new())
                    .plugin(DeletePlugin::new())
                    .plugin(HistoryPlugin::new())
                    .plugin(MinimapPlugin::new())
                    .plugin(ClipboardPlugin::new())
                    .plugin(ContextMenuPlugin::new())
                    .plugin(SelectAllViewportPlugin::new())
                    .plugin(AlignPlugin::new())
                    .plugin(FocusSelectionPlugin::new())
                    .plugin(ZoomControlsPlugin::new())
                    .plugin(SnapGuidesPlugin::new())
                    .build()
            })
        })
        .unwrap();
    });
}

pub fn generate_chain_edges(graph: &mut Graph, node_ids: Vec<NodeId>) {
    for window in node_ids.windows(2) {
        let from = window[0];
        let to = window[1];

        let from_node = graph.get_node(&from).unwrap();
        let to_node = graph.get_node(&to).unwrap();

        let source_port = from_node.outputs()[0];
        let target_port = to_node.inputs()[0];

        graph
            .create_edge()
            .source(source_port)
            .target(target_port)
            .build();
    }
}
