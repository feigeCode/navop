use ferrum_flow::*;
use gpui::{
    AnyElement, AppContext as _, Application, Element as _, ParentElement as _, Styled,
    WindowOptions, div, px, rgb, white,
};
use serde_json::json;

/// A beginner-friendly custom plugin example.
///
/// Run with:
/// `cargo run -p ferrum-flow --example plugin`
fn main() {
    Application::new().run(|cx| {
        let mut graph = Graph::new();

        graph
            .create_node("default")
            .position(120.0, 120.0)
            .input()
            .output()
            .data(json!({ "label": "Base Node" }))
            .build();

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .default_plugins()
                    .plugin(StarterPlugin::new())
                    .plugin(ToastPlugin::new())
                    .build()
            })
        })
        .unwrap();
    });
}

/// A tiny plugin that demonstrates:
/// 1) plugin state
/// 2) input handling
/// 3) custom overlay rendering
/// 4) mutating graph data through `PluginContext`
struct StarterPlugin {
    next_index: usize,
    clicks: usize,
    show_hud: bool,
}

impl StarterPlugin {
    fn new() -> Self {
        Self {
            next_index: 1,
            clicks: 0,
            show_hud: true,
        }
    }

    fn add_demo_node(&mut self, ctx: &mut PluginContext) {
        let i = self.next_index;
        self.next_index += 1;

        let x = 120.0 + ((i % 6) as f32) * 180.0;
        let y = 280.0 + ((i / 6) as f32) * 140.0;

        ctx.create_node("default")
            .position(x, y)
            .input()
            .output()
            .data(json!({ "label": format!("Plugin Node {i}") }))
            .build();

        ctx.emit(FlowEvent::custom(ToastMessage::success(format!(
            "Created node #{i} from StarterPlugin"
        ))));
    }
}

impl Plugin for StarterPlugin {
    fn name(&self) -> &'static str {
        "starter_plugin"
    }

    fn setup(&mut self, _ctx: &mut InitPluginContext) {
        // Put one initial node index behind the first generated node label.
        self.next_index = 1;
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::KeyDown(ev)) = event {
            if ev.keystroke.key == "n" {
                self.add_demo_node(ctx);
                return EventResult::Stop;
            }
            if ev.keystroke.key == "h" {
                self.show_hud = !self.show_hud;
                ctx.notify();
                return EventResult::Stop;
            }
        }

        if let FlowEvent::Input(InputEvent::MouseDown(_)) = event {
            self.clicks += 1;
            ctx.notify();
        }

        EventResult::Continue
    }

    fn render(&mut self, _ctx: &mut RenderContext) -> Option<AnyElement> {
        if !self.show_hud {
            return None;
        }

        Some(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(12.0))
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(rgb(0x001F2937))
                .text_color(white())
                .child(div().text_sm().child("StarterPlugin (custom example)"))
                .child(
                    div()
                        .text_sm()
                        .child(format!("Mouse clicks: {}", self.clicks)),
                )
                .child(div().text_sm().child("Press N: create node"))
                .child(div().text_sm().child("Press H: hide/show this panel"))
                .into_any(),
        )
    }

    fn priority(&self) -> i32 {
        120
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }
}
