use gpui::{App, AppContext, Entity, EventEmitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderConfigEvent {
    Changed,
}

pub struct ProviderConfigNotifier;

impl EventEmitter<ProviderConfigEvent> for ProviderConfigNotifier {}

#[derive(Clone)]
pub struct GlobalProviderConfigNotifier(pub Entity<ProviderConfigNotifier>);

impl gpui::Global for GlobalProviderConfigNotifier {}

pub fn init(cx: &mut App) {
    let notifier = cx.new(|_| ProviderConfigNotifier);
    cx.set_global(GlobalProviderConfigNotifier(notifier));
}

pub fn get_notifier(cx: &App) -> Option<Entity<ProviderConfigNotifier>> {
    cx.try_global::<GlobalProviderConfigNotifier>()
        .map(|global| global.0.clone())
}

pub fn emit_provider_config_changed(cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    notifier.update(cx, |_, cx| cx.emit(ProviderConfigEvent::Changed));
}
