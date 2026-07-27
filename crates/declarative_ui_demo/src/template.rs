use std::collections::BTreeMap;

use thiserror::Error;

use crate::{
    CompileLimits, ComponentRegistry, Diagnostic, DiagnosticCode, DiagnosticPhase, Diagnostics,
    HtmlParseError, NodePath, VElement, VNode, parse_classes, parse_html_with_limits,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Strict,
    Permissive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub validation: ValidationMode,
    pub limits: CompileLimits,
}

impl CompileOptions {
    pub const fn strict() -> Self {
        Self {
            validation: ValidationMode::Strict,
            limits: CompileLimits::DEFAULT,
        }
    }

    pub const fn permissive() -> Self {
        Self {
            validation: ValidationMode::Permissive,
            limits: CompileLimits::DEFAULT,
        }
    }

    pub const fn with_limits(mut self, limits: CompileLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self::strict()
    }
}

#[derive(Clone, Debug)]
pub struct CompiledTemplate {
    source: String,
    root: VNode,
    diagnostics: Diagnostics,
}

impl CompiledTemplate {
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn root(&self) -> &VNode {
        &self.root
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TemplateCompileError {
    #[error(transparent)]
    Parse(#[from] HtmlParseError),
    #[error("template validation failed")]
    Validation(Diagnostics),
}

impl TemplateCompileError {
    pub fn diagnostics(&self) -> Option<&Diagnostics> {
        match self {
            Self::Parse(_) => None,
            Self::Validation(diagnostics) => Some(diagnostics),
        }
    }
}

pub fn compile_template(
    source: &str,
    registry: &ComponentRegistry,
    options: CompileOptions,
) -> Result<CompiledTemplate, TemplateCompileError> {
    let root = parse_html_with_limits(source, options.limits)?;
    let diagnostics = validate_template(&root, registry, options);
    if diagnostics.has_errors() {
        return Err(TemplateCompileError::Validation(diagnostics));
    }
    Ok(CompiledTemplate {
        source: source.to_owned(),
        root,
        diagnostics,
    })
}

fn validate_template(
    root: &VNode,
    registry: &ComponentRegistry,
    options: CompileOptions,
) -> Diagnostics {
    let mut validator = TemplateValidator {
        registry,
        options,
        diagnostics: Diagnostics::default(),
        identities: BTreeMap::new(),
    };
    validator.visit(root, &NodePath::root());
    validator.diagnostics
}

struct TemplateValidator<'a> {
    registry: &'a ComponentRegistry,
    options: CompileOptions,
    diagnostics: Diagnostics,
    identities: BTreeMap<String, NodePath>,
}

impl TemplateValidator<'_> {
    fn visit(&mut self, node: &VNode, path: &NodePath) {
        match node {
            VNode::Element(element) => self.visit_element(element, path),
            VNode::Fragment(children) => self.visit_children(children, path),
            VNode::Text(_) => {}
        }
    }

    fn visit_element(&mut self, element: &VElement, path: &NodePath) {
        self.validate_identity(element, path);
        self.validate_classes(element, path);
        match self.registry.schema(&element.tag) {
            Some(schema) => self.diagnostics.extend(schema.validate(element, path)),
            None => self.push_support_diagnostic(
                DiagnosticCode::UnknownTag,
                format!("component <{}> is not registered", element.tag),
                path,
            ),
        }
        self.visit_children(&element.children, path);
    }

    fn visit_children(&mut self, children: &[VNode], path: &NodePath) {
        for (index, child) in children.iter().enumerate() {
            self.visit(child, &path.child(index));
        }
    }

    fn validate_identity(&mut self, element: &VElement, path: &NodePath) {
        for attribute in ["id", "key"] {
            let Some(value) = element.attr(attribute) else {
                continue;
            };
            if value.trim().is_empty() {
                self.diagnostics.push(
                    compile_error(
                        DiagnosticCode::EmptyAttribute,
                        format!("attribute `{attribute}` must not be empty"),
                    )
                    .at_path(path.clone()),
                );
                continue;
            }
            let identity = value.to_owned();
            if let Some(first_path) = self.identities.insert(identity, path.clone()) {
                self.diagnostics.push(
                    compile_error(
                        DiagnosticCode::DuplicateIdentity,
                        format!(
                            "duplicate explicit identity `{value}` from `{attribute}`; \
                             first declared at {first_path}"
                        ),
                    )
                    .at_path(path.clone()),
                );
            }
        }
    }

    fn validate_classes(&mut self, element: &VElement, path: &NodePath) {
        for class in parse_classes(&element.classes).unsupported {
            self.push_support_diagnostic(
                DiagnosticCode::UnsupportedClass,
                format!("unsupported Tailwind utility `{class}`"),
                path,
            );
        }
    }

    fn push_support_diagnostic(&mut self, code: DiagnosticCode, message: String, path: &NodePath) {
        let diagnostic = match self.options.validation {
            ValidationMode::Strict => Diagnostic::error(DiagnosticPhase::Compile, code, message),
            ValidationMode::Permissive => {
                Diagnostic::warning(DiagnosticPhase::Compile, code, message)
            }
        };
        self.diagnostics.push(diagnostic.at_path(path.clone()));
    }
}

fn compile_error(code: DiagnosticCode, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(DiagnosticPhase::Compile, code, message)
}
