//! Where the plugin-approval signing key comes from.
//!
//! Order: an explicit passphrase from `DISCARD_PLUGIN_PASSPHRASE`, then the operating system's secret store.
//! If neither works, there is no key, and Discard loads no user plugins.

use discard_lua::keystore::{Key, KeyError, KeyStore, PassphraseKeyStore, UnavailableKeyStore};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A place that can keep one secret string. The OS keychain implements this.
pub trait SecretBackend: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, value: &str) -> Result<(), String>;
}

/// A key kept in a secret backend. Created on first use, then cached for the life of the process so the
/// operating system is asked once, not on every approval check.
pub struct BackendKeyStore<B: SecretBackend> {
    backend: B,
    cached: Mutex<Option<Key>>,
}

impl<B: SecretBackend> BackendKeyStore<B> {
    pub fn new(backend: B) -> Self {
        Self { backend, cached: Mutex::new(None) }
    }
}

pub fn encode_key(key: &Key) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn decode_key(text: &str) -> Option<Key> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// 256 random bits from the operating system's random source.
fn random_key() -> Key {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key
}

impl<B: SecretBackend> KeyStore for BackendKeyStore<B> {
    fn key(&self) -> Result<Key, KeyError> {
        let mut cached = self.cached.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(key) = *cached {
            return Ok(key);
        }
        let unavailable = |m: String| KeyError::Unavailable(m);
        let key = match self.backend.get().map_err(unavailable)? {
            Some(text) => decode_key(&text).ok_or_else(|| KeyError::Unavailable("the stored plugin key is corrupt".into()))?,
            None => {
                let key = random_key();
                self.backend.set(&encode_key(&key)).map_err(unavailable)?;
                key
            }
        };
        *cached = Some(key);
        Ok(key)
    }
}

/// Pick the key store. `passphrase` is the value of `DISCARD_PLUGIN_PASSPHRASE`.
pub fn choose(passphrase: Option<&str>, data_dir: &Path, system: Arc<dyn KeyStore>) -> Arc<dyn KeyStore> {
    let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) else { return system };
    // The salt is not secret. It only has to stay the same, so the same passphrase gives the same key.
    let salt_path = data_dir.join("plugin-key-salt");
    let salt = match std::fs::read(&salt_path) {
        Ok(salt) if !salt.is_empty() => Ok(salt),
        _ => {
            let salt = uuid::Uuid::new_v4().as_bytes().to_vec();
            std::fs::write(&salt_path, &salt).map(|()| salt)
        }
    };
    match salt {
        Ok(salt) => Arc::new(PassphraseKeyStore::new(passphrase, &salt)),
        Err(e) => Arc::new(UnavailableKeyStore(format!("could not save the passphrase salt: {e}"))),
    }
}

#[cfg(target_os = "macos")]
struct Keychain;

