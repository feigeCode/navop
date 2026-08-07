use std::fmt;

use zeroize::Zeroizing;

use crate::error::WindowsRdpHostError;
use crate::ffi::{NavopRdpBorrowedSecret, NavopRdpCredentialBundle};

/// One-shot server and Gateway secrets for the native RDP host.
///
/// The two fields intentionally remain independent: an RDP server password
/// must not be silently reused as a Gateway password. This owner is not
/// `Clone`, `Serialize`, or `Deserialize`; callers should move or borrow it
/// explicitly rather than creating accidental credential copies.
#[derive(Default)]
pub struct WindowsRdpCredentialBundle {
    server_password: Option<Zeroizing<Vec<u16>>>,
    gateway_password: Option<Zeroizing<Vec<u16>>>,
}

impl WindowsRdpCredentialBundle {
    pub fn new() -> Self {
        Self::default()
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

    pub fn set_gateway_password(&mut self, password: String) {
        self.gateway_password = Some(encode_owned_secret(password));
    }

    pub fn clear_server_password(&mut self) {
        self.server_password = None;
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
        })
    }
}

impl fmt::Debug for WindowsRdpCredentialBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRdpCredentialBundle")
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
    fn server_and_gateway_secrets_are_independent_and_utf16_encoded() {
        let credentials = WindowsRdpCredentialBundle::new()
            .with_server_password("server-secret".to_owned())
            .with_gateway_password("gateway-secret".to_owned());
        let native = credentials.as_native().expect("credentials should fit ABI");

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
    fn empty_and_absent_secrets_use_the_null_zero_length_borrowed_form() {
        let credentials = WindowsRdpCredentialBundle::new().with_server_password(String::new());
        let native = credentials
            .as_native()
            .expect("empty credentials should be valid");

        assert!(native.server_password.data.is_null());
        assert_eq!(native.server_password.len, 0);
        assert!(native.gateway_password.data.is_null());
        assert_eq!(native.gateway_password.len, 0);
    }

    #[test]
    fn debug_redacts_both_secrets_without_clone_or_serialization_surface() {
        let credentials = WindowsRdpCredentialBundle::new()
            .with_server_password("server-debug-sentinel".to_owned())
            .with_gateway_password("gateway-debug-sentinel".to_owned());
        let debug = format!("{credentials:?}");

        assert!(debug.contains("<redacted"));
        assert!(!debug.contains("server-debug-sentinel"));
        assert!(!debug.contains("gateway-debug-sentinel"));
    }

    #[test]
    fn clearing_one_secret_does_not_change_the_other() {
        let mut credentials = WindowsRdpCredentialBundle::new()
            .with_server_password("server-secret".to_owned())
            .with_gateway_password("gateway-secret".to_owned());

        credentials.clear_server_password();
        let native = credentials
            .as_native()
            .expect("credentials should remain valid");

        assert_eq!(native.server_password.len, 0);
        assert!(!native.gateway_password.data.is_null());
        assert_eq!(
            native.gateway_password.len,
            "gateway-secret".encode_utf16().count() as u32
        );
    }
}
