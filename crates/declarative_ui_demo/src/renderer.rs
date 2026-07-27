use std::rc::Rc;

use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
};

use crate::{
    ActionEvent, BindingResolution, CompiledTemplate, ComponentRegistry, Diagnostic,
    DiagnosticCode, DiagnosticPhase, Diagnostics, DiffError, NodePath, Patch, Runtime,
    RuntimeEvent, VNode, apply_patches, diff,
    input_cache::InputCache,
    render_context::{ActionDispatcher, RenderContext, RenderEnvironment},
    resolve_bindings_checked,
};

#[derive(Clone)]
pub struct DeclarativeViewConfig {
    template: CompiledTemplate,
    runtime: Entity<Runtime>,
    registry: ComponentRegistry,
}

impl DeclarativeViewConfig {
    pub fn new(
        template: CompiledTemplate,
        runtime: Entity<Runtime>,
        registry: ComponentRegistry,
    ) -> Self {
        Self {
            template,
            runtime,
            registry,
        }
    }
}

pub struct DeclarativeView {
    template: CompiledTemplate,
    rendered: VNode,
    registry: ComponentRegistry,
    runtime: Entity<Runtime>,
    input_cache: InputCache,
    last_patches: Vec<Patch>,
    diagnostics: Diagnostics,
    warnings: Vec<String>,
    last_error: Option<ViewError>,
    _runtime_subscription: Subscription,
}

impl DeclarativeView {
    pub fn new(config: DeclarativeViewConfig, cx: &mut Context<Self>) -> Self {
        let resolution = resolve_runtime_bindings(&config.template, &config.runtime, cx);
        let mut diagnostics = config.template.diagnostics().clone();
        diagnostics.replace_phase(DiagnosticPhase::Binding, resolution.diagnostics);
        let runtime_subscription =
            cx.subscribe(&config.runtime, |view, _, event: &RuntimeEvent, cx| {
                view.handle_runtime_event(event, cx);
            });
        Self {
            template: config.template,
            rendered: resolution.root,
            registry: config.registry,
            runtime: config.runtime,
            input_cache: InputCache::default(),
            last_patches: Vec::new(),
            diagnostics,
            warnings: Vec::new(),
            last_error: None,
            _runtime_subscription: runtime_subscription,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reconcile_or_record(cx);
        cx.notify();
    }

    pub fn rendered(&self) -> &VNode {
        &self.rendered
    }

    pub fn runtime(&self) -> &Entity<Runtime> {
        &self.runtime
    }

    pub fn last_patches(&self) -> &[Patch] {
        &self.last_patches
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_ref().map(|error| error.message.as_str())
    }

    fn handle_runtime_event(&mut self, event: &RuntimeEvent, cx: &mut Context<Self>) {
        match event {
            RuntimeEvent::StateChanged(_) => self.reconcile_or_record(cx),
            RuntimeEvent::ActionCompleted { .. } => self.clear_action_error(),
            RuntimeEvent::ActionFailed { event, error } => {
                self.set_runtime_error(ViewError::action(event, error));
            }
        }
        cx.notify();
    }

    fn reconcile_or_record(&mut self, cx: &mut App) {
        match self.reconcile(cx) {
            Ok(()) => self.clear_runtime_error(),
            Err(error) => self.set_runtime_error(ViewError::reconciliation(error)),
        }
    }

    fn reconcile(&mut self, cx: &mut App) -> Result<(), DiffError> {
        let resolution = resolve_runtime_bindings(&self.template, &self.runtime, cx);
        self.diagnostics
            .replace_phase(DiagnosticPhase::Binding, resolution.diagnostics);
        let patches = diff(&self.rendered, &resolution.root);
        let mut next = self.rendered.clone();
        apply_patches(&mut next, &patches)?;
        debug_assert_eq!(next, resolution.root);
        self.rendered = next;
        self.input_cache.retain_live(&self.rendered);
        self.last_patches = patches;
        Ok(())
    }

    fn clear_action_error(&mut self) {
        if self
            .last_error
            .as_ref()
            .is_some_and(ViewError::is_action_failure)
        {
            self.clear_runtime_error();
        }
    }

    fn clear_runtime_error(&mut self) {
        self.last_error = None;
        self.diagnostics
            .replace_phase(DiagnosticPhase::Runtime, std::iter::empty());
    }

    fn set_runtime_error(&mut self, error: ViewError) {
        self.diagnostics
            .replace_phase(DiagnosticPhase::Runtime, [error.diagnostic()]);
        self.last_error = Some(error);
    }
}

impl Render for DeclarativeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.diagnostics
            .replace_phase(DiagnosticPhase::Render, std::iter::empty());
        self.warnings.clear();
        let dispatcher = action_dispatcher(self.runtime.clone());
        let rendered = self.rendered.clone();
        let environment = RenderEnvironment {
            registry: &self.registry,
            input_cache: &mut self.input_cache,
            runtime: self.runtime.clone(),
            dispatcher,
            diagnostics: &mut self.diagnostics,
            warnings: &mut self.warnings,
            window,
            cx,
        };
        let content = RenderContext::new(environment).render_root(&rendered);
        let mut root = div().size_full().child(content);
        if let Some(error) = self.last_error() {
            root = root.child(div().child(error.to_owned()));
        }
        root
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewError {
    code: DiagnosticCode,
    message: String,
    path: Option<NodePath>,
}

impl ViewError {
    fn action(event: &ActionEvent, error: &crate::RuntimeError) -> Self {
        Self {
            code: DiagnosticCode::RuntimeActionFailed,
            message: error.to_string(),
            path: Some(event.source_path().clone()),
        }
    }

    fn reconciliation(error: DiffError) -> Self {
        Self {
            code: DiagnosticCode::ReconciliationFailed,
            message: error.to_string(),
            path: None,
        }
    }

    fn is_action_failure(&self) -> bool {
        self.code == DiagnosticCode::RuntimeActionFailed
    }

    fn diagnostic(&self) -> Diagnostic {
        let diagnostic =
            Diagnostic::error(DiagnosticPhase::Runtime, self.code, self.message.clone());
        match &self.path {
            Some(path) => diagnostic.at_path(path.clone()),
            None => diagnostic,
        }
    }
}

fn resolve_runtime_bindings(
    template: &CompiledTemplate,
    runtime: &Entity<Runtime>,
    cx: &App,
) -> BindingResolution {
    resolve_bindings_checked(template.root(), runtime.read(cx).state())
}

fn action_dispatcher(runtime: Entity<Runtime>) -> ActionDispatcher {
    Rc::new(move |event, cx| {
        let _ = runtime.update(cx, |runtime, cx| runtime.dispatch(event, cx));
    })
}

#[cfg(test)]
mod tests {
    use super::ViewError;
    use crate::{DiagnosticCode, DiffError, NodePath};

    #[test]
    fn only_action_failures_are_action_errors() {
        let reconciliation = ViewError::reconciliation(DiffError::InvalidPath(NodePath::root()));
        assert_eq!(DiagnosticCode::ReconciliationFailed, reconciliation.code);
        assert!(!reconciliation.is_action_failure());
    }
}
