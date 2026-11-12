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
        Self::acquire_named(w!("Local\\WiniOS.Agent"))
    }
    fn acquire_named(name: windows::core::PCWSTR) -> Result<Self, AppError> {
        let handle = unsafe { CreateMutexW(None, false, name)? };
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
    read_token_at(TARGET)
}
fn read_token_at(target: windows::core::PCWSTR) -> Result<Option<BearerToken>, AppError> {
    let mut pointer = std::ptr::null_mut();
    match unsafe { CredReadW(target, CRED_TYPE_GENERIC, None, &mut pointer) } {
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

/// Public, ephemeral HTTP snapshot namespace, unrelated to the bearer credential.
pub(crate) fn snapshot_namespace() -> windows::core::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    unsafe {
        BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG).ok()?;
    }
    Ok(bytes)
}

/// Called only after the service's single-instance guard is acquired.
pub fn load_or_create_token() -> Result<BearerToken, AppError> {
    load_or_create_token_at(TARGET)
}
fn load_or_create_token_at(target: windows::core::PCWSTR) -> Result<BearerToken, AppError> {
    if let Some(token) = read_token_at(target)? {
        return Ok(token);
    }
    let token = generate_token()?;
    let mut bytes = token.secret().as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.0.cast_mut()),
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
    fn singleton_blocks_duplicate_and_releases_after_shutdown() {
        let name: Vec<u16> = format!("Local\\WiniOS.SingletonTest.{}", std::process::id())
            .encode_utf16()
            .chain([0])
            .collect();
        let name = windows::core::PCWSTR(name.as_ptr());
        let first = SingleInstance::acquire_named(name).unwrap();
        assert!(SingleInstance::acquire_named(name).is_err());
        drop(first);
        assert!(SingleInstance::acquire_named(name).is_ok());
    }
    #[test]
    fn native_rng_generates_valid_distinct_tokens_without_persisting() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_eq!(first.secret().len(), 64);
        // Do not include either token in assertion failure output.
        assert!(first.secret() != second.secret());
    }
    #[test]
    fn disposable_first_run_credentials_are_unique_reused_and_invalid_values_protected() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let names = [
            format!("WiniOS/Tests/Onboarding/{suffix}/a"),
            format!("WiniOS/Tests/Onboarding/{suffix}/b"),
        ];
        let wide = names.map(|s| s.encode_utf16().chain([0]).collect::<Vec<_>>());
        struct Cleanup([Vec<u16>; 2]);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for name in &self.0 {
                    unsafe {
                        let _ = CredDeleteW(
                            windows::core::PCWSTR(name.as_ptr()),
                            CRED_TYPE_GENERIC,
                            None,
                        );
                    }
                }
            }
        }
        let cleanup = Cleanup(wide);
        let a = windows::core::PCWSTR(cleanup.0[0].as_ptr());
        let b = windows::core::PCWSTR(cleanup.0[1].as_ptr());
        assert!(read_token_at(a).unwrap().is_none() && read_token_at(b).unwrap().is_none());
        let first = load_or_create_token_at(a).unwrap();
        let second = load_or_create_token_at(b).unwrap();
        assert!(first.secret() != second.secret());
        assert!(load_or_create_token_at(a).unwrap().secret() == first.secret());
        // A malformed existing credential must not be silently rotated.
        let mut invalid = [b'x'];
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(b.0.cast_mut()),
            CredentialBlobSize: 1,
            CredentialBlob: invalid.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe {
            CredWriteW(&credential, 0).unwrap();
        }
        assert!(load_or_create_token_at(b).is_err());
        assert!(read_token_at(b).is_err());
        // Neither helper above ever receives the production TARGET.
    }
}
