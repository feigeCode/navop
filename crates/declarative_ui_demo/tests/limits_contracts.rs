use declarative_ui_demo::{
    CompileLimits, CompileOptions, ComponentRegistry, HtmlParseError, ParseResource,
    TemplateCompileError, compile_template, parse_html_with_limits,
};

#[test]
fn source_limit_counts_original_utf8_bytes() {
    let source = "<span>你好</span>";
    let exact = CompileLimits {
        max_source_bytes: source.len(),
        ..CompileLimits::default()
    };
    parse_html_with_limits(source, exact).expect("the exact byte limit is accepted");

    let exceeded = CompileLimits {
        max_source_bytes: source.len() - 1,
        ..exact
    };
    assert_limit(
        parse_html_with_limits(source, exceeded),
        ParseResource::SourceBytes,
        source.len() - 1,
        source.len(),
    );
}

#[test]
fn node_limit_counts_elements_and_non_empty_text_nodes() {
    let source = "<div>hello<span>world</span></div>";
    let exact = CompileLimits {
        max_nodes: 4,
        ..CompileLimits::default()
    };
    parse_html_with_limits(source, exact).expect("four renderable nodes fit");

    let exceeded = CompileLimits {
        max_nodes: 3,
        ..exact
    };
    assert_limit(
        parse_html_with_limits(source, exceeded),
        ParseResource::Nodes,
        3,
        4,
    );
}

#[test]
fn depth_limit_treats_root_elements_as_depth_one() {
    let source = "<div><span><button>save</button></span></div>";
    let exact = CompileLimits {
        max_depth: 3,
        ..CompileLimits::default()
    };
    parse_html_with_limits(source, exact).expect("depth three fits");

    let exceeded = CompileLimits {
        max_depth: 2,
        ..exact
    };
    assert_limit(
        parse_html_with_limits(source, exceeded),
        ParseResource::Depth,
        2,
        3,
    );
}

#[test]
fn attribute_and_class_limits_are_document_totals() {
    let attrs = r#"<div id="root" data-a="1"><span data-b="2"></span></div>"#;
    let exact_attrs = CompileLimits {
        max_attributes: 3,
        ..CompileLimits::default()
    };
    parse_html_with_limits(attrs, exact_attrs).expect("three attributes fit");
    assert_limit(
        parse_html_with_limits(
            attrs,
            CompileLimits {
                max_attributes: 2,
                ..exact_attrs
            },
        ),
        ParseResource::Attributes,
        2,
        3,
    );

    let classes = r#"<div class="flex gap-2"><span class="text-sm"></span></div>"#;
    let exact_classes = CompileLimits {
        max_classes: 3,
        ..CompileLimits::default()
    };
    parse_html_with_limits(classes, exact_classes).expect("three class tokens fit");
    assert_limit(
        parse_html_with_limits(
            classes,
            CompileLimits {
                max_classes: 2,
                ..exact_classes
            },
        ),
        ParseResource::Classes,
        2,
        3,
    );
}

#[test]
fn custom_self_closing_and_explicit_tags_have_equal_cost() {
    let limits = CompileLimits {
        max_nodes: 2,
        max_depth: 2,
        ..CompileLimits::default()
    };
    parse_html_with_limits("<div><sql-editor /></div>", limits)
        .expect("self-closing custom component fits");
    parse_html_with_limits("<div><sql-editor></sql-editor></div>", limits)
        .expect("explicit custom component fits");
}

#[test]
fn permissive_validation_cannot_bypass_resource_limits() {
    let registry = ComponentRegistry::with_defaults();
    let options = CompileOptions::permissive().with_limits(CompileLimits {
        max_nodes: 1,
        ..CompileLimits::default()
    });
    let error = compile_template("<div><span></span></div>", &registry, options)
        .expect_err("resource limits are hard failures in every validation mode");

    assert!(matches!(
        error,
        TemplateCompileError::Parse(HtmlParseError::ResourceLimitExceeded {
            resource: ParseResource::Nodes,
            limit: 1,
            actual: 2,
        })
    ));
}

fn assert_limit(
    result: Result<declarative_ui_demo::VNode, HtmlParseError>,
    resource: ParseResource,
    limit: usize,
    actual: usize,
) {
    assert_eq!(
        Err(HtmlParseError::ResourceLimitExceeded {
            resource,
            limit,
            actual,
        }),
        result
    );
}
