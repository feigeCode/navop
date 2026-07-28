use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

use gpui::{
    AnyElement, App, Entity, IntoElement, ParentElement, ScrollHandle, Styled, Window, div,
};
use gpui_component::input::InputState;
use gpui_component::slider::SliderState;

use crate::{
    ActionEvent, ComponentProps, ComponentRegistry, Diagnostic, DiagnosticCode, DiagnosticPhase,
    DiagnosticSeverity, Diagnostics, NodePath, Runtime, VElement, VNode, apply_modifiers,
    input_cache::{InputCache, InputEnvironment, InputRequest},
    parse_classes,
    slider_cache::{SliderCache, SliderEnvironment, SliderRequest},
};

pub(crate) type ActionDispatcher = Rc<dyn Fn(ActionEvent, &mut App)>;
pub(crate) type StateDispatcher = Rc<dyn Fn(String, String, &mut App)>;

pub(crate) struct RenderEnvironment<'a> {
    pub(crate) registry: &'a ComponentRegistry,
    pub(crate) input_cache: &'a mut InputCache,
    pub(crate) slider_cache: &'a mut SliderCache,
    pub(crate) runtime: Entity<Runtime>,
    pub(crate) dispatcher: ActionDispatcher,
    pub(crate) diagnostics: &'a mut Diagnostics,
    pub(crate) warnings: &'a mut Vec<String>,
    pub(crate) window: &'a mut Window,
    pub(crate) cx: &'a mut App,
}

pub struct RenderContext<'a> {
    registry: &'a ComponentRegistry,
    input_cache: &'a mut InputCache,
    slider_cache: &'a mut SliderCache,
    runtime: Entity<Runtime>,
    dispatcher: ActionDispatcher,
    diagnostics: &'a mut Diagnostics,
    warnings: &'a mut Vec<String>,
    window: &'a mut Window,
    cx: &'a mut App,
}

impl<'a> RenderContext<'a> {
    pub(crate) fn new(environment: RenderEnvironment<'a>) -> Self {
        Self {
            registry: environment.registry,
            input_cache: environment.input_cache,
            slider_cache: environment.slider_cache,
            runtime: environment.runtime,
            dispatcher: environment.dispatcher,
            diagnostics: environment.diagnostics,
            warnings: environment.warnings,
            window: environment.window,
            cx: environment.cx,
        }
    }

