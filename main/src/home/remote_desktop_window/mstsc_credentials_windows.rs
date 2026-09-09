use std::{
    collections::HashMap,
    ptr, slice,
    sync::{Mutex, OnceLock},
};

use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CRED_PERSIST, CRED_PERSIST_SESSION, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW,
            CredFree, CredReadW, CredWriteW,
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};
use zeroize::Zeroize;

use super::MstscCredentials;

struct CredentialSnapshot {
    username: String,
    blob: Vec<u8>,
    persist: CRED_PERSIST,
    comment: String,
}

impl Drop for CredentialSnapshot {
    fn drop(&mut self) {
        self.blob.zeroize();
    }
}

struct ActiveCredential {
    count: usize,
    username: String,
    blob: Vec<u8>,
}

impl Drop for ActiveCredential {
    fn drop(&mut self) {
        self.blob.zeroize();
    }
}

static ACTIVE_CREDENTIALS: OnceLock<Mutex<HashMap<String, ActiveCredential>>> = OnceLock::new();

pub(crate) struct CredentialLease {
    target: String,
    managed: bool,
    restored: bool,
}

impl CredentialLease {
    pub(crate) fn restore_after(mut self, mut child: std::process::Child) {
        let _ = child.wait();
        std::thread::sleep(super::HANDOFF_GRACE_PERIOD);
        if let Err(error) = self.restore() {
            tracing::warn!(?error, target = %self.target, "无法恢复 Windows RDP 凭据");
        }
    }

    fn restore(&mut self) -> windows::core::Result<()> {
        if self.restored {
            return Ok(());
        }
        let result = self
            .managed
            .then(|| release_temporary(&self.target))
            .transpose()
            .map(|_| ());
        if result.is_ok() {
            self.restored = true;
        }
        result
    }
}

impl Drop for CredentialLease {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            tracing::warn!(?error, target = %self.target, "无法清理 Windows RDP 临时凭据");
        }
    }
}

pub(crate) fn store_temporary(
    credentials: &MstscCredentials,
) -> windows::core::Result<CredentialLease> {
    let mut active = active_credentials()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut blob = password_blob(&credentials.password);
    if let Some(current) = active.get_mut(&credentials.target) {
        if !current_credential_matches(&credentials.target, current)? {
            blob.zeroize();
            return Ok(unmanaged_lease(credentials));
        }
        write_or_zeroize(credentials, &mut blob)?;
        current.count += 1;
        current.username.clone_from(&credentials.username);
        current.blob.zeroize();
        current.blob.clone_from(&blob);
        blob.zeroize();
        return Ok(managed_lease(credentials));
    }
    let existing = read_credential(&credentials.target)?;
    if existing.is_some() && !existing.as_ref().is_some_and(is_navop_temporary) {
        blob.zeroize();
        return Ok(unmanaged_lease(credentials));
    }
    write_or_zeroize(credentials, &mut blob)?;
    active.insert(
        credentials.target.clone(),
        ActiveCredential {
            count: 1,
            username: credentials.username.clone(),
            blob: blob.clone(),
        },
    );
    blob.zeroize();
    Ok(managed_lease(credentials))
}

fn managed_lease(credentials: &MstscCredentials) -> CredentialLease {
    CredentialLease {
        target: credentials.target.clone(),
        managed: true,
        restored: false,
    }
}

fn unmanaged_lease(credentials: &MstscCredentials) -> CredentialLease {
    CredentialLease {
        target: credentials.target.clone(),
        managed: false,
        restored: false,
    }
}

fn active_credentials() -> &'static Mutex<HashMap<String, ActiveCredential>> {
    ACTIVE_CREDENTIALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn release_temporary(target: &str) -> windows::core::Result<()> {
    let mut active = active_credentials()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(current) = active.get_mut(target) else {
        return Ok(());
    };
    if current.count > 1 {
        current.count -= 1;
        return Ok(());
    }
    if current_credential_matches(target, current)? {
        delete_credential(target)?;
    }
    active.remove(target);
    Ok(())
}

fn current_credential_matches(
    target: &str,
    expected: &ActiveCredential,
) -> windows::core::Result<bool> {
    Ok(read_credential(target)?.is_some_and(|credential| {
        is_navop_temporary(&credential)
            && credential.username == expected.username
            && credential.blob == expected.blob
    }))
}

fn read_credential(target: &str) -> windows::core::Result<Option<CredentialSnapshot>> {
    let target = wide_null(target);
    let mut raw = ptr::null_mut();
    if let Err(error) =
        unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) }
    {
        return if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let credential = unsafe { &*raw };
    let snapshot = CredentialSnapshot {
        username: unsafe { pwstr_to_string(credential.UserName) },
        blob: unsafe { credential_blob(credential) },
        persist: credential.Persist,
        comment: unsafe { pwstr_to_string(credential.Comment) },
    };
    unsafe { CredFree(raw.cast()) };
    Ok(Some(snapshot))
}

fn is_navop_temporary(credential: &CredentialSnapshot) -> bool {
    credential.persist == CRED_PERSIST_SESSION
        && credential.comment == super::NAVOP_CREDENTIAL_MARKER
}

fn password_blob(password: &str) -> Vec<u8> {
    password.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn write_or_zeroize(
    credentials: &MstscCredentials,
    blob: &mut Vec<u8>,
) -> windows::core::Result<()> {
    if let Err(error) = write_temporary(credentials, blob) {
        blob.zeroize();
        return Err(error);
    }
    Ok(())
}

fn write_temporary(credentials: &MstscCredentials, blob: &[u8]) -> windows::core::Result<()> {
    let mut target = wide_null(&credentials.target);
    let mut username = wide_null(&credentials.username);
    let mut comment = wide_null(super::NAVOP_CREDENTIAL_MARKER);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        Comment: PWSTR(comment.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_SESSION,
        UserName: PWSTR(username.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }
}

fn delete_credential(target: &str) -> windows::core::Result<()> {
    let target = wide_null(target);
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) => Ok(()),
        result => result,
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

unsafe fn pwstr_to_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut len = 0;
    while unsafe { *value.0.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { slice::from_raw_parts(value.0, len) })
}

unsafe fn credential_blob(credential: &CREDENTIALW) -> Vec<u8> {
    if credential.CredentialBlobSize == 0 {
        return Vec::new();
    }
    unsafe {
        slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
        .to_vec()
    }
}
