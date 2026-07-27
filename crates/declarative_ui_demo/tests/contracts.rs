use declarative_ui_demo::{
    ActionEvent, DiffError, HtmlParseError, NodePath, Patch, PatchKind, Runtime, RuntimeError,
    StateStore, TailwindModifier, VNode, apply_patches, diff, parse_classes, parse_html,
    resolve_bindings,
};
use gpui::{AppContext, TestAppContext};

const SAMPLE: &str = r#"
    <div class="flex flex-col gap-2">
        <button id="save" action="save">保存</button>
    </div>
"#;

#[test]
fn parses_html_fragment_into_vnodes() {
    let root = parse_html(SAMPLE).expect("valid declarative HTML");
    let element = root.element().expect("one root element");
    assert_eq!("div", element.tag);
    assert_eq!(vec!["flex", "flex-col", "gap-2"], element.classes);

    let button = element.children[0].element().expect("button element");
    assert_eq!(Some("save"), button.attr("action"));
    assert_eq!("保存", button.text_content());
}

#[test]
fn custom_self_closing_components_are_siblings() {
    let root = parse_html(r#"<div><connection-tree /><sql-editor /><db-table /></div>"#)
        .expect("valid declarative HTML");
    let children = &root.element().expect("root div").children;

    assert_eq!(3, children.len());
    assert_eq!(
        vec!["connection-tree", "sql-editor", "db-table"],
        children
            .iter()
            .map(|child| child.element().expect("custom element").tag.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn rejects_executable_or_css_attributes() {
    assert!(matches!(
        parse_html(r#"<div style="color: red"></div>"#),
        Err(HtmlParseError::ForbiddenAttribute { name }) if name == "style"
    ));
    assert!(matches!(
        parse_html(r#"<button onclick="alert(1)">x</button>"#),
        Err(HtmlParseError::ForbiddenAttribute { name }) if name == "onclick"
    ));
    assert!(matches!(
        parse_html(r#"<sql-editor onclick="alert(1)" />"#),
        Err(HtmlParseError::ForbiddenAttribute { name }) if name == "onclick"
    ));
    assert!(matches!(
        parse_html("<script>danger()</script>"),
        Err(HtmlParseError::ForbiddenElement { tag }) if tag == "script"
    ));
}

#[test]
fn parses_supported_tailwind_subset_and_reports_unknown_classes() {
    let classes = ["flex", "flex-col", "gap-2", "p-4", "made-up"]
        .map(str::to_owned)
        .to_vec();
    let parsed = parse_classes(&classes);

    assert_eq!(
        vec![
            TailwindModifier::Flex,
            TailwindModifier::FlexColumn,
            TailwindModifier::Gap(2),
            TailwindModifier::Padding(4),
        ],
        parsed.modifiers
    );
    assert_eq!(vec!["made-up"], parsed.unsupported);
}

#[gpui::test]
fn action_handlers_update_shared_state(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut state = StateStore::default();
        state.set("count", "0");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(state);
            runtime
                .on("save", |context| {
                    let next = context
                        .get("count")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or_default()
                        + 1;
                    context.set("count", next.to_string());
                    Ok(())
                })
                .expect("register action");
            runtime
        });

        runtime
            .update(cx, |runtime, cx| {
                runtime.dispatch(
                    ActionEvent::new("save", "button:save", NodePath::root()),
                    cx,
                )
            })
            .expect("registered action");
        assert_eq!(Some("1"), runtime.read(cx).get("count"));
        assert_eq!(
            Err(RuntimeError::UnknownAction("missing".to_owned())),
            runtime.update(cx, |runtime, cx| {
                runtime.dispatch(
                    ActionEvent::new("missing", "button:missing", NodePath::root()),
                    cx,
                )
            })
        );
    });
}

#[test]
fn binding_keeps_element_shell_and_replaces_children() {
    let template =
        parse_html(r#"<span class="text-lg" bind="username"></span>"#).expect("valid binding");
    let mut state = StateStore::default();
    state.set("username", "admin");

    let bound = resolve_bindings(&template, &state);
    let element = bound.element().expect("span remains an element");
    assert_eq!(vec!["text-lg"], element.classes);
    assert_eq!(vec![VNode::Text("admin".to_owned())], element.children);
}

#[test]
fn diffs_and_applies_text_updates() {
    let old = parse_html(r#"<span id="status">idle</span>"#).expect("old vnode");
    let new = parse_html(r#"<span id="status">saved</span>"#).expect("new vnode");
    let patches = diff(&old, &new);

    assert!(matches!(
        patches.as_slice(),
        [patch] if matches!(&patch.kind, PatchKind::SetText { text } if text == "saved")
    ));

    let mut updated = old;
    apply_patches(&mut updated, &patches).expect("patch applies");
    assert_eq!(new, updated);
}

#[test]
fn diff_round_trips_structural_and_property_changes() {
    let cases = [
        (
            r#"<div class="flex"><span id="a">A</span><span id="b">B</span></div>"#,
            r#"<div class="flex flex-col" data-state="ready"><button id="a">A</button></div>"#,
        ),
        (
            r#"<div><span>A</span></div>"#,
            r#"<div><span>A</span><span>B</span></div>"#,
        ),
    ];

    for (old_source, new_source) in cases {
        let old = parse_html(old_source).expect("old vnode");
        let expected = parse_html(new_source).expect("new vnode");
        let mut actual = old.clone();

        apply_patches(&mut actual, &diff(&old, &expected)).expect("patches apply");
        assert_eq!(expected, actual);
    }
}

#[test]
fn invalid_patch_path_returns_a_typed_error() {
    let mut root = parse_html("<div></div>").expect("root vnode");
    let path = NodePath(vec![99]);
    let patch = Patch {
        path: path.clone(),
        kind: PatchKind::SetText {
            text: "unreachable".to_owned(),
        },
    };

    assert_eq!(
        Err(DiffError::InvalidPath(path)),
        apply_patches(&mut root, &[patch])
    );
}

#[test]
fn applying_a_patch_batch_is_transactional() {
    let original = parse_html("<div><span>old</span></div>").expect("root vnode");
    let mut actual = original.clone();
    let patches = vec![
        Patch {
            path: NodePath(vec![0, 0]),
            kind: PatchKind::SetText {
                text: "partially-applied".to_owned(),
            },
        },
        Patch {
            path: NodePath(vec![99]),
            kind: PatchKind::SetText {
                text: "unreachable".to_owned(),
            },
        },
    ];

    assert!(apply_patches(&mut actual, &patches).is_err());
    assert_eq!(
        original, actual,
        "a failed batch must leave the tree unchanged"
    );
}
