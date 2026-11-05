//! Bearer validation only. No Debug implementation: tokens must not enter logs.
use axum::http::{HeaderMap, header::AUTHORIZATION};
use subtle::ConstantTimeEq;

pub struct BearerToken([u8; 64]);

impl BearerToken {
    pub fn from_hex(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != 64
            || !bytes
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(
                "Invalid credential format; remove the WiniOS credential and restart to regenerate it.",
            );
        }
        let mut token = [0; 64];
        token.copy_from_slice(bytes);
        Ok(Self(token))
    }

    pub fn authorizes(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some((scheme, supplied)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("Bearer") || supplied.len() != 64 {
            return false;
        }
        bool::from(self.0.as_slice().ct_eq(supplied.as_bytes()))
    }

    /// Only for the explicitly requested --show-token command and native storage.
    #[cfg(windows)]
    pub fn secret(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated ASCII token")
    }
}
