# `nirdosha-guard-macros`

Proc-macro surface for Nirdosha Guard policies and dataset bindings.

The canonical syntax front-end for RFC 0023:
`policy!`, `#[dataset]`, `#[relation]`, `#[classify]`,
`#[materialize]`, `#[reference]`, `#[mask_transform]`, `#[invariant]`,
`#[purpose]`, `audit_sampling!`, `audit_rules!`, `enumerate!`,
`break_glass!`, `approval_chain!`.

Current phase: scaffolding. The macros parse their input and pass it
through unchanged; later phases will emit registry descriptors and lower
the lowerable subset to `AccessPlan`/`WritePlan`.
