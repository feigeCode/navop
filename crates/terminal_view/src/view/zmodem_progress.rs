use super::*;
use gpui_component::progress::Progress;

impl TerminalView {
    pub(super) fn render_zmodem_progress(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let progress = self.terminal.read(cx).zmodem_transfer_progress()?;
        let percent = progress.percent();
        let file_number = progress.file_index().saturating_add(1);
        let total_status = format!(
            "{}% · {} / {}",
            percent,
            format_zmodem_bytes(progress.transferred()),
            format_zmodem_bytes(progress.total())
        );
        let file_status = format!(
            "{} ({}/{})",
            progress.file_name(),
            file_number,
            progress.file_count()
        );

        Some(
            v_flex()
                .debug_selector(|| "terminal-zmodem-upload".to_string())
                .flex_shrink_0()
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .px_2()
                .py_1()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(IconName::ArrowUp)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("TerminalZmodem.uploading").to_string()),
                        )
                        .child(
                            div()
                                .debug_selector(|| "terminal-zmodem-upload-name".to_string())
                                .flex_1()
                                .text_xs()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(file_status),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(total_status),
                        ),
                )
                .child(Progress::new("terminal-zmodem-upload-progress").value(percent as f32))
                .into_any_element(),
        )
    }
}

pub(super) fn format_zmodem_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::format_zmodem_bytes;

    #[test]
    fn formats_upload_byte_counts() {
        assert_eq!("0 B", format_zmodem_bytes(0));
        assert_eq!("1.0 KB", format_zmodem_bytes(1024));
        assert_eq!("1.0 MB", format_zmodem_bytes(1024 * 1024));
    }
}
