use declarative_ui_demo::{
    CompileOptions, ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer,
    ComponentResult, DeclarativeView, DeclarativeViewConfig, DiagnosticCode, DiagnosticPhase,
    DiagnosticSeverity, Diagnostics, NodePath, RenderContext, Runtime, compile_template,
};
use gpui::{AppContext, TestAppContext, VisualTestContext, WindowOptions};
use gpui_component::Root;

struct ErrorComponent;

impl ComponentRenderer for ErrorComponent {
    fn render(&self, _props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new("renderer rejected its props"))
    }
}

struct PanickingComponent;

impl ComponentRenderer for PanickingComponent {
    fn render(&self, _props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        panic!("renderer exploded")
    }
}

#[gpui::test]
fn renderer_errors_become_typed_render_diagnostics(cx: &mut TestAppContext) {
    let mut registry = ComponentRegistry::with_defaults();
    registry
        .register("broken-widget", ErrorComponent)
        .expect("register component");

    let diagnostics = render_diagnostics("<div><broken-widget /></div>", registry, cx);
    let failure = diagnostic(
        &diagnostics,
        DiagnosticCode::ComponentRenderFailed,
        DiagnosticPhase::Render,
    );

    assert_eq!(DiagnosticSeverity::Error, failure.severity);
    assert_eq!(Some(NodePath(vec![0])), failure.path);
    assert!(failure.message.contains("renderer rejected its props"));
}

#[gpui::test]
fn renderer_panics_are_caught_at_the_component_boundary(cx: &mut TestAppContext) {
    let mut registry = ComponentRegistry::with_defaults();
    registry
        .register("panic-widget", PanickingComponent)
        .expect("register component");

    let diagnostics = render_diagnostics("<div><panic-widget /></div>", registry, cx);
    let failure = diagnostic(
        &diagnostics,
        DiagnosticCode::ComponentPanicked,
        DiagnosticPhase::Render,
    );

    assert_eq!(DiagnosticSeverity::Error, failure.severity);
    assert_eq!(Some(NodePath(vec![0])), failure.path);
    assert!(failure.message.contains("renderer exploded"));
}

#[gpui::test]
fn permissive_unknown_components_keep_compile_and_render_warnings(cx: &mut TestAppContext) {
    let diagnostics = render_diagnostics(
        "<div><unregistered-widget /></div>",
        ComponentRegistry::with_defaults(),
        cx,
    );

    for phase in [DiagnosticPhase::Compile, DiagnosticPhase::Render] {
        let warning = diagnostic(&diagnostics, DiagnosticCode::UnknownTag, phase);
        assert_eq!(DiagnosticSeverity::Warning, warning.severity);
        assert_eq!(Some(NodePath(vec![0])), warning.path);
    }
}

fn render_diagnostics(
    source: &str,
    registry: ComponentRegistry,
    cx: &mut TestAppContext,
) -> Diagnostics {
    cx.update(gpui_component::init);
    let template = compile_template(source, &registry, CompileOptions::permissive())
        .expect("permissive template compilation");
    let (window, view) = cx.update(|cx| {
        let runtime = cx.new(|_| Runtime::default());
        let mut mounted_view = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| {
                    DeclarativeView::new(
                        DeclarativeViewConfig::new(template, runtime, registry),
                        cx,
                    )
                });
                mounted_view = Some(view.clone());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("test window opens");
        (window, mounted_view.expect("view is mounted"))
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    view.read_with(&visual, |view, _| view.diagnostics().clone())
}

fn diagnostic(
    diagnostics: &Diagnostics,
    code: DiagnosticCode,
    phase: DiagnosticPhase,
) -> &declarative_ui_demo::Diagnostic {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code && diagnostic.phase == phase)
        .expect("expected diagnostic")
}
