#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxVersion {
    pub major: u8,
    pub minor: u8,
    pub suffix: String,
}

impl TmuxVersion {
    pub fn parse(output: &str) -> Option<Self> {
        let value = output
            .trim()
            .strip_prefix("tmux ")?
            .strip_prefix("next-")
            .unwrap_or_else(|| output.trim().strip_prefix("tmux ").unwrap_or(""));
        let value = value.trim();
        let value = value.strip_prefix('v').unwrap_or(value);
        let (numeric, suffix) = value
            .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
            .map(|index| (&value[..index], &value[index..]))
            .unwrap_or((value, ""));
        let mut parts = numeric.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some(Self {
            major,
            minor,
            suffix: suffix.to_string(),
        })
    }

    pub fn supports_format_subscription(&self) -> bool {
        (self.major, self.minor) >= (3, 2)
    }

    pub fn supports_control_mode_integration(&self) -> bool {
        (self.major, self.minor) >= (3, 2)
    }

    pub fn supports_command_state(&self) -> bool {
        (self.major, self.minor) >= (3, 8)
    }

    pub fn supports_split_with_cwd(&self) -> bool {
        self.major >= 3
    }
}

#[cfg(test)]
mod tests {
    use super::TmuxVersion;

    #[test]
    fn parses_regular_and_suffix_versions() {
        assert_eq!(
            TmuxVersion::parse("tmux 3.8").expect("3.8"),
            TmuxVersion {
                major: 3,
                minor: 8,
                suffix: String::new()
            }
        );
        assert_eq!(TmuxVersion::parse("tmux 3.3a").expect("3.3a").suffix, "a");
        assert_eq!(
            TmuxVersion::parse("tmux next-3.4").expect("next-3.4").minor,
            4
        );
    }

    #[test]
    fn version_gates_do_not_reject_future_major_versions() {
        let version = TmuxVersion::parse("tmux 4.0").expect("4.0");
        assert!(version.supports_format_subscription());
        assert!(version.supports_command_state());
        assert!(
            !TmuxVersion::parse("tmux 3.1")
                .unwrap()
                .supports_format_subscription()
        );
        assert!(
            !TmuxVersion::parse("tmux 1.8")
                .unwrap()
                .supports_control_mode_integration()
        );
    }

    #[test]
    fn rejects_unknown_output() {
        assert!(TmuxVersion::parse("tmux next").is_none());
        assert!(TmuxVersion::parse("").is_none());
        assert!(TmuxVersion::parse("not tmux").is_none());
    }
}