#[cfg(target_os = "macos")]
impl SecretBackend for Keychain {
    fn get(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new("Discard", "plugin-approval-key").map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(text) => Ok(Some(text)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, value: &str) -> Result<(), String> {
        keyring::Entry::new("Discard", "plugin-approval-key")
            .and_then(|entry| entry.set_password(value))
            .map_err(|e| e.to_string())
    }
}

/// The operating system's secret store, if this platform has one that Discard supports.
pub fn system_store() -> Arc<dyn KeyStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(BackendKeyStore::new(Keychain))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnavailableKeyStore("this platform has no supported secret store yet; set DISCARD_PLUGIN_PASSPHRASE".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Memory {
        value: Mutex<Option<String>>,
        gets: Mutex<usize>,
        fail: bool,
    }

    impl SecretBackend for Arc<Memory> {
        fn get(&self) -> Result<Option<String>, String> {
            *self.gets.lock().unwrap() += 1;
            if self.fail {
                return Err("keychain locked".into());
            }
            Ok(self.value.lock().unwrap().clone())
        }
        fn set(&self, value: &str) -> Result<(), String> {
            if self.fail {
                return Err("keychain locked".into());
            }
            *self.value.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
    }

    #[test]
    fn keys_round_trip_through_text() {
        let key: Key = std::array::from_fn(|i| i as u8 * 7);
        assert_eq!(decode_key(&encode_key(&key)), Some(key));
        assert_eq!(encode_key(&key).len(), 64);
    }

    #[test]
    fn malformed_stored_keys_are_rejected() {
        for bad in ["", "zz", &"a".repeat(63), &"a".repeat(65), &"g".repeat(64), "not a key"] {
            assert_eq!(decode_key(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_first_use_creates_and_saves_a_random_key_and_later_uses_return_it() {
        let backend = Arc::new(Memory::default());
        let store = BackendKeyStore::new(Arc::clone(&backend));
        let key = store.key().unwrap();
        assert_ne!(key, [0u8; 32], "the key must be random, not blank");
        assert_eq!(decode_key(backend.value.lock().unwrap().as_deref().unwrap()), Some(key));
        // A new process (a new store on the same backend) gets the same key.
        assert_eq!(BackendKeyStore::new(Arc::clone(&backend)).key().unwrap(), key);
    }

    #[test]
    fn two_installs_get_different_keys() {
        let a = BackendKeyStore::new(Arc::new(Memory::default())).key().unwrap();
        let b = BackendKeyStore::new(Arc::new(Memory::default())).key().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_backend_is_asked_once_per_process() {
        let backend = Arc::new(Memory::default());
        let store = BackendKeyStore::new(Arc::clone(&backend));
        for _ in 0..5 {
            store.key().unwrap();
        }
        assert_eq!(*backend.gets.lock().unwrap(), 1);
    }

    #[test]
    fn a_corrupt_stored_value_is_an_error_not_a_silent_new_key() {
        let backend = Arc::new(Memory::default());
        *backend.value.lock().unwrap() = Some("corrupt".into());
        let err = BackendKeyStore::new(Arc::clone(&backend)).key().unwrap_err();
        assert!(matches!(err, KeyError::Unavailable(m) if m.contains("corrupt")));
        assert_eq!(backend.value.lock().unwrap().as_deref(), Some("corrupt"), "an unreadable key must not be overwritten");
    }

    #[test]
    fn backend_failures_become_unavailable_errors() {
        let backend = Arc::new(Memory { fail: true, ..Memory::default() });
        let err = BackendKeyStore::new(backend).key().unwrap_err();
        assert!(matches!(err, KeyError::Unavailable(m) if m.contains("keychain locked")));
    }

    struct Fixed(Key);
    impl KeyStore for Fixed {
        fn key(&self) -> Result<Key, KeyError> {
            Ok(self.0)
        }
    }

    #[test]
    fn a_passphrase_wins_over_the_system_store_and_is_stable_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let system: Arc<dyn KeyStore> = Arc::new(Fixed([9; 32]));
        let a = choose(Some("correct horse"), dir.path(), Arc::clone(&system)).key().unwrap();
        let b = choose(Some("correct horse"), dir.path(), Arc::clone(&system)).key().unwrap();
        assert_eq!(a, b, "the salt is saved, so the same passphrase gives the same key");
        assert_ne!(a, [9; 32]);
        assert!(dir.path().join("plugin-key-salt").exists());
        assert_ne!(choose(Some("another one"), dir.path(), system).key().unwrap(), a);
    }

    #[test]
    fn no_passphrase_or_an_empty_one_uses_the_system_store() {
        let dir = tempfile::tempdir().unwrap();
        let system: Arc<dyn KeyStore> = Arc::new(Fixed([9; 32]));
        assert_eq!(choose(None, dir.path(), Arc::clone(&system)).key().unwrap(), [9; 32]);
        assert_eq!(choose(Some(""), dir.path(), system).key().unwrap(), [9; 32]);
    }

    #[test]
    fn if_the_salt_cannot_be_saved_there_is_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("not-a-dir");
        std::fs::write(&blocked, "file").unwrap();
        let store = choose(Some("pw"), &blocked, Arc::new(Fixed([9; 32])));
        assert!(matches!(store.key(), Err(KeyError::Unavailable(_))), "a passphrase that cannot be made stable must not fall back silently");
    }
}
