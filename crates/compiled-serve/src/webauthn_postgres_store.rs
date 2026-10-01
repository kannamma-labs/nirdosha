//! A durable, Postgres-backed [`crate::webauthn_store::PasskeyCredentialStore`]
//! -- the adapter [`crate::webauthn_store::InMemoryPasskeyCredentialStore`]'s
//! own doc comment names as real, separate follow-up work, built here by
//! reusing this workspace's own already-real, pooled+TLS Postgres
//! connection primitive (`nirdosha_runtime_kernels::kernel::db::connect`,
//! `docs/adr/0005-postgres-pooling-and-tls.md`) rather than re-implementing
//! pooling a third time the way `crates/nirdosha-guard-store-postgres` had
//! to -- that crate sits in a *different* Cargo workspace from
//! `runtime-kernels` and genuinely couldn't reach this code; `compiled-serve`
//! is already a member of the same workspace `runtime-kernels` is, so this
//! is a real dependency, not a duplicate implementation.
//!
//! **One held connection, not a self-managed pool.** `kernel::db::connect`
//! is designed around the compiled `.nir` language's own `db_connect`/
//! `db_stop` session pair (its own doc comment: a successful `connect`
//! holds one `domain::db()` admission slot that must be released exactly
//! once, when "this connection's session ends" -- a lifecycle this
//! module has no natural place to plug into, since it has no `.nir`
//! program's `HandleTable` closing a real user session around it).
//! Rather than guess at that release protocol from the outside and risk
//! leaking an admission slot on every credential-store call, this
//! adapter calls `connect` exactly once, at construction, and holds that
//! one pooled connection for its own entire lifetime, serializing access
//! through a [`nirdosha_rt::prelude::SharedCell`] (`postgres::Client`'s
//! methods need `&mut self`, and this dialect forbids a raw
//! `std::sync::Mutex` outright).
//! Honest tradeoff, not hidden: this is one dedicated connection, not a
//! pool of its own -- fine for WebAuthn's inherently low, human-paced
//! request rate (an interactive ceremony, never a hot path); a
//! higher-throughput consumer of this same pattern would need its own,
//! real per-call checkout story.

use crate::webauthn_store::{CredentialStoreError, PasskeyCredential, PasskeyCredentialStore};
use nirdosha_runtime_kernels::kernel::db;
use nirdosha_rt::prelude::SharedCell;

// Plain `CREATE TABLE IF NOT EXISTS` is *not* safe under concurrent
// callers: Postgres's existence check and the actual creation aren't one
// atomic step across separate sessions, so two `connect()` calls racing
// (e.g. a rolling restart bringing up several `compiled-serve` replicas
// at once, or -- as first caught this crate's own
// `tests/webauthn_postgres_store.rs` -- `cargo test`'s default parallel
// test threads each independently provisioning schema) can both pass the
// "does it exist" check before either commits, and the loser gets a real
// `duplicate_table` error instead of silently no-op'ing. Confirmed
// directly: 2 of 3 tests failed with `IF NOT EXISTS` alone under the
// default parallel test runner, 0 failed under `--test-threads=1`.
// Same idempotent-DDL problem `nirdosha-guard-store-postgres`'s own
// `RLS_DDL` already solved for `CREATE POLICY` (which has no
// `IF NOT EXISTS` form at all) -- here for `CREATE TABLE`, which does
// have that form but still needs the same real fix: a `DO` block gives
// an implicit savepoint, so catching `duplicate_table` there tolerates
// the loser of the race rather than erroring out.
const TABLE_DDL: &str = r#"
DO $$
BEGIN
    CREATE TABLE compiled_serve_webauthn_credentials (
        subject        TEXT PRIMARY KEY,
        credential_id  BYTEA NOT NULL,
        public_key_x   BYTEA NOT NULL,
        public_key_y   BYTEA NOT NULL,
        sign_count     BIGINT NOT NULL
    );
EXCEPTION
    WHEN duplicate_table THEN NULL;
END $$;
"#;

pub struct PostgresPasskeyCredentialStore {
    // The whole `DbConn` handle is kept, not just the `postgres::Client`
    // inside it -- `r2d2::PooledConnection` (what `DbConn::Postgres`
    // wraps) has no supported way to hand back an owned `postgres::Client`
    // without breaking its own "return to the pool on drop" guarantee;
    // `as_postgres_mut()` is called fresh, inside `SharedCell::with`,
    // every time this store needs `&mut postgres::Client` for one query.
    // `SharedCell` (not `std::sync::Mutex`) because this dialect forbids
    // raw locks outright (`cargo nirdosha verify`: "raw locks are not
    // supported") -- the same reason `webauthn_challenge.rs` and
    // `webauthn_store.rs`'s in-memory store use `SharedTable` instead of
    // one.
    conn: SharedCell<db::DbConn>,
}

