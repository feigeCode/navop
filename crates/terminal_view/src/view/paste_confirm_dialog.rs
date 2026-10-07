//! 粘贴确认弹窗。
//!
//! 三类提示共用同一个弹窗，只有文案和「能关掉什么」不同：
//!
//! - [`PasteConfirmKind::MultilineUnbracketed`]：远端未开启 bracketed paste 的多行粘贴
//! - [`PasteConfirmKind::HighRiskCommand`]：命中高危命令列表
//! - [`PasteConfirmKind::LargePaste`]：超过行数/字节阈值（硬阈值，没有开关）
//!
//! 弹窗直接给出「打开设置」和「不再提示」，避免用户为了关掉提示还得自己
//! 去终端工具侧边栏里翻设置面板。

use super::clipboard_image::join_paste_as_single_line;
use super::*;

/// 粘贴确认弹窗的触发原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PasteConfirmKind {
    /// 远端未开启 bracketed paste 的多行粘贴。
    MultilineUnbracketed,
    /// 命中高危命令列表（`rm -rf`、`reboot`、`mkfs` 等）。
    HighRiskCommand,
    /// 内容超过行数或字节阈值。
    LargePaste,
}

/// 可以被弹窗「不再提示」关闭的安全确认开关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PasteSafetySetting {
    /// 对应 `terminal_confirm_multiline_paste`。
    MultilinePaste,
    /// 对应 `terminal_confirm_high_risk_command`。
    HighRiskCommand,
}

impl PasteConfirmKind {
    pub(super) fn title(self) -> String {
        match self {
            Self::MultilineUnbracketed => t!("TerminalView.multiline_paste_title"),
            Self::HighRiskCommand => t!("TerminalView.high_risk_paste_title"),
            Self::LargePaste => t!("TerminalView.large_paste_title"),
        }
        .to_string()
    }

    pub(super) fn message(self) -> String {
        match self {
            Self::MultilineUnbracketed => t!("TerminalView.multiline_paste_message"),
            Self::HighRiskCommand => t!("TerminalView.high_risk_paste_message"),
            Self::LargePaste => t!("TerminalView.large_paste_message"),
        }
        .to_string()
    }

    /// 「不再提示」对应哪一个开关；`None` 表示该提示没有开关（大段粘贴是硬阈值）。
    pub(super) fn disable_setting(self) -> Option<PasteSafetySetting> {
        match self {
            Self::MultilineUnbracketed => Some(PasteSafetySetting::MultilinePaste),
            Self::HighRiskCommand => Some(PasteSafetySetting::HighRiskCommand),
            Self::LargePaste => None,
        }
    }
}

/// 预览面板：标题行（含行数与字符数）加可滚动的正文预览。
fn paste_preview_panel(preview: &str, summary: &str, cx: &App) -> Div {
    v_flex()
        .gap_2()
        .min_h_0()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_xs().child(t!("TerminalView.paste_preview")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary.to_string()),
                ),
        )
        .child(
            v_flex()
                .id("paste-preview")
                .max_h(px(160.0))
                .overflow_y_scroll()
                .text_xs()
                .child(preview.to_string()),
        )
}

/// 弹窗正文里的可选动作按钮。
///
/// 「合并为单行粘贴」只对多行内容有意义；「打开设置」和「不再提示」只在
/// 该提示存在对应开关时才出现（大段粘贴是硬阈值，两者都不显示）。
fn paste_confirm_actions(
    view: &Entity<TerminalView>,
    text: &str,
    single_line_available: bool,
    disable_setting: Option<PasteSafetySetting>,
) -> Div {
    let view_single = view.clone();
    let text_single = text.to_string();
    let view_settings = view.clone();
    let view_disable = view.clone();

    h_flex()
        .gap_1()
        .when(single_line_available, |this| {
            this.child(
                Button::new("paste-single-line")
                    .label(t!("TerminalView.paste_as_single_line"))
                    .small()
                    .outline()
                    .on_click(move |_event, window, cx| {
                        let joined = join_paste_as_single_line(&text_single);
                        window.close_dialog(cx);
                        view_single.update(cx, |this, cx| {
                            this.paste_text_unchecked(&joined, window, cx);
                        });
                    }),
            )
        })
        .when_some(disable_setting, |this, setting| {
            this.child(
                Button::new("paste-open-settings")
                    .label(t!("TerminalView.paste_open_settings"))
                    .small()
                    .outline()
                    .on_click(move |_event, window, cx| {
                        window.close_dialog(cx);
                        view_settings.update(cx, |this, cx| {
                            this.open_paste_safety_settings(cx);
                        });
                    }),
            )
            .child(
                Button::new("paste-dont-ask-again")
                    .label(t!("TerminalView.paste_dont_ask_again"))
                    .small()
                    .outline()
                    .on_click(move |_event, window, cx| {
                        window.close_dialog(cx);
                        view_disable.update(cx, |this, cx| {
                            this.disable_paste_confirmation(setting, cx);
                        });
                    }),
            )
        })
}

impl TerminalView {
    pub(super) fn show_paste_confirm_dialog(
        &mut self,
        text: String,
        kind: PasteConfirmKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = kind.title();
        let message = kind.message();
        let preview_text = Self::paste_preview_text(&text);
        let summary_text = Self::paste_summary_text(&text);
        let single_line_available = multiline_non_empty_line_count(&text) > 1;
        let disable_setting = kind.disable_setting();
        let view = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_ok = view.clone();
            let text_ok = text.clone();

            dialog
                .title(title.clone())
                .child(
                    v_flex()
                        .gap_2()
                        .min_h_0()
                        .child(div().text_sm().child(message.clone()))
                        .child(paste_preview_panel(&preview_text, &summary_text, _cx))
                        .child(paste_confirm_actions(
                            &view,
                            &text,
                            single_line_available,
                            disable_setting,
                        ))
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.ok"))
                        .cancel_text(t!("Common.cancel")),
                )
                .on_ok(move |_event, window, cx| {
                    view_ok.update(cx, |this, cx| {
                        this.paste_text_unchecked(&text_ok, window, cx);
                    });
                    true
                })
        });
    }

    /// 打开终端工具侧边栏的设置面板（粘贴弹窗的「打开设置」入口）。
    ///
    /// 只负责把面板切到设置页，用户在「安全确认」区逐条调整。
    pub(super) fn open_paste_safety_settings(&mut self, cx: &mut Context<Self>) {
        if self.sidebar.read(cx).active_panel() == Some(SidebarPanel::Settings) {
            return;
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active_panel(Some(SidebarPanel::Settings), cx);
        });
        cx.notify();
    }

    /// 关闭某条粘贴确认并持久化，等价于用户在设置面板里手动关掉它。
    pub(super) fn disable_paste_confirmation(
        &mut self,
        setting: PasteSafetySetting,
        cx: &mut Context<Self>,
    ) {
        match setting {
            PasteSafetySetting::MultilinePaste => {
                let _ = update_settings(cx, |settings| settings.confirm_multiline_paste = false);
                self.apply_confirm_multiline_paste(false, cx);
            }
            PasteSafetySetting::HighRiskCommand => {
                let _ = update_settings(cx, |settings| settings.confirm_high_risk_command = false);
                self.apply_confirm_high_risk_command(false, cx);
            }
        }
    }
}
