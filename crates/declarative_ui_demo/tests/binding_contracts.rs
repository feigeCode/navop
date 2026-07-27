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

fn element_attr<'a>(node: &'a VNode, name: &str) -> Option<&'a str> {
    node.element().and_then(|element| element.attr(name))
}
