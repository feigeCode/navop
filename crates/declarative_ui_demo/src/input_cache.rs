use std::collections::HashMap;

use gpui::{App, AppContext, Entity, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};

use crate::{
    ComponentProps, Runtime, VNode,
    stateful_nodes::{StatefulInputSpec, stateful_input_specs},
};

pub(crate) struct InputRequest {
    id: String,
    spec: StatefulInputSpec,
    runtime: Entity<Runtime>,
}

impl InputRequest {
    pub(crate) fn new(props: &ComponentProps, multiline: bool, runtime: Entity<Runtime>) -> Self {
        Self {
            id: props.stable_id(),
            spec: StatefulInputSpec::from_element(&props.element, multiline),
            runtime,
        }
    }
}

pub(crate) struct InputEnvironment<'a> {
    pub(crate) window: &'a mut Window,
    pub(crate) cx: &'a mut App,
}

struct InputEntry {
    state: Entity<InputState>,
    spec: StatefulInputSpec,
    _subscription: Option<Subscription>,
}

#[derive(Default)]
pub(crate) struct InputCache {
    entries: HashMap<String, InputEntry>,
}

impl InputCache {
    pub(crate) fn resolve(
        &mut self,
        request: InputRequest,
        environment: InputEnvironment<'_>,
    ) -> Entity<InputState> {
        if let Some(entry) = self.entries.get_mut(&request.id)
            && entry.spec.has_same_configuration(&request.spec)
        {
            entry.sync_bound_value(&request.spec, environment);
            return entry.state.clone();
        }

        let id = request.id.clone();
        let entry = InputEntry::new(request, environment);
        let state = entry.state.clone();
        self.entries.insert(id, entry);
        state
    }

    pub(crate) fn retain_live(&mut self, root: &VNode) {
        let live = stateful_input_specs(root);
        self.entries.retain(|id, _| live.contains_key(id));
    }
}

impl InputEntry {
    fn new(request: InputRequest, environment: InputEnvironment<'_>) -> Self {
        let placeholder = request.spec.placeholder.clone();
        let value = request.spec.value.clone();
        let multiline = request.spec.multiline;
        let state = environment.cx.new(|cx| {
            let mut state = InputState::new(environment.window, cx).multi_line(multiline);
            if let Some(text) = placeholder {
                state = state.placeholder(text);
            }
            if let Some(text) = value {
                state = state.default_value(text);
            }
            state
        });
        let subscription = subscribe_binding(&state, &request, environment.cx);
        Self {
            state,
            spec: request.spec,
            _subscription: subscription,
        }
    }

    fn sync_bound_value(&mut self, next: &StatefulInputSpec, environment: InputEnvironment<'_>) {
        if next.bind.is_some() && self.spec.value != next.value {
            let value = next.value.clone().unwrap_or_default();
            self.state.update(environment.cx, |state, cx| {
                state.set_value(value, environment.window, cx);
            });
        }
        self.spec = next.clone();
    }
}

fn subscribe_binding(
    state: &Entity<InputState>,
    request: &InputRequest,
    cx: &mut App,
) -> Option<Subscription> {
    let key = request.spec.bind.clone()?;
    let runtime = request.runtime.clone();
    Some(cx.subscribe(state, move |input, event: &InputEvent, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let value = input.read(cx).value().to_string();
        let runtime = runtime.clone();
        let key = key.clone();
        cx.defer(move |cx| {
            runtime.update(cx, |runtime, cx| {
                runtime.set(key, value, cx);
            });
        });
    }))
}
