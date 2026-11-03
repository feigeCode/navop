//! provider 运行监视：把 supervisor 的事件翻译成页面该做什么。
//!
//! `decide_runtime_change` 只做「事件 → 动作」的纯映射，好让判定逻辑能被
//! 单测直接覆盖，不必在 GPUI 里造真实 provider 进程。

use extension_plugin_adapter::RuntimeMonitorEvent;

/// 页面对一次监视事件的响应动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeChangeDecision {
    /// 与本 tab 无关,或只是瞬时抖动:保持现状。
    Ignore,
    /// provider 已无法自动恢复(被移除,或重启预算耗尽):置为 Failed。
    Fail,
    /// provider 已换成新进程(generation 推进):释放旧 session 并重连。
    Reconnect,
}

/// 推导 tab 对一次监视事件的响应动作。
///
/// 判定依据是 **provider 进程 generation** 而不是事件类型本身:
///
/// - `RuntimeRemoved` / 宿主已不再持有该 runtime:真没了,只能失败;
/// - 宿主 generation 已推进 = 真实重启已完成,旧 session 绑定的旧进程已消失,
///   必须重建(resource session 与 activation 都按 generation 判定归属);
/// - generation 未变且 session 已关闭、状态为 `Failed`/`CrashLoop` =
///   自动重启被禁用或重启预算耗尽,tab 无法自愈;
/// - 其余情况(`Degraded`,或仍在退避中的 `Restarting`)是瞬时抖动,忽略,
///   否则会在重启窗口内反复重建连接。
pub(crate) fn decide_runtime_change(
    event: &RuntimeMonitorEvent,
    runtime_id: &str,
    closing: bool,
    activation_generation: Option<u64>,
    current_generation: Option<u64>,
) -> RuntimeChangeDecision {
    use extension_plugin_adapter::RuntimeActivationState;

    if closing || event.runtime_id() != runtime_id {
        return RuntimeChangeDecision::Ignore;
    }
    if matches!(event, RuntimeMonitorEvent::RuntimeRemoved { .. }) {
        return RuntimeChangeDecision::Fail;
    }
    // 尚未建立连接(Connecting/Failed)时没有可失效的 lease。
    let Some(activation_generation) = activation_generation else {
        return RuntimeChangeDecision::Ignore;
    };
    // 宿主已不再持有该 runtime。
    let Some(current_generation) = current_generation else {
        return RuntimeChangeDecision::Fail;
    };
    if current_generation != activation_generation {
        return RuntimeChangeDecision::Reconnect;
    }
    match event {
        RuntimeMonitorEvent::HealthChanged { health, .. }
            if health.session_closed
                && matches!(
                    health.state,
                    RuntimeActivationState::Failed | RuntimeActivationState::CrashLoop
                ) =>
        {
            RuntimeChangeDecision::Fail
        }
        _ => RuntimeChangeDecision::Ignore,
    }
}
