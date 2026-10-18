//! 检查更新的共享状态。
//!
//! 手动与自动检查都会更新这里：设置页据此渲染按钮的加载态，并展示最近一次检查结果。
//! 目的是消除「点了按钮毫无反馈，几十秒后突然弹窗」这种突兀体验。

use chrono::Local;
use gpui::{App, AppContext, AsyncApp, Entity, Global};
use rust_i18n::t;

/// 一次检查结束后的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CheckReport {
    UpToDate,
    Available { version: String },
    Failed { reason: String },
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LastCheck {
    /// 完成时刻，形如 `09:12`。
    at: String,
    report: CheckReport,
}

pub(crate) struct UpdateCheckState {
    checking: bool,
    last_check: Option<LastCheck>,
}

impl UpdateCheckState {
    fn new() -> Self {
        Self {
            checking: false,
            last_check: None,
        }
    }

    pub(crate) fn is_checking(&self) -> bool {
        self.checking
    }

    /// 进入「检查中」。已有检查在跑时返回 `false`，调用方不得重复发起。
    fn begin_if_idle(&mut self) -> bool {
        if self.checking {
            return false;
        }
        self.checking = true;
        true
    }

    fn finish(&mut self, report: CheckReport) {
        self.checking = false;
        self.last_check = Some(LastCheck {
            at: Local::now().format("%H:%M").to_string(),
            report,
        });
    }

    /// 行内展示的最近一次检查结果。失败原因只进通知，这里保持一行短句。
    pub(crate) fn last_check_label(&self) -> Option<String> {
        let last = self.last_check.as_ref()?;
        Some(
            t!(
                "Settings.About.Update.last_check",
                time = last.at.as_str(),
                result = report_text(&last.report)
            )
            .to_string(),
        )
    }
}

fn report_text(report: &CheckReport) -> String {
    match report {
        CheckReport::UpToDate => t!("Settings.About.Update.result_up_to_date").to_string(),
        CheckReport::Available { version } => {
            t!("Settings.About.Update.result_available", version = version).to_string()
        }
        CheckReport::Failed { .. } => t!("Settings.About.Update.result_failed").to_string(),
        CheckReport::TimedOut => t!("Settings.About.Update.result_timeout").to_string(),
    }
}

struct GlobalUpdateCheckState(Entity<UpdateCheckState>);

impl Global for GlobalUpdateCheckState {}

/// 取得应用级检查状态，必要时创建。启动时的自动检查可能早于设置页打开，
/// 状态必须挂在 app 上才能被两边共用。
pub(crate) fn update_check_state(cx: &mut App) -> Entity<UpdateCheckState> {
    if let Some(global) = cx.try_global::<GlobalUpdateCheckState>() {
        return global.0.clone();
    }

    let state = cx.new(|_| UpdateCheckState::new());
    cx.set_global(GlobalUpdateCheckState(state.clone()));
    state
}

/// 标记检查开始；已有检查在跑时返回 `false`。
pub(crate) fn begin_check(cx: &mut App) -> bool {
    let state = update_check_state(cx);
    state.update(cx, |state, _| state.begin_if_idle())
}

/// 标记检查结束并记录结果，自动与手动检查都要调用。
pub(crate) fn finish_check(cx: &mut AsyncApp, report: CheckReport) {
    let _ = cx.update(move |cx| {
        let state = update_check_state(cx);
        state.update(cx, |state, _| state.finish(report));
    });
}

#[cfg(test)]
mod tests {
    use super::{CheckReport, UpdateCheckState};

    #[test]
    fn a_second_check_is_rejected_while_one_is_running() {
        let mut state = UpdateCheckState::new();

        assert!(!state.is_checking());
        assert!(state.begin_if_idle());
        assert!(!state.begin_if_idle(), "检查进行中不得重复发起");
        assert!(state.is_checking());

        state.finish(CheckReport::UpToDate);
        assert!(!state.is_checking());
        assert!(state.begin_if_idle());
    }

    #[test]
    fn no_last_check_label_before_the_first_check() {
        let state = UpdateCheckState::new();

        assert_eq!(None, state.last_check_label());
    }

    #[test]
    fn finished_checks_are_recorded_with_their_outcome() {
        for report in [
            CheckReport::UpToDate,
            CheckReport::Available {
                version: "9.9.9".to_string(),
            },
            CheckReport::Failed {
                reason: "dns failed".to_string(),
            },
            CheckReport::TimedOut,
        ] {
            let mut state = UpdateCheckState::new();
            state.begin_if_idle();
            state.finish(report.clone());

            let label = state.last_check_label().expect("检查完成后应有行内记录");
            assert!(label.contains(":"), "记录应包含 HH:MM 时刻: {label}");
            match &report {
                CheckReport::Available { version } => assert!(
                    label.contains(version),
                    "发现新版本的记录应带上版本号: {label}"
                ),
                CheckReport::Failed { reason } => assert!(
                    !label.contains(reason),
                    "行内记录不应塞入完整失败原因: {label}"
                ),
                _ => {}
            }
        }
    }
}
