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

pub use nirdosha_macros::{
    app_shell, app_shell_from_toml, approval_inbox, categorical_actions, communication_feed, contract, crud_screens,
    dashboard, kanban_board, landing, login, report_builder, settings_screen, static_embed, tree_view, wizard,
    workflow, workspace,
};

pub use nirdosha_guard_macros::{
    approval_chain, audit_rules, audit_sampling, break_glass, classify, dataset, enumerate,
    invariant, materialize, mask_transform, matcher, mcp_tools, model_artifact,
    policy as guard_policy, purpose, reference, relation, stream_port, window,
};
pub use nirdosha_guard_registry as guard_registry;

// RFC 0026: the metadata plane. The lineage crate itself is re-exported as a
// submodule (types, collector, projection, GraphStore); the macro surface is
// the parse scaffold — expansion lands per the RFC's phasing.
pub use nirdosha_lineage_macros::{
    data_contract, lineage_query, migration_plan, policy_simulation,
};
pub use nirdosha_lineage as lineage;

pub mod approval_inbox;
pub mod audit_projection;
pub mod dashboard;
pub mod export;
pub mod nfr;
pub mod logging_guard;
pub mod policy;
pub mod prelude;
#[cfg(feature = "native")]
pub mod native;
pub mod resource;
pub mod role;
pub mod board;
pub mod feed;
pub mod screens;
pub mod showcase_screens;
pub mod theme;
pub mod watermark;
pub mod web;
pub mod workspace;
pub mod wizard;

pub use nfr::{enter, events, reset_events, Guard, Limits, NfrEvent};
pub use policy::{crud_forbidden, requires_encryption, Policy};
pub use resource::{acquire, release, Resource};
pub use role::{Auth, AuthError, Claim, ClaimProof, Role, RoleProof};
pub use web::{html_escape, page_shell, NavLink, PathParams, Request, Response, Router};
pub use theme::{load as load_theme, themed_page_shell, themed_page_shell_ex, Theme};

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
/// Item muncher backing [`roles!`]. Recurses one item at a time because the
/// body is heterogeneous — plain `Name = "wire_name";`, bare `role Name;`
/// (wire name defaults to `stringify!(Name)`), and `principal Name =
/// "wire_name";` all appear in the same invocation (see RTM's
/// `examples/rtm/roles-N-guard_policy.md` `00_core.nir` for the case that
/// motivated the last two forms) — a single flat repetition pattern can't
/// express "each item is one of three shapes," so each shape gets its own
/// arm, tried in order, with the most specific (literal leading keyword)
/// arms before the generic fallback.
#[doc(hidden)]
#[macro_export]
macro_rules! __nirdosha_roles_item {
    (role $Name:ident ; $($rest:tt)*) => {
        pub struct $Name;
        impl ::nirdosha_rt::Role for $Name {
            const NAME: &'static str = stringify!($Name);
        }
        $crate::__nirdosha_roles_item!($($rest)*);
    };
    (principal $Name:ident = $name:literal ; $($rest:tt)*) => {
        pub struct $Name;
        impl ::nirdosha_rt::Role for $Name {
            const NAME: &'static str = $name;
        }
        $crate::__nirdosha_roles_item!($($rest)*);
    };
    ($Name:ident = $name:literal ; $($rest:tt)*) => {
        pub struct $Name;
        impl ::nirdosha_rt::Role for $Name {
            const NAME: &'static str = $name;
        }
        $crate::__nirdosha_roles_item!($($rest)*);
    };
    () => {};
}

/// ```
/// nirdosha_rt::roles! {
///     HrStaff = "hr_staff";
///     Manager = "manager";
///     SecurityOps = "security_ops";
/// }
/// ```
///
/// also accepts, in the same invocation, bare human roles (wire name
/// defaults to the type name) and explicitly-named service principals —
/// both used throughout RTM-style policy catalogs:
///
/// ```
/// nirdosha_rt::roles! {
///     role Analyst;
///     principal SvcIngest = "spiffe://acme/ns/rtm/sa/ingest";
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
    ( $($body:tt)* ) => {
        pub mod nirdosha_roles {
            $crate::__nirdosha_roles_item!($($body)*);
        }
    };
}

/// Declare your application's claim vocabulary — the `requires(claim =
/// "..", "..")` sibling to [`roles!`]. Invoke exactly once, at crate
/// root. Each entry declares a marker type plus its wire `(name,
/// value)`; `requires(claim = "name", "value")` in a contract resolves
/// to the type whose own `(name, value)` matches exactly — mechanically
/// (`PascalCase(name) ++ PascalCase(value)`, `nirdosha-contract-core::
/// claim::claim_ident`'s own doc comment has the full mapping), never
/// by lookup. A claim with no matching declared type is a compile
/// error when the proof parameter is injected, the same guarantee
/// [`roles!`] already gives a role with no declared type.
///
/// ```
/// nirdosha_rt::claims! {
///     DepartmentCardiology = "department" -> "cardiology";
/// }
/// ```
///
/// expands to
///
/// ```ignore
/// pub mod nirdosha_claims {
///     pub struct DepartmentCardiology;
///     impl nirdosha_rt::Claim for DepartmentCardiology {
///         const NAME: &'static str = "department";
///         const VALUE: &'static str = "cardiology";
///     }
/// }
/// ```
#[macro_export]
macro_rules! claims {
    ( $( $Name:ident = $name:literal -> $value:literal ; )+ ) => {
        pub mod nirdosha_claims {
            $(
                pub struct $Name;
                impl ::nirdosha_rt::Claim for $Name {
                    const NAME: &'static str = $name;
                    const VALUE: &'static str = $value;
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
