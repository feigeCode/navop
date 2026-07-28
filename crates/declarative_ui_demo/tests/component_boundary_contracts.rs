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

#[gpui::test]
fn structured_builtin_components_render_without_crossing_the_error_boundary(
    cx: &mut TestAppContext,
) {
    let source = r#"
        <main class="flex flex-col gap-2">
            <form columns="2" size="sm">
                <field label="Name"><input value="admin" /></field>
                <field label="Controls">
                    <checkbox checked="true">Notify</checkbox>
                    <switch checked="false">Sync</switch>
                    <radio checked="true">Beta</radio>
                </field>
            </form>
            <table size="sm">
                <thead><tr><th>Name</th><th align="right">State</th></tr></thead>
                <tbody>
                    <tr>
                        <td>Primary</td>
                        <td align="right"><tag variant="success">Ready</tag></td>
                    </tr>
                </tbody>
                <tfoot><tr><td colspan="2">One row</td></tr></tfoot>
                <caption>Connections</caption>
            </table>
            <list>
                <list-item selected>Selected</list-item>
                <list-item confirmed>Confirmed</list-item>
                <list-item disabled>Disabled</list-item>
            </list>
            <alert variant="success" title="Status">Ready</alert>
            <badge count="3" max="9"><span>Saved</span></badge>
            <progress value="72.5" size="sm"></progress>
            <spinner size="xs"></spinner>
            <separator dashed label="Separator"></separator>
            <divider orientation="horizontal" label="Divider alias"></divider>
            <skeleton class="w-full"></skeleton>
            <avatar name="Ada Lovelace" size="sm"></avatar>
            <avatar-group limit="2" ellipsis size="sm">
                <avatar name="Ada Lovelace"></avatar>
                <avatar name="Grace Hopper"></avatar>
                <avatar name="Margaret Hamilton"></avatar>
            </avatar-group>
            <description-list columns="2" label-width="120" size="sm">
                <description-item label="Owner">Platform</description-item>
                <description-item label="State" span="2">
                    <tag variant="success">Ready</tag>
                </description-item>
            </description-list>
            <breadcrumb>
                <breadcrumb-item action="navigate" data-page="home">Home</breadcrumb-item>
                <breadcrumb-item disabled>Connections</breadcrumb-item>
            </breadcrumb>
            <pagination current-page="3" total-pages="20" visible-pages="7" size="sm"></pagination>
            <rating value="4" max="5" size="sm"></rating>
            <tabs selected-index="1" variant="underline" size="sm">
                <tab>Overview</tab>
                <tab>Activity</tab>
            </tabs>
            <stepper selected-index="1" size="sm">
                <stepper-item>Configure</stepper-item>
                <stepper-item>Review</stepper-item>
            </stepper>
            <div class="flex items-center gap-2">
                <kbd stroke="cmd-enter" appearance="true" outline></kbd>
                <slider
                    id="volume"
                    value="35"
                    min="0"
                    max="100"
                    step="1"
                    orientation="horizontal"
                    scale="linear"
                    class="w-full"
                ></slider>
            </div>
            <accordion
                id="settings"
                open-indices="[0,1,0]"
                multiple
                bordered="false"
                size="sm"
                class="w-full"
            >
                <accordion-item title="General" class="p-2">
                    <tag variant="info">General settings</tag>
                </accordion-item>
                <accordion-item title="Advanced">
                    <span>Advanced settings</span>
                </accordion-item>
            </accordion>
            <collapsible open class="gap-2">
                <span>Summary</span>
                <collapsible-content class="p-2">
                    <tag variant="info">Visible details</tag>
                </collapsible-content>
            </collapsible>
            <resizable id="layout" orientation="horizontal" size="260" class="w-full">
                <resizable-panel size="220" min-size="100" max-size="400">
                    <span>Navigation</span>
                </resizable-panel>
                <resizable-panel min-size="120">
                    <span>Content</span>
                </resizable-panel>
            </resizable>
            <scroll
                id="audit-log"
                axis="vertical"
                scrollbar-show="always"
                width="320"
                height="180"
                class="w-full"
            >
                <div class="flex flex-col gap-2">
                    <span>Connected to primary</span>
                    <span>Schema refreshed</span>
                </div>
            </scroll>
        </main>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    assert!(
        diagnostics.iter().all(|diagnostic| {
            !matches!(
                diagnostic.code,
                DiagnosticCode::UnknownTag
                    | DiagnosticCode::ComponentRenderFailed
                    | DiagnosticCode::ComponentPanicked
            )
        }),
        "built-in render diagnostics: {diagnostics:?}"
    );
}

