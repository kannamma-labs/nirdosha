//! Unforgeable role proofs — the `requires(role = "..")` enforcement.
//!
//! The design goal: **uncallable is a property of types, not of
//! tooling.** `RoleProof<R>` has a private constructor; the only way to
//! obtain one is `Auth::prove::<R>()`, which checks the session's real
//! role set. Under plain cargo the check runs at runtime as ordinary
//! code; under `cargo nirdosha` the additional compile-time checks
//! (masking, provenance) are layered on top. No compiler can make a
//! proof-token *more* unforgeable than a private constructor makes it.

use std::fmt;
use std::marker::PhantomData;

/// A role in the application's vocabulary. Declared via
/// [`crate::roles!`].
pub trait Role {
    const NAME: &'static str;
}

/// A claim key/value pair in the application's vocabulary. Declared via
/// [`crate::claims!`] — the `requires(claim = "..", "..")` sibling to
/// [`Role`]/[`crate::roles!`]. Unlike a role (presence alone), holding
/// this claim means the session's own claim set has `NAME` mapped to
/// exactly `VALUE` — `Auth::prove_claim::<C>()` checks both.
pub trait Claim {
    const NAME: &'static str;
    const VALUE: &'static str;
}

/// An unforgeable proof that the authenticated session holds role `R`.
///
/// Cannot be constructed outside this module. The `#[contract]` macro
/// injects a `&RoleProof<R>` parameter for every
/// `requires(role = "…")` claim, so a caller physically cannot invoke
/// the function without a minted proof — under any Rust toolchain.
pub struct RoleProof<R: Role> {
    _priv: (),
    _marker: PhantomData<R>,
}

impl<R: Role> RoleProof<R> {
    pub fn role(&self) -> &'static str {
        R::NAME
    }
}

impl<R: Role> fmt::Debug for RoleProof<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RoleProof<{}>", R::NAME)
    }
}

/// An unforgeable proof that the authenticated session holds claim `C`
/// (i.e. its claim set maps `C::NAME` to exactly `C::VALUE`). The
/// `requires(claim = "..", "..")` sibling to [`RoleProof`] — same
/// private-constructor design, same reasoning: see this module's own
/// top doc comment.
pub struct ClaimProof<C: Claim> {
    _priv: (),
    _marker: PhantomData<C>,
}

impl<C: Claim> ClaimProof<C> {
    pub fn name(&self) -> &'static str {
        C::NAME
    }

    pub fn value(&self) -> &'static str {
        C::VALUE
    }
}

impl<C: Claim> fmt::Debug for ClaimProof<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClaimProof<{}={}>", C::NAME, C::VALUE)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The session does not hold the required role.
    MissingRole(&'static str),
    /// The session's claim set has no `NAME` entry, or `NAME` is
    /// present but mapped to a different value than the one required.
    MissingClaim(&'static str, &'static str),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::MissingRole(role) => write!(f, "session does not hold role `{role}`"),
            AuthError::MissingClaim(name, value) => write!(f, "session does not hold claim `{name}` = `{value}`"),
        }
    }
}

impl std::error::Error for AuthError {}

/// An authenticated session. Stage 1 is deliberately demo-grade: the
/// session's role set is asserted by whoever constructs it (an IdP
/// handshake, a test fixture, …). What matters dialect-wide is that
/// *only* this type can mint proofs, and every mint consults the role
/// set — the proof token is the enforcement, the session is the source
/// of truth. Stage 2 wires real identity (Row-12-style
/// `check_role`) underneath this same interface.
#[derive(Clone)]
pub struct Auth {
    user: String,
    roles: Vec<String>,
    /// `(name, value)` pairs — empty unless `with_claim`/`with_claims`
    /// was used. Kept as a separate, additive builder rather than a
    /// third `login` parameter so every existing `Auth::login(user,
    /// roles)` call site (this crate's own tests included) keeps
    /// compiling unchanged.
    claims: Vec<(String, String)>,
    /// RFC 9449 §4.1's `cnf.jkt` confirmation claim — `None` for an
    /// ordinary bearer token. Set via `with_cnf_jkt`, by whatever real
    /// `authenticate` closure the app supplies after it verifies a real
    /// access token's own claims (this crate has no opinion on token
    /// formats — see `Router`'s own doc comment — so it never reads
    /// this itself; `web::Router::with_sender_constrained_tokens`'s DPoP
    /// check is the one reader). Same additive-builder shape as
    /// `claims`, for the identical reason: `Auth::login`'s signature
    /// never changes.
    cnf_jkt: Option<String>,
}

impl Auth {
    /// Establish a session for `user` holding `roles` (wire names).
    pub fn login(user: impl Into<String>, roles: &[&str]) -> Auth {
        Auth {
            user: user.into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            claims: Vec::new(),
            cnf_jkt: None,
        }
    }

    /// Adds one claim `name = value` to this session — chainable, e.g.
    /// `Auth::login("sita", &["hr_staff"]).with_claim("department",
    /// "cardiology")`.
    pub fn with_claim(mut self, name: impl Into<String>, value: impl Into<String>) -> Auth {
        self.claims.push((name.into(), value.into()));
        self
    }

