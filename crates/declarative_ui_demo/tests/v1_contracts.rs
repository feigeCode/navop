use std::collections::BTreeMap;

use declarative_ui_demo::{
    ActionError, ActionEvent, CompileOptions, ComponentRegistry, DeclarativeView,
    DeclarativeViewConfig, DiagnosticCode, DiagnosticSeverity, NodePath, Runtime, StateStore,
    TemplateCompileError, compile_template,
};
use gpui::{AppContext, TestAppContext};

#[test]
fn strict_compilation_rejects_unknown_tags_and_classes() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"<unknown-widget class="flex imaginary"><span>value</span></unknown-widget>"#;

    let error = compile_template(source, &registry, CompileOptions::strict())
        .expect_err("strict compilation must reject unsupported DSL");
    let diagnostics = match error {
        TemplateCompileError::Validation(diagnostics) => diagnostics,
        other => panic!("expected validation diagnostics, got {other:?}"),
    };

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::UnknownTag
            && diagnostic.severity == DiagnosticSeverity::Error
            && diagnostic.path == Some(NodePath::root())
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::UnsupportedClass
            && diagnostic.severity == DiagnosticSeverity::Error
    }));
}

#[test]
fn permissive_compilation_preserves_typed_warnings() {
    let registry = ComponentRegistry::with_defaults();
    let compiled = compile_template(
        r#"<unknown-widget class="made-up" />"#,
        &registry,
        CompileOptions::permissive(),
    )
    .expect("permissive compilation should retain unsupported nodes");

    assert_eq!(2, compiled.diagnostics().warnings().count());
    assert!(!compiled.diagnostics().has_errors());
}

#[test]
fn compilation_rejects_duplicate_explicit_identities() {
    let registry = ComponentRegistry::with_defaults();
    let error = compile_template(
        r#"<div><input key="field" /><textarea key="field"></textarea></div>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect_err("keys are unique within one compiled view");

    let diagnostics = error
        .diagnostics()
        .expect("validation errors expose diagnostics");
    let duplicate = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateIdentity)
        .expect("duplicate key diagnostic");
    assert!(duplicate.message.contains("field"));
    assert_eq!(Some(NodePath(vec![1])), duplicate.path);
}

#[test]
fn identity_values_share_one_namespace_across_id_and_key_attributes() {
    let registry = ComponentRegistry::with_defaults();
    let error = compile_template(
        r#"<div><input id="field" /><textarea key="field"></textarea></div>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect_err("id and key values must not alias one stateful identity");

    let duplicate = error
        .diagnostics()
        .expect("validation diagnostics")
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateIdentity)
        .expect("cross-attribute duplicate identity");
    assert_eq!(Some(NodePath(vec![1])), duplicate.path);
    assert!(duplicate.message.contains("field"));
}

#[test]
fn component_schemas_reject_missing_and_unsupported_attributes() {
    let registry = ComponentRegistry::with_defaults();
    let error = compile_template(
        r#"<div mystery="value"><img /></div>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect_err("component contracts must be checked before rendering");
    let diagnostics = error.diagnostics().expect("validation diagnostics");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::UnsupportedAttribute
            && diagnostic.path == Some(NodePath::root())
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::MissingAttribute
            && diagnostic.path == Some(NodePath(vec![0]))
    }));
}

#[gpui::test]
fn action_payload_is_structured_and_state_updates_commit_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut initial = StateStore::default();
        initial.set("status", "idle");
        initial.set("count", "0");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(initial);
            runtime
                .on("save", |context| {
                    assert_eq!(
                        Some("42"),
                        context.event().payload().get("record").map(String::as_str)
                    );
                    context.set("status", "saving");
                    context.set("count", "1");
                    context.set("status", "saved");
                    Ok(())
                })
                .expect("register action");
            runtime
        });
        let event = ActionEvent::new("save", "button:save", NodePath::root())
            .with_payload(BTreeMap::from([("record".to_owned(), "42".to_owned())]));

        let outcome = runtime
            .update(cx, |runtime, cx| runtime.dispatch(event, cx))
            .expect("action succeeds");
        let runtime = runtime.read(cx);

        assert!(outcome.state_changed);
        assert_eq!(1, runtime.revision());
        assert_eq!(Some("saved"), runtime.state().get("status"));
        assert_eq!(Some("1"), runtime.state().get("count"));
    });
}

#[gpui::test]
fn failed_action_rolls_back_all_state_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut initial = StateStore::default();
        initial.set("status", "idle");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(initial);
            runtime
                .on("save", |context| {
                    context.set("status", "half-written");
                    Err(ActionError::new("storage unavailable"))
                })
                .expect("register action");
            runtime
        });

        let result = runtime.update(cx, |runtime, cx| {
            runtime.dispatch(
                ActionEvent::new("save", "button:save", NodePath::root()),
                cx,
            )
        });

        assert!(result.is_err());
        let runtime = runtime.read(cx);
        assert_eq!(0, runtime.revision());
        assert_eq!(Some("idle"), runtime.state().get("status"));
    });
}

#[gpui::test]
fn runtime_changes_reconcile_the_mounted_view_automatically(cx: &mut TestAppContext) {
    let (runtime, view) = cx.update(|cx| {
        let registry = ComponentRegistry::with_defaults();
        let template = compile_template(
            r#"<span bind="status"></span>"#,
            &registry,
            CompileOptions::strict(),
        )
        .expect("valid template");
        let mut initial = StateStore::default();
        initial.set("status", "idle");
        let runtime = cx.new(|_| Runtime::new(initial));
        let config = DeclarativeViewConfig::new(template, runtime.clone(), registry);
        let view = cx.new(|cx| DeclarativeView::new(config, cx));
        (runtime, view)
    });
    cx.run_until_parked();

    cx.update(|cx| {
        runtime.update(cx, |runtime, cx| {
            runtime.set("status", "updated", cx);
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!("updated", view.read(cx).rendered().text_content());
        assert_eq!(1, view.read(cx).last_patches().len());
    });
}
