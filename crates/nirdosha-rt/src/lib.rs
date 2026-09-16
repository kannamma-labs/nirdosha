//! # The Nirdosha runtime
//!
//! One dependency to depend on: this crate re-exports the
//! [`contract`](macro@contract) attribute macro and provides the
//! enforcement machinery the dialect injects behind your back — role
//! proofs, NFR guards, the flight recorder.
//!
//! To the external world, a program using this crate is just another
//! Rust program: it builds and runs under plain `cargo` with no
//! Nirdosha toolchain present. Under `cargo nirdosha` the *same source*
//! additionally gets its contract claims verified and certified.
//!
//! ```
//! nirdosha_rt::roles! {
//!     HrStaff = "hr_staff";
//! }
//!
//! #[nirdosha_rt::contract(effects(pure), requires(role = "hr_staff"))]
//! fn monthly_total(salaries: &[u64]) -> u64 {
//!     salaries.iter().sum()
//! }
//!
//! fn main() {}
//! ```

pub use nirdosha_macros::contract;

pub mod nfr;
pub mod policy;
pub mod prelude;
#[cfg(feature = "native")]
pub mod native;
pub mod role;

pub use nfr::{enter, events, reset_events, Guard, Limits, NfrEvent};
pub use policy::{crud_forbidden, requires_encryption, Policy};
pub use role::{Auth, AuthError, Role, RoleProof};

/// Declare your application's role vocabulary. Invoke exactly once, at
/// crate root. Each entry declares a marker type plus its wire name;
/// `requires(role = "..")` in a contract resolves to the same-named type
/// here, and a role with no declared type is a compile error when the
/// proof parameter is injected.
///
/// ```
/// nirdosha_rt::roles! {
///     HrStaff = "hr_staff";
///     Manager = "manager";
///     SecurityOps = "security_ops";
/// }
/// ```
///
/// expands to
///
/// ```ignore
/// pub mod nirdosha_roles {
///     pub struct HrStaff;
///     impl nirdosha_rt::Role for HrStaff { const NAME: &'static str = "hr_staff"; }
///     pub struct Manager;
///     impl nirdosha_rt::Role for Manager { const NAME: &'static str = "manager"; }
///     pub struct SecurityOps;
///     impl nirdosha_rt::Role for SecurityOps { const NAME: &'static str = "security_ops"; }
/// }
/// ```
#[macro_export]
macro_rules! roles {
    ( $( $Name:ident = $name:literal ; )+ ) => {
        pub mod nirdosha_roles {
            $(
                pub struct $Name;
                impl ::nirdosha_rt::Role for $Name {
                    const NAME: &'static str = $name;
                }
            )+
        }
    };
}

/// Declare a compliance policy. Invoke at crate root, same convention
/// as [`roles!`]. `policy = "…"` in a contract resolves to the
/// same-named type here, exactly like `requires(role = "…")` does for
/// [`roles!`] — a policy with no declared type is a compile error when
/// the `crud_op` assertion is emitted.
///
/// ```
/// nirdosha_rt::policy! {
///     FinancialUs = "financial_us" {
///         forbids: [delete],
///         encryption: [at_rest, in_transit],
///     }
/// }
/// ```
///
/// expands to
///
/// ```ignore
/// pub mod nirdosha_policies {
///     pub struct FinancialUs;
///     impl nirdosha_rt::Policy for FinancialUs {
///         const NAME: &'static str = "financial_us";
///         const FORBIDDEN_OPS: &'static [&'static str] = &["delete"];
///         const ENCRYPTED_CONCERNS: &'static [&'static str] = &["at_rest", "in_transit"];
///     }
/// }
/// ```
#[macro_export]
macro_rules! policy {
    ( $( $Name:ident = $name:literal {
        forbids: [ $($forbid:ident),* $(,)? ],
        encryption: [ $($enc:ident),* $(,)? ] $(,)?
    } )+ ) => {
        pub mod nirdosha_policies {
            $(
                pub struct $Name;
                impl ::nirdosha_rt::Policy for $Name {
                    const NAME: &'static str = $name;
                    const FORBIDDEN_OPS: &'static [&'static str] = &[$(stringify!($forbid)),*];
                    const ENCRYPTED_CONCERNS: &'static [&'static str] = &[$(stringify!($enc)),*];
                }
            )+
        }
    };
}
