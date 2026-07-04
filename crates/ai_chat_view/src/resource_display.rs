pub(crate) fn first_visible_alias(aliases: &[String]) -> Option<String> {
    aliases.iter().find(|alias| visible_alias(alias)).cloned()
}

pub(crate) fn visible_alias(alias: &str) -> bool {
    !looks_like_system_identifier(alias)
}

fn looks_like_system_identifier(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }

    let hyphen_count = value.chars().filter(|ch| *ch == '-').count();
    let hex_or_hyphen = value.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-');

    hex_or_hyphen && ((value.len() >= 30 && hyphen_count >= 3) || value.len() >= 32)
}

#[cfg(test)]
mod tests {
    use super::{first_visible_alias, visible_alias};

    #[test]
    fn uuid_like_alias_is_not_visible() {
        assert!(!visible_alias("abfcee0a-2827-4588-9f6-587a7a95d1e9"));
        assert!(!visible_alias("abfcee0a282745889f6a587a7a95d1e9"));
    }

    #[test]
    fn host_alias_is_visible() {
        assert!(visible_alias("10.1.131.181"));
        assert!(visible_alias("prod-db.internal"));
    }

    #[test]
    fn first_visible_alias_skips_uuid_like_values() {
        let aliases = vec![
            "abfcee0a-2827-4588-9f6-587a7a95d1e9".to_string(),
            "10.1.131.181".to_string(),
        ];

        assert_eq!(
            Some("10.1.131.181".to_string()),
            first_visible_alias(&aliases)
        );
    }
}
