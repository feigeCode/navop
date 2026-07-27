//! Standalone v1 runtime for rendering a restricted declarative UI DSL with GPUI.
//!
//! The crate compiles constrained HTML into an input-independent [`VNode`],
//! resolves string state bindings, diffs the resolved tree, and maps registered
//! components plus a bounded Tailwind utility subset to native GPUI elements.
//!
//! Templates cannot execute JavaScript or inline event code. Parsing, resource
//! limits, typed diagnostics, transactional actions, bidirectional input
//! binding, stateful identity, component error boundaries, and GPUI rendering
//! remain separate layers so a future JSON or AI frontend can target the same
//! VNode model without turning this crate into a browser.
//!
//! This crate does not define the Navop extension/WASM ABI. Custom Rust
//! [`ComponentRenderer`] implementations and action handlers are trusted
//! in-process host code.

mod binding;
mod builtin_components;
mod component;
mod diagnostic;
pub mod diff;
mod html_source;
mod input_cache;
#[cfg(test)]
mod input_cache_tests;
mod limits;
pub mod parser;
mod render_context;
mod renderer;
pub mod runtime;
mod stateful_nodes;
pub mod tailwind;
mod tailwind_style;
mod template;
pub mod vnode;

pub use binding::{BindingResolution, resolve_bindings, resolve_bindings_checked};
pub use component::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticPhase, DiagnosticSeverity, Diagnostics, SourceSpan,
};
pub use diff::{DiffError, NodePath, Patch, PatchKind, apply_patches, diff};
pub use limits::{
    CompileLimits, DEFAULT_MAX_ATTRIBUTES, DEFAULT_MAX_CLASSES, DEFAULT_MAX_DEPTH,
    DEFAULT_MAX_NODES, DEFAULT_MAX_SOURCE_BYTES, ParseResource,
};
pub use parser::{HtmlParseError, parse_html, parse_html_with_limits};
pub use render_context::RenderContext;
pub use renderer::{DeclarativeView, DeclarativeViewConfig};
pub use runtime::{
    ActionContext, ActionError, ActionEvent, ActionOutcome, Runtime, RuntimeError, RuntimeEvent,
    StateChange, StateChangeOrigin, StateStore,
};
pub use tailwind::{
    ColorToken, MAX_SPACING_SCALE, TailwindModifier, TailwindParseResult, parse_classes,
};
pub use tailwind_style::apply_modifiers;
pub use template::{
    CompileOptions, CompiledTemplate, TemplateCompileError, ValidationMode, compile_template,
};
pub use vnode::{VElement, VNode};
