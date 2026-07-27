use declarative_ui_demo::{
    CompileOptions, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    DeclarativeView, DeclarativeViewConfig, RenderContext, Runtime, StateStore, compile_template,
};
use gpui::{
    AppContext, Bounds, IntoElement, ParentElement, QuitMode, Styled, TitlebarOptions,
    WindowBounds, WindowOptions, div, px, rgb, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;

const WINDOW_WIDTH_PX: f32 = 900.0;
const WINDOW_HEIGHT_PX: f32 = 700.0;
const WINDOW_MIN_WIDTH_PX: f32 = 640.0;
const WINDOW_MIN_HEIGHT_PX: f32 = 480.0;
const SQL_EDITOR_GAP_PX: f32 = 8.0;
const MUTED_TEXT_RGB: u32 = 0xa1_a1_aa;
const PRIMARY_TEXT_RGB: u32 = 0xf4_f4_f5;

const DEMO_HTML: &str = r#"
<div class="flex flex-col gap-4 p-4 h-full bg-zinc-950 text-zinc-100">
    <div class="flex flex-col gap-2">
        <span class="text-xl font-semibold">Declarative UI Standalone v1</span>
        <span class="text-sm text-zinc-400">
            Restricted HTML → VNode → Runtime → Native GPUI
        </span>
        <span class="text-sm text-zinc-400">
            No JavaScript · typed diagnostics · transactional actions
        </span>
    </div>

    <div class="flex flex-col gap-2 p-4 bg-zinc-900 border border-zinc-700 rounded-lg">
        <span class="text-sm text-zinc-400">当前状态</span>
        <span class="text-lg font-semibold" bind="status"></span>
    </div>

    <input
        id="username-input"
        bind="username"
        placeholder="GPUI TextInput"
        class="w-full"
    />

    <div class="flex items-center justify-between gap-2">
        <span class="text-sm text-zinc-400" bind="save_count"></span>
        <button
            id="save-button"
            action="save"
            data-record="profile"
            class="bg-blue-600 text-white rounded-md px-4 py-2"
        >
            保存
        </button>
    </div>

    <sql-editor
        id="sql-editor"
        class="flex-1 min-h-0 p-4 bg-zinc-900 rounded-lg border border-zinc-700"
    />
</div>
"#;

fn main() {
    let mut registry = ComponentRegistry::with_defaults();
    if let Err(error) = registry.register("sql-editor", SqlEditorComponent) {
        eprintln!("failed to register demo component: {error}");
        return;
    }
    let template = match compile_template(DEMO_HTML, &registry, CompileOptions::strict()) {
        Ok(template) => template,
        Err(error) => {
            eprintln!("failed to compile demo template: {error}");
            return;
        }
    };

    gpui_platform::application()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_component::init(cx);
            let runtime = cx.new(|_| demo_runtime());
            let bounds =
                Bounds::centered(None, size(px(WINDOW_WIDTH_PX), px(WINDOW_HEIGHT_PX)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Declarative UI Standalone v1".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(WINDOW_MIN_WIDTH_PX), px(WINDOW_MIN_HEIGHT_PX))),
                ..Default::default()
            };

            if let Err(error) = cx.open_window(options, move |window, cx| {
                window.activate_window();
                let view = cx.new(|cx| {
                    let config = DeclarativeViewConfig::new(
                        template.clone(),
                        runtime.clone(),
                        registry.clone(),
                    );
                    DeclarativeView::new(config, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            }) {
                eprintln!("failed to open declarative UI demo window: {error}");
            }
        });
}

fn demo_runtime() -> Runtime {
    let mut state = StateStore::default();
    state.set("username", "admin");
    state.set("status", "等待保存");
    state.set("save_count_value", "0");
    state.set("save_count", "保存次数: 0");
    let mut runtime = Runtime::new(state);
    runtime
        .on("save", |context| {
            let username = context.get("username").unwrap_or_default().to_owned();
            let count = context
                .get("save_count_value")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_default()
                + 1;
            context.set("save_count_value", count.to_string());
            context.set("save_count", format!("保存次数: {count}"));
            context.set("status", format!("已保存用户: {username}"));
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
}

struct SqlEditorComponent;

impl ComponentRenderer for SqlEditorComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let element = div()
            .flex()
            .flex_col()
            .gap(px(SQL_EDITOR_GAP_PX))
            .child(div().text_color(rgb(MUTED_TEXT_RGB)).child("SQL Editor"))
            .child(
                div()
                    .text_color(rgb(PRIMARY_TEXT_RGB))
                    .child("SELECT * FROM connections;"),
            );
        Ok(context.style(element, &props).into_any_element())
    }
}
