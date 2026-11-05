//! Windows secure RNG, per-user credential storage, and single service instance.
use crate::auth::BearerToken;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, ERROR_NOT_FOUND, GetLastError, HANDLE},
        Security::{
            Credentials::*,
            Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
        },
        System::Threading::CreateMutexW,
    },
    core::{PWSTR, w},
};

pub type AppError = Box<dyn std::error::Error + Send + Sync>;
const TARGET: windows::core::PCWSTR = w!("WiniOS/Agent/Bearer/v1");

pub struct SingleInstance(HANDLE);
impl SingleInstance {
    pub fn acquire() -> Result<Self, AppError> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\WiniOS.Agent"))? };
        let instance = Self(handle);
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(
                "A WiniOS agent is already running in this Windows session. Stop it first.".into(),
            );
        }
        Ok(instance)
    }
}
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Credential(*mut CREDENTIALW);
impl Drop for Credential {
    fn drop(&mut self) {
        unsafe {
            CredFree(self.0.cast());
        }
    }
}

pub fn read_token() -> Result<Option<BearerToken>, AppError> {
    let mut pointer = std::ptr::null_mut();
    match unsafe { CredReadW(TARGET, CRED_TYPE_GENERIC, None, &mut pointer) } {
        Ok(()) => {
            let credential = Credential(pointer);
            // The allocation and blob are owned by CredRead until CredFree.
            let native = unsafe { &*credential.0 };
            if native.CredentialBlobSize != 64 || native.CredentialBlob.is_null() {
                return Err(
                    "Invalid WiniOS credential; remove it in Credential Manager and restart."
                        .into(),
                );
            }
            let bytes = unsafe { std::slice::from_raw_parts(native.CredentialBlob, 64) };
            Ok(Some(BearerToken::from_hex(bytes)?))
        }
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn generate_token() -> Result<BearerToken, AppError> {
    let mut random = [0u8; 32];
    unsafe {
        BCryptGenRandom(None, &mut random, BCRYPT_USE_SYSTEM_PREFERRED_RNG).ok()?;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = [0; 64];
    for (index, byte) in random.iter().enumerate() {
        encoded[index * 2] = HEX[(byte >> 4) as usize];
        encoded[index * 2 + 1] = HEX[(byte & 15) as usize];
    }
    Ok(BearerToken::from_hex(&encoded)?)
}

/// Called only after the service's single-instance guard is acquired.
pub fn load_or_create_token() -> Result<BearerToken, AppError> {
    if let Some(token) = read_token()? {
        return Ok(token);
    }
    let token = generate_token()?;
    let mut bytes = token.secret().as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(TARGET.0.cast_mut()),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(w!("WiniOS").0.cast_mut()),
        ..Default::default()
    };
    // CredWrite copies the blob; it does not retain these Rust pointers.
    unsafe {
        CredWriteW(&credential, 0)?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_rng_generates_valid_distinct_tokens_without_persisting() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_eq!(first.secret().len(), 64);
        // Do not include either token in assertion failure output.
        assert!(first.secret() != second.secret());
    }
}
