//! ACP agent 配置变更通知。
//!
//! 设置页改的是磁盘上的 `acp-agents.json` 与 `AppSettings.ai_chat.last_acp_agent_id`，
//! 而已经挂载的聊天面板缓存着自己的 agent 列表/当前后端。改完不通知就会出现
//! 「设置里切了 agent，聊天还连在旧的上面」，所以这里用一个全局通知器把变更广播给
//! 所有活着的聊天视图。

use gpui::{App, AppContext as _, Entity, EventEmitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpAgentConfigEvent {
    /// agent 列表、启用状态或当前选中项发生了变化。
    Changed,
}

pub struct AcpAgentConfigNotifier;

impl EventEmitter<AcpAgentConfigEvent> for AcpAgentConfigNotifier {}

#[derive(Clone)]
struct GlobalAcpAgentConfigNotifier(Entity<AcpAgentConfigNotifier>);

impl gpui::Global for GlobalAcpAgentConfigNotifier {}

pub(crate) fn init(cx: &mut App) {
    let notifier = cx.new(|_| AcpAgentConfigNotifier);
    cx.set_global(GlobalAcpAgentConfigNotifier(notifier));
}

/// 订阅入口；通知器还没建好时返回 `None`（调用方按「不会收到事件」处理即可）。
pub fn acp_agent_config_notifier(cx: &App) -> Option<Entity<AcpAgentConfigNotifier>> {
    cx.try_global::<GlobalAcpAgentConfigNotifier>()
        .map(|global| global.0.clone())
}

pub fn emit_acp_agent_config_changed(cx: &mut App) {
    let Some(notifier) = acp_agent_config_notifier(cx) else {
        return;
    };
    notifier.update(cx, |_, cx| cx.emit(AcpAgentConfigEvent::Changed));
}
