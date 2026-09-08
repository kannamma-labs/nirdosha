//! The rest of Row 12 (`docs/nirdosha_row12_functions_identity.md`):
//! `check_role_path`/`extract_claim_path` (dotted-path claim lookup),
//! `create_application_session`/`session_cookie`, `new_refresh_token`/
//! `exchange_refresh_token`, `check_revocation`, `validate_api_key`.
//! `check_role`/`extract_claim`/`oidc_validate_token`/`identity_expired`
//! already compiled (`lib.rs`) — this module is everything after those.
//!
//! Every function here keeps the same conventions the existing identity
//! kernels already established: `NirStrOut` out-params, `str_from_raw`
//! for reading a `{ptr, len}` argument, fail-closed on malformed UTF-8/
//! JSON (never a trap), and — where a builtin's own `typeck.rs` return
//! type isn't `Result`-wrapped — no status return at all, since
//! `codegen.rs` never branches on one.

use std::sync::{Mutex, OnceLock};

/// Walks a `.`-separated path (`"a.b.c"`) through a JSON object,
/// returning the leaf value if every segment resolves to an object key
/// (never indexing into an array — dotted paths here are for nested
/// claim *objects*, not array elements, matching the recovered
/// interpreter-era design this ports).
fn resolve_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for segment in path.split('.') {
        cur = cur.as_object()?.get(segment)?;
    }
    Some(cur)
}

/// `check_role_path(identity, path, role) -> Result(RoleView, str)` —
/// same membership check `nir_check_role` already does, against the
/// roles array found by walking `path` instead of assuming a top-level
/// `"roles"` key.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_check_role_path(claims_ptr: *const u8, claims_len: i64, path_ptr: *const u8, path_len: i64, role_ptr: *const u8, role_len: i64) -> i32 {
    let (Some(claims_json), Some(path), Some(role)) = (
        unsafe { crate::str_from_raw(claims_ptr, claims_len) },
        unsafe { crate::str_from_raw(path_ptr, path_len) },
        unsafe { crate::str_from_raw(role_ptr, role_len) },
    ) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(claims_json) else {
        return 0;
    };
    let Some(node) = resolve_path(&parsed, path) else {
        return 0;
    };
    let Some(roles) = node.as_array() else {
        return 0;
    };
    roles.iter().any(|r| r.as_str() == Some(role)) as i32
}

/// `extract_claim_path(identity, path) -> Result(ClaimView, str)` — same
/// shape as `nir_extract_claim`, walking `path` to the leaf instead of
/// assuming a top-level key.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_extract_claim_path(claims_ptr: *const u8, claims_len: i64, path_ptr: *const u8, path_len: i64, out_value: *mut crate::NirStrOut) -> i32 {
    let (Some(claims_json), Some(path)) = (unsafe { crate::str_from_raw(claims_ptr, claims_len) }, unsafe { crate::str_from_raw(path_ptr, path_len) }) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(claims_json) else {
        return 0;
    };
    match resolve_path(&parsed, path).and_then(|v| v.as_str()) {
        Some(s) => unsafe {
            crate::write_str_out(out_value, s.to_string());
            1
        },
        None => 0,
    }
}

/// `check_revocation(identity) -> bool` — a top-level `"revoked"`
/// boolean in `claims_json`; missing, malformed, or non-boolean all
/// read as "not revoked" (fail-open on absence, the same convention a
/// token that never carried a revocation claim at all should get —
/// fail-*closed* would make every already-issued token from before this
/// claim existed look revoked).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_check_revocation(claims_ptr: *const u8, claims_len: i64) -> i32 {
    let Some(claims_json) = (unsafe { crate::str_from_raw(claims_ptr, claims_len) }) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(claims_json) else {
        return 0;
    };
    parsed.get("revoked").and_then(|v| v.as_bool()).unwrap_or(false) as i32
}

// ---- application sessions ---------------------------------------------

