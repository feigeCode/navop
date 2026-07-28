use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use gpui::AnyElement;
use thiserror::Error;

use crate::{
    Diagnostic, DiagnosticCode, DiagnosticPhase, Diagnostics, NodePath, RenderContext, VElement,
    binding::binding_attribute,
};

const GLOBAL_ATTRIBUTES: &[&str] = &["id", "key"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentProps {
    pub element: VElement,
    pub path: NodePath,
}

impl ComponentProps {
    pub(crate) fn new(element: VElement, path: NodePath) -> Self {
        Self { element, path }
    }

    pub fn stable_id(&self) -> String {
        stable_component_id(&self.element, &self.path)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ComponentError {
    message: String,
}

impl ComponentError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub type ComponentResult = Result<AnyElement, ComponentError>;

pub trait ComponentRenderer: 'static {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentSchema {
    allowed_attributes: BTreeSet<String>,
    required_attributes: BTreeSet<String>,
    allow_data_attributes: bool,
}

impl ComponentSchema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attribute(mut self, name: impl Into<String>) -> Self {
        self.allowed_attributes.insert(name.into());
        self
    }

    pub fn required_attribute(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.allowed_attributes.insert(name.clone());
        self.required_attributes.insert(name);
        self
    }

    pub fn data_attributes(mut self) -> Self {
        self.allow_data_attributes = true;
        self
    }

    pub(crate) fn validate(&self, element: &VElement, path: &NodePath) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        for name in element.attrs.keys() {
            if !self.supports(name) {
                diagnostics.push(compile_diagnostic(
                    DiagnosticCode::UnsupportedAttribute,
                    format!("attribute `{name}` is not supported by <{}>", element.tag),
                    path,
                ));
            }
        }
        for name in &self.required_attributes {
            if element.attr(name).is_none_or(str::is_empty) {
                diagnostics.push(compile_diagnostic(
                    DiagnosticCode::MissingAttribute,
                    format!("<{}> requires non-empty attribute `{name}`", element.tag),
                    path,
                ));
            }
        }
        validate_attribute_values(element, path, &mut diagnostics);
        diagnostics
    }

    fn supports(&self, name: &str) -> bool {
        GLOBAL_ATTRIBUTES.contains(&name)
            || self.allowed_attributes.contains(name)
            || (self.allow_data_attributes && name.starts_with("data-"))
    }
}

#[derive(Clone)]
struct ComponentEntry {
    schema: ComponentSchema,
    renderer: Rc<dyn ComponentRenderer>,
}

#[derive(Clone, Default)]
pub struct ComponentRegistry {
    entries: BTreeMap<String, ComponentEntry>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("component tag must not be empty")]
    EmptyTag,
    #[error("component <{tag}> is already registered")]
    AlreadyRegistered { tag: String },
}

impl ComponentRegistry {
    pub fn with_defaults() -> Self {
        let mut registry = Self::default();
        crate::builtin_components::register_default_components(&mut registry)
            .expect("built-in component declarations must be unique and valid");
        registry
    }

    pub fn register(
        &mut self,
        tag: impl Into<String>,
        renderer: impl ComponentRenderer,
    ) -> Result<(), RegistryError> {
        self.register_with_schema(tag, ComponentSchema::new(), renderer)
    }

    pub fn register_with_schema(
        &mut self,
        tag: impl Into<String>,
        schema: ComponentSchema,
        renderer: impl ComponentRenderer,
    ) -> Result<(), RegistryError> {
        let tag = normalize_tag(&tag.into());
        if tag.is_empty() {
            return Err(RegistryError::EmptyTag);
        }
        if self.entries.contains_key(&tag) {
            return Err(RegistryError::AlreadyRegistered { tag });
        }
        self.entries.insert(
            tag,
            ComponentEntry {
                schema,
                renderer: Rc::new(renderer),
            },
        );
        Ok(())
    }

    pub fn contains(&self, tag: &str) -> bool {
        self.entries.contains_key(&normalize_tag(tag))
    }

    pub(crate) fn renderer(&self, tag: &str) -> Option<Rc<dyn ComponentRenderer>> {
        self.entries
            .get(&normalize_tag(tag))
            .map(|entry| entry.renderer.clone())
    }

    pub(crate) fn schema(&self, tag: &str) -> Option<&ComponentSchema> {
        self.entries
            .get(&normalize_tag(tag))
            .map(|entry| &entry.schema)
    }
}

pub(crate) fn stable_component_id(element: &VElement, path: &NodePath) -> String {
    let identity = element
        .key()
        .map(str::to_owned)
        .unwrap_or_else(|| format_path(path));
    format!("{}:{identity}", normalize_tag(&element.tag))
}

fn validate_attribute_values(element: &VElement, path: &NodePath, diagnostics: &mut Diagnostics) {
    for name in ["action", "bind"] {
        if element.attr(name).is_some_and(str::is_empty) {
            diagnostics.push(compile_diagnostic(
                DiagnosticCode::EmptyAttribute,
                format!("attribute `{name}` must not be empty"),
                path,
            ));
        }
    }
    if element.attr("bind").is_some()
        && let Some(attribute) = binding_attribute(&element.tag)
        && element.attr(attribute).is_some()
    {
        diagnostics.push(compile_diagnostic(
            DiagnosticCode::ConflictingAttributes,
            format!("`bind` and `{attribute}` cannot be declared together"),
            path,
        ));
    }
}

fn compile_diagnostic(
    code: DiagnosticCode,
    message: impl Into<String>,
    path: &NodePath,
) -> Diagnostic {
    Diagnostic::error(DiagnosticPhase::Compile, code, message).at_path(path.clone())
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().to_ascii_lowercase()
}

fn format_path(path: &NodePath) -> String {
    if path.0.is_empty() {
        return "root".to_owned();
    }
    path.0
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{NodePath, VElement};

    use super::{ComponentProps, ComponentRegistry};

    #[test]
    fn renderer_lookup_normalizes_tag_names() {
        let registry = ComponentRegistry::with_defaults();
        assert!(registry.renderer(" DIV ").is_some());
    }

    #[test]
    fn stable_identity_normalizes_tag_names() {
        let props = ComponentProps::new(
            VElement {
                tag: " INPUT ".to_owned(),
                attrs: BTreeMap::from([("id".to_owned(), "username".to_owned())]),
                classes: Vec::new(),
                children: Vec::new(),
            },
            NodePath::root(),
        );
        assert_eq!("input:username", props.stable_id());
    }
}
