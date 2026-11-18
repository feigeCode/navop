pub fn resolve_endpoint(base_url: &str, required_prefix: &str, leaf_path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let prefix = required_prefix.trim_end_matches('/');
    let leaf = leaf_path.trim_start_matches('/');

    if !leaf.is_empty() && (base == leaf || base.ends_with(&format!("/{leaf}"))) {
        return base.to_string();
    }

    if prefix.is_empty() || base.ends_with(prefix) {
        format!("{}/{}", base, leaf)
    } else {
        format!("{}/{}/{}", base, prefix.trim_start_matches('/'), leaf)
    }
}

pub fn resolve_prefixed_endpoint(
    base_url: &str,
    required_prefix: &str,
    suffix_path: &str,
) -> String {
    let base = base_url.trim_end_matches('/');
    let prefix = required_prefix.trim_end_matches('/');
    let suffix = suffix_path.trim_start_matches('/');

    if prefix.is_empty() || base.ends_with(prefix) {
        format!("{}/{}", base, suffix)
    } else {
        format!("{}/{}/{}", base, prefix.trim_start_matches('/'), suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_endpoint;

    #[test]
    fn resolve_endpoint_preserves_a_complete_chat_completions_url() {
        let endpoint = "https://ark.cn-beijing.volces.com/api/v3/chat/completions";

        assert_eq!(
            endpoint,
            resolve_endpoint(endpoint, "", "/chat/completions")
        );
    }

    #[test]
    fn resolve_endpoint_still_appends_the_leaf_to_a_base_url() {
        assert_eq!(
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            resolve_endpoint(
                "https://ark.cn-beijing.volces.com/api/v3",
                "",
                "/chat/completions"
            )
        );
    }
}