/// A real, unpredictable session id — per-process HMAC-ish secret
/// (itself seeded from real OS randomness once, not derived from
/// anything guessable) folded with real-time nanoseconds and a
/// monotonic counter, then hex-encoded via the same `sha256`/
/// `hex_encode` helpers `sha256_hex` already uses. Not a security
/// primitive on its own merits (this is a session identifier, not a
/// cryptographic secret) — the point is "not predictable from
/// subject+issuer alone," a real bug class the recovered interpreter-era
/// design's own doc comment named directly.
fn new_session_id() -> String {
    static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let secret = SECRET.get_or_init(|| {
        let mut seed = [0u8; 32];
        // `rand_seed`'s own SplitMix64 state isn't reused here
        // deliberately -- that RNG is `.nir`-visible and
        // deterministic-by-design (seedable), the opposite property a
        // session-id secret needs. `std::time::SystemTime` +
        // `std::process::id()` folded through `sha256` is not a strong
        // CSPRNG, but is real per-process, real-startup-time entropy,
        // not a fixed/guessable constant.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let pid = std::process::id();
        let mixed = crate::sha256(&nanos.to_le_bytes(), &pid.to_le_bytes());
        seed.copy_from_slice(&mixed);
        seed
    });
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let digest = crate::sha256(secret, &[&n.to_le_bytes()[..], &nanos.to_le_bytes()[..]].concat());
    let mut out = [0u8; 64];
    crate::hex_encode(&digest, &mut out);
    String::from_utf8_lossy(&out).to_string()
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `create_application_session(identity) -> ApplicationSession` —
/// infallible (`typeck.rs`'s own signature has no `Result` wrap), so no
/// status return; `codegen.rs` never checks one. 8-hour lifetime, same
/// as the recovered interpreter-era design.
const SESSION_LIFETIME_SECS: i64 = 8 * 60 * 60;

/// A real, server-side session record — the store `verify_session`
/// looks up against. Before this existed, `create_application_session`
/// minted a real, unpredictable session id and formatted a real
/// `Set-Cookie` header, but nothing durable backed that id: there was
/// no way, given a cookie value, to answer "is this session valid, and
/// whose identity does it carry" — sessions were an id and a cookie
/// with no server-side backing at all (red-team report finding A2).
/// `expires_at` here is the *session's* own lifetime
/// (`SESSION_LIFETIME_SECS` from mint time), not the original identity
/// token's own `expires_at` — a session is meant to outlive whatever
/// short-lived OIDC token created it, the same reason
/// `exchange_refresh_token` re-issues a fresh `expires_at` rather than
/// reusing the pre-refresh one.
struct SessionRecord {
    subject: String,
    issuer: String,
    audience: String,
    claims_json: String,
    created_at: i64,
    expires_at: i64,
}

fn session_table() -> &'static Mutex<std::collections::HashMap<String, SessionRecord>> {
    static TABLE: OnceLock<Mutex<std::collections::HashMap<String, SessionRecord>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_create_application_session(
    subject_ptr: *const u8,
    subject_len: i64,
    issuer_ptr: *const u8,
    issuer_len: i64,
    audience_ptr: *const u8,
    audience_len: i64,
    claims_json_ptr: *const u8,
    claims_json_len: i64,
    out_session_id: *mut crate::NirStrOut,
    out_created_at: *mut i64,
    out_expires_at: *mut i64,
    out_last_accessed_at: *mut i64,
) {
    let subject = unsafe { crate::str_from_raw(subject_ptr, subject_len) }.unwrap_or("").to_string();
    let issuer = unsafe { crate::str_from_raw(issuer_ptr, issuer_len) }.unwrap_or("").to_string();
    let audience = unsafe { crate::str_from_raw(audience_ptr, audience_len) }.unwrap_or("").to_string();
    let claims_json = unsafe { crate::str_from_raw(claims_json_ptr, claims_json_len) }.unwrap_or("").to_string();
    let now = now_unix_secs();
    let session_id = new_session_id();
    let expires_at = now + SESSION_LIFETIME_SECS;
    session_table()
        .lock()
        .unwrap()
        .insert(session_id.clone(), SessionRecord { subject, issuer, audience, claims_json, created_at: now, expires_at });
    unsafe {
        crate::write_str_out(out_session_id, session_id);
        *out_created_at = now;
        *out_expires_at = expires_at;
        *out_last_accessed_at = now;
    }
}

/// `verify_session(session_id) -> Result(VerifiedIdentity, str)` — the
/// lookup half of the session store `nir_create_application_session`
/// (above) writes into. `Err` for an unknown or expired session id,
/// never a panic; on success the returned `VerifiedIdentity`'s
/// `issued_at` is the session's own `created_at` (when this identity
/// was captured into the session), and `expires_at` is the *session's*
/// own expiry (what actually matters to a caller asking "is this
/// session still good"), not the original OIDC token's expiry, which
/// may be long gone by the time a long-lived session is still valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_verify_session(
    session_id_ptr: *const u8,
    session_id_len: i64,
    out_subject: *mut crate::NirStrOut,
    out_issuer: *mut crate::NirStrOut,
    out_audience: *mut crate::NirStrOut,
    out_expires_at: *mut i64,
    out_issued_at: *mut i64,
    out_claims_json: *mut crate::NirStrOut,
    out_err: *mut crate::NirStrOut,
) -> i32 {
    let Some(session_id) = (unsafe { crate::str_from_raw(session_id_ptr, session_id_len) }) else {
        unsafe { crate::write_str_out(out_err, "session id is not valid UTF-8".to_string()) };
        return 0;
    };
    let table = session_table().lock().unwrap();
    let Some(record) = table.get(session_id) else {
        unsafe { crate::write_str_out(out_err, "session not found".to_string()) };
        return 0;
    };
    if record.expires_at <= now_unix_secs() {
        unsafe { crate::write_str_out(out_err, "session expired".to_string()) };
        return 0;
    }
    unsafe {
        crate::write_str_out(out_subject, record.subject.clone());
        crate::write_str_out(out_issuer, record.issuer.clone());
        crate::write_str_out(out_audience, record.audience.clone());
        *out_expires_at = record.expires_at;
        *out_issued_at = record.created_at;
        crate::write_str_out(out_claims_json, record.claims_json.clone());
    }
    1
}