    pub fn render_children(&mut self, props: &ComponentProps) -> Vec<AnyElement> {
        props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| self.render_node(child, &props.path.child(index)))
            .collect()
    }

    pub(crate) fn render_child(&mut self, props: &ComponentProps, index: usize) -> AnyElement {
        self.render_node(&props.element.children[index], &props.path.child(index))
    }

    pub fn style<E: Styled>(&mut self, element: E, props: &ComponentProps) -> E {
        let parsed = parse_classes(&props.element.classes);
        self.record_unsupported_classes(props, &parsed.unsupported);
        apply_modifiers(element, &parsed.modifiers)
    }

    pub(crate) fn render_root(&mut self, node: &VNode) -> AnyElement {
        self.render_node(node, &NodePath::root())
    }

    pub(crate) fn action_dispatcher(&self) -> ActionDispatcher {
        self.dispatcher.clone()
    }

    pub(crate) fn runtime_entity(&self) -> Entity<Runtime> {
        self.runtime.clone()
    }

    pub(crate) fn state_dispatcher(&self) -> StateDispatcher {
        let runtime = self.runtime.clone();
        Rc::new(move |key, value, cx| {
            runtime.update(cx, |runtime, cx| {
                runtime.set(key, value, cx);
            });
        })
    }

    pub(crate) fn input_state(
        &mut self,
        props: &ComponentProps,
        multiline: bool,
    ) -> Entity<InputState> {
        let request = InputRequest::new(props, multiline, self.runtime.clone());
        let environment = InputEnvironment {
            window: self.window,
            cx: self.cx,
        };
        self.input_cache.resolve(request, environment)
    }

    pub(crate) fn slider_state(&mut self, request: SliderRequest) -> Entity<SliderState> {
        let environment = SliderEnvironment {
            runtime: self.runtime.clone(),
            dispatcher: self.dispatcher.clone(),
            window: self.window,
            cx: self.cx,
        };
        self.slider_cache.resolve(request, environment)
    }

    pub(crate) fn scroll_handle(&mut self, props: &ComponentProps) -> ScrollHandle {
        self.window
            .use_keyed_state(scroll_handle_state_key(props), self.cx, |_, _| {
                ScrollHandle::default()
            })
            .read(self.cx)
            .clone()
    }

    fn render_node(&mut self, node: &VNode, path: &NodePath) -> AnyElement {
        match node {
            VNode::Text(text) => div().child(text.clone()).into_any_element(),
            VNode::Fragment(children) => self.render_fragment(children, path),
            VNode::Element(element) => self.render_element(element, path),
        }
    }

    fn render_fragment(&mut self, children: &[VNode], path: &NodePath) -> AnyElement {
        let children = children
            .iter()
            .enumerate()
            .map(|(index, child)| self.render_node(child, &path.child(index)))
            .collect::<Vec<_>>();
        div().children(children).into_any_element()
    }

    fn render_element(&mut self, element: &VElement, path: &NodePath) -> AnyElement {
        let props = ComponentProps::new(element.clone(), path.clone());
        let Some(renderer) = self.registry.renderer(&element.tag) else {
            return self.render_unknown_component(&props);
        };
        let result = catch_unwind(AssertUnwindSafe(|| renderer.render(props.clone(), self)));
        match result {
            Ok(Ok(element)) => element,
            Ok(Err(error)) => self.render_component_failure(
                &props,
                RenderFailure::error(DiagnosticCode::ComponentRenderFailed, error.to_string()),
            ),
            Err(payload) => self.render_component_failure(
                &props,
                RenderFailure::error(DiagnosticCode::ComponentPanicked, panic_message(payload)),
            ),
        }
    }

    fn render_unknown_component(&mut self, props: &ComponentProps) -> AnyElement {
        self.render_component_failure(
            props,
            RenderFailure::warning(
                DiagnosticCode::UnknownTag,
                format!("component <{}> is not registered", props.element.tag),
            ),
        )
    }

    fn render_component_failure(
        &mut self,
        props: &ComponentProps,
        failure: RenderFailure,
    ) -> AnyElement {
        self.diagnostics
            .push(failure.diagnostic().at_path(props.path.clone()));
        render_failure(&props.element.tag, &failure.message)
    }

    fn record_unsupported_classes(&mut self, props: &ComponentProps, classes: &[String]) {
        self.warnings.extend(
            classes
                .iter()
                .map(|class| format!("{}: unsupported class `{class}`", props.stable_id())),
        );
    }
}

fn scroll_handle_state_key(props: &ComponentProps) -> String {
    format!("{}:scroll-handle", props.stable_id())
}

struct RenderFailure {
    severity: DiagnosticSeverity,
    code: DiagnosticCode,
    message: String,
}

impl RenderFailure {
    fn error(code: DiagnosticCode, message: String) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code,
            message,
        }
    }

    fn warning(code: DiagnosticCode, message: String) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            code,
            message,
        }
    }

    fn diagnostic(&self) -> Diagnostic {
        match self.severity {
            DiagnosticSeverity::Error => {
                Diagnostic::error(DiagnosticPhase::Render, self.code, self.message.clone())
            }
            DiagnosticSeverity::Warning => {
                Diagnostic::warning(DiagnosticPhase::Render, self.code, self.message.clone())
            }
        }
    }
}

fn render_failure(tag: &str, message: &str) -> AnyElement {
    div()
        .child(format!("component <{tag}> failed: {message}"))
        .into_any_element()
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{ComponentProps, NodePath, VElement};

    use super::scroll_handle_state_key;

    #[test]
    fn explicit_scroll_identity_keeps_handle_state_independent_of_tree_path() {
        let element = VElement {
            tag: "scroll".to_owned(),
            attrs: BTreeMap::from([("id".to_owned(), "audit-log".to_owned())]),
            classes: Vec::new(),
            children: Vec::new(),
        };
        let root = ComponentProps::new(element.clone(), NodePath::root());
        let nested = ComponentProps::new(element, NodePath(vec![4, 2]));

        assert_eq!(
            "scroll:audit-log:scroll-handle",
            scroll_handle_state_key(&root)
        );
        assert_eq!(
            scroll_handle_state_key(&root),
            scroll_handle_state_key(&nested)
        );
    }
}
