use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

const ACCESS_TOKEN_HYPHEN_PLACEHOLDER: &str = "{access-token}";
const REFRESH_TOKEN_HYPHEN_PLACEHOLDER: &str = "{refresh-token}";
const ACCESS_TOKEN_UNDERSCORE_PLACEHOLDER: &str = "{access_token}";
const REFRESH_TOKEN_UNDERSCORE_PLACEHOLDER: &str = "{refresh_token}";

pub(crate) fn build_team_management_url(
    template: &str,
    access_token: &str,
    refresh_token: &str,
) -> String {
    let access_token = encode_url_component(access_token);
    let refresh_token = encode_url_component(refresh_token);

    template
        .replace(ACCESS_TOKEN_HYPHEN_PLACEHOLDER, &access_token)
        .replace(REFRESH_TOKEN_HYPHEN_PLACEHOLDER, &refresh_token)
        .replace(ACCESS_TOKEN_UNDERSCORE_PLACEHOLDER, &access_token)
        .replace(REFRESH_TOKEN_UNDERSCORE_PLACEHOLDER, &refresh_token)
}

pub(crate) fn resolve_team_management_url(url: &str, public_base_url: Option<&str>) -> String {
    let url = url.trim();
    if !url.starts_with('/') {
        return url.to_string();
    }

    public_base_url
        .map(|base_url| format!("{}{}", base_url.trim_end_matches('/'), url))
        .unwrap_or_else(|| url.to_string())
}

fn encode_url_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::{build_team_management_url, resolve_team_management_url};

    #[test]
    fn replaces_hyphen_placeholders() {
        let url = build_team_management_url(
            "/zh-CN/auth/desktop?access_token={access-token}&refresh_token={refresh-token}",
            "access.jwt",
            "refresh.jwt",
        );

        assert_eq!(
            "/zh-CN/auth/desktop?access_token=access%2Ejwt&refresh_token=refresh%2Ejwt",
            url
        );
    }

    #[test]
    fn replaces_underscore_placeholders() {
        let url = build_team_management_url(
            "https://example.com/auth?access={access_token}&refresh={refresh_token}",
            "access",
            "refresh",
        );

        assert_eq!(
            "https://example.com/auth?access=access&refresh=refresh",
            url
        );
    }

    #[test]
    fn encodes_query_component_values() {
        let url = build_team_management_url(
            "https://example.com/auth?access={access-token}&refresh={refresh-token}",
            "a+b c",
            "r/s?x=1",
        );

        assert_eq!(
            "https://example.com/auth?access=a%2Bb%20c&refresh=r%2Fs%3Fx%3D1",
            url
        );
    }

    #[test]
    fn prefixes_relative_url_with_public_base_url() {
        let url = resolve_team_management_url(
            "/zh-CN/auth/desktop?access_token=a",
            Some("https://example.com/"),
        );

        assert_eq!("https://example.com/zh-CN/auth/desktop?access_token=a", url);
    }

    #[test]
    fn leaves_absolute_url_unchanged() {
        let url = resolve_team_management_url(
            "https://example.com/zh-CN/auth/desktop?access_token=a",
            Some("https://other.example.com"),
        );

        assert_eq!("https://example.com/zh-CN/auth/desktop?access_token=a", url);
    }
}
