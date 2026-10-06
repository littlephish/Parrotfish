pub mod eax;
pub mod handshake;
pub mod keys;
pub mod license;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use sha1::Digest as _;
use sha1::Sha1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    Base64(&'static str),
    Puzzle(String),
    License(String),
    Handshake(String),
    Identity(String),
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::Base64(what) => write!(f, "invalid base64 in {what}"),
            CryptoError::Puzzle(s) => write!(f, "puzzle: {s}"),
            CryptoError::License(s) => write!(f, "license: {s}"),
            CryptoError::Handshake(s) => write!(f, "handshake: {s}"),
            CryptoError::Identity(s) => write!(f, "identity: {s}"),
        }
    }
}

impl std::error::Error for CryptoError {}

pub fn hash_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }
    B64.encode(Sha1::digest(password.as_bytes()))
}

pub(crate) fn decode_b64(what: &'static str, value: &str) -> Result<Vec<u8>, CryptoError> {
    B64.decode(value.trim().as_bytes()).map_err(|_| CryptoError::Base64(what))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_matches_teamspeak() {
        assert_eq!(hash_password(""), "");
        assert_eq!(hash_password("secret"), "5en6G6MezRroT3XKqkdPOmY/BfQ=");
    }
}
