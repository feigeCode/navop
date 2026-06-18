const MAX_STATUS_ERROR_CHARS: usize = 240;
const TRUNCATION_SUFFIX: &str = "...";

pub(crate) fn format_status_error(prefix: &str, err: &anyhow::Error) -> String {
    let detail = collapse_status_whitespace(&format!("{err:#}"));
    truncate_status_message(format!("{prefix}: {detail}"))
}

pub(crate) fn format_notification_error(prefix: &str, err: &anyhow::Error) -> String {
    let detail = collapse_status_whitespace(&format!("{err:#}"));
    format!("{prefix}: {detail}")
}

fn collapse_status_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_status_message(message: String) -> String {
    if message.chars().count() <= MAX_STATUS_ERROR_CHARS {
        return message;
    }
    let limit = MAX_STATUS_ERROR_CHARS.saturating_sub(TRUNCATION_SUFFIX.len());
    let mut truncated = message.chars().take(limit).collect::<String>();
    truncated.push_str(TRUNCATION_SUFFIX);
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_uses_display_chain_without_debug_backtrace() {
        let err = anyhow::anyhow!("missing driver.json").context("review local tarball");

        let message = format_status_error("安装失败", &err);

        assert_eq!(
            "安装失败: review local tarball: missing driver.json",
            message
        );
        assert!(!message.contains("Stack backtrace"));
        assert!(!message.contains('\n'));
    }

    #[test]
    fn notification_error_keeps_full_display_chain() {
        let long_url = format!("https://example.test/{}.json", "a".repeat(260));
        let err = anyhow::anyhow!("network down")
            .context(format!("fetch release manifest from {long_url}"));

        let message = format_notification_error("加载扩展市场失败", &err);

        assert!(message.contains(&long_url));
        assert!(message.contains("network down"));
        assert!(!message.contains('\n'));
    }
}