#[gpui::test]
fn scroll_validates_axis_show_mode_and_viewport_dimensions_without_panicking(
    cx: &mut TestAppContext,
) {
    let source = r#"
        <div>
            <scroll id="invalid-axis" axis="diagonal"></scroll>
            <scroll id="invalid-show" scrollbar-show="sometimes"></scroll>
            <scroll id="empty-width" width=""></scroll>
            <scroll id="zero-width" width="0"></scroll>
            <scroll id="negative-width" width="-1"></scroll>
            <scroll id="nan-width" width="NaN"></scroll>
            <scroll id="infinite-height" height="inf"></scroll>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    let failures = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Render
                && diagnostic.code == DiagnosticCode::ComponentRenderFailed
        })
        .collect::<Vec<_>>();
    assert_eq!(7, failures.len(), "render diagnostics: {diagnostics:?}");

    for expected in [
        "axis",
        "scrollbar-show",
        "width",
        "height",
        "finite positive",
    ] {
        assert!(
            failures
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` in render diagnostics: {diagnostics:?}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != DiagnosticCode::ComponentPanicked),
        "invalid declarative input must not panic: {diagnostics:?}"
    );
}

#[gpui::test]
fn scroll_renders_all_supported_axes_and_show_modes(cx: &mut TestAppContext) {
    let source = r#"
        <div>
            <scroll id="vertical-default" width="120" height="80">
                <span>Default vertical and scrolling mode</span>
            </scroll>
            <scroll
                id="horizontal-hover"
                axis="horizontal"
                scrollbar-show="hover"
                width="120"
                height="80"
            >
                <span>Horizontal content</span>
            </scroll>
            <scroll
                id="both-always"
                axis="both"
                scrollbar-show="always"
                width="120"
                height="80"
            >
                <span>Two-dimensional content</span>
            </scroll>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    assert!(
        diagnostics.iter().all(|diagnostic| {
            !matches!(
                diagnostic.code,
                DiagnosticCode::UnknownTag
                    | DiagnosticCode::ComponentRenderFailed
                    | DiagnosticCode::ComponentPanicked
            )
        }),
        "scroll render diagnostics: {diagnostics:?}"
    );
}

#[gpui::test]
fn accordion_validates_structure_state_and_attributes_without_panicking(cx: &mut TestAppContext) {
    let source = r#"
        <div>
            <accordion></accordion>
            <accordion><span>Wrong child</span></accordion>
            <accordion-item title="Orphan">Orphan body</accordion-item>
            <accordion>
                <accordion-item title=" ">Empty title</accordion-item>
            </accordion>
            <accordion open-indices="not-json">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion open-indices="{}">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion open-indices="[-1]">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion open-indices="[2]" multiple>
                <accordion-item title="General">General</accordion-item>
                <accordion-item title="Advanced">Advanced</accordion-item>
            </accordion>
            <accordion open-indices="[0,1]">
                <accordion-item title="General">General</accordion-item>
                <accordion-item title="Advanced">Advanced</accordion-item>
            </accordion>
            <accordion multiple="sometimes">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion bordered="sometimes">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion disabled="sometimes">
                <accordion-item title="General">General</accordion-item>
            </accordion>
            <accordion size="huge">
                <accordion-item title="General">General</accordion-item>
            </accordion>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    let failures = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Render
                && diagnostic.code == DiagnosticCode::ComponentRenderFailed
        })
        .collect::<Vec<_>>();
    assert_eq!(13, failures.len(), "render diagnostics: {diagnostics:?}");

    for expected in [
        "at least one direct <accordion-item>",
        "only accepts direct <accordion-item>",
        "<accordion-item> must be rendered inside",
        "requires non-empty `title`",
        "JSON array of non-negative integers",
        "out of range",
        "`multiple=false`",
        "multiple",
        "bordered",
        "disabled",
        "size",
    ] {
        assert!(
            failures
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` in render diagnostics: {diagnostics:?}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != DiagnosticCode::ComponentPanicked),
        "invalid declarative input must not panic: {diagnostics:?}"
    );
}

