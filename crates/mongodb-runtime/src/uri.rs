pub(crate) fn replace_mongodb_uri_authority(
    connection_string: &str,
    host: &str,
    port: u16,
) -> String {
    let Some((scheme, rest)) = connection_string.split_once("://") else {
        return connection_string.to_string();
    };
    let scheme = if scheme.eq_ignore_ascii_case("mongodb+srv") {
        "mongodb"
    } else {
        scheme
    };
    let split_at = rest
        .char_indices()
        .find(|(_, character)| matches!(character, '/' | '?'))
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let (authority, suffix) = rest.split_at(split_at);
    let userinfo = authority
        .rfind('@')
        .map(|index| &authority[..=index])
        .unwrap_or("");
    let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    };

    format!("{scheme}://{userinfo}{host}:{port}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::replace_mongodb_uri_authority;

    #[test]
    fn tunnel_uri_preserves_userinfo_path_and_query() {
        let uri = replace_mongodb_uri_authority(
            "mongodb://user:p%40ss@mongo.internal:27017/app?authSource=admin",
            "127.0.0.1",
            49152,
        );

        assert_eq!(
            "mongodb://user:p%40ss@127.0.0.1:49152/app?authSource=admin",
            uri
        );
    }

    #[test]
    fn tunnel_uri_converts_srv_and_brackets_ipv6() {
        let uri = replace_mongodb_uri_authority(
            "mongodb+srv://mongo.example.com/app?retryWrites=true",
            "::1",
            49152,
        );

        assert_eq!("mongodb://[::1]:49152/app?retryWrites=true", uri);
    }
}
