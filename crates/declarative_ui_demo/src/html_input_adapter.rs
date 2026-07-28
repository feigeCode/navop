use crate::{Diagnostic, DiagnosticCode, DiagnosticPhase, Diagnostics, NodePath, VElement, VNode};

const TEXT_INPUT_TYPES: &[&str] = &["text", "password", "email", "search", "url", "tel"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextInputMode {
    Plain,
    Password,
}

pub(crate) fn adapt_html_inputs(root: VNode) -> (VNode, Diagnostics) {
    let mut diagnostics = Diagnostics::default();
    let root = adapt_node(root, &NodePath::root(), &mut diagnostics);
    (root, diagnostics)
}

pub(crate) fn text_input_mode(element: &VElement) -> Result<TextInputMode, String> {
    let input_type = normalized_input_type(element.attr("type"));
    match input_type.as_str() {
        "text" | "email" | "search" | "url" | "tel" => Ok(TextInputMode::Plain),
        "password" => Ok(TextInputMode::Password),
        _ => Err(format!(
            "attribute `type` on <input> must be one of {}, got `{input_type}`",
            TEXT_INPUT_TYPES.join(", ")
        )),
    }
}

fn adapt_node(node: VNode, path: &NodePath, diagnostics: &mut Diagnostics) -> VNode {
    match node {
        VNode::Element(mut element) => {
            element.children = element
                .children
                .into_iter()
                .enumerate()
                .map(|(index, child)| adapt_node(child, &path.child(index), diagnostics))
                .collect();
            if element.tag.eq_ignore_ascii_case("input") {
                adapt_input(&mut element, path, diagnostics);
            }
            VNode::Element(element)
        }
        VNode::Fragment(children) => VNode::Fragment(
            children
                .into_iter()
                .enumerate()
                .map(|(index, child)| adapt_node(child, &path.child(index), diagnostics))
                .collect(),
        ),
        VNode::Text(text) => VNode::Text(text),
    }
}

fn adapt_input(element: &mut VElement, path: &NodePath, diagnostics: &mut Diagnostics) {
    let declared_type = element.attrs.remove("type");
    let input_type = normalized_input_type(declared_type.as_deref());

    match input_type.as_str() {
        "checkbox" => element.tag = "checkbox".to_owned(),
        "radio" => element.tag = "radio".to_owned(),
        "range" => element.tag = "slider".to_owned(),
        "button" | "submit" | "reset" => {
            element.tag = "button".to_owned();
            if let Some(label) = element.attrs.remove("value") {
                element.attrs.insert("label".to_owned(), label);
            }
        }
        _ => {
            if declared_type.is_some_and(|value| !value.trim().is_empty()) {
                element.attrs.insert("type".to_owned(), input_type);
            }
            canonicalize_read_only(element, path, diagnostics);
        }
    }
}

fn canonicalize_read_only(element: &mut VElement, path: &NodePath, diagnostics: &mut Diagnostics) {
    let Some(value) = element.attrs.remove("readonly") else {
        return;
    };
    if element.attrs.contains_key("read-only") {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticPhase::Compile,
                DiagnosticCode::ConflictingAttributes,
                "`readonly` and `read-only` are aliases and cannot be declared together",
            )
            .at_path(path.clone()),
        );
        return;
    }
    element.attrs.insert("read-only".to_owned(), value);
}

fn normalized_input_type(value: Option<&str>) -> String {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    value.unwrap_or("text").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use crate::{DiagnosticCode, VNode, parse_html};

    use super::{TextInputMode, adapt_html_inputs, text_input_mode};

    #[test]
    fn adapts_common_html_input_types_recursively() {
        let root = parse_html(
            r#"
            <div>
                <input id="text" type=" TEXT " readonly />
                <section>
                    <input id="password" type="PASSWORD" />
                    <input id="check" type="checkbox" checked />
                    <input id="choice" type="radio" />
                    <input id="range" type="range" min="0" max="10" />
                    <input id="save" type="submit" value="Save" action="save" />
                </section>
            </div>
            "#,
        )
        .expect("valid HTML");

        let (root, diagnostics) = adapt_html_inputs(root);
        assert!(diagnostics.is_empty());
        let root = root.element().expect("root element");
        let text = root.children[0].element().expect("text input");
        assert_eq!("input", text.tag);
        assert_eq!(Some("text"), text.attr("type"));
        assert_eq!(Some(""), text.attr("read-only"));
        assert_eq!(None, text.attr("readonly"));

        let section = root.children[1].element().expect("nested section");
        let expected_tags = ["input", "checkbox", "radio", "slider", "button"];
        assert_eq!(
            expected_tags,
            section
                .children
                .iter()
                .map(|child| child.element().expect("element").tag.as_str())
                .collect::<Vec<_>>()
                .as_slice()
        );
        let password = section.children[0].element().expect("password input");
        assert_eq!(Ok(TextInputMode::Password), text_input_mode(password));
        let button = section.children[4].element().expect("button");
        assert_eq!(Some("Save"), button.attr("label"));
        assert_eq!(None, button.attr("value"));
        assert_eq!(None, button.attr("type"));
    }

    #[test]
    fn text_aliases_and_empty_type_use_plain_input_mode() {
        for input_type in ["", "text", "EMAIL", " search ", "url", "tel"] {
            let root = parse_html(&format!(r#"<input type="{input_type}" />"#))
                .expect("valid text-like input");
            let (root, diagnostics) = adapt_html_inputs(root);
            assert!(diagnostics.is_empty());
            assert_eq!(
                Ok(TextInputMode::Plain),
                text_input_mode(root.element().expect("input"))
            );
        }
    }

    #[test]
    fn unsupported_types_remain_inputs_for_typed_render_validation() {
        let root = parse_html(r#"<input id="birthday" type=" DATE " />"#).expect("valid HTML");
        let (root, diagnostics) = adapt_html_inputs(root);

        assert!(diagnostics.is_empty());
        let element = root.element().expect("input");
        assert_eq!("input", element.tag);
        assert_eq!(Some("date"), element.attr("type"));
        assert!(text_input_mode(element).is_err());
    }

    #[test]
    fn read_only_alias_conflicts_are_compile_errors() {
        let root = parse_html(r#"<input readonly read-only="false" />"#).expect("valid input HTML");
        let (root, diagnostics) = adapt_html_inputs(root);

        assert_eq!(
            1,
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::ConflictingAttributes)
                .count()
        );
        let element = match root {
            VNode::Element(element) => element,
            _ => panic!("input element"),
        };
        assert_eq!(Some("false"), element.attr("read-only"));
        assert_eq!(None, element.attr("readonly"));
    }
}