#[gpui::test]
fn kbd_and_slider_validate_declarations_without_panicking(cx: &mut TestAppContext) {
    let source = r#"
        <div>
            <kbd stroke=" "></kbd>
            <kbd stroke="cmd-enter-extra"></kbd>
            <kbd stroke="cmd-enter">unexpected child</kbd>
            <kbd stroke="cmd-enter" appearance="sometimes"></kbd>
            <kbd stroke="cmd-enter" outline="sometimes"></kbd>
            <slider><span>unexpected child</span></slider>
            <slider min="NaN"></slider>
            <slider max="inf"></slider>
            <slider step="0"></slider>
            <slider min="4" max="4"></slider>
            <slider min="0" scale="logarithmic"></slider>
            <slider min="0" max="10" value="11"></slider>
            <slider orientation="diagonal"></slider>
            <slider scale="exponential"></slider>
            <slider disabled="sometimes"></slider>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    let failures = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Render
                && diagnostic.code == DiagnosticCode::ComponentRenderFailed
        })
        .collect::<Vec<_>>();
    assert_eq!(15, failures.len(), "render diagnostics: {diagnostics:?}");

    for expected in [
        "stroke",
        "valid GPUI keystroke",
        "does not accept children",
        "appearance",
        "outline",
        "finite number",
        "step",
        "less than",
        "logarithmic",
        "value",
        "orientation",
        "scale",
        "disabled",
    ] {
        assert!(
            failures
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` in render diagnostics: {diagnostics:?}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != DiagnosticCode::ComponentPanicked),
        "invalid declarative input must not panic: {diagnostics:?}"
    );
}

#[gpui::test]
fn collapsible_and_resizable_validate_structure_and_values_without_panicking(
    cx: &mut TestAppContext,
) {
    let source = r#"
        <div>
            <collapsible open="sometimes">
                <collapsible-content>Details</collapsible-content>
            </collapsible>
            <collapsible><span>Summary only</span></collapsible>
            <collapsible>
                <collapsible-content>First</collapsible-content>
                <collapsible-content>Second</collapsible-content>
            </collapsible>
            <collapsible-content>Orphan</collapsible-content>

            <resizable></resizable>
            <resizable><resizable-panel></resizable-panel></resizable>
            <resizable>
                <resizable-panel></resizable-panel>
                <span>Bad child</span>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable orientation="diagonal">
                <resizable-panel></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable size="NaN">
                <resizable-panel></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable size="0">
                <resizable-panel></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable-panel>Orphan</resizable-panel>
            <resizable>
                <resizable-panel min-size="-1"></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable>
                <resizable-panel min-size="200" max-size="100"></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable>
                <resizable-panel size="50" min-size="100"></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
            <resizable>
                <resizable-panel visible="sometimes"></resizable-panel>
                <resizable-panel></resizable-panel>
            </resizable>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    let failures = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Render
                && diagnostic.code == DiagnosticCode::ComponentRenderFailed
        })
        .collect::<Vec<_>>();
    assert_eq!(15, failures.len(), "render diagnostics: {diagnostics:?}");

    for expected in [
        "open",
        "exactly one direct <collapsible-content>",
        "<collapsible-content> must be rendered inside",
        "at least two direct <resizable-panel>",
        "only accepts direct <resizable-panel>",
        "orientation",
        "finite positive number",
        "<resizable-panel> must be rendered inside",
        "min-size",
        "greater than `min-size`",
        "must be within",
        "visible",
    ] {
        assert!(
            failures
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` in render diagnostics: {diagnostics:?}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != DiagnosticCode::ComponentPanicked),
        "invalid declarative input must not panic: {diagnostics:?}"
    );
}

#[gpui::test]
fn structural_and_numeric_component_contracts_fail_at_the_render_boundary(cx: &mut TestAppContext) {
    let source = r#"
        <div>
            <tab></tab>
            <tabs><div></div></tabs>
            <avatar-group><span></span></avatar-group>
            <description-list><div></div></description-list>
            <tabs selected-index="2"><tab>A</tab><tab>B</tab></tabs>
            <stepper selected-index="2">
                <stepper-item>A</stepper-item>
                <stepper-item>B</stepper-item>
            </stepper>
            <description-list columns="11">
                <description-item label="Name">Primary</description-item>
            </description-list>
            <description-list columns="2">
                <description-item label="Name" span="3">Primary</description-item>
            </description-list>
            <rating max="101"></rating>
            <pagination visible-pages="101"></pagination>
            <avatar-group size="sm"><avatar name="Ada" size="lg"></avatar></avatar-group>
        </div>
    "#;

    let diagnostics = render_diagnostics(source, ComponentRegistry::with_defaults(), cx);
    let failures = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Render
                && diagnostic.code == DiagnosticCode::ComponentRenderFailed
        })
        .collect::<Vec<_>>();
    assert_eq!(11, failures.len(), "render diagnostics: {diagnostics:?}");

    for expected in [
        "<tab> must be rendered inside",
        "<tabs> only accepts direct <tab> children",
        "<avatar-group> only accepts direct <avatar> children",
        "<description-list> only accepts direct <description-item> children",
        "selected-index",
        "columns",
        "span",
        "max",
        "visible-pages",
        "must inherit `size`",
    ] {
        assert!(
            failures
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` in render diagnostics: {diagnostics:?}"
        );
    }
    assert_eq!(
        2,
        failures
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("selected-index"))
            .count(),
        "tabs and stepper should both reject out-of-range selection"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != DiagnosticCode::ComponentPanicked),
        "invalid declarative input must not panic: {diagnostics:?}"
    );
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