    /// Same as repeated `with_claim`, from a slice — the shape an
    /// `authenticate` closure reading claims off a real token's own
    /// claim set will usually already have.
    pub fn with_claims(mut self, claims: &[(&str, &str)]) -> Auth {
        self.claims.extend(claims.iter().map(|(n, v)| (n.to_string(), v.to_string())));
        self
    }

    /// Binds this session's access token to `jkt` (RFC 7638, an EC/P-256
    /// JWK thumbprint) — chainable, e.g. an `authenticate` closure
    /// verifying a real sender-constrained token calls
    /// `Auth::login(sub, roles).with_cnf_jkt(claims["cnf"]["jkt"])`.
    pub fn with_cnf_jkt(mut self, jkt: impl Into<String>) -> Auth {
        self.cnf_jkt = Some(jkt.into());
        self
    }

    pub fn cnf_jkt(&self) -> Option<&str> {
        self.cnf_jkt.as_deref()
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// `true` iff this session carries at least one role — i.e. a real
    /// signed-in identity, not the zero-role default an app's
    /// `authenticate` closure typically returns for an anonymous
    /// request (e.g. `Auth::login("anon", &[])`). Distinguishes "any
    /// authenticated human" from "literally anyone, logged in or not",
    /// which a bare `has_role` check can't: an `AllHuman`/`AllRoles`
    /// nav entry should mean the former, not the latter.
    pub fn has_any_role(&self) -> bool {
        !self.roles.is_empty()
    }

    /// `true` iff the session's claim set maps `name` to exactly
    /// `value` — an absent `name`, or `name` present with a different
    /// value, are both `false`. Never a substring/prefix match.
    pub fn has_claim(&self, name: &str, value: &str) -> bool {
        self.claims.iter().any(|(n, v)| n == name && v == value)
    }

    /// Mint a proof for role `R` — the sole constructor of
    /// `RoleProof<R>`. Fails when the session does not hold the role.
    pub fn prove<R: Role>(&self) -> Result<RoleProof<R>, AuthError> {
        if self.has_role(R::NAME) {
            Ok(RoleProof {
                _priv: (),
                _marker: PhantomData,
            })
        } else {
            Err(AuthError::MissingRole(R::NAME))
        }
    }

    /// Mint a proof for claim `C` — the sole constructor of
    /// `ClaimProof<C>`. Fails when the session's claim set doesn't map
    /// `C::NAME` to exactly `C::VALUE`.
    pub fn prove_claim<C: Claim>(&self) -> Result<ClaimProof<C>, AuthError> {
        if self.has_claim(C::NAME, C::VALUE) {
            Ok(ClaimProof {
                _priv: (),
                _marker: PhantomData,
            })
        } else {
            Err(AuthError::MissingClaim(C::NAME, C::VALUE))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HrStaff;
    impl Role for HrStaff {
        const NAME: &'static str = "hr_staff";
    }
    struct Manager;
    impl Role for Manager {
        const NAME: &'static str = "manager";
    }

    fn guarded(_proof: &RoleProof<HrStaff>) -> &'static str {
        "allowed"
    }

    #[test]
    fn proof_mints_only_for_held_roles() {
        let session = Auth::login("sita", &["hr_staff"]);
        let proof = session.prove::<HrStaff>().expect("role held");
        assert_eq!(guarded(&proof), "allowed");
        assert!(matches!(
            session.prove::<Manager>(),
            Err(AuthError::MissingRole("manager"))
        ));
    }

    #[test]
    fn unheld_role_is_uncallable() {
        let outsider = Auth::login("ravana", &["janitor"]);
        assert!(outsider.prove::<HrStaff>().is_err());
    }

    struct Cardiology;
    impl Claim for Cardiology {
        const NAME: &'static str = "department";
        const VALUE: &'static str = "cardiology";
    }
    struct Oncology;
    impl Claim for Oncology {
        const NAME: &'static str = "department";
        const VALUE: &'static str = "oncology";
    }

    fn guarded_by_claim(_proof: &ClaimProof<Cardiology>) -> &'static str {
        "allowed"
    }

    #[test]
    fn claim_proof_mints_only_when_the_session_holds_the_exact_name_and_value() {
        let session = Auth::login("sita", &[]).with_claim("department", "cardiology");
        let proof = session.prove_claim::<Cardiology>().expect("claim held");
        assert_eq!(guarded_by_claim(&proof), "allowed");
        // Same claim *name*, different required *value* -- not a match.
        assert!(matches!(
            session.prove_claim::<Oncology>(),
            Err(AuthError::MissingClaim("department", "oncology"))
        ));
    }

    #[test]
    fn a_session_with_no_claims_at_all_cannot_prove_one() {
        let session = Auth::login("ravana", &["janitor"]);
        assert!(session.prove_claim::<Cardiology>().is_err());
    }

    #[test]
    fn with_claims_from_a_slice_matches_repeated_with_claim() {
        let a = Auth::login("x", &[]).with_claim("department", "cardiology").with_claim("tier", "gold");
        let b = Auth::login("x", &[]).with_claims(&[("department", "cardiology"), ("tier", "gold")]);
        assert!(a.has_claim("department", "cardiology") && b.has_claim("department", "cardiology"));
        assert!(a.has_claim("tier", "gold") && b.has_claim("tier", "gold"));
    }
}