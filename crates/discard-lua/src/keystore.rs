//! Where the approval-signing key lives. The key signs approval records so edits made outside Discard are detected.
//! The real stores are OS secret stores (macOS Keychain and others). They live in the app crate.

use sha2::Sha256;

pub type Key = [u8; 32];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("no secure key storage is available: {0}")]
    Unavailable(String),
}

pub trait KeyStore: Send + Sync {
    /// The signing key. Creates and saves one on first use.
    fn key(&self) -> Result<Key, KeyError>;
}

/// A fixed key. For tests.
pub struct StaticKeyStore(pub Key);

impl KeyStore for StaticKeyStore {
    fn key(&self) -> Result<Key, KeyError> {
        Ok(self.0)
    }
}

/// Always fails. Used to test the fail-closed path.
pub struct UnavailableKeyStore(pub String);

impl KeyStore for UnavailableKeyStore {
    fn key(&self) -> Result<Key, KeyError> {
        Err(KeyError::Unavailable(self.0.clone()))
    }
}

/// A key derived from a passphrase with PBKDF2-HMAC-SHA256. The fallback when no secret store exists.
pub struct PassphraseKeyStore {
    passphrase: String,
    salt: Vec<u8>,
}

pub const PBKDF2_ROUNDS: u32 = 200_000;

impl PassphraseKeyStore {
    pub fn new(passphrase: &str, salt: &[u8]) -> Self {
        Self { passphrase: passphrase.to_string(), salt: salt.to_vec() }
    }
}

impl KeyStore for PassphraseKeyStore {
    fn key(&self) -> Result<Key, KeyError> {
        if self.passphrase.is_empty() {
            return Err(KeyError::Unavailable("the passphrase is empty".into()));
        }
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(self.passphrase.as_bytes(), &self.salt, PBKDF2_ROUNDS, &mut key);
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_static_store_returns_its_key() {
        assert_eq!(StaticKeyStore([7; 32]).key(), Ok([7; 32]));
    }

    #[test]
    fn an_unavailable_store_fails_with_its_reason() {
        assert_eq!(UnavailableKeyStore("no keychain".into()).key(), Err(KeyError::Unavailable("no keychain".into())));
    }

    #[test]
    fn the_same_passphrase_and_salt_give_the_same_key() {
        let a = PassphraseKeyStore::new("correct horse", b"salt-1").key().unwrap();
        let b = PassphraseKeyStore::new("correct horse", b"salt-1").key().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_passphrase_or_salt_gives_a_different_key() {
        let base = PassphraseKeyStore::new("correct horse", b"salt-1").key().unwrap();
        assert_ne!(base, PassphraseKeyStore::new("correct horsf", b"salt-1").key().unwrap());
        assert_ne!(base, PassphraseKeyStore::new("correct horse", b"salt-2").key().unwrap());
    }

    #[test]
    fn the_derived_key_matches_the_pbkdf2_reference_construction() {
        // Guards against changing the algorithm or round count by accident: existing approvals would stop verifying.
        let mut expected = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(b"pw", b"salt", PBKDF2_ROUNDS, &mut expected);
        assert_eq!(PassphraseKeyStore::new("pw", b"salt").key().unwrap(), expected);
    }

    #[test]
    fn an_empty_passphrase_is_refused() {
        assert!(PassphraseKeyStore::new("", b"salt").key().is_err());
    }
}
