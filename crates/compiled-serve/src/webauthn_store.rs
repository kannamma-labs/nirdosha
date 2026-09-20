//! The storage half of WebAuthn support: a port for persisting
//! registered passkey credentials, kept deliberately separate from
//! [`crate::webauthn_crypto::PasskeyCryptoAdapter`] (the vendor-specific
//! crypto/protocol concern) — a swappable storage backend and a
//! swappable crypto backend are two independent decisions, and forcing
//! them behind one trait would make neither cleanly swappable on its
//! own.

/// One registered passkey: the public key COSE-decoded at registration
/// time (kept in its already-verified, already-parsed form — a store
/// implementation never needs to re-parse CBOR), and the authenticator's
/// own signature counter, WebAuthn's own cloned-authenticator detection
/// mechanism (a counter that ever *decreases* between logins means two
/// physical authenticators are sharing one credential).
#[derive(Debug, Clone)]
pub struct PasskeyCredential {
    pub credential_id: Vec<u8>,
    pub public_key_x: [u8; 32],
    pub public_key_y: [u8; 32],
    pub sign_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialStoreError {
    NotFound,
    Backend(String),
}

impl std::fmt::Display for CredentialStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialStoreError::NotFound => write!(f, "no passkey credential registered for this subject"),
            CredentialStoreError::Backend(msg) => write!(f, "passkey credential store error: {msg}"),
        }
    }
}

/// Storage port for passkey credentials. `Send + Sync`: shared across
/// this server's connection-handling threads the same way `ServeConfig`
/// itself is (behind an `Arc`, never cloned per-request).
pub trait PasskeyCredentialStore: Send + Sync {
    /// Registers `credential` for `subject`, replacing any credential
    /// previously registered for that subject -- v1 scope is one
    /// credential per subject, stated plainly in
    /// `crate::webauthn_crypto`'s own module doc comment, not hidden
    /// inside this method's behavior.
    fn save(&self, subject: &str, credential: PasskeyCredential) -> Result<(), CredentialStoreError>;

    /// The credential registered for `subject`, if any.
    fn load(&self, subject: &str) -> Result<PasskeyCredential, CredentialStoreError>;

    /// Advances the stored sign counter after a successful login --
    /// separate from `save` because a login's counter update must never
    /// touch the public key or credential ID, only the counter, and a
    /// storage backend should not have to reconstruct the whole
    /// credential just to bump one field.
    fn advance_sign_count(&self, subject: &str, new_count: u32) -> Result<(), CredentialStoreError>;
}

/// Reference implementation: per-process, in-memory, **not durable** --
/// every registered passkey is lost on restart. Stated here as plainly
/// as `MemStoreDriver`'s own doc comment states the same limitation for
/// the guard's write path (`crates/nirdosha-guard-mic`) -- a real
/// deployment needs a durable backend (a natural fit for the same
/// pattern `crates/nirdosha-guard-store-postgres` already established
/// for a different port), not built here.
///
/// Backed by `nirdosha_rt::prelude::SharedTable`, not a raw
/// `std::sync::Mutex<HashMap<..>>` -- the dialect's own managed
/// primitive for exactly this shape of state, the same reason
/// `RelationResolver`/`StoreDriver`'s in-crate reference stores follow
/// suit rather than reaching for `Mutex` directly (confirmed via
/// `cargo nirdosha verify`: a raw `Mutex` is a real, named dialect
/// restriction violation, "raw locks are not supported").
pub struct InMemoryPasskeyCredentialStore {
    credentials: nirdosha_rt::prelude::SharedTable<String, PasskeyCredential>,
}

impl InMemoryPasskeyCredentialStore {
    pub fn new() -> Self {
        InMemoryPasskeyCredentialStore { credentials: nirdosha_rt::prelude::SharedTable::new() }
    }
}

impl Default for InMemoryPasskeyCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyCredentialStore for InMemoryPasskeyCredentialStore {
    fn save(&self, subject: &str, credential: PasskeyCredential) -> Result<(), CredentialStoreError> {
        self.credentials.insert(subject.to_string(), credential);
        Ok(())
    }

    fn load(&self, subject: &str) -> Result<PasskeyCredential, CredentialStoreError> {
        self.credentials.get(&subject.to_string()).ok_or(CredentialStoreError::NotFound)
    }

    fn advance_sign_count(&self, subject: &str, new_count: u32) -> Result<(), CredentialStoreError> {
        self.credentials.update(&subject.to_string(), |entry| match entry {
            Some(credential) => {
                credential.sign_count = new_count;
                Ok(())
            }
            None => Err(CredentialStoreError::NotFound),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_credential() -> PasskeyCredential {
        PasskeyCredential { credential_id: vec![1, 2, 3], public_key_x: [7u8; 32], public_key_y: [9u8; 32], sign_count: 0 }
    }

    #[test]
    fn save_then_load_round_trips() {
        let store = InMemoryPasskeyCredentialStore::new();
        store.save("alice", sample_credential()).unwrap();
        let loaded = store.load("alice").unwrap();
        assert_eq!(loaded.credential_id, vec![1, 2, 3]);
        assert_eq!(loaded.sign_count, 0);
    }

    #[test]
    fn loading_an_unregistered_subject_is_not_found() {
        let store = InMemoryPasskeyCredentialStore::new();
        assert_eq!(store.load("nobody").unwrap_err(), CredentialStoreError::NotFound);
    }

    #[test]
    fn advancing_sign_count_only_touches_the_counter() {
        let store = InMemoryPasskeyCredentialStore::new();
        store.save("alice", sample_credential()).unwrap();
        store.advance_sign_count("alice", 5).unwrap();
        let loaded = store.load("alice").unwrap();
        assert_eq!(loaded.sign_count, 5);
        assert_eq!(loaded.credential_id, vec![1, 2, 3]);
    }

    #[test]
    fn advancing_sign_count_for_an_unregistered_subject_is_not_found() {
        let store = InMemoryPasskeyCredentialStore::new();
        assert_eq!(store.advance_sign_count("nobody", 1).unwrap_err(), CredentialStoreError::NotFound);
    }

    #[test]
    fn saving_again_replaces_the_previous_credential() {
        let store = InMemoryPasskeyCredentialStore::new();
        store.save("alice", sample_credential()).unwrap();
        let mut second = sample_credential();
        second.credential_id = vec![9, 9, 9];
        store.save("alice", second).unwrap();
        assert_eq!(store.load("alice").unwrap().credential_id, vec![9, 9, 9]);
    }
}
