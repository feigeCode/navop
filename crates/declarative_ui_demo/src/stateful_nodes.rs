use std::collections::HashMap;

use crate::{NodePath, VElement, VNode, component::stable_component_id};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatefulInputSpec {
    pub multiline: bool,
    pub placeholder: Option<String>,
    pub value: Option<String>,
    pub bind: Option<String>,
}

impl StatefulInputSpec {
    pub(crate) fn from_element(element: &VElement, multiline: bool) -> Self {
        Self {
            multiline,
            placeholder: element.attr("placeholder").map(str::to_owned),
            value: element.attr("value").map(str::to_owned),
            bind: element.attr("bind").map(str::to_owned),
        }
    }

    pub(crate) fn has_same_configuration(&self, next: &Self) -> bool {
        self.multiline == next.multiline
            && self.placeholder == next.placeholder
            && self.bind == next.bind
            && (self.bind.is_some() || self.value == next.value)
    }
}

pub(crate) fn stateful_input_specs(root: &VNode) -> HashMap<String, StatefulInputSpec> {
    let mut specs = HashMap::new();
    collect_specs(root, &NodePath::root(), &mut specs);
    specs
}

fn collect_specs(node: &VNode, path: &NodePath, specs: &mut HashMap<String, StatefulInputSpec>) {
    match node {
        VNode::Element(element) => {
            if let Some(multiline) = input_multiline(&element.tag) {
                specs.insert(
                    stable_component_id(element, path),
                    StatefulInputSpec::from_element(element, multiline),
                );
            }
            collect_children(&element.children, path, specs);
        }
        VNode::Fragment(children) => collect_children(children, path, specs),
        VNode::Text(_) => {}
    }
}

fn collect_children(
    children: &[VNode],
    path: &NodePath,
    specs: &mut HashMap<String, StatefulInputSpec>,
) {
    for (index, child) in children.iter().enumerate() {
        collect_specs(child, &path.child(index), specs);
    }
}

fn input_multiline(tag: &str) -> Option<bool> {
    if tag.eq_ignore_ascii_case("input") {
        Some(false)
    } else if tag.eq_ignore_ascii_case("textarea") {
        Some(true)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::{parse_html, stateful_nodes::stateful_input_specs};

    #[test]
    fn collects_only_input_component_identities() {
        let root = parse_html(
            r#"
            <div>
                <input id="username" />
                <section>
                    <textarea key="notes"></textarea>
                    <input />
                </section>
                <button id="save">Save</button>
            </div>
            "#,
        )
        .expect("valid template");

        assert_eq!(
            HashSet::from([
                "input:username".to_owned(),
                "textarea:notes".to_owned(),
                "input:1.1".to_owned(),
            ]),
            stateful_input_specs(&root)
                .into_keys()
                .collect::<HashSet<_>>()
        );
    }

    #[test]
    fn removed_inputs_are_absent_from_live_identities() {
        let old = parse_html(r#"<div><input id="obsolete" /></div>"#).expect("old template");
        let next = parse_html("<div></div>").expect("next template");

        assert!(stateful_input_specs(&old).contains_key("input:obsolete"));
        assert!(!stateful_input_specs(&next).contains_key("input:obsolete"));
    }

    #[test]
    fn input_spec_tracks_stateful_attributes() {
        let root = parse_html(r#"<textarea placeholder="SQL" value="select 1"></textarea>"#)
            .expect("valid input");
        let element = root.element().expect("textarea element");

        assert_eq!(
            super::StatefulInputSpec {
                multiline: true,
                placeholder: Some("SQL".to_owned()),
                value: Some("select 1".to_owned()),
                bind: None,
            },
            super::StatefulInputSpec::from_element(element, true)
        );
    }
}
