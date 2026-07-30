//! Network helpers shared by the embedded editor.
//!
//! The host application owns GPUI's global HTTP client so proxy, authentication,
//! and transport policy stay consistent across the whole application.

use std::str::FromStr;

use gpui::http_client;

pub(crate) fn is_remote_image_source(source: &str) -> bool {
    http_client::Uri::from_str(source)
        .ok()
        .and_then(|uri| uri.scheme_str().map(str::to_owned))
        .is_some_and(|scheme| scheme == "http" || scheme == "https")
}

#[cfg(test)]
mod tests {
    use super::is_remote_image_source;

    #[test]
    fn detects_remote_http_sources() {
        assert!(is_remote_image_source("https://example.com/image.png"));
        assert!(is_remote_image_source("http://example.com/image.gif"));
        assert!(!is_remote_image_source("./image.png"));
        assert!(!is_remote_image_source("images/photo.jpg"));
    }
}
