use super::*;

impl TerminalView {
    pub(super) fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn show_unbracketed_paste_block_dialog(
        &mut self,
        text: &str,
        hazard: UnbracketedPasteHazard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = t!("TerminalView.unbracketed_paste_block_title").to_string();
        let message = match hazard {
            UnbracketedPasteHazard::HereDoc => {
                t!("TerminalView.unbracketed_paste_heredoc_message").to_string()
            }
            UnbracketedPasteHazard::UnterminatedQuote => {
                t!("TerminalView.unbracketed_paste_quote_message").to_string()
            }
        };
        let text = text.to_string();
        let preview_text = Self::paste_preview_text(&text);
        let summary_text = Self::paste_summary_text(&text);
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
                        .child(
                            h_flex()
                                .justify_between()
                                .child(div().text_xs().child(t!("TerminalView.paste_preview")))
                                .child(div().text_xs().child(summary_text.clone())),
                        )
                        .child(
                            v_flex()
                                .id("paste-preview")
                                .max_h(px(160.0))
                                .overflow_y_scroll()
                                .text_xs()
                                .child(preview_text.clone()),
                        )
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("TerminalView.paste_anyway"))
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

    pub(super) fn contains_high_risk_command(text: &str) -> bool {
        text.lines().any(|line| {
            let cmd = line.trim().to_lowercase();
            if cmd.is_empty() {
                return false;
            }

            cmd.starts_with("rm -rf")
                || cmd.contains(" rm -rf ")
                || cmd.starts_with("mkfs")
                || cmd.starts_with("dd if=")
                || cmd.starts_with("shutdown ")
                || cmd.starts_with("reboot")
                || cmd.starts_with("poweroff")
                || cmd.starts_with("systemctl stop ")
                || cmd.starts_with("systemctl disable ")
                || cmd.starts_with("chmod -r 777 /")
                || cmd.starts_with("chown -r ")
                || cmd.contains(":(){")
                || cmd.contains("curl ") && (cmd.contains("| sh") || cmd.contains("| bash"))
                || cmd.contains("wget ") && (cmd.contains("| sh") || cmd.contains("| bash"))
        })
    }
}
