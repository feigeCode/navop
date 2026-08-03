pub fn resolve_endpoint(base_url: &str, required_prefix: &str, leaf_path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let prefix = required_prefix.trim_end_matches('/');
    let leaf = leaf_path.trim_start_matches('/');

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
