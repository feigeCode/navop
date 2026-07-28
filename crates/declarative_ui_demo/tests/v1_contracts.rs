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
fn scroll_schema_requires_an_explicit_non_empty_id() {
    let registry = ComponentRegistry::with_defaults();
    for source in [r#"<scroll></scroll>"#, r#"<scroll id=""></scroll>"#] {
        let error = compile_template(source, &registry, CompileOptions::strict())
            .expect_err("scroll declarations require a stable identity");
        assert!(
            error
                .diagnostics()
                .expect("validation diagnostics")
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == DiagnosticCode::MissingAttribute
                        && diagnostic.path == Some(NodePath::root())
                }),
            "missing id diagnostic for {source}"
        );
    }
}

#[test]
fn scroll_ids_share_the_global_identity_namespace() {
    let registry = ComponentRegistry::with_defaults();
    let error = compile_template(
        r#"<div><input key="audit-log" /><scroll id="audit-log"></scroll></div>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect_err("scroll ids must not alias another stateful identity");

    let duplicate = error
        .diagnostics()
        .expect("validation diagnostics")
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateIdentity)
        .expect("scroll duplicate identity");
    assert_eq!(Some(NodePath(vec![1])), duplicate.path);
    assert!(duplicate.message.contains("audit-log"));
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

#[test]
fn kbd_schema_requires_a_non_empty_stroke() {
    let registry = ComponentRegistry::with_defaults();
    for source in [r#"<kbd></kbd>"#, r#"<kbd stroke=""></kbd>"#] {
        let error = compile_template(source, &registry, CompileOptions::strict())
            .expect_err("kbd declarations require a non-empty keystroke");
        assert!(
            error
                .diagnostics()
                .expect("validation diagnostics")
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == DiagnosticCode::MissingAttribute
                        && diagnostic.path == Some(NodePath::root())
                }),
            "missing stroke diagnostic for {source}"
        );
    }
}

#[test]
fn accordion_item_schema_requires_a_non_empty_title() {
    let registry = ComponentRegistry::with_defaults();
    for source in [
        r#"<accordion><accordion-item>Body</accordion-item></accordion>"#,
        r#"<accordion><accordion-item title="">Body</accordion-item></accordion>"#,
    ] {
        let error = compile_template(source, &registry, CompileOptions::strict())
            .expect_err("accordion item declarations require a non-empty title");
        assert!(
            error
                .diagnostics()
                .expect("validation diagnostics")
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == DiagnosticCode::MissingAttribute
                        && diagnostic.path == Some(NodePath(vec![0]))
                }),
            "missing title diagnostic for {source}"
        );
    }
}

