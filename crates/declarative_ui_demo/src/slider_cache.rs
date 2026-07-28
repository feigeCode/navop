use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui::{App, AppContext, Entity, Subscription, Window};
use gpui_component::slider::{SliderEvent, SliderScale, SliderState, SliderValue};

use crate::{
    ActionEvent, NodePath, Runtime, VNode, component::stable_component_id,
    render_context::ActionDispatcher,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SliderConfig {
    pub(crate) min: f32,
    pub(crate) max: f32,
    pub(crate) step: f32,
    pub(crate) value: f32,
    pub(crate) scale: SliderScale,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SliderCallbacks {
    binding: Option<String>,
    action: Option<ActionEvent>,
}

impl SliderCallbacks {
    pub(crate) fn new(binding: Option<String>, action: Option<ActionEvent>) -> Self {
        Self { binding, action }
    }
}

pub(crate) struct SliderRequest {
    id: String,
    config: SliderConfig,
    callbacks: SliderCallbacks,
}

impl SliderRequest {
    pub(crate) fn new(
        id: impl Into<String>,
        config: SliderConfig,
        callbacks: SliderCallbacks,
    ) -> Self {
        Self {
            id: id.into(),
            config,
            callbacks,
        }
    }
}

pub(crate) struct SliderEnvironment<'a> {
    pub(crate) runtime: Entity<Runtime>,
    pub(crate) dispatcher: ActionDispatcher,
    pub(crate) window: &'a mut Window,
    pub(crate) cx: &'a mut App,
}

struct SliderEntry {
    state: Entity<SliderState>,
    config: SliderConfig,
    callbacks: Rc<RefCell<SliderCallbacks>>,
    _subscription: Subscription,
}

#[derive(Default)]
pub(crate) struct SliderCache {
    entries: HashMap<String, SliderEntry>,
}

impl SliderCache {
    pub(crate) fn resolve(
        &mut self,
        request: SliderRequest,
        environment: SliderEnvironment<'_>,
    ) -> Entity<SliderState> {
        let id = request.id.clone();
        if let Some(entry) = self.entries.get_mut(&id) {
            if entry.can_reuse(&request) {
                entry.update(request, environment);
                return entry.state.clone();
            }
        }

        let entry = SliderEntry::new(request, environment);
        let state = entry.state.clone();
        self.entries.insert(id, entry);
        state
    }

    pub(crate) fn retain_live(&mut self, root: &VNode) {
        let live = stateful_slider_ids(root);
        self.entries.retain(|id, _| live.contains(id));
    }
}

impl SliderEntry {
    fn new(request: SliderRequest, environment: SliderEnvironment<'_>) -> Self {
        let config = request.config;
        let state = environment.cx.new(|_| {
            SliderState::new()
                .min(config.min)
                .max(config.max)
                .step(config.step)
                .default_value(config.value)
                .scale(config.scale)
        });
        let callbacks = Rc::new(RefCell::new(request.callbacks));
        let subscription = subscribe(
            &state,
            callbacks.clone(),
            config,
            environment.runtime,
            environment.dispatcher,
            environment.cx,
        );
        Self {
            state,
            config,
            callbacks,
            _subscription: subscription,
        }
    }

    fn can_reuse(&self, request: &SliderRequest) -> bool {
        let current_bound = self.callbacks.borrow().binding.is_some();
        let next_bound = request.callbacks.binding.is_some();
        same_slider_configuration(self.config, request.config)
            && current_bound == next_bound
            && (next_bound || self.config.value == request.config.value)
    }

    fn update(&mut self, request: SliderRequest, environment: SliderEnvironment<'_>) {
        self.callbacks.replace(request.callbacks);
        if self.callbacks.borrow().binding.is_some()
            && self.state.read(environment.cx).value() != SliderValue::Single(request.config.value)
        {
            self.state.update(environment.cx, |state, cx| {
                state.set_value(request.config.value, environment.window, cx);
            });
        }
        self.config = request.config;
    }
}

fn subscribe(
    state: &Entity<SliderState>,
    callbacks: Rc<RefCell<SliderCallbacks>>,
    config: SliderConfig,
    runtime: Entity<Runtime>,
    dispatcher: ActionDispatcher,
    cx: &mut App,
) -> Subscription {
    cx.subscribe(state, move |_, event: &SliderEvent, cx| {
        let callbacks = callbacks.borrow().clone();
        match event {
            SliderEvent::Change(value) => {
                write_binding(
                    &callbacks,
                    value.clamp(config.min, config.max),
                    &runtime,
                    cx,
                );
            }
            SliderEvent::Release(value) => {
                write_binding(
                    &callbacks,
                    value.clamp(config.min, config.max),
                    &runtime,
                    cx,
                );
                if let Some(event) = &callbacks.action {
                    dispatcher(event.clone(), cx);
                }
            }
        }
    })
}

fn write_binding(
    callbacks: &SliderCallbacks,
    value: SliderValue,
    runtime: &Entity<Runtime>,
    cx: &mut App,
) {
    let (Some(binding), SliderValue::Single(value)) = (&callbacks.binding, value) else {
        return;
    };
    runtime.update(cx, |runtime, cx| {
        runtime.set(binding.clone(), value.to_string(), cx);
    });
}

fn same_slider_configuration(current: SliderConfig, next: SliderConfig) -> bool {
    current.min == next.min
        && current.max == next.max
        && current.step == next.step
        && current.scale == next.scale
}

fn stateful_slider_ids(root: &VNode) -> HashSet<String> {
    let mut ids = HashSet::new();
    collect_slider_ids(root, &NodePath::root(), &mut ids);
    ids
}

fn collect_slider_ids(node: &VNode, path: &NodePath, ids: &mut HashSet<String>) {
    match node {
        VNode::Element(element) => {
            if element.tag.eq_ignore_ascii_case("slider") {
                ids.insert(stable_component_id(element, path));
            }
            collect_slider_children(&element.children, path, ids);
        }
        VNode::Fragment(children) => collect_slider_children(children, path, ids),
        VNode::Text(_) => {}
    }
}

fn collect_slider_children(children: &[VNode], path: &NodePath, ids: &mut HashSet<String>) {
    for (index, child) in children.iter().enumerate() {
        collect_slider_ids(child, &path.child(index), ids);
    }
}