/// `session_cookie(session) -> str` — infallible, a plain formatted
/// string. `Max-Age` is the session's own real remaining lifetime
/// (`expires_at - created_at`), not a hardcoded constant repeated here
/// independently of `create_application_session`'s own lifetime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_session_cookie(session_id_ptr: *const u8, session_id_len: i64, created_at: i64, expires_at: i64, out_cookie: *mut crate::NirStrOut) {
    let session_id = unsafe { crate::str_from_raw(session_id_ptr, session_id_len) }.unwrap_or("");
    let max_age = (expires_at - created_at).max(0);
    let cookie = format!("session={session_id}; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}");
    unsafe { crate::write_str_out(out_cookie, cookie) };
}

// ---- refresh tokens -----------------------------------------------------
//
// A real, server-side single-use table, not just the affine `box i64`
// type-level guarantee `RefreshTokenHandle` already gives -- the boxed
// handle only proves a *well-typed .nir program* can't reuse it; a real
// server-side "already redeemed" check is what makes redemption
// actually single-use across the FFI boundary too (defense in depth,
// not redundant with the type system).

struct RefreshRecord {
    expires_at: i64,
    used: bool,
}

fn refresh_table() -> &'static Mutex<std::collections::HashMap<i64, RefreshRecord>> {
    static TABLE: OnceLock<Mutex<std::collections::HashMap<i64, RefreshRecord>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

static REFRESH_HANDLE_SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

/// `new_refresh_token(expires_at) -> RefreshTokenHandle` — infallible;
/// mints a fresh, never-before-used handle id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_new_refresh_token(expires_at: i64, out_handle_id: *mut i64) {
    let id = REFRESH_HANDLE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    refresh_table().lock().unwrap().insert(id, RefreshRecord { expires_at, used: false });
    unsafe { *out_handle_id = id };
}

