use gpui::{App, AppContext, Entity, EventEmitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentToolConfigEvent {
    Changed,
}

pub struct AgentToolConfigNotifier;

impl EventEmitter<AgentToolConfigEvent> for AgentToolConfigNotifier {}

#[derive(Clone)]
struct GlobalAgentToolConfigNotifier(Entity<AgentToolConfigNotifier>);

impl gpui::Global for GlobalAgentToolConfigNotifier {}

pub(crate) fn init(cx: &mut App) {
    let notifier = cx.new(|_| AgentToolConfigNotifier);
    cx.set_global(GlobalAgentToolConfigNotifier(notifier));
}

pub(crate) fn get_notifier(cx: &App) -> Option<Entity<AgentToolConfigNotifier>> {
    cx.try_global::<GlobalAgentToolConfigNotifier>()
        .map(|global| global.0.clone())
}

pub fn emit_agent_tool_config_changed(cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    notifier.update(cx, |_, cx| cx.emit(AgentToolConfigEvent::Changed));
}
