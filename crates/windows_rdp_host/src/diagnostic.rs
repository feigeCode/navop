/// Controls whether a username may appear in an explicitly requested
/// diagnostic snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpUsernameRedaction {
    #[default]
    Redacted,
    Visible,
}

/// Borrowed connection metadata used to create a logging-safe snapshot.
///
/// Endpoints are always reduced to lengths. The username defaults to the same
/// representation and is copied only when `Visible` is explicitly requested.
pub struct WindowsRdpDiagnosticContext<'a> {
    endpoint: &'a str,
    gateway_endpoint: Option<&'a str>,
    username: Option<&'a str>,
}

impl<'a> WindowsRdpDiagnosticContext<'a> {
    pub const fn new(endpoint: &'a str) -> Self {
        Self {
            endpoint,
            gateway_endpoint: None,
            username: None,
        }
    }

    pub const fn with_gateway_endpoint(mut self, gateway_endpoint: &'a str) -> Self {
        self.gateway_endpoint = Some(gateway_endpoint);
        self
    }

    pub const fn with_username(mut self, username: &'a str) -> Self {
        self.username = Some(username);
        self
    }

    pub fn snapshot(
        self,
        username_redaction: WindowsRdpUsernameRedaction,
    ) -> WindowsRdpDiagnosticSnapshot {
        WindowsRdpDiagnosticSnapshot {
            endpoint: WindowsRdpRedactedValue::from_text(self.endpoint),
            gateway_endpoint: self
                .gateway_endpoint
                .map(WindowsRdpRedactedValue::from_text),
            username: self.username.map(|username| match username_redaction {
                WindowsRdpUsernameRedaction::Redacted => {
                    WindowsRdpRedactedValue::from_text(username)
                }
                WindowsRdpUsernameRedaction::Visible => {
                    WindowsRdpRedactedValue::Visible(username.to_owned())
                }
            }),
        }
    }
}

/// An owned diagnostic context that never retains complete endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpDiagnosticSnapshot {
    endpoint: WindowsRdpRedactedValue,
    gateway_endpoint: Option<WindowsRdpRedactedValue>,
    username: Option<WindowsRdpRedactedValue>,
}

impl WindowsRdpDiagnosticSnapshot {
    pub const fn endpoint(&self) -> &WindowsRdpRedactedValue {
        &self.endpoint
    }

    pub const fn gateway_endpoint(&self) -> Option<&WindowsRdpRedactedValue> {
        self.gateway_endpoint.as_ref()
    }

    pub const fn username(&self) -> Option<&WindowsRdpRedactedValue> {
        self.username.as_ref()
    }
}

/// A field value safe to include in structured diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsRdpRedactedValue {
    Redacted { utf16_code_units: usize },
    Visible(String),
}

impl WindowsRdpRedactedValue {
    fn from_text(value: &str) -> Self {
        Self::Redacted {
            utf16_code_units: value.encode_utf16().count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENDPOINT: &str = "alice@example.com:server-secret@[2001:db8::1]:3390";
    const GATEWAY: &str = "https://gw.example.test:443/path";
    const USERNAME: &str = "域/alice@example.com:研发";

    #[test]
    fn default_snapshot_redacts_username_and_complete_endpoints() {
        let snapshot = WindowsRdpDiagnosticContext::new(ENDPOINT)
            .with_gateway_endpoint(GATEWAY)
            .with_username(USERNAME)
            .snapshot(WindowsRdpUsernameRedaction::default());
        let debug = format!("{snapshot:?}");

        assert!(matches!(
            snapshot.endpoint(),
            WindowsRdpRedactedValue::Redacted { .. }
        ));
        assert!(matches!(
            snapshot.gateway_endpoint(),
            Some(WindowsRdpRedactedValue::Redacted { .. })
        ));
        assert!(matches!(
            snapshot.username(),
            Some(WindowsRdpRedactedValue::Redacted { .. })
        ));
        assert!(!debug.contains(ENDPOINT));
        assert!(!debug.contains(GATEWAY));
        assert!(!debug.contains(USERNAME));
        assert!(!debug.contains("server-secret"));
    }

    #[test]
    fn explicit_username_visibility_never_reveals_endpoints() {
        let snapshot = WindowsRdpDiagnosticContext::new(ENDPOINT)
            .with_gateway_endpoint(GATEWAY)
            .with_username(USERNAME)
            .snapshot(WindowsRdpUsernameRedaction::Visible);
        let debug = format!("{snapshot:?}");

        assert!(matches!(
            snapshot.username(),
            Some(WindowsRdpRedactedValue::Visible(username)) if username == USERNAME
        ));
        assert!(debug.contains(USERNAME));
        assert!(!debug.contains(ENDPOINT));
        assert!(!debug.contains(GATEWAY));
        assert!(!debug.contains("server-secret"));
    }
}