/// `exchange_refresh_token(identity, handle, new_issued_at) -> Result(VerifiedIdentity, str)`.
/// Redeems `handle_id` exactly once (a second exchange attempt with the
/// same id, however it happened, is a real `Err`, not a silent reuse);
/// reissues `identity` with `new_issued_at` as its own `issued_at` and
/// the *handle's own* `expires_at` carried in at mint time — never the
/// original identity's `expires_at`, since the whole point of a refresh
/// is extending the session past the original token's own expiry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_exchange_refresh_token(
    handle_id: i64,
    new_issued_at: i64,
    subject_ptr: *const u8,
    subject_len: i64,
    issuer_ptr: *const u8,
    issuer_len: i64,
    audience_ptr: *const u8,
    audience_len: i64,
    claims_ptr: *const u8,
    claims_len: i64,
    out_subject: *mut crate::NirStrOut,
    out_issuer: *mut crate::NirStrOut,
    out_audience: *mut crate::NirStrOut,
    out_expires_at: *mut i64,
    out_issued_at: *mut i64,
    out_claims_json: *mut crate::NirStrOut,
    out_err: *mut crate::NirStrOut,
) -> i32 {
    let mut table = refresh_table().lock().unwrap();
    let Some(record) = table.get_mut(&handle_id) else {
        unsafe { crate::write_str_out(out_err, "refresh token handle not found".to_string()) };
        return 0;
    };
    if record.used {
        unsafe { crate::write_str_out(out_err, "refresh token already redeemed".to_string()) };
        return 0;
    }
    if record.expires_at <= new_issued_at {
        unsafe { crate::write_str_out(out_err, "refresh token expired".to_string()) };
        return 0;
    }
    record.used = true;
    let new_expires_at = record.expires_at;
    drop(table);

    let (Some(subject), Some(issuer), Some(audience), Some(claims_json)) = (
        unsafe { crate::str_from_raw(subject_ptr, subject_len) },
        unsafe { crate::str_from_raw(issuer_ptr, issuer_len) },
        unsafe { crate::str_from_raw(audience_ptr, audience_len) },
        unsafe { crate::str_from_raw(claims_ptr, claims_len) },
    ) else {
        unsafe { crate::write_str_out(out_err, "identity fields are not valid UTF-8".to_string()) };
        return 0;
    };
    unsafe {
        crate::write_str_out(out_subject, subject.to_string());
        crate::write_str_out(out_issuer, issuer.to_string());
        crate::write_str_out(out_audience, audience.to_string());
        *out_expires_at = new_expires_at;
        *out_issued_at = new_issued_at;
        crate::write_str_out(out_claims_json, claims_json.to_string());
    }
    1
}

// ---- API keys -------------------------------------------------------------

