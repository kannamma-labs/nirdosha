//! Real bearer-token identity for compiled `serve` — demo mode and
//! production mode both go through the *same* verification path,
//! `nirdosha_runtime_kernels::nir_oidc_validate_token` (real HS256/
//! RS256/ES256 JWT/JWKS signature checking, already compiled into
//! every `nirdosha`-built binary for the `.nir`-facing
//! `oidc_validate_token` builtin — `runtime-kernels/src/lib.rs`).
//! Demo mode never bypasses that check; it just supplies a self-minted
//! `AuthConfig` instead of a real IdP's, so a demo token is only ever
//! trusted because it actually verifies against *some* real JWKS, the
//! same way a production token verifies against the org's real one.
//!
//! This closes the gap the P0 red-team fix (`unverified_bearer_token_json`)
//! deliberately left open and disclosed rather than silently patched:
//! that fix stopped this crate from *lying* about verifying a token;
//! this module is the actual verification the ABI doc always promised.

use nirdosha_runtime_kernels::{nir_dpop_verify, nir_mock_issue_token, nir_oidc_validate_token, NirStrOut};

/// The JWKS/issuer/audience trio every bearer token on this server is
/// checked against — either a real IdP's (production mode, supplied by
/// whoever starts the binary) or [`AuthConfig::demo()`]'s own
/// ephemeral, self-generated one (demo mode, the default — see
/// [`ServeConfig`](crate::ServeConfig)'s own `Default` impl).
#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub jwks_json: String,
    pub issuer: String,
    pub audience: String,
}

impl AuthConfig {
    /// Ephemeral, per-process demo identity: a random 32-byte HMAC
    /// secret drawn from `/dev/urandom` once at construction, never
    /// persisted or logged, wrapped in a synthetic single-key JWKS
    /// (`kty: "oct"` — the only key type [`nir_mock_issue_token`] can
    /// sign with). Restarting the process mints a new secret, which is
    /// exactly why every token issued under it stops verifying the
    /// moment the process that minted it is gone — the same
    /// "ephemeral, never a stand-in for production identity" property
    /// `ui_gen.rs`'s own demo-mode badge already documents to a human.
    /// Recovered from `git show 05a747c~1:crates/compiler/src/serve.rs`
    /// (`AuthConfig::demo()`, deleted alongside the interpreter on
    /// 2026-09-06) as this port's own ground truth, not re-derived.
    pub fn demo() -> AuthConfig {
        use base64::Engine as _;
        let mut buf = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
            .expect("OS entropy source for the ephemeral demo signing key");
        let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
        let jwks_json = serde_json::json!({"keys": [{"kid": "demo", "kty": "oct", "k": secret}]}).to_string();
        AuthConfig { jwks_json, issuer: "nirdosha-demo".to_string(), audience: "nirdosha-demo".to_string() }
    }
}

/// A verified bearer token's full claim set — every field
/// `ast::prelude_structs`' own `VerifiedIdentity` declares (`subject`/
/// `issuer`/`audience`/`expires_at`/`issued_at`/`claims_json`), so a
/// future compiled route wrapper (`rfcs/0010`'s dispatch table, this
/// revival's own Stage 3) can reconstruct one faithfully instead of
/// guessing at defaults for whatever a narrower wire shape left out.
pub struct VerifiedClaims {
    pub subject: String,
    pub issuer: String,
    pub audience: String,
    pub expires_at: i64,
    pub issued_at: i64,
    /// Every claim *except* the six registered OIDC ones above, as a
    /// JSON *object's own text* (e.g. `{"roles":["admin"]}`) — exactly
    /// the shape `nir_check_role`/`nir_extract_claim` already expect
    /// when they read a real `VerifiedIdentity.claims_json` field, so
    /// this needs no reshaping before a route wrapper stores it there
    /// verbatim.
    pub claims_json: String,
}

/// Reads a `{ptr,len}` [`NirStrOut`] back as an owned `String` —
/// `nir_oidc_validate_token`'s own `out_*` convention (leaked, never
/// freed, `NirStrOut`'s own doc comment) means this is always safe to
/// call exactly once per out-param, immediately after the FFI call
/// that populated it.
unsafe fn read_str_out(out: &NirStrOut) -> String {
    if out.ptr.is_null() || out.len <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(out.ptr, out.len as usize) }).into_owned()
}

