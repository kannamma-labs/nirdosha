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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The session does not hold the required role.
    MissingRole(&'static str),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::MissingRole(role) => write!(f, "session does not hold role `{role}`"),
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
}

impl Auth {
    /// Establish a session for `user` holding `roles` (wire names).
    pub fn login(user: impl Into<String>, roles: &[&str]) -> Auth {
        Auth {
            user: user.into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
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
}