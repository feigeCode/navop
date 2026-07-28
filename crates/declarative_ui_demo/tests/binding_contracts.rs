use declarative_ui_demo::{
    CompileOptions, ComponentRegistry, DeclarativeView, DeclarativeViewConfig, DiagnosticCode,
    DiagnosticPhase, DiagnosticSeverity, NodePath, Runtime, StateStore, VNode, compile_template,
    parse_html, resolve_bindings_checked,
};
use gpui::{AppContext, TestAppContext};

#[test]
fn missing_bindings_resolve_to_empty_values_with_typed_paths() {
    let template = parse_html(
        r#"
        <div>
            <span bind="missing"></span>
            <input bind="missing" />
            <textarea bind="missing"></textarea>
        </div>
        "#,
    )
    .expect("valid bindings");

    let resolution = resolve_bindings_checked(&template, &StateStore::default());
    let diagnostics = resolution.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(3, diagnostics.len());
    for (diagnostic, path) in
        diagnostics
            .iter()
            .zip([NodePath(vec![0]), NodePath(vec![1]), NodePath(vec![2])])
    {
        assert_eq!(DiagnosticSeverity::Warning, diagnostic.severity);
        assert_eq!(DiagnosticPhase::Binding, diagnostic.phase);
        assert_eq!(DiagnosticCode::MissingBinding, diagnostic.code);
        assert_eq!(Some(path), diagnostic.path);
    }

    let children = &resolution.root.element().expect("root div").children;
    assert_eq!("", children[0].text_content());
    assert_eq!(Some(""), element_attr(&children[1], "value"));
    assert_eq!(Some(""), element_attr(&children[2], "value"));
}

#[gpui::test]
fn adding_a_missing_state_key_clears_warning_and_updates_the_view(cx: &mut TestAppContext) {
    let (runtime, view) = cx.update(|cx| {
        let registry = ComponentRegistry::with_defaults();
        let template = compile_template(
            r#"<span bind="username"></span>"#,
            &registry,
            CompileOptions::strict(),
        )
        .expect("valid template");
        let runtime = cx.new(|_| Runtime::default());
        let view = cx.new(|cx| {
            DeclarativeView::new(
                DeclarativeViewConfig::new(template, runtime.clone(), registry),
                cx,
            )
        });
        (runtime, view)
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!("", view.rendered().text_content());
        assert_eq!(
            1,
            view.diagnostics().phase(DiagnosticPhase::Binding).count()
        );
        runtime.update(cx, |runtime, cx| {
            runtime.set("username", "admin", cx);
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!("admin", view.rendered().text_content());
        assert_eq!(
            0,
            view.diagnostics().phase(DiagnosticPhase::Binding).count()
        );
    });
}

#[gpui::test]
fn repeated_reconciliation_replaces_instead_of_accumulating_binding_warnings(
    cx: &mut TestAppContext,
) {
    let view = cx.update(|cx| {
        let registry = ComponentRegistry::with_defaults();
        let template = compile_template(
            r#"<span bind="missing"></span>"#,
            &registry,
            CompileOptions::strict(),
        )
        .expect("valid template");
        let runtime = cx.new(|_| Runtime::default());
        cx.new(|cx| {
            DeclarativeView::new(DeclarativeViewConfig::new(template, runtime, registry), cx)
        })
    });
    cx.run_until_parked();

    for _ in 0..3 {
        cx.update(|cx| view.update(cx, |view, cx| view.refresh(cx)));
        cx.run_until_parked();
    }

    cx.update(|cx| {
        let diagnostics = view.read(cx).diagnostics().clone();
        assert_eq!(1, diagnostics.phase(DiagnosticPhase::Binding).count());
        assert_eq!(1, diagnostics.len());
    });
}

#[test]
fn component_bindings_target_native_attributes_without_destroying_labels() {
    let template = parse_html(
        r#"
        <div>
            <checkbox bind="notifications">Email alerts</checkbox>
            <switch bind="auto_sync">Auto-sync metadata</switch>
            <radio bind="beta_mode">Beta channel</radio>
            <progress bind="completion"></progress>
            <badge bind="save_count"><span>Saved</span></badge>
            <pagination bind="page" total-pages="20"></pagination>
            <rating bind="score"></rating>
            <tabs bind="selected_tab">
                <tab>Overview</tab>
                <tab>Activity</tab>
            </tabs>
            <stepper bind="selected_step">
                <stepper-item>Configure</stepper-item>
                <stepper-item>Review</stepper-item>
            </stepper>
            <slider bind="volume" min="0" max="100" step="1"></slider>
            <accordion bind="open_sections" multiple>
                <accordion-item title="General">
                    <span>General settings</span>
                </accordion-item>
                <accordion-item title="Advanced">Advanced settings</accordion-item>
            </accordion>
            <collapsible bind="details_open">
                <span>Summary</span>
                <collapsible-content>Details</collapsible-content>
            </collapsible>
            <span bind="status">stale fallback</span>
        </div>
        "#,
    )
    .expect("valid component bindings");
    let mut state = StateStore::default();
    state.set("notifications", "true");
    state.set("auto_sync", "false");
    state.set("beta_mode", "1");
    state.set("completion", "72.5");
    state.set("save_count", "4");
    state.set("page", "7");
    state.set("score", "3");
    state.set("selected_tab", "1");
    state.set("selected_step", "1");
    state.set("volume", "35");
    state.set("open_sections", "[0,1]");
    state.set("details_open", "true");
    state.set("status", "ready");

    let resolution = resolve_bindings_checked(&template, &state);
    assert!(resolution.diagnostics.is_empty());
    let children = &resolution.root.element().expect("root div").children;

    assert_eq!(Some("true"), element_attr(&children[0], "checked"));
    assert_eq!("Email alerts", children[0].text_content());
    assert_eq!(Some("false"), element_attr(&children[1], "checked"));
    assert_eq!("Auto-sync metadata", children[1].text_content());
    assert_eq!(Some("1"), element_attr(&children[2], "checked"));
    assert_eq!("Beta channel", children[2].text_content());
    assert_eq!(Some("72.5"), element_attr(&children[3], "value"));
    assert_eq!(Some("4"), element_attr(&children[4], "count"));
    assert_eq!("Saved", children[4].text_content());
    assert_eq!(Some("7"), element_attr(&children[5], "current-page"));
    assert_eq!(Some("3"), element_attr(&children[6], "value"));
    assert_eq!(Some("1"), element_attr(&children[7], "selected-index"));
    assert_eq!("OverviewActivity", children[7].text_content());
    assert_eq!(Some("1"), element_attr(&children[8], "selected-index"));
    assert_eq!("ConfigureReview", children[8].text_content());
    assert_eq!(Some("35"), element_attr(&children[9], "value"));
    assert_eq!(Some("[0,1]"), element_attr(&children[10], "open-indices"));
    assert_eq!(
        "General settingsAdvanced settings",
        children[10].text_content()
    );
    assert_eq!(Some("true"), element_attr(&children[11], "open"));
    assert_eq!("SummaryDetails", children[11].text_content());
    assert_eq!("ready", children[12].text_content());
}

fn element_attr<'a>(node: &'a VNode, name: &str) -> Option<&'a str> {
    node.element().and_then(|element| element.attr(name))
}