impl PostgresPasskeyCredentialStore {
    /// Connects (real, pooled, TLS-by-default for a non-local host --
    /// `kernel::db::connect`'s own contract) and idempotently ensures
    /// the schema exists. Fails loudly at construction rather than
    /// lazily on first use.
    pub fn connect(conn_str: &str) -> Result<Self, String> {
        let mut handle = db::connect(conn_str)?;
        let client = handle.as_postgres_mut().ok_or_else(|| format!("{conn_str:?} did not resolve to a Postgres connection"))?;
        client.batch_execute(TABLE_DDL).map_err(|e| format!("creating compiled_serve_webauthn_credentials: {e}"))?;
        Ok(PostgresPasskeyCredentialStore { conn: SharedCell::new(handle) })
    }
}

impl PasskeyCredentialStore for PostgresPasskeyCredentialStore {
    fn save(&self, subject: &str, credential: PasskeyCredential) -> Result<(), CredentialStoreError> {
        self.conn.with(|handle| {
            let client = handle.as_postgres_mut().ok_or_else(|| CredentialStoreError::Backend("connection is not Postgres".to_string()))?;
            let sign_count = i64::from(credential.sign_count);
            client
                .execute(
                    "INSERT INTO compiled_serve_webauthn_credentials (subject, credential_id, public_key_x, public_key_y, sign_count) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (subject) DO UPDATE SET \
                         credential_id = EXCLUDED.credential_id, \
                         public_key_x = EXCLUDED.public_key_x, \
                         public_key_y = EXCLUDED.public_key_y, \
                         sign_count = EXCLUDED.sign_count",
                    &[&subject, &credential.credential_id, &credential.public_key_x.as_slice(), &credential.public_key_y.as_slice(), &sign_count],
                )
                .map_err(|e| CredentialStoreError::Backend(e.to_string()))?;
            Ok(())
        })
    }

    fn load(&self, subject: &str) -> Result<PasskeyCredential, CredentialStoreError> {
        self.conn.with(|handle| {
            let client = handle.as_postgres_mut().ok_or_else(|| CredentialStoreError::Backend("connection is not Postgres".to_string()))?;
            let row = client
                .query_opt("SELECT credential_id, public_key_x, public_key_y, sign_count FROM compiled_serve_webauthn_credentials WHERE subject = $1", &[&subject])
                .map_err(|e| CredentialStoreError::Backend(e.to_string()))?
                .ok_or(CredentialStoreError::NotFound)?;
            row_to_credential(&row)
        })
    }

    fn advance_sign_count(&self, subject: &str, new_count: u32) -> Result<(), CredentialStoreError> {
        self.conn.with(|handle| {
            let client = handle.as_postgres_mut().ok_or_else(|| CredentialStoreError::Backend("connection is not Postgres".to_string()))?;
            let sign_count = i64::from(new_count);
            let rows_affected = client
                .execute("UPDATE compiled_serve_webauthn_credentials SET sign_count = $1 WHERE subject = $2", &[&sign_count, &subject])
                .map_err(|e| CredentialStoreError::Backend(e.to_string()))?;
            if rows_affected == 0 {
                return Err(CredentialStoreError::NotFound);
            }
            Ok(())
        })
    }
}

fn row_to_credential(row: &postgres::Row) -> Result<PasskeyCredential, CredentialStoreError> {
    let credential_id: Vec<u8> = row.get(0);
    let public_key_x: Vec<u8> = row.get(1);
    let public_key_y: Vec<u8> = row.get(2);
    let sign_count: i64 = row.get(3);
    let public_key_x: [u8; 32] = public_key_x.try_into().map_err(|v: Vec<u8>| CredentialStoreError::Backend(format!("stored public_key_x is {} bytes, expected 32", v.len())))?;
    let public_key_y: [u8; 32] = public_key_y.try_into().map_err(|v: Vec<u8>| CredentialStoreError::Backend(format!("stored public_key_y is {} bytes, expected 32", v.len())))?;
    let sign_count = u32::try_from(sign_count).map_err(|_| CredentialStoreError::Backend(format!("stored sign_count {sign_count} does not fit in u32")))?;
    Ok(PasskeyCredential { credential_id, public_key_x, public_key_y, sign_count })
}
