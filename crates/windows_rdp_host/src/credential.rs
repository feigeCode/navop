use std::fmt;

use zeroize::Zeroizing;

use crate::error::WindowsRdpHostError;
use crate::ffi::{NavopRdpBorrowedSecret, NavopRdpBorrowedUtf16, NavopRdpCredentialBundle};

/// One-shot identity and secrets for the native RDP host.
///
/// The server and Gateway passwords intentionally remain independent: an RDP
/// server password must not be silently reused as a Gateway password. This
/// owner is not `Clone`, `Serialize`, or `Deserialize`; callers should move or
/// borrow it explicitly rather than creating accidental credential copies.
#[derive(Default)]
pub struct WindowsRdpCredentialBundle {
    username: Option<Vec<u16>>,
    domain: Option<Vec<u16>>,
    server_password: Option<Zeroizing<Vec<u16>>>,
    gateway_password: Option<Zeroizing<Vec<u16>>>,
}

impl WindowsRdpCredentialBundle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username.encode_utf16().collect());
        self
    }

    pub fn with_domain(mut self, domain: String) -> Self {
        self.domain = Some(domain.encode_utf16().collect());
        self
    }

    /// Replaces the server password, taking ownership of the input `String`.
    ///
    /// The input string is wrapped before UTF-16 encoding so its original
    /// allocation is zeroized as soon as this method finishes.
    pub fn with_server_password(mut self, password: String) -> Self {
        self.server_password = Some(encode_owned_secret(password));
        self
    }

    /// Replaces the Gateway password, taking ownership of the input `String`.
    ///
    /// The input string is wrapped before UTF-16 encoding so its original
    /// allocation is zeroized as soon as this method finishes.
    pub fn with_gateway_password(mut self, password: String) -> Self {
        self.gateway_password = Some(encode_owned_secret(password));
        self
    }

    pub fn set_server_password(&mut self, password: String) {
        self.server_password = Some(encode_owned_secret(password));
    }

    pub fn set_username(&mut self, username: String) {
        self.username = Some(username.encode_utf16().collect());
    }

    pub fn set_domain(&mut self, domain: String) {
        self.domain = Some(domain.encode_utf16().collect());
    }

    pub fn set_gateway_password(&mut self, password: String) {
        self.gateway_password = Some(encode_owned_secret(password));
    }

    pub fn clear_server_password(&mut self) {
        self.server_password = None;
    }

    pub fn clear_username(&mut self) {
        self.username = None;
    }

    pub fn clear_domain(&mut self) {
        self.domain = None;
    }

    pub fn clear_gateway_password(&mut self) {
        self.gateway_password = None;
    }

    pub(crate) fn as_native(&self) -> Result<NavopRdpCredentialBundle, WindowsRdpHostError> {
        Ok(NavopRdpCredentialBundle {
            struct_size: std::mem::size_of::<NavopRdpCredentialBundle>() as u32,
            abi_version: crate::ffi::ABI_VERSION,
            server_password: borrowed_secret(
                self.server_password
                    .as_ref()
                    .map(|password| password.as_slice()),
            )?,
            gateway_password: borrowed_secret(
                self.gateway_password
                    .as_ref()
                    .map(|password| password.as_slice()),
            )?,
            flags: 0,
            username: borrowed_utf16(self.username.as_deref())?,
            domain: borrowed_utf16(self.domain.as_deref())?,
        })
    }
}

impl fmt::Debug for WindowsRdpCredentialBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRdpCredentialBundle")
            .field("username", &redacted_text(&self.username))
            .field("domain", &redacted_text(&self.domain))
            .field("server_password", &redacted_secret(&self.server_password))
            .field("gateway_password", &redacted_secret(&self.gateway_password))
            .finish()
    }
}

fn encode_owned_secret(password: String) -> Zeroizing<Vec<u16>> {
    let password = Zeroizing::new(password);
    Zeroizing::new(password.encode_utf16().collect())
}

fn redacted_secret(secret: &Option<Zeroizing<Vec<u16>>>) -> String {
    match secret {
        Some(secret) => format!("<redacted, {} UTF-16 code units>", secret.len()),
        None => "<absent>".to_owned(),
    }
}

fn redacted_text(text: &Option<Vec<u16>>) -> String {
    match text {
        Some(text) => format!("<present, {} UTF-16 code units>", text.len()),
        None => "<absent>".to_owned(),
    }
}

fn borrowed_utf16(text: Option<&[u16]>) -> Result<NavopRdpBorrowedUtf16, WindowsRdpHostError> {
    let Some(text) = text else {
        return Ok(NavopRdpBorrowedUtf16 {
            data: std::ptr::null(),
            len: 0,
        });
    };

    let len = u32::try_from(text.len()).map_err(|_| WindowsRdpHostError::InvalidArgument)?;
    Ok(NavopRdpBorrowedUtf16 {
        data: if text.is_empty() {
            std::ptr::null()
        } else {
            text.as_ptr()
        },
        len,
    })
}