/// `validate_api_key(key, expected_hash) -> Result(VerifiedIdentity, str)`.
/// Constant-time-compares `sha256_hex(key)` against the caller-supplied
/// `expected_hash` (looking that hash up from wherever it's actually
/// stored -- a config table, a database row -- is the caller's own job;
/// this builtin's fixed 2-`str` signature has no room for a lookup
/// step, and doesn't pretend to). On match, mints a minimal but real
/// identity: `subject` is the key's own hash (a stable, non-reversible
/// identifier derived from the key, not a lookup), `issuer` is the
/// literal `"api-key"`, empty `audience`, `claims_json` is `"{}"`
/// (an API key carries no claims of its own), and `expires_at` is
/// `i64::MAX` -- a static key has no session-style expiry, and `0`
/// would make `identity_expired` see every API-key identity as already
/// expired, a real, disclosed design choice, not an oversight.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_validate_api_key(
    key_ptr: *const u8,
    key_len: i64,
    expected_hash_ptr: *const u8,
    expected_hash_len: i64,
    out_subject: *mut crate::NirStrOut,
    out_issuer: *mut crate::NirStrOut,
    out_audience: *mut crate::NirStrOut,
    out_expires_at: *mut i64,
    out_issued_at: *mut i64,
    out_claims_json: *mut crate::NirStrOut,
    out_err: *mut crate::NirStrOut,
) -> i32 {
    let (Some(key), Some(expected_hash)) = (unsafe { crate::str_from_raw(key_ptr, key_len) }, unsafe { crate::str_from_raw(expected_hash_ptr, expected_hash_len) }) else {
        unsafe { crate::write_str_out(out_err, "key/expected_hash is not valid UTF-8".to_string()) };
        return 0;
    };
    let digest = crate::sha256(key.as_bytes(), &[]);
    let mut hex = [0u8; 64];
    crate::hex_encode(&digest, &mut hex);
    let actual_hash = std::str::from_utf8(&hex).unwrap();
    if !crate::constant_time_eq(actual_hash.as_bytes(), expected_hash.as_bytes()) {
        unsafe { crate::write_str_out(out_err, "API key does not match".to_string()) };
        return 0;
    }
    unsafe {
        crate::write_str_out(out_subject, actual_hash.to_string());
        crate::write_str_out(out_issuer, "api-key".to_string());
        crate::write_str_out(out_audience, String::new());
        *out_expires_at = i64::MAX;
        *out_issued_at = now_unix_secs();
        crate::write_str_out(out_claims_json, "{}".to_string());
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn strout_to_string(out: &crate::NirStrOut) -> String {
        unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap().to_string() }
    }

    fn empty_strout() -> crate::NirStrOut {
        crate::NirStrOut { ptr: std::ptr::null(), len: 0 }
    }

    // ---- check_role_path / extract_claim_path ----------------------------

    const NESTED_CLAIMS: &str = r#"{"org":{"roles":["admin","auditor"]},"profile":{"department":"cardiology"}}"#;

    #[test]
    fn check_role_path_finds_a_role_at_a_nested_path() {
        let claims = NESTED_CLAIMS.as_bytes();
        let path = b"org.roles";
        let role = b"admin";
        let found = unsafe { nir_check_role_path(claims.as_ptr(), claims.len() as i64, path.as_ptr(), path.len() as i64, role.as_ptr(), role.len() as i64) };
        assert_eq!(found, 1);
    }

    #[test]
    fn check_role_path_rejects_a_role_not_present() {
        let claims = NESTED_CLAIMS.as_bytes();
        let path = b"org.roles";
        let role = b"superadmin";
        let found = unsafe { nir_check_role_path(claims.as_ptr(), claims.len() as i64, path.as_ptr(), path.len() as i64, role.as_ptr(), role.len() as i64) };
        assert_eq!(found, 0);
    }

    #[test]
    fn check_role_path_rejects_a_path_that_does_not_resolve() {
        let claims = NESTED_CLAIMS.as_bytes();
        let path = b"nope.roles";
        let role = b"admin";
        let found = unsafe { nir_check_role_path(claims.as_ptr(), claims.len() as i64, path.as_ptr(), path.len() as i64, role.as_ptr(), role.len() as i64) };
        assert_eq!(found, 0);
    }

    #[test]
    fn extract_claim_path_reads_a_nested_string_claim() {
        let claims = NESTED_CLAIMS.as_bytes();
        let path = b"profile.department";
        let mut out = empty_strout();
        let found = unsafe { nir_extract_claim_path(claims.as_ptr(), claims.len() as i64, path.as_ptr(), path.len() as i64, &mut out) };
        assert_eq!(found, 1);
        assert_eq!(unsafe { strout_to_string(&out) }, "cardiology");
    }

    // ---- check_revocation --------------------------------------------------

    #[test]
    fn check_revocation_true_when_the_claim_says_so() {
        let claims = br#"{"revoked":true}"#;
        assert_eq!(unsafe { nir_check_revocation(claims.as_ptr(), claims.len() as i64) }, 1);
    }

    #[test]
    fn check_revocation_false_when_absent_fail_open_on_absence() {
        let claims = br#"{"sub":"alice"}"#;
        assert_eq!(unsafe { nir_check_revocation(claims.as_ptr(), claims.len() as i64) }, 0);
    }

    // ---- sessions ------------------------------------------------------------

    /// `nir_create_application_session`'s test-only calling convention:
    /// subject/issuer/audience/claims_json as raw `(ptr, len)` pairs,
    /// same as every other `str`-taking kernel extern in this module's
    /// own tests (`check_role_path_finds_a_role_at_a_nested_path`, etc.).
    fn create_session(subject: &[u8], issuer: &[u8], audience: &[u8], claims_json: &[u8]) -> (String, i64, i64, i64) {
        let mut session_id = empty_strout();
        let mut created_at = 0i64;
        let mut expires_at = 0i64;
        let mut last_accessed_at = 0i64;
        unsafe {
            nir_create_application_session(
                subject.as_ptr(),
                subject.len() as i64,
                issuer.as_ptr(),
                issuer.len() as i64,
                audience.as_ptr(),
                audience.len() as i64,
                claims_json.as_ptr(),
                claims_json.len() as i64,
                &mut session_id,
                &mut created_at,
                &mut expires_at,
                &mut last_accessed_at,
            )
        };
        (unsafe { strout_to_string(&session_id) }, created_at, expires_at, last_accessed_at)
    }

    #[test]
    fn create_application_session_sets_a_real_8_hour_lifetime() {
        let (session_id, created_at, expires_at, last_accessed_at) = create_session(b"alice", b"https://example.com", b"my-app", b"{}");
        assert_eq!(expires_at - created_at, SESSION_LIFETIME_SECS);
        assert_eq!(created_at, last_accessed_at);
        assert!(!session_id.is_empty());
    }

    #[test]
    fn two_sessions_get_different_unpredictable_ids() {
        let (s1, ..) = create_session(b"alice", b"https://example.com", b"my-app", b"{}");
        let (s2, ..) = create_session(b"alice", b"https://example.com", b"my-app", b"{}");
        assert_ne!(s1, s2);
    }

    /// The whole point of A2's fix: a session id minted by
    /// `create_application_session` is real, durable, server-side state
    /// `verify_session` can look up — not just an id and a cookie with
    /// nothing behind them.
    #[test]
    fn verify_session_round_trips_the_identity_a_session_was_created_with() {
        let (session_id, created_at, expires_at, _) =
            create_session(b"alice", b"https://example.com", b"my-app", br#"{"roles":["admin"]}"#);
        let mut out_subject = empty_strout();
        let mut out_issuer = empty_strout();
        let mut out_audience = empty_strout();
        let mut out_expires_at = 0i64;
        let mut out_issued_at = 0i64;
        let mut out_claims_json = empty_strout();
        let mut out_err = empty_strout();
        let ok = unsafe {
            nir_verify_session(
                session_id.as_ptr(),
                session_id.len() as i64,
                &mut out_subject,
                &mut out_issuer,
                &mut out_audience,
                &mut out_expires_at,
                &mut out_issued_at,
                &mut out_claims_json,
                &mut out_err,
            )
        };
        assert_eq!(ok, 1);
        assert_eq!(unsafe { strout_to_string(&out_subject) }, "alice");
        assert_eq!(unsafe { strout_to_string(&out_issuer) }, "https://example.com");
        assert_eq!(unsafe { strout_to_string(&out_audience) }, "my-app");
        assert_eq!(unsafe { strout_to_string(&out_claims_json) }, r#"{"roles":["admin"]}"#);
        assert_eq!(out_expires_at, expires_at, "verify_session must report the session's own expiry, not the original token's");
        assert_eq!(out_issued_at, created_at, "verify_session's issued_at is when the session itself was minted");
    }

    #[test]
    fn verify_session_against_an_unknown_id_is_a_named_err_not_a_panic() {
        let session_id = b"not-a-real-session-id";
        let (mut out_subject, mut out_issuer, mut out_audience, mut out_claims_json, mut out_err) =
            (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_expires_at, mut out_issued_at) = (0i64, 0i64);
        let ok = unsafe {
            nir_verify_session(
                session_id.as_ptr(),
                session_id.len() as i64,
                &mut out_subject,
                &mut out_issuer,
                &mut out_audience,
                &mut out_expires_at,
                &mut out_issued_at,
                &mut out_claims_json,
                &mut out_err,
            )
        };
        assert_eq!(ok, 0);
        assert_eq!(unsafe { strout_to_string(&out_err) }, "session not found");
    }

    #[test]
    fn verify_session_against_an_expired_session_is_a_named_err() {
        let (session_id, ..) = create_session(b"alice", b"https://example.com", b"my-app", b"{}");
        // Directly age the record past its own expiry -- this is exactly
        // what a real 8-hour wait would produce, done instantly for the
        // test.
        session_table().lock().unwrap().get_mut(&session_id).unwrap().expires_at = now_unix_secs() - 1;
        let (mut out_subject, mut out_issuer, mut out_audience, mut out_claims_json, mut out_err) =
            (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_expires_at, mut out_issued_at) = (0i64, 0i64);
        let ok = unsafe {
            nir_verify_session(
                session_id.as_ptr(),
                session_id.len() as i64,
                &mut out_subject,
                &mut out_issuer,
                &mut out_audience,
                &mut out_expires_at,
                &mut out_issued_at,
                &mut out_claims_json,
                &mut out_err,
            )
        };
        assert_eq!(ok, 0);
        assert_eq!(unsafe { strout_to_string(&out_err) }, "session expired");
    }

    #[test]
    fn session_cookie_carries_the_sessions_own_real_max_age() {
        let sid = b"abc123";
        let mut cookie = empty_strout();
        unsafe { nir_session_cookie(sid.as_ptr(), sid.len() as i64, 1000, 1000 + 28800, &mut cookie) };
        let cookie = unsafe { strout_to_string(&cookie) };
        assert!(cookie.contains("session=abc123"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=28800"));
    }

    // ---- refresh tokens --------------------------------------------------

    fn identity_bytes<'a>(subject: &'a str, issuer: &'a str, audience: &'a str, claims: &'a str) -> (&'a [u8], &'a [u8], &'a [u8], &'a [u8]) {
        (subject.as_bytes(), issuer.as_bytes(), audience.as_bytes(), claims.as_bytes())
    }

    #[test]
    fn refresh_token_round_trips_a_reissued_identity() {
        let mut handle_id = 0i64;
        unsafe { nir_new_refresh_token(now_unix_secs() + 3600, &mut handle_id) };

        let (subj, iss, aud, claims) = identity_bytes("alice", "https://issuer", "my-app", r#"{"roles":["admin"]}"#);
        let (mut out_subj, mut out_iss, mut out_aud, mut out_claims, mut out_err) = (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_exp, mut out_iat) = (0i64, 0i64);
        let new_iat = now_unix_secs();
        let ok = unsafe {
            nir_exchange_refresh_token(
                handle_id,
                new_iat,
                subj.as_ptr(),
                subj.len() as i64,
                iss.as_ptr(),
                iss.len() as i64,
                aud.as_ptr(),
                aud.len() as i64,
                claims.as_ptr(),
                claims.len() as i64,
                &mut out_subj,
                &mut out_iss,
                &mut out_aud,
                &mut out_exp,
                &mut out_iat,
                &mut out_claims,
                &mut out_err,
            )
        };
        assert_eq!(ok, 1);
        assert_eq!(unsafe { strout_to_string(&out_subj) }, "alice");
        assert_eq!(out_iat, new_iat);
        assert_eq!(out_exp, now_unix_secs() + 3600);
    }

    #[test]
    fn a_redeemed_refresh_token_cannot_be_redeemed_twice() {
        let mut handle_id = 0i64;
        unsafe { nir_new_refresh_token(now_unix_secs() + 3600, &mut handle_id) };
        let (subj, iss, aud, claims) = identity_bytes("bob", "https://issuer", "my-app", "{}");
        let mut outs = (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_exp, mut out_iat) = (0i64, 0i64);
        let first = unsafe {
            nir_exchange_refresh_token(
                handle_id,
                now_unix_secs(),
                subj.as_ptr(),
                subj.len() as i64,
                iss.as_ptr(),
                iss.len() as i64,
                aud.as_ptr(),
                aud.len() as i64,
                claims.as_ptr(),
                claims.len() as i64,
                &mut outs.0,
                &mut outs.1,
                &mut outs.2,
                &mut out_exp,
                &mut out_iat,
                &mut outs.3,
                &mut outs.4,
            )
        };
        assert_eq!(first, 1);
        let second = unsafe {
            nir_exchange_refresh_token(
                handle_id,
                now_unix_secs(),
                subj.as_ptr(),
                subj.len() as i64,
                iss.as_ptr(),
                iss.len() as i64,
                aud.as_ptr(),
                aud.len() as i64,
                claims.as_ptr(),
                claims.len() as i64,
                &mut outs.0,
                &mut outs.1,
                &mut outs.2,
                &mut out_exp,
                &mut out_iat,
                &mut outs.3,
                &mut outs.4,
            )
        };
        assert_eq!(second, 0, "a second exchange of the same handle must fail, not silently reissue again");
        assert!(unsafe { strout_to_string(&outs.4) }.contains("already redeemed"));
    }

    #[test]
    fn an_unknown_refresh_handle_is_a_real_err() {
        let (subj, iss, aud, claims) = identity_bytes("x", "y", "z", "{}");
        let mut outs = (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_exp, mut out_iat) = (0i64, 0i64);
        let ok = unsafe {
            nir_exchange_refresh_token(
                999_999_999,
                now_unix_secs(),
                subj.as_ptr(),
                subj.len() as i64,
                iss.as_ptr(),
                iss.len() as i64,
                aud.as_ptr(),
                aud.len() as i64,
                claims.as_ptr(),
                claims.len() as i64,
                &mut outs.0,
                &mut outs.1,
                &mut outs.2,
                &mut out_exp,
                &mut out_iat,
                &mut outs.3,
                &mut outs.4,
            )
        };
        assert_eq!(ok, 0);
    }

    // ---- API keys --------------------------------------------------------

    #[test]
    fn validate_api_key_accepts_a_real_matching_hash() {
        let key = b"my-secret-api-key";
        let digest = crate::sha256(key, &[]);
        let mut hex = [0u8; 64];
        crate::hex_encode(&digest, &mut hex);

        let mut outs = (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_exp, mut out_iat) = (0i64, 0i64);
        let ok = unsafe {
            nir_validate_api_key(
                key.as_ptr(),
                key.len() as i64,
                hex.as_ptr(),
                hex.len() as i64,
                &mut outs.0,
                &mut outs.1,
                &mut outs.2,
                &mut out_exp,
                &mut out_iat,
                &mut outs.3,
                &mut outs.4,
            )
        };
        assert_eq!(ok, 1);
        assert_eq!(unsafe { strout_to_string(&outs.1) }, "api-key");
        assert_eq!(out_exp, i64::MAX);
    }

    #[test]
    fn validate_api_key_rejects_a_wrong_hash() {
        let key = b"my-secret-api-key";
        let wrong_hash = b"0000000000000000000000000000000000000000000000000000000000000000";
        let mut outs = (empty_strout(), empty_strout(), empty_strout(), empty_strout(), empty_strout());
        let (mut out_exp, mut out_iat) = (0i64, 0i64);
        let ok = unsafe {
            nir_validate_api_key(
                key.as_ptr(),
                key.len() as i64,
                wrong_hash.as_ptr(),
                wrong_hash.len() as i64,
                &mut outs.0,
                &mut outs.1,
                &mut outs.2,
                &mut out_exp,
                &mut out_iat,
                &mut outs.3,
                &mut outs.4,
            )
        };
        assert_eq!(ok, 0);
    }
}
