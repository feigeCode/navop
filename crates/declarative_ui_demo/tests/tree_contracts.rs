use declarative_ui_demo::{
    CompileOptions, ComponentRegistry, DiagnosticCode, DiagnosticSeverity, NodePath,
    TemplateCompileError, compile_template,
};

/// The registry must include the new virtualized-data tags.
#[test]
fn registry_contains_tree_and_data_list_components() {
    let registry = ComponentRegistry::with_defaults();
    assert!(registry.contains("tree"));
    assert!(registry.contains("tree-node"));
    assert!(registry.contains("data-list"));
}

/// A `<tree>` with nested `<tree-node>` children compiles in strict mode.
#[test]
fn strict_compilation_accepts_nested_tree_nodes() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"
        <tree id="explorer">
            <tree-node label="src" expanded>
                <tree-node label="main.rs"></tree-node>
                <tree-node label="lib.rs"></tree-node>
            </tree-node>
            <tree-node label="Cargo.toml"></tree-node>
            <tree-node label="README.md" disabled></tree-node>
        </tree>
    "#;
    compile_template(source, &registry, CompileOptions::strict())
        .expect("nested tree should compile");
}

/// `<tree>` without an `id` is a compile error.
#[test]
fn tree_requires_id() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"<tree><tree-node label="a"></tree-node></tree>"#;
    let error = compile_template(source, &registry, CompileOptions::strict())
        .expect_err("tree without id should fail");
    let diagnostics = match error {
        TemplateCompileError::Validation(d) => d,
        other => panic!("expected validation error, got {other:?}"),
    };
    assert!(diagnostics.iter().any(|d| {
        d.code == DiagnosticCode::MissingAttribute
            && d.severity == DiagnosticSeverity::Error
            && d.path == Some(NodePath::root())
    }));
}

/// `<data-list>` requires `id`. Either `data-items` or `data-count` provides
/// the row data — this is a runtime check, not a compile-time requirement.
#[test]
fn data_list_requires_id() {
    let registry = ComponentRegistry::with_defaults();

    // `data-items` alone compiles (runtime reads JSON from state).
    compile_template(
        r#"<data-list id="rows" data-items="row_data"></data-list>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect("data-list with data-items should compile");

    // `data-count` alone compiles.
    compile_template(
        r#"<data-list id="rows" data-count="100"></data-list>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect("data-list with data-count should compile");

    // Neither data source still compiles (runtime check).
    compile_template(
        r#"<data-list id="rows"></data-list>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect("data-list with no data source still compiles");

    // Missing id.
    let error = compile_template(
        r#"<data-list data-count="100"></data-list>"#,
        &registry,
        CompileOptions::strict(),
    )
    .expect_err("data-list without id should fail");
    let diagnostics = match error {
        TemplateCompileError::Validation(d) => d,
        other => panic!("expected validation error, got {other:?}"),
    };
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::MissingAttribute)
    );
}

/// `<tree-node>` expanded and disabled bare attributes work.
#[test]
fn tree_node_accepts_expanded_and_disabled_attributes() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"
        <tree id="t">
            <tree-node label="a" expanded disabled></tree-node>
        </tree>
    "#;
    compile_template(source, &registry, CompileOptions::strict())
        .expect("tree-node with expanded and disabled should compile");
}

/// `<tree-node>` supports `action` and `data-*` attributes.
#[test]
fn tree_node_supports_action_and_data_attributes() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"
        <tree id="t" action="navigate" data-view="explorer">
            <tree-node label="file.rs" action="open-file" data-path="/src/file.rs">
            </tree-node>
        </tree>
    "#;
    compile_template(source, &registry, CompileOptions::strict())
        .expect("tree-node with action and data-* should compile");
}

/// `<data-list>` supports `bind`, `action`, and `data-*` attributes.
#[test]
fn data_list_supports_binding_and_action() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"
        <data-list
            id="log-entries"
            data-count="5000"
            bind="selected_log"
            action="select-log"
            data-label="Log entry {n}"
        >
        </data-list>
    "#;
    compile_template(source, &registry, CompileOptions::strict())
        .expect("data-list with bind, action and data-label should compile");
}

/// `<tree-node>` rendered outside `<tree>` still compiles (it just renders
/// a structural error at runtime, like other structural-only tags).
#[test]
fn tree_node_outside_tree_is_known_tag() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"<div><tree-node label="orphan"></tree-node></div>"#;
    // Should compile without UnknownTag error.
    let compiled = compile_template(source, &registry, CompileOptions::strict())
        .expect("tree-node is a known tag");
    assert!(
        compiled
            .diagnostics()
            .iter()
            .all(|d| d.code != DiagnosticCode::UnknownTag)
    );
}

/// Unsupported attributes on `<tree>` are rejected in strict mode.
#[test]
fn tree_rejects_unsupported_attributes() {
    let registry = ComponentRegistry::with_defaults();
    let source = r#"<tree id="t" color="red"></tree>"#;
    let error = compile_template(source, &registry, CompileOptions::strict())
        .expect_err("unsupported attribute should fail");
    let diagnostics = match error {
        TemplateCompileError::Validation(d) => d,
        other => panic!("expected validation error, got {other:?}"),
    };
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::UnsupportedAttribute)
    );
}