/// A vendor-neutral port for bearer-token verification: the *whole*
/// authentication surface should be swappable, not just the newer
/// passkey path (`crate::webauthn_crypto::PasskeyCryptoAdapter`) built
/// alongside an otherwise-hardcoded OIDC implementation. `Send + Sync`:
/// shared across connection-handling threads behind an `Arc`, the same
/// way `ServeConfig`'s other adapter fields are.
///
/// Deliberately **not** `#[nirdosha_rt::contract(effects(pure))]`,
/// unlike `PasskeyCryptoAdapter`'s methods: [`RuntimeKernelsOidcVerifier`]'s
/// real implementation genuinely reads the wall clock (the `exp` check
/// `validate_token_against` already documents doing on purpose, at this
/// Rust boundary rather than inside the kernel) — claiming purity here
/// would be exactly the kind of lie `examples/rt-payroll-lying`
/// demonstrates getting caught, not a contract to add just to match a
/// pattern.
pub trait BearerTokenVerifier: Send + Sync {
    fn verify(&self, token: &str, auth: &AuthConfig) -> Result<VerifiedClaims, String>;
}

/// The default, real adapter: today's only implementation, wrapping the
/// existing `nir_oidc_validate_token`-backed verification unchanged.
/// Named for what it actually calls into, not "the" verifier — a future
/// adapter (a different JWT library, a different IdP's own quirks) is a
/// new `impl BearerTokenVerifier`, never a change to
/// [`validate_token`]'s own multi-provider dispatch below.
pub struct RuntimeKernelsOidcVerifier;

impl BearerTokenVerifier for RuntimeKernelsOidcVerifier {
    fn verify(&self, token: &str, auth: &AuthConfig) -> Result<VerifiedClaims, String> {
        validate_token_against(token, auth)
    }
}

/// `ServeConfig::auth`'s real dispatcher (ROADMAP.md A6, "Multi-IdP
/// registry") — the public entry point every caller (`lib.rs::
/// resolve_identity`) uses. One provider (the common case): verified
/// against it directly, no issuer dispatch needed at all. More than one:
/// [`peek_unverified_issuer`] reads `token`'s own *unverified* `iss`
/// claim first (no signature check yet — that's exactly why it's
/// needed, to pick *which* provider's JWKS to verify the signature
/// against) and looks up the matching provider by its own `issuer`; no
/// match is a real, honest `Err` (never a silent fallback to some other
/// provider or to demo mode). `providers` is never empty in practice
/// (`ServeConfig::auth`'s own invariant), but an empty slice still fails
/// cleanly here rather than panicking. `verifier` is
/// `ServeConfig::bearer_verifier`'s trait object — this function no
/// longer calls `validate_token_against` directly, so a caller with a
/// different `BearerTokenVerifier` gets its own logic exercised here
/// too, not just at a call site that forgot to route through it.
pub fn validate_token(token: &str, providers: &[AuthConfig], verifier: &dyn BearerTokenVerifier) -> Result<VerifiedClaims, String> {
    let auth = match providers {
        [] => return Err("invalid token: no identity provider is configured".to_string()),
        [only] => only,
        many => {
            let issuer = peek_unverified_issuer(token)
                .ok_or_else(|| "invalid token: could not read an issuer claim to select an identity provider".to_string())?;
            many.iter()
                .find(|p| p.issuer == issuer)
                .ok_or_else(|| format!("invalid token: issuer {issuer:?} does not match any configured identity provider"))?
        }
    };
    verifier.verify(token, auth)
}

/// `token`'s own `iss` claim, read directly out of its base64url-decoded
/// JWT payload segment — deliberately **not** signature-verified (there
/// is no key to verify against yet; that's the whole reason this exists,
/// to pick one first). Never trusted as a real identity fact on its
/// own — [`validate_token`] only ever uses this to select *which*
/// provider's JWKS to run the real, signature-verifying check against;
/// the actual trust decision still happens entirely inside
/// [`validate_token_against`].
fn peek_unverified_issuer(token: &str) -> Option<String> {
    use base64::Engine as _;
    let payload_b64 = token.split('.').nth(1)?;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("iss").and_then(|v| v.as_str()).map(str::to_string)
}

