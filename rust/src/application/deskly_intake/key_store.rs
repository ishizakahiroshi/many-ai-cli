//! The Deskly orchestrator key lives in Windows Credential Manager as a
//! generic credential (decided 2026-10-30): no plaintext file, not shared with
//! other secrets. Register it once with
//! `cmdkey /generic:many-ai-cli/deskly-orchestrator-key /user:deskly /pass`
//! (cmdkey prompts for the value, so it never appears in shell history).
//! The value is only ever turned into a `MachineKey`, whose Debug is redacted.
use super::mcp::MachineKey;

pub const CREDENTIAL_TARGET: &str = "many-ai-cli/deskly-orchestrator-key";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialError {
    NotFound,
    Unreadable,
    /// Stored, but not a valid key (wrong encoding, length or characters).
    Invalid,
    Unsupported,
}

/// cmdkey and the Credential Manager UI store the password as UTF-16LE.
pub(super) fn decode_blob(blob: &[u8]) -> Result<MachineKey, CredentialError> {
    if blob.is_empty() || blob.len() % 2 != 0 {
        return Err(CredentialError::Invalid);
    }
    let units: Vec<u16> = blob
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).map_err(|_| CredentialError::Invalid)?;
    MachineKey::new(text).map_err(|_| CredentialError::Invalid)
}

#[cfg(windows)]
pub fn load_machine_key() -> Result<MachineKey, CredentialError> {
    use windows_sys::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW,
    };
    let target: Vec<u16> = CREDENTIAL_TARGET
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call; on success
    // `credential` points to a buffer owned by the OS until CredFree.
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 || credential.is_null() {
        let code = std::io::Error::last_os_error().raw_os_error();
        // ERROR_NOT_FOUND
        return Err(if code == Some(1168) {
            CredentialError::NotFound
        } else {
            CredentialError::Unreadable
        });
    }
    // SAFETY: CredReadW succeeded, so the struct and its blob are valid for
    // CredentialBlobSize bytes until CredFree below.
    let result = unsafe {
        let item = &*credential;
        if item.CredentialBlob.is_null() {
            Err(CredentialError::Invalid)
        } else {
            decode_blob(std::slice::from_raw_parts(
                item.CredentialBlob,
                item.CredentialBlobSize as usize,
            ))
        }
    };
    // SAFETY: frees the buffer CredReadW allocated, exactly once.
    unsafe { CredFree(credential.cast()) };
    result
}

#[cfg(not(windows))]
pub fn load_machine_key() -> Result<MachineKey, CredentialError> {
    Err(CredentialError::Unsupported)
}
