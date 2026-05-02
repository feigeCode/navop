use ferrum_flow::{FlowCanvas, Graph};
use gpui::{AppContext as _, Application, WindowOptions};
use serde_json::json;

fn main() {
    Application::new().run(|cx| {
        let mut graph = Graph::new();

        graph
            .create_node("default")
            .position(100.0, 100.0)
            .data(json!({ "label": "Hello World" }))
            .build();

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .default_plugins()
                    .build()
            })
        })
        .unwrap();
    });
}