fn borrowed_secret(secret: Option<&[u16]>) -> Result<NavopRdpBorrowedSecret, WindowsRdpHostError> {
    let Some(secret) = secret else {
        return Ok(NavopRdpBorrowedSecret {
            data: std::ptr::null(),
            len: 0,
        });
    };

    let len = u32::try_from(secret.len()).map_err(|_| WindowsRdpHostError::InvalidArgument)?;

    Ok(NavopRdpBorrowedSecret {
        data: if secret.is_empty() {
            std::ptr::null()
        } else {
            secret.as_ptr()
        },
        len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_secrets_are_independent_and_utf16_encoded() {
        let credentials = WindowsRdpCredentialBundle::new()
            .with_username("用户".to_owned())
            .with_domain("EXAMPLE".to_owned())
            .with_server_password("server-secret".to_owned())
            .with_gateway_password("gateway-secret".to_owned());
        let native = credentials.as_native().expect("credentials should fit ABI");

        let username = unsafe {
            std::slice::from_raw_parts(native.username.data, native.username.len as usize)
        };
        let domain =
            unsafe { std::slice::from_raw_parts(native.domain.data, native.domain.len as usize) };
        // SAFETY: both slices point into the credentials-owned, non-empty
        // UTF-16 vectors, which remain alive for this test.
        let server = unsafe {
            std::slice::from_raw_parts(
                native.server_password.data,
                native.server_password.len as usize,
            )
        };
        // SAFETY: both slices point into the credentials-owned, non-empty
        // UTF-16 vectors, which remain alive for this test.
        let gateway = unsafe {
            std::slice::from_raw_parts(
                native.gateway_password.data,
                native.gateway_password.len as usize,
            )
        };

        assert_eq!(String::from_utf16(username).unwrap(), "用户");
        assert_eq!(String::from_utf16(domain).unwrap(), "EXAMPLE");
        assert_eq!(
            String::from_utf16(server).expect("server secret should be valid UTF-16"),
            "server-secret"
        );
        assert_eq!(
            String::from_utf16(gateway).expect("Gateway secret should be valid UTF-16"),
            "gateway-secret"
        );
        assert_ne!(native.server_password.data, native.gateway_password.data);
    }

    #[test]
    fn empty_and_absent_values_use_the_null_zero_length_borrowed_form() {
        let credentials = WindowsRdpCredentialBundle::new()
            .with_username(String::new())
            .with_server_password(String::new());
        let native = credentials
            .as_native()
            .expect("empty credentials should be valid");

        assert!(native.username.data.is_null());
        assert_eq!(native.username.len, 0);
        assert!(native.domain.data.is_null());
        assert_eq!(native.domain.len, 0);
        assert!(native.server_password.data.is_null());
        assert_eq!(native.server_password.len, 0);
        assert!(native.gateway_password.data.is_null());
        assert_eq!(native.gateway_password.len, 0);
    }

    #[test]
    fn debug_redacts_identity_and_both_secrets_without_clone_or_serialization_surface() {
        let credentials = WindowsRdpCredentialBundle::new()
            .with_username("username-debug-sentinel".to_owned())
            .with_domain("domain-debug-sentinel".to_owned())
            .with_server_password("server-debug-sentinel".to_owned())
            .with_gateway_password("gateway-debug-sentinel".to_owned());
        let debug = format!("{credentials:?}");

        assert!(debug.contains("<redacted"));
        assert!(!debug.contains("username-debug-sentinel"));
        assert!(!debug.contains("domain-debug-sentinel"));
        assert!(!debug.contains("server-debug-sentinel"));
        assert!(!debug.contains("gateway-debug-sentinel"));
    }

    #[test]
    fn clearing_values_does_not_change_the_others() {
        let mut credentials = WindowsRdpCredentialBundle::new()
            .with_username("operator".to_owned())
            .with_domain("EXAMPLE".to_owned())
            .with_server_password("server-secret".to_owned())
            .with_gateway_password("gateway-secret".to_owned());

        credentials.clear_server_password();
        credentials.clear_domain();
        let native = credentials
            .as_native()
            .expect("credentials should remain valid");

        assert_eq!(
            native.username.len,
            "operator".encode_utf16().count() as u32
        );
        assert_eq!(native.domain.len, 0);
        assert_eq!(native.server_password.len, 0);
        assert!(!native.gateway_password.data.is_null());
        assert_eq!(
            native.gateway_password.len,
            "gateway-secret".encode_utf16().count() as u32
        );
    }
}
