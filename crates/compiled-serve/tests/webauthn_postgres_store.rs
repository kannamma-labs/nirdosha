//! Opt-in, real-Postgres proof for `PostgresPasskeyCredentialStore`,
//! matching this workspace's existing `NIRDOSHA_TEST_POSTGRES_URL`-gated,
//! `#[ignore]`-by-default convention
//! (`crates/runtime-kernels/src/kernel/db.rs`'s own tests,
//! `crates/nirdosha-guard-store-postgres/tests/postgres_store_driver.rs`
//! in the root workspace): never run by CI, fails loudly rather than
//! skipping when no real server is reachable.
//!
//! Run against `docker-compose.dev.yml`:
//!   docker compose -f docker-compose.dev.yml up -d
//!   NIRDOSHA_TEST_POSTGRES_URL=postgres://nirdosha:nirdosha@127.0.0.1:5432/nirdosha_dev \
//!       cargo test -p nirdosha-compiled-serve --test webauthn_postgres_store -- --ignored

use nirdosha_compiled_serve::{CredentialStoreError, PasskeyCredential, PasskeyCredentialStore, PostgresPasskeyCredentialStore};

fn test_url() -> String {
    std::env::var("NIRDOSHA_TEST_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5432/postgres".to_string())
}

fn sample_credential(id: &[u8]) -> PasskeyCredential {
    PasskeyCredential { credential_id: id.to_vec(), public_key_x: [7u8; 32], public_key_y: [9u8; 32], sign_count: 0 }
}

#[test]
#[ignore]
fn save_load_and_advance_round_trip_through_real_postgres() {
    let store = PostgresPasskeyCredentialStore::connect(&test_url()).expect("connect + provision schema");
    let subject = format!("test-subject-{}", std::process::id());

    store.save(&subject, sample_credential(b"real-pg-cred")).expect("save should succeed");
    let loaded = store.load(&subject).expect("load should find what was just saved");
    assert_eq!(loaded.credential_id, b"real-pg-cred");
    assert_eq!(loaded.sign_count, 0);

    store.advance_sign_count(&subject, 42).expect("advance_sign_count should succeed");
    let after = store.load(&subject).expect("load after advance");
    assert_eq!(after.sign_count, 42);
    assert_eq!(after.credential_id, b"real-pg-cred", "advancing the counter must not disturb the credential itself");
}

#[test]
#[ignore]
fn loading_an_unregistered_subject_is_not_found_through_real_postgres() {
    let store = PostgresPasskeyCredentialStore::connect(&test_url()).expect("connect + provision schema");
    let subject = format!("never-registered-{}", std::process::id());
    assert_eq!(store.load(&subject).unwrap_err(), CredentialStoreError::NotFound);
}

#[test]
#[ignore]
fn a_credential_registered_in_one_connect_call_survives_a_fresh_one() {
    // Proves this store is actually durable -- a second, independent
    // `connect()` call (standing in for a process restart) still finds
    // what an earlier one saved, unlike `InMemoryPasskeyCredentialStore`,
    // which this adapter exists specifically to not be.
    let subject = format!("durability-check-{}", std::process::id());
    {
        let store = PostgresPasskeyCredentialStore::connect(&test_url()).expect("first connect");
        store.save(&subject, sample_credential(b"durable-cred")).expect("save should succeed");
    }
    let store = PostgresPasskeyCredentialStore::connect(&test_url()).expect("second, independent connect");
    let loaded = store.load(&subject).expect("a fresh connection must still see the earlier save");
    assert_eq!(loaded.credential_id, b"durable-cred");
}