#[test]
fn strict_compilation_accepts_structured_component_templates() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"
        <main class="flex flex-col gap-4 min-h-0 overflow-y-scroll">
            <form
                layout="horizontal"
                columns="2"
                label-width="120"
                label-text-size="0.875"
                size="sm"
            >
                <field
                    label="Name"
                    label-justify="end"
                    col-start="-2"
                    col-end="2"
                    required
                >
                    <input bind="name" />
                </field>
                <field label="Enabled" label-indent="false">
                    <checkbox bind="enabled">Notifications</checkbox>
                </field>
            </form>
            <table size="sm">
                <thead>
                    <tr><th>Name</th><th align="right">Status</th></tr>
                </thead>
                <tbody>
                    <tr>
                        <td>Primary</td>
                        <td align="right"><tag variant="success">Ready</tag></td>
                    </tr>
                </tbody>
                <tfoot>
                    <tr><td colspan="2">One connection</td></tr>
                </tfoot>
                <caption>Connections</caption>
            </table>
            <list>
                <list-item selected action="select" data-id="primary">Primary</list-item>
                <list-item secondary-selected>Secondary</list-item>
                <list-item separator>Archived</list-item>
            </list>
            <alert variant="info">Ready</alert>
            <badge count="2"><span>Saved</span></badge>
            <progress value="75"></progress>
            <spinner size="sm"></spinner>
            <separator dashed label="Details"></separator>
            <avatar name="Ada Lovelace" size="sm"></avatar>
            <avatar-group limit="2" ellipsis size="sm">
                <avatar name="Ada Lovelace"></avatar>
                <avatar name="Grace Hopper"></avatar>
                <avatar name="Margaret Hamilton"></avatar>
            </avatar-group>
            <description-list layout="horizontal" columns="2" label-width="120" size="sm">
                <description-item label="Owner">Platform</description-item>
                <description-item label="State" span="2">
                    <tag variant="success">Ready</tag>
                </description-item>
            </description-list>
            <breadcrumb>
                <breadcrumb-item action="navigate" data-page="home">Home</breadcrumb-item>
                <breadcrumb-item disabled>Connections</breadcrumb-item>
            </breadcrumb>
            <pagination bind="page" total-pages="20" visible-pages="7" size="sm"></pagination>
            <rating bind="score" max="5" size="sm"></rating>
            <tabs bind="selected_tab" variant="underline" size="sm">
                <tab>Overview</tab>
                <tab>Activity</tab>
            </tabs>
            <stepper bind="selected_step" layout="horizontal" size="sm">
                <stepper-item>Configure</stepper-item>
                <stepper-item>Review</stepper-item>
            </stepper>
            <div class="flex items-center gap-2">
                <span>Shortcut</span>
                <kbd stroke="cmd-enter" outline></kbd>
            </div>
            <slider
                id="volume"
                bind="volume"
                min="0"
                max="100"
                step="1"
                orientation="horizontal"
                scale="linear"
                action="selection-changed"
                data-control="slider"
                class="w-full"
            ></slider>
            <accordion
                id="settings-sections"
                bind="open_sections"
                multiple
                bordered="false"
                size="sm"
                action="selection-changed"
                data-control="accordion"
                class="w-full"
            >
                <accordion-item title="General">
                    <span>General settings</span>
                </accordion-item>
                <accordion-item title="Advanced">
                    <tag variant="info">Advanced settings</tag>
                </accordion-item>
            </accordion>
            <collapsible bind="details_open" class="gap-2">
                <button variant="secondary" size="sm">Advanced details</button>
                <collapsible-content class="p-2">
                    <tag variant="info">Controlled content</tag>
                </collapsible-content>
            </collapsible>
            <resizable
                id="workspace-layout"
                orientation="horizontal"
                size="240"
                class="w-full"
            >
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

    let template = compile_template(source, &registry, CompileOptions::strict())
        .expect("the structured component template is inside the strict DSL");
    assert!(!template.diagnostics().has_errors());
}

#[test]
fn attribute_bindings_conflict_with_explicit_native_values() {
    let registry = ComponentRegistry::with_defaults();
    for source in [
        r#"<input bind="value" value="explicit" />"#,
        r#"<textarea bind="value" value="explicit"></textarea>"#,
        r#"<checkbox bind="enabled" checked="true">Enabled</checkbox>"#,
        r#"<switch bind="enabled" checked="true">Enabled</switch>"#,
        r#"<radio bind="enabled" checked="true">Enabled</radio>"#,
        r#"<progress bind="completion" value="50"></progress>"#,
        r#"<badge bind="count" count="2"><span>Saved</span></badge>"#,
        r#"<pagination bind="page" current-page="2"></pagination>"#,
        r#"<rating bind="score" value="3"></rating>"#,
        r#"<tabs bind="tab" selected-index="1"><tab>A</tab><tab>B</tab></tabs>"#,
        r#"<stepper bind="step" selected-index="1"><stepper-item>A</stepper-item><stepper-item>B</stepper-item></stepper>"#,
        r#"<slider bind="volume" value="50"></slider>"#,
        r#"<accordion bind="sections" open-indices="[0]"><accordion-item title="General">General settings</accordion-item></accordion>"#,
        r#"<collapsible bind="details" open="true"><collapsible-content>Details</collapsible-content></collapsible>"#,
    ] {
        let error = compile_template(source, &registry, CompileOptions::strict())
            .expect_err("bind and its resolved target attribute must conflict");
        assert!(
            error
                .diagnostics()
                .expect("validation diagnostics")
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::ConflictingAttributes),
            "missing conflict diagnostic for {source}"
        );
    }
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
