use semver::Version;

pub(crate) fn parse_version(value: &str) -> Option<Version> {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix('v').unwrap_or(trimmed);
    Version::parse(trimmed).ok()
}

pub(crate) fn format_bytes(value: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let value = value as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.2} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{} B", value as u64)
    }
}

pub(crate) enum UpdateInstallAction {
    Quit,
    Noop,
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{format_bytes, parse_version};

    #[test]
    fn test_parse_version() {
        let parsed = parse_version("v1.2.3").expect("应解析版本号");
        assert_eq!(parsed, Version::new(1, 2, 3));
    }

    #[test]
    fn test_version_compare() {
        let newer = parse_version("1.2.0").expect("新版本解析失败");
        let older = parse_version("1.1.9").expect("旧版本解析失败");

        assert!(newer > older);
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(is_newer("1.1.1", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
    }

    fn is_newer(latest: &str, current: &str) -> bool {
        let latest = parse_version(latest).expect("最新版本解析失败");
        let current = parse_version(current).expect("当前版本解析失败");
        latest > current
    }
}
