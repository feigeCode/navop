pub(super) fn is_valid_baud_rate_text(text: &str) -> bool {
    let text = text.trim();
    text.is_empty() || text.parse::<u32>().is_ok_and(|value| value > 0)
}

pub(super) fn resolve_baud_rate(custom_text: &str, selected: Option<u32>) -> Option<u32> {
    let custom_text = custom_text.trim();
    if custom_text.is_empty() {
        return selected.filter(|value| *value > 0);
    }

    custom_text.parse::<u32>().ok().filter(|value| *value > 0)
}

pub(super) fn custom_baud_rate_text(baud_rate: u32, presets: &[u32]) -> Option<String> {
    (!presets.contains(&baud_rate)).then(|| baud_rate.to_string())
}

#[cfg(test)]
mod tests {
    use super::{custom_baud_rate_text, is_valid_baud_rate_text, resolve_baud_rate};

    #[test]
    fn custom_baud_rate_overrides_selected_preset() {
        assert_eq!(Some(1_500_000), resolve_baud_rate("1500000", Some(115_200)));
    }

    #[test]
    fn blank_custom_baud_rate_uses_selected_preset() {
        assert_eq!(Some(115_200), resolve_baud_rate("  ", Some(115_200)));
    }

    #[test]
    fn custom_baud_rate_must_be_a_positive_u32() {
        assert!(is_valid_baud_rate_text(""));
        assert!(is_valid_baud_rate_text("1500000"));
        assert!(!is_valid_baud_rate_text("0"));
        assert!(!is_valid_baud_rate_text("-1"));
        assert!(!is_valid_baud_rate_text("4294967296"));
    }

    #[test]
    fn editing_non_preset_baud_rate_populates_custom_input() {
        let presets = [9_600, 115_200, 921_600];

        assert_eq!(
            Some("1500000".to_string()),
            custom_baud_rate_text(1_500_000, &presets)
        );
        assert_eq!(None, custom_baud_rate_text(115_200, &presets));
    }
}