/// Verifies `token` against `auth`'s JWKS/issuer/audience via the real,
/// compiled `nir_oidc_validate_token` — the exact same check a
/// `.nir` program's own `oidc_validate_token(...)` call compiles down
/// to (`codegen.rs::emit_oidc_validate_token`). Also checks `exp`
/// against the real wall clock, which `nir_oidc_validate_token` itself
/// deliberately never does (it has to stay a pure function of its
/// inputs — see its own doc comment) — this Rust-side HTTP boundary is
/// exactly where a real-clock check belongs instead, the same fix a
/// red-team finding already made for the now-deleted interpreted
/// `serve.rs::resolve_identity` (recovered as this fn's own ground
/// truth), ported here since compiled `serve` needs it just as much.
fn validate_token_against(token: &str, auth: &AuthConfig) -> Result<VerifiedClaims, String> {
    let mut out_subject = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_issuer = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_audience = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_expires_at: i64 = 0;
    let mut out_issued_at: i64 = 0;
    let mut out_claims_json = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_err = NirStrOut { ptr: std::ptr::null(), len: 0 };

    let ok = unsafe {
        nir_oidc_validate_token(
            token.as_ptr(),
            token.len() as i64,
            auth.issuer.as_ptr(),
            auth.issuer.len() as i64,
            auth.audience.as_ptr(),
            auth.audience.len() as i64,
            auth.jwks_json.as_ptr(),
            auth.jwks_json.len() as i64,
            &mut out_subject,
            &mut out_issuer,
            &mut out_audience,
            &mut out_expires_at,
            &mut out_issued_at,
            &mut out_claims_json,
            &mut out_err,
        )
    };
    if ok == 0 {
        return Err(unsafe { read_str_out(&out_err) });
    }
    let claims = VerifiedClaims {
        subject: unsafe { read_str_out(&out_subject) },
        issuer: unsafe { read_str_out(&out_issuer) },
        audience: unsafe { read_str_out(&out_audience) },
        expires_at: out_expires_at,
        issued_at: out_issued_at,
        claims_json: unsafe { read_str_out(&out_claims_json) },
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(i64::MAX);
    if now > claims.expires_at {
        return Err("invalid token: token has expired".to_string());
    }
    Ok(claims)
}

/// Mints a real token for a self-declared `{subject, roles, claims}` —
/// `/api/_demo_login`'s own implementation (`lib.rs::dispatch`), demo
/// mode only. Signs via `auth`'s own JWKS (always [`AuthConfig::demo()`]'s
/// in practice, since this is only ever called when `ServeConfig::demo_mode`
/// is set), so the token this returns round-trips through
/// [`validate_token`] unchanged, the same real verification a production
/// token goes through. `roles`/`claims` merge into the token's own
/// `claims_json` payload exactly as `nir_check_role`/`nir_extract_claim`
/// expect to read them back out later — a top-level `"roles"` array
/// plus flat top-level claim keys.
pub fn mock_issue_token(subject: &str, auth: &AuthConfig, roles: &[String], claims: &[(String, String)]) -> Result<String, String> {
    let mut claims_obj = serde_json::Map::new();
    claims_obj.insert("roles".to_string(), serde_json::json!(roles));
    for (k, v) in claims {
        claims_obj.insert(k.clone(), serde_json::json!(v));
    }
    let claims_json = serde_json::to_string(&serde_json::Value::Object(claims_obj)).expect("built from plain strings, always serializes");

    let mut out_token = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_err = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ok = unsafe {
        nir_mock_issue_token(
            subject.as_ptr(),
            subject.len() as i64,
            auth.issuer.as_ptr(),
            auth.issuer.len() as i64,
            auth.audience.as_ptr(),
            auth.audience.len() as i64,
            now,
            3600,
            claims_json.as_ptr(),
            claims_json.len() as i64,
            auth.jwks_json.as_ptr(),
            auth.jwks_json.len() as i64,
            &mut out_token,
            &mut out_err,
        )
    };
    if ok == 0 {
        return Err(unsafe { read_str_out(&out_err) });
    }
    Ok(unsafe { read_str_out(&out_token) })
}

/// A verified DPoP proof (RFC 9449) -- `lib.rs::resolve_identity`'s own
/// real check, once `ServeConfig::require_sender_constrained_tokens` is
/// set. See `nirdosha_runtime_kernels::nir_dpop_verify`'s own doc
/// comment for the full scope (P-256/ES256 only; replay protection
/// lives in this crate's own `dpop_replay` module, not here, since the
/// kernel stays a pure function of its inputs). `jkt` is real, public
/// API surface even though `lib.rs`'s one caller today doesn't read it
/// (it already passed `expected_jkt` in, so the binding was already
/// checked) -- a future caller minting a fresh access token from a
/// proof it just verified (binding a new token's `cnf.jkt` to a client
/// key it just saw proven) is exactly what this field is for.
#[allow(dead_code)]
pub struct DpopVerified {
    pub jkt: String,
    pub jti: String,
}

/// Verifies `proof` (a raw `DPoP` header value) against this specific
/// request's method/URL and (mandatorily, once this is called at all --
/// see `lib.rs::resolve_identity`'s own reasoning) the access token's
/// `ath` hash and `cnf.jkt` binding, via the real, compiled
/// `nir_dpop_verify`. `now` is the real wall clock, read here (not
/// inside the kernel) for the same reason `validate_token_against`'s own
/// `exp` check reads it here rather than trusting an ambient clock
/// inside a function that must stay pure.
pub fn verify_dpop_proof(proof: &str, method: &str, url: &str, expected_ath: &str, expected_jkt: &str, max_age_secs: i64) -> Result<DpopVerified, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut out_jkt = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_jti = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let mut out_err = NirStrOut { ptr: std::ptr::null(), len: 0 };
    let ok = unsafe {
        nir_dpop_verify(
            proof.as_ptr(),
            proof.len() as i64,
            method.as_ptr(),
            method.len() as i64,
            url.as_ptr(),
            url.len() as i64,
            expected_ath.as_ptr(),
            expected_ath.len() as i64,
            expected_jkt.as_ptr(),
            expected_jkt.len() as i64,
            max_age_secs,
            now,
            &mut out_jkt,
            &mut out_jti,
            &mut out_err,
        )
    };
    if ok == 0 {
        return Err(unsafe { read_str_out(&out_err) });
    }
    Ok(DpopVerified { jkt: unsafe { read_str_out(&out_jkt) }, jti: unsafe { read_str_out(&out_jti) } })
}

