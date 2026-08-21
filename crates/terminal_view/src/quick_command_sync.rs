use gpui::{App, AppContext, Context, Entity, EventEmitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuickCommandSyncEvent {
    Changed,
}

pub(crate) struct QuickCommandSyncNotifier;

impl EventEmitter<QuickCommandSyncEvent> for QuickCommandSyncNotifier {}

#[derive(Clone)]
pub(crate) struct GlobalQuickCommandSyncNotifier(pub Entity<QuickCommandSyncNotifier>);

impl gpui::Global for GlobalQuickCommandSyncNotifier {}

pub(crate) fn init_quick_command_sync(cx: &mut App) {
    if cx.try_global::<GlobalQuickCommandSyncNotifier>().is_none() {
        let notifier = cx.new(|_| QuickCommandSyncNotifier);
        cx.set_global(GlobalQuickCommandSyncNotifier(notifier));
    }
}

pub(crate) fn quick_command_sync_notifier(cx: &App) -> Option<Entity<QuickCommandSyncNotifier>> {
    cx.try_global::<GlobalQuickCommandSyncNotifier>()
        .map(|global| global.0.clone())
}

pub(crate) fn emit_quick_commands_changed<T>(cx: &mut Context<T>) {
    let Some(notifier) = quick_command_sync_notifier(cx) else {
        return;
    };
    notifier.update(cx, |_, cx| cx.emit(QuickCommandSyncEvent::Changed));
}