/// The `ath` claim's own required value (RFC 9449 §4.3): `base64url(no
/// padding, SHA-256(access_token))` -- the access token's *string form
/// exactly as it appears in the `Authorization` header*, not its
/// decoded claims.
pub fn access_token_hash(token: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// The wire shape `RouteHandler`'s own `identity_json` parameter
/// carries (`lib.rs`'s ABI doc comment) — deliberately keyed by
/// `VerifiedIdentity`'s own real field names (`ast::prelude_structs`:
/// `subject`/`issuer`/`audience`/`expires_at`/`issued_at`/
/// `claims_json`), not the shorter OIDC-standard claim names
/// (`sub`/`iss`/`aud`/`exp`/`iat`) an early sketch of this ABI's doc
/// comment used before real verification existed. Matching the real
/// field names exactly means a `--serve` route wrapper
/// (`codegen.rs`'s Stage 3) can decode this straight into a real
/// `VerifiedIdentity` with the *exact same* generic struct-JSON
/// decoder it already uses for any other struct-typed argument — no
/// identity-specific decode logic needed at all. `claims_json` is
/// embedded as a JSON *string* (escaped, not a nested object) for the
/// same reason: the generic decoder already knows how to decode a
/// plain `str` field, so this needs no special case there either.
pub fn identity_json(claims: &VerifiedClaims) -> String {
    serde_json::json!({
        "subject": claims.subject,
        "issuer": claims.issuer,
        "audience": claims.audience,
        "expires_at": claims.expires_at,
        "issued_at": claims.issued_at,
        "claims_json": claims.claims_json,
    })
    .to_string()
}
