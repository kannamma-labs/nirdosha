//! The v2 MCP tool layer, shared by both of its consumers: `nirdosha-hi
//! mcp` (stdio JSON-RPC, for external agents) and `nirdosha-hi` itself
//! (the embedded console's self-repair loop, in-process -- no
//! subprocess, no wire). One implementation of the capability surface
//! and one call log, not one per transport.
//!
//! Split out of the old `crates/compiler/src/mcp_tools.rs`
//! (2026-09-16) as part of pulling `hi` out of the native compiler
//! crate entirely: this half is exactly the v2-targeted tool bodies
//! (`verify_code`/`get_grammar`/`fix`/`describe`/`certify_code`/
//! `get_nirdosha_constructs`/`get_ui_conventions`), which check real
//! Rust + `nirdosha_rt` macro source via `v2_verify.rs` (`cargo build`
//! + `cargo-nirdosha`), never the native `.nir` grammar. The native
//! verify/fix/certify pipeline (`Certificate`/`run_verify_pipeline`/
//! Z3-backed `VerifyVerdict`/...) that used to live alongside this in
//! the same file stayed behind in `crates/compiler/src/
//! verify_pipeline.rs` -- this crate never depends on the native
//! compiler crate, and it never will again: every "does this compile /
//! describe it / build it" question a v2 candidate raises gets
//! answered here, through `v2_verify.rs`, not by reaching for
//! `crate::token`/`crate::parser`/`crate::ast`.

use crate::hi_graph::sha256_hex;
use serde_json::json;
use std::io::Write as _;

/// The v2 declarative layer's real grammar: `docs/
/// nirdosha-v2-comment-layer.md` §4's spec, plus every comment kind
/// actually shipped in `examples/nirdosha-v2-corpus` today (not a
/// forward-looking wishlist -- `nirdosha:validate`/`workflow`/
/// `screen`/etc. are inert under every checker in this codebase right
/// now; this teaches the *encoding*, not a claim that they're
/// cross-referenced). Hand-maintained the same way `get_ui_conventions`
/// already is (this module's own doc comment on that function explains
/// why: static content, not `include_str!`, so it needs a matching edit
/// if the registry moves) -- update this alongside a new comment kind
/// landing in the corpus.
const NIRDOSHA_V2_COMMENT_GRAMMAR: &str = r#"Every declarative construct rides one or more `/// nirdosha:<kind> {json}` doc-comment lines directly above the Rust item they describe (a struct, a fn). Consecutive `///` lines are joined before parsing, so a multi-line JSON payload is fine. The payload is always exactly one JSON object; unknown keys are a hard error (deny_unknown_fields), the same rule the shipped `nirdosha:contract` encoding already enforces. A single item may carry more than one kind (e.g. `nirdosha:contract` + `nirdosha:transact` on the same fn).

Known kinds, and the shape their JSON object takes (every example below is real, taken from a file in examples/nirdosha-v2-corpus/src that plain cargo builds and runs today):

- nirdosha:contract {"effects":["pure"|"io"|...], "requires":{"role":"..."}, "nfr":{"latency_ms":N,"concurrency_max":N}, "crud":{...}, "logging":{"domain":"...","country":"...","entity":"...","event_class":"..."}} -- the shipped form; also expressible as the `#[nirdosha_rt::contract(effects(pure), requires(role = "..."), nfr(latency_ms = 50, concurrency_max = 1000), logging(domain = "...", country = "...", entity = "..."))]` attribute macro, which additionally injects the unforgeable RoleProof parameter (see get_grammar's `role_gating` field). `logging` resolves the default or `NIRDOSHA_LOGGING_POLICY_PATH` policy register at macro time and injects a runtime guard that scrubs emitted events against that jurisdiction's rules. The ONLY kinds any checker in this codebase (`cargo-nirdosha`) actually validates today are `effects`, `requires`, `nfr`, `crud`, and the syntax of `logging`; the policy field dictionary and dataset field maps are enforced at runtime.

- nirdosha:dataset -- not a doc-comment kind; use the `#[nirdosha_rt::dataset(entity = "transaction", store = "pg_transactions", maps = { cvv = ["_CCV", "card_verification_value"], pin = ["PIN_HASH"] })]` attribute macro on a type/fn/const, or the inline `fields = ["_CCV as cvv", "card_verification_value as cvv"]` spelling. Registers `(entity, concept, physical_path)` tuples into `LOGGING_FIELD_MAPS` so a `#[contract(logging(...))]` guard can resolve policy concepts like `cvv` to real store columns.

- nirdosha:validate {"fn":"name","pre":["expr", ...],"post":["expr", ...]} -- Hoare pre/post clauses over the decorated fn's own parameter names and `result`. One JSON array entry per clause; every `pre` is a conjunctive hypothesis, every `post` is checked independently.

- nirdosha:workflow {"name":"...", "data":{"struct":"StructName","fields":{...}}, "states":{"StateName":{"transitions":{"EventName":"NextState"}, "terminal":true, "on_entry":"fn_name"}, ...}} -- decorates the state-carrying struct; the state-transition functions themselves (e.g. `start_x`/`advance_x`) are ordinary Rust fns nearby, referenced by name, not enumerated in the JSON.

- nirdosha:screen {"for":"StructName", "title":"...", "fields":{"field_name":{"label":"..."}}} -- decorates the struct a generated UI screen is about. Composes with the plain naming-convention inference (`list_/create_/update_/delete_<struct>` fns) -- this comment is additive, never a replacement for it.

- nirdosha:dashboard {"tiles":[...], "charts":[...]} -- each tile/chart names an existing `stat_*`/`chart_*` fn.

- nirdosha:transact {"verify":"fn_name", "compensate":"fn_name"} (+ optional "retry"/"timeout") -- decorates a fn whose real Rust body performs the saga (calls its own verify/commit/compensate fns visibly); the comment is a claim about what that body does, checked declared-vs-actual, never a replacement for the body.

- nirdosha:serve {"routes":[...]} -- serve-surface metadata for a fn exposing an HTTP route.

- nirdosha:field {"struct":"StructName","field":"field_name","requires":{"role":"..."}} -- field-level masking: the named field must be independently gated in the fn body via a hand-written `Option<&RoleProof<R>>` parameter checked by a `mask_unless`-shaped helper (per-field gating is not a macro feature the way fn-level `requires` is).

- nirdosha:schema / nirdosha:role_mapping -- db-table/column and role-to-table-permission convention declarations (46_db_schema_and_role_mapping_conventions.nir).

- nirdosha:visual / nirdosha:workspace / nirdosha:layout / nirdosha:nav -- additional declarative-UI kinds (graph/heatmap/timeline visuals, workspace panels, page layout, module nav grouping) with the same "one JSON object, deny_unknown_fields" shape as the rest.

- nirdosha:audited -- the one `//`-form (not `///`) kind: doc comments cannot attach to a statement inside a fn body, so an audited block is marked `// nirdosha:audited` immediately above it with an `'audited: <label>` loop/block label, not a doc comment on an item.

The registry is extensible: a signed plugin may declare `nirdosha:plugin:<name>:...` kinds later (not shipped yet)."#;

fn require_str_arg<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    arguments.get(name).and_then(|v| v.as_str()).ok_or_else(|| format!("Missing required parameter '{name}'"))
}

/// `verify_code` -- v2 (`docs/nirdosha-v2-comment-layer.md`): runs
/// `v2_verify::verify_v2_source` against inline source (real Rust,
/// checked by the two readers a v2 file is actually held to: `cargo
/// build`, then `cargo-nirdosha`'s in-process contract scanner). There
/// is no Z3/SMT proof pipeline for v2 yet (Phase 4 of the migration
/// doc is still open), so unlike the retired native `VerifyVerdict`
/// this never reports a `Proved`/`Disproved` verdict -- `passed` is
/// exactly "it builds and its `nirdosha:contract` claims are well-
/// formed," nothing stronger claimed.
pub fn verify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    let mut value = serde_json::to_value(&verdict).expect("V2Verdict always serializes");
    value["verdict"] = json!(verdict.verdict());
    Ok(value)
}

/// `get_grammar` -- v2 has no GBNF: a v2 `.nir` file is *valid Rust*,
/// a grammar every model already knows deeply, so there is nothing to
/// constrain-decode against beyond what any Rust-capable model already
/// has. What v2 actually adds is the `nirdosha:*` doc-comment
/// declarative layer (`docs/nirdosha-v2-comment-layer.md` §4) -- the
/// one part of v2 syntax a model has never seen, and the one this tool
/// returns.
pub fn get_grammar(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(json!({
        "format": "v2-comment-layer",
        "base_syntax": "A v2 .nir file is valid, ordinary Rust -- compiled unmodified by plain `cargo`, extension intact. Write normal Rust: `fn`/`struct`/`enum`/`match`, `String`/`&str`, `println!`, semicolons, no bare top-level `return` needed on a trailing expression. Every declarative construct native `.nir` used a keyword for (`workflow`, `screen`, `dashboard`, `transact`, `validate`) is NOT special syntax in v2 -- it rides a `/// nirdosha:<kind> {json}` doc comment on the Rust item it describes instead.",
        "comment_layer_grammar": NIRDOSHA_V2_COMMENT_GRAMMAR,
        "role_gating": "requires(role: \"...\") becomes #[nirdosha_rt::contract(requires(role = \"...\"))] on the fn, after `nirdosha_rt::roles! { Name = \"...\"; }` declares the role once at the crate root. The macro injects an unforgeable `&nirdosha_rt::RoleProof<Name>` as the function's actual (hidden) leading parameter -- callers obtain one via `nirdosha_rt::Auth::login(subject, &roles).prove::<nirdosha_roles::Name>()` and pass it as the first argument.",
    }))
}

/// `fix` -- v2 has no auto-patcher: `write_auto_patches` rewrites
/// diagnostics against this crate's own native `ast::Program`, which a
/// v2 candidate (real Rust, `syn`-parsed) never produces. Rather than
/// fabricate one, `fix` runs the identical `verify_code` check and
/// always reports `applied: []` -- an honest "here is what's wrong,
/// nothing was auto-patched" until a real v2 fixer exists. `apply` is
/// still accepted (and echoed) so an existing caller's request shape
/// doesn't break; it has no effect yet.
pub fn fix(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let apply = arguments.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    let mut before = serde_json::to_value(&verdict).expect("V2Verdict always serializes");
    before["verdict"] = json!(verdict.verdict());
    Ok(json!({
        "before": before,
        "applied": Vec::<String>::new(),
        "apply_requested": apply,
        "note": "no v2 auto-fixer exists yet; nothing is applied. Re-read the diagnostics in `before` and edit the source yourself, then call verify_code again.",
    }))
}

/// `describe` -- v2: parses inline source as plain Rust (`syn`, does
/// not require it to build, the same "still legitimate to inspect"
/// posture the native `describe` took) and returns a curated
/// structural summary via `v2_verify::describe_v2_source`: every
/// fn/struct/enum, plus every `nirdosha:*` doc-comment declaration
/// found and which item it decorates.
pub fn describe(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    crate::v2_verify::describe_v2_source(source)
}

/// `certify_code` -- v2: verifies (`verify_code`'s exact pipeline),
/// then wraps the result in a minimal, honestly-scoped certificate.
/// Deliberately **not** the native `Certificate` shape
/// (`build_certificate`): that struct's `proof_obligations`/
/// `verdict_summary` encode Z3/SMT-proved facts about a native
/// `ast::Program`, and no such proof pipeline exists for v2 source
/// (Phase 4 of `docs/nirdosha-v2-comment-layer.md` is still open) --
/// claiming that shape for v2 would assert a stronger guarantee than
/// was actually checked. `evidence_tier` mirrors `cargo-nirdosha`'s own
/// Stage-1 posture (`nirdosha_contract_core::evidence::Coverage::
/// source_scan`): "source scanned + built", not "proved".
///
/// Deliberately **unsigned**, same reasoning as the native tool: key
/// custody is a human decision that must not cross an agent-callable
/// boundary.
pub fn certify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    Ok(crate::v2_verify::certificate_json(source, &verdict))
}

/// `get_nirdosha_constructs` -- v2 analogue of the native tool: the
/// live, two-reader-verified inventory of v2's major constructs
/// (`crate::v2_capabilities`): `fn`, `struct`, `enum`/`match`,
/// `nirdosha:validate`, `nirdosha:workflow`, `nirdosha:transact`,
/// `nirdosha:screen`+`serve`, `serde_json`, identity+`RoleProof`. Each
/// entry's `source` is a real program just run through `cargo build` +
/// `cargo-nirdosha`'s scanner against *this* build -- not a hand-typed
/// claim that can drift stale, same discipline the native
/// `capabilities.rs` documents for why that drift is a real,
/// previously-observed failure mode.
pub fn get_nirdosha_constructs(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let constructs: Vec<serde_json::Value> = crate::v2_capabilities::run_v2_capability_checks()
        .into_iter()
        .map(|r| json!({ "name": r.name, "supported": r.passed, "example": r.source, "diagnostic": r.diagnostic }))
        .collect();
    Ok(json!({ "constructs": constructs }))
}

/// `get_ui_conventions` -- v2: there is no `ui_gen.rs`-style naming-
/// convention inference over v2/Rust source anywhere in this codebase
/// (that machinery only ever walked the native `ast::Program`) -- so
/// unlike the retired native tool, this isn't teaching an inference
/// rule, it's pointing at the REAL, working v2 UI mechanism: a family
/// of declarative `nirdosha_rt::*!` proc-macros (`crates/
/// nirdosha-macros/src/{crud_screens,dashboard,kanban_board,wizard,
/// settings_screen,communication_feed,categorical}.rs`, RFC 0009 Track
/// C) that expand to ordinary, `rustc`-checked Rust -- a typo'd field
/// or a wrong access level is a real compile error, not a silent
/// no-screen-generated gap the way a naming-convention miss used to be
/// in the native tier. Every invocation shape below is quoted verbatim
/// from that macro's own module doc comment, not paraphrased. The
/// `nirdosha:screen`/`dashboard`/`workspace`/`layout`/`nav`/`visual`
/// comment-layer kinds (`get_grammar`'s `comment_layer_grammar`) are a
/// SEPARATE, currently-inert declarative layer (no checker cross-
/// references them yet anywhere in this codebase) -- prefer the
/// macros below for anything that needs to actually render today.
pub fn get_ui_conventions(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(json!({
        "archetype_macros": [
            {
                "macro": "nirdosha_rt::crud_screens!",
                "archetype": "List/Detail/Create-Form/Edit-Form/Delete-confirmation over one entity",
                "invocation": "nirdosha_rt::crud_screens! {\n    mount: mount_product_screens,\n    entity: Product,\n    store: product_store,\n    path: \"/products\",\n    fields: [ name: String, price_cents: i64, cost_cents: i64 ],\n    create: requires role \"admin\",\n    read: public,\n    update: requires role \"admin\",\n    delete: requires role \"admin\",\n}",
                "requirements": "entity must derive Clone, Default, and serde::Serialize; every field type must implement ParseField (a missing impl is an ordinary rustc trait-bound error).",
                "checked_by": "rustc (compile-time); a typo'd field name is a real \"no such field\" error"
            },
            {
                "macro": "nirdosha_rt::dashboard!",
                "archetype": "Dashboard -- stat tiles + bar charts",
                "invocation": "nirdosha_rt::dashboard! {\n    mount: mount_sales_dashboard,\n    path: \"/dashboard\",\n    title: \"Sales Overview\",\n    refresh_seconds: 300,\n    widgets {\n        Metric { label: \"Revenue MTD\", fn: stat_revenue_mtd, target: 1000000, alert_below: true },\n        Chart  { label: \"By Region\", fn: chart_revenue_by_region, mark: bar },\n    }\n}",
                "requirements": "each widget's fn is called directly in generated code -- a wrong name or signature is an ordinary rustc error, not a runtime surprise.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::kanban_board!",
                "archetype": "Board/Canvas -- drag-and-drop, grouped by one categorical field",
                "invocation": "nirdosha_rt::kanban_board! {\n    mount: mount_status_board,\n    entity: Product,\n    store: product_store,\n    path: \"/board\",\n    access: public,\n    title_field: name,\n    column_field: status,\n    columns: [ \"backlog\", \"in_progress\", \"done\" ],\n    move_path: \"/products/{id}/move/{to}\",\n}",
                "requirements": "presentational only -- a card move POSTs to move_path, typically a route the app wires by hand to a categorical_actions!-generated fn. The board itself carries no authorization logic; the security boundary is that move endpoint.",
                "checked_by": "rustc (compile-time) for the board itself; the move endpoint's own #[contract(requires(role=..))] for authorization"
            },
            {
                "macro": "nirdosha_rt::wizard!",
                "archetype": "Workflow/Wizard -- multi-step form, server-side progress",
                "invocation": "nirdosha_rt::wizard! {\n    mount: mount_onboarding_wizard,\n    entity: Employee,\n    store: employee_store,\n    path: \"/onboarding\",\n    access: public,\n    steps: [\n        { name: \"Basics\", fields: [ name: String, department: String ] },\n        { name: \"Compensation\", fields: [ salary: f64 ] },\n    ],\n}",
                "requirements": "each step's fields are fixed at compile time (one literal route per step); the final step assembles the full entity and inserts it via the same datasource crud_screens! uses.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::settings_screen!",
                "archetype": "Settings/Configuration -- one always-present record, no list, no id, no delete",
                "invocation": "nirdosha_rt::settings_screen! {\n    mount: mount_app_settings,\n    entity: AppSettings,\n    store: app_settings_store,\n    path: \"/settings\",\n    fields: [ site_name: String, maintenance_mode: bool ],\n    access: requires role \"admin\",\n}",
                "requirements": "the datasource is a bare `fn() -> &'static Mutex<Entity>` (exactly one row), not the HashMap<i64, Entity> crud_screens!/wizard! use.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::communication_feed!",
                "archetype": "Communication -- append-only, newest-first message feed",
                "invocation": "nirdosha_rt::communication_feed! {\n    mount: mount_team_feed,\n    entity: Message,\n    store: message_store,\n    path: \"/feed\",\n    fields: [ author: String, body: String ],\n    post_access: requires role \"member\",\n    read_access: public,\n    refresh_seconds: 5,\n}",
                "requirements": "honest client-side polling (a <meta http-equiv=\"refresh\"> tag, dashboard!'s exact mechanism) -- not real server push; the generated routes never pretend otherwise.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::categorical_actions!",
                "archetype": "One role-gated transition function per value of a categorical field",
                "invocation": "nirdosha_rt::categorical_actions! {\n    entity: Product,\n    store: product_store,\n    field: is_approved: bool,\n    actions {\n        true  => approve_product    requires role \"approver\",\n        false => disapprove_product requires role \"compliance_officer\",\n    }\n}",
                "requirements": "generates real #[contract(requires(role = ..))]-gated functions (the only thing that decides access) plus a derived, NON-authoritative role_for_<field> projection for audit logs/UI hints -- never itself consulted for authorization. Leaving a field value uncovered is rustc's own E0004 non-exhaustive-patterns error, not a scanner check.",
                "checked_by": "rustc (compile-time) for both the actions and their exhaustiveness"
            }
        ],
        "role_gating": "Every archetype macro's `access`/`create`/`read`/`update`/`delete`/`post_access`/`read_access` field takes `public` or `requires role \"<name>\"`, expanding to the same #[nirdosha_rt::contract(requires(role = \"...\"))] mechanism get_grammar's `role_gating` field describes -- one unforgeable RoleProof-gated fn per protected action, never a runtime string comparison.",
        "declarative_comment_layer": "A separate, currently-inert layer: nirdosha:screen/dashboard/workspace/layout/nav/visual doc-comment kinds (see get_grammar's comment_layer_grammar) describe the SAME archetypes in JSON form, but nothing in this codebase cross-references them against real code yet (docs/nirdosha-v2-comment-layer.md's own Phase 2, 'emit-ui reads the comment layer,' is still open). Use them to document intent machine-readably; don't rely on them to make anything render -- use the archetype macros above for that.",
        "sources": [
            "crates/nirdosha-macros/src/{crud_screens,dashboard,kanban_board,wizard,settings_screen,communication_feed,categorical}.rs (every invocation above is quoted verbatim from that file's own module doc comment)",
            "rfcs/0009-ui-catalog-extensibility.md (Track C archetypes)",
            "docs/nirdosha-v2-comment-layer.md §3/§4/§6 (the comment-layer kinds' design, and why they're additive/inert today)"
        ]
    }))
}

/// Wraps one tool handler's structured output in the MCP `tools/call`
/// result shape -- `content[0].text` is the same JSON serialized as
/// text (the spec's own documented backward-compatibility rule for
/// `structuredContent`: "a tool that returns structured content SHOULD
/// also return the serialized JSON in a TextContent block"), and
/// `isError` stays `false` here unconditionally: a `DISPROVED`/
/// `UNKNOWN` verdict, or an `Assisted`/`Manual` (unfixable) diagnostic,
/// is a normal, successful, informative answer to the question asked
/// -- not a tool failure. `isError: true` is reserved for
/// `tools_call` never reaching a handler at all (a missing
/// argument or unknown tool name is a *protocol* error instead, per
/// the spec's own two-tier error model -- see `tools_call`).
fn tool_ok(structured: serde_json::Value) -> serde_json::Value {
    let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".to_string());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured,
        "isError": false,
    })
}

/// `tools/list` -- one entry per tool `tools_call` dispatches to:
/// `verify_code`, `get_grammar`, `fix`, `describe`, `certify_code`
/// (parity target: Acutis, Imandra, Kōdo), plus `get_nirdosha_constructs`
/// and `get_ui_conventions` (this project's own additions: a live
/// capability inventory and a curated UI/naming/annotation reference,
/// neither a parity-target tool). Every `inputSchema` is plain JSON
/// Schema, per spec.
pub fn tools_list() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "verify_code",
                "title": "Verify v2 Nirdosha source",
                "description": "Checks inline v2 Nirdosha source (real Rust + nirdosha_rt macros, plus the nirdosha:* doc-comment layer) against the two readers it's actually held to: `cargo build` and `cargo-nirdosha`'s in-process nirdosha:contract scanner. Returns a V2Verdict (builds, build_diagnostic, contracts_found, violations, verdict: clean/violations_found/build_failed) -- there is no Z3/SMT proof pipeline for v2 yet, so unlike the retired native verdict this never reports PROVED/DISPROVED, only whether it builds and its contract claims are well-formed.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "v2 Nirdosha (real Rust) source code to verify" } },
                    "required": ["source"],
                },
            },
            {
                "name": "get_grammar",
                "title": "Get the v2 comment-layer grammar",
                "description": "v2 has no GBNF: a v2 .nir file is valid, ordinary Rust, a grammar every model already knows. Returns the one part of v2 syntax a model hasn't seen -- the nirdosha:* declarative doc-comment layer's real encoding -- plus how requires(role: ...) desugars to a #[nirdosha_rt::contract(...)]-gated RoleProof parameter.",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "name": "fix",
                "title": "Check v2 source for issues (no auto-fixer yet)",
                "description": "Runs the same check as verify_code. There is no v2 auto-patcher yet (the native byte-offset FixPatch machinery only ever understood this crate's own retired native AST, which a v2 candidate -- real Rust, syn-parsed -- never produces): `applied` is always empty. Re-read the diagnostics in `before` and edit the source yourself, then call verify_code again.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "description": "v2 Nirdosha (real Rust) source code to check" },
                        "apply": { "type": "boolean", "description": "Accepted and echoed back as apply_requested for compatibility; nothing is ever applied yet." },
                    },
                    "required": ["source"],
                },
            },
            {
                "name": "describe",
                "title": "Describe a v2 source file's structure",
                "description": "Parses the given source as Rust (via syn; does not require it to build) and returns a curated structural summary: every fn's name/param count, every struct's fields, every enum's variants, and every nirdosha:* doc-comment declaration attached to an item.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "v2 Nirdosha (real Rust) source code to describe" } },
                    "required": ["source"],
                },
            },
            {
                "name": "certify_code",
                "title": "Issue a source-scan certificate for v2 source",
                "description": "Runs the same check as verify_code and wraps the result in a minimal, honestly-scoped certificate (certificate_version: nirdosha.certificate/v2-source-scan) -- source hash, toolchain version, builds/violations, nothing stronger. Deliberately not the native Certificate shape: that shape's proof_obligations/verdict_summary encode Z3/SMT-proved facts about a native ast::Program, and no such proof pipeline exists for v2 yet. Unsigned by design.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "v2 Nirdosha (real Rust) source code to certify" } },
                    "required": ["source"],
                },
            },
            {
                "name": "get_nirdosha_constructs",
                "title": "Get Nirdosha's supported v2 language constructs",
                "description": "Returns a live, compiler-verified inventory of v2's major constructs (fn, struct, enum/match, validate/Hoare contracts via the nirdosha:validate doc comment, ...) -- each one a small program actually run through verify_code's own two-reader pipeline (cargo build + cargo-nirdosha) against this build. Every entry carries a worked source example and whether it currently builds; a failing one also carries the real diagnostic. Call this to discover what v2 actually supports right now, instead of relying on documentation that can drift stale.",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "name": "get_ui_conventions",
                "title": "Get Nirdosha v2's UI archetype macros and role gating",
                "description": "Returns a structured reference for generating a program that actually gets a web UI in v2: (1) the declarative nirdosha_rt::*! archetype macros (crud_screens!, dashboard!, kanban_board!, wizard!, settings_screen!, communication_feed!, categorical_actions!) -- what each expands to and its real invocation shape, quoted from crates/nirdosha-macros's own doc comments; (2) how access/create/read/update/delete fields desugar to #[nirdosha_rt::contract(requires(role = \"...\"))]-gated fns, checked by rustc at compile time, never a runtime string comparison; (3) the separate, currently-inert nirdosha:screen/dashboard/workspace/layout/nav/visual doc-comment kinds that describe the same archetypes in JSON form but aren't cross-referenced against real code yet. Call this before writing a v2 program that's meant to render a UI.",
                "inputSchema": { "type": "object", "properties": {} },
            },
        ],
    })
}

/// `tools/call` -- dispatches `params.name` to one of the seven MCP
/// handlers above with `params.arguments`, logging every call (from
/// either surface) through `log`. A missing `name`, an unknown tool
/// name, or a missing required argument are all *protocol* errors
/// (JSON-RPC `-32602 Invalid params`, matching the phrasing Kōdo's own
/// `missing_param_error` uses) per the spec's own distinction between
/// protocol errors ("Unknown tools", "Invalid arguments") and
/// tool-execution errors (`isError: true` in a successful result) --
/// see `tool_ok`'s doc comment for why every path that reaches a
/// handler at all comes back `isError: false`.
pub fn tools_call(params: &serde_json::Value, log: &mut McpCallLog) -> Result<serde_json::Value, (i64, String)> {
    let call_id = log.next_call_id();
    let started = std::time::Instant::now();
    // A missing `name` keeps the exact protocol error the wire path
    // has always answered with (`tests/mcp_server.rs` asserts it) --
    // logged first so even a malformed call leaves a record.
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        let message = "Missing required parameter 'name'".to_string();
        log.record_call(call_id, "", params, &Err(message.clone()), started.elapsed().as_millis());
        return Err((-32602, message));
    };
    let empty = json!({});
    let arguments = params.get("arguments").unwrap_or(&empty);

    let outcome = match name {
        "verify_code" => verify_code(arguments),
        "get_grammar" => get_grammar(arguments),
        "fix" => fix(arguments),
        "describe" => describe(arguments),
        "certify_code" => certify_code(arguments),
        "get_nirdosha_constructs" => get_nirdosha_constructs(arguments),
        "get_ui_conventions" => get_ui_conventions(arguments),
        other => Err(format!("Unknown tool: {other}")),
    };
    let latency_ms = started.elapsed().as_millis();
    log.record_call(call_id, name, arguments, &outcome, latency_ms);

    outcome
        .map(tool_ok)
        .map_err(|message| (-32602, message))
}

/// The one call log both MCP surfaces share -- newline-delimited JSON
/// appended to a disclosed file, one record per `tools_call` regardless
/// of which surface (`nirdosha mcp`'s stdio wire or `nirdosha hi`'s
/// embedded in-process calls) the call came in on. "Disclosed, not
/// hidden": the same convention hi's session log and
/// `typecheck_and_build_to_temp_file` already follow -- the path is
/// printed by the caller at startup, never only discoverable by
/// knowing where to look. What was called, with what input (hashed +
/// measured, never the full source bloating the log), what came back,
/// from which surface, when, and how long it took: enough to
/// reconstruct any session after the fact without having been watching
/// it live.
pub struct McpCallLog {
    surface: &'static str,
    session: String,
    next_id: u64,
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
}

impl McpCallLog {
    /// Opens (creating if absent) the log file for this process and
    /// surface. Logging is best-effort by design: if the file can't be
    /// opened, the tool layer still works and every call still returns
    /// -- a broken log must never break verification -- only the
    /// record of what happened is lost.
    pub fn new(surface: &'static str) -> Self {
        let path = std::env::temp_dir().join(format!("nirdosha_mcp_{surface}_{}.ndjson", std::process::id()));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        Self { surface, session: format!("{surface}-{}", std::process::id()), next_id: 0, file, path }
    }

    /// The disclosed path the caller should print at startup.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn next_call_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// One session-start record, so the log names its own surface and
    /// protocol context before any call has happened -- `cmd_mcp` and
    /// `nirdosha hi` each write one, making the "where" of every later
    /// record self-evident even in a merged/rotated file.
    pub fn log_session_start(&mut self, details: serde_json::Value) {
        self.append(json!({
            "ts": rfc3339_now(),
            "session": self.session,
            "surface": self.surface,
            "event": "session_start",
            "details": details,
        }));
    }

    fn record_call(
        &mut self,
        call_id: u64,
        tool: &str,
        arguments: &serde_json::Value,
        outcome: &Result<serde_json::Value, String>,
        latency_ms: u128,
    ) {
        // Input metadata: identify the source without dumping it into
        // the log -- byte length + SHA-256 are enough to correlate the
        // record with the caller's own copy of what it sent (and with
        // a certificate's `source_hash`, below).
        let source_meta = arguments.get("source").and_then(|v| v.as_str()).map(|s| {
            json!({ "bytes": s.len(), "sha256": sha256_hex(s.as_bytes()) })
        });
        let apply = arguments.get("apply").and_then(|v| v.as_bool());

        let mut record = json!({
            "ts": rfc3339_now(),
            "session": self.session,
            "surface": self.surface,
            "call_id": call_id,
            "tool": tool,
            "latency_ms": latency_ms,
        });
        if let Some(source_meta) = source_meta {
            record["source"] = source_meta;
        }
        if let Some(apply) = apply {
            record["apply"] = json!(apply);
        }
        match outcome {
            Ok(value) => {
                record["outcome"] = json!("ok");
                // Outcome summary, generic across tools: the verdict a
                // verify/fix/certify carries (nested for fix's before/
                // after, wrapped for certify's verdict_summary), the
                // patch count a fix reports, and the hash a certificate
                // pins -- present when the tool produced one, absent
                // otherwise (get_grammar/describe).
                for (key, pointer) in [
                    ("verdict", "/verdict"),
                    ("verdict", "/before/verdict"),
                    ("verdict", "/after/verdict"),
                    ("verdict", "/verdict_summary/verdict"),
                ] {
                    if let Some(v) = value.pointer(pointer) {
                        record[key] = v.clone();
                        break;
                    }
                }
                if let Some(applied) = value.get("applied").and_then(|v| v.as_array()) {
                    record["patches_applied"] = json!(applied.len());
                }
                if let Some(hash) = value.get("source_hash").and_then(|v| v.as_str()) {
                    record["cert_source_hash"] = json!(hash);
                }
            }
            Err(message) => {
                record["outcome"] = json!("error");
                record["error"] = json!(message);
            }
        }
        self.append(record);
    }

    fn append(&mut self, entry: serde_json::Value) {
        let Some(file) = &mut self.file else { return };
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

/// RFC 3339 UTC timestamp with millisecond precision, hand-rolled
/// against the Unix epoch (days-from-civil, the standard Howard
/// Hinnant form) -- no time/chrono dependency, consistent with this
/// workspace's "no dependency this repo doesn't already need
/// elsewhere" posture. Correct for every date >= 1970-01-01, which is
/// every date a `SystemTime::now()` can ever produce.
fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    )
}

/// Days since 1970-01-01 -> (year, month, day) in the proleptic
/// Gregorian calendar. Howard Hinnant's `civil_from_days`, the
/// standard closed form (see chrono's own `internal.rs` -- same
/// algorithm) with the eras math done in signed arithmetic so every
/// input >= 0 needs no special-casing.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}


#[cfg(test)]
mod tests {
    use super::*;

    use super::*;

    #[test]
    fn tools_list_advertises_all_seven_tools() {
        let list = tools_list();
        let names: Vec<&str> = list["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["verify_code", "get_grammar", "fix", "describe", "certify_code", "get_nirdosha_constructs", "get_ui_conventions"]);
    }

    #[test]
    fn tools_call_unknown_tool_is_a_protocol_error_and_is_logged() {
        let mut log = McpCallLog::new("unit-test-a");
        log.log_session_start(json!({ "purpose": "unit test" }));
        let err = tools_call(&json!({ "name": "no_such_tool", "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("Unknown tool: no_such_tool"));

        let contents = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "session_start + one call record");
        let start: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(start["event"], "session_start");
        let record: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(record["surface"], "unit-test-a");
        assert_eq!(record["tool"], "no_such_tool");
        assert_eq!(record["call_id"], 1);
        assert_eq!(record["outcome"], "error");
        assert!(record["error"].as_str().unwrap().contains("Unknown tool: no_such_tool"));
        assert!(record["ts"].as_str().unwrap().ends_with('Z'));
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn missing_required_argument_is_a_protocol_error_and_is_logged() {
        let mut log = McpCallLog::new("unit-test-b");
        let err = tools_call(&json!({ "name": "verify_code", "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("Missing required parameter 'source'"));

        let contents = std::fs::read_to_string(log.path()).unwrap();
        let record: serde_json::Value = serde_json::from_str(contents.lines().last().unwrap()).unwrap();
        assert_eq!(record["tool"], "verify_code");
        assert_eq!(record["outcome"], "error");
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn missing_name_keeps_its_dedicated_protocol_error() {
        // The wire path has always answered a nameless tools/call with
        // this exact message (`tests/mcp_server.rs` asserts it) -- the
        // relocation must not change observable protocol behavior.
        let mut log = McpCallLog::new("unit-test-b2");
        let err = tools_call(&json!({ "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert_eq!(err.1, "Missing required parameter 'name'");
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn call_ids_are_monotonic_per_session() {
        let mut log = McpCallLog::new("unit-test-c");
        for tool in ["verify_code", "describe"] {
            let _ = tools_call(&json!({ "name": tool, "arguments": {} }), &mut log);
        }
        let contents = std::fs::read_to_string(log.path()).unwrap();
        let ids: Vec<u64> = contents.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["call_id"].as_u64().unwrap()).collect();
        assert_eq!(ids, [1, 2]);
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // Values cross-checked with `date -u -d @$((days * 86400))`, not
        // hand-computed -- the first revision of this test was.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_645), (2026, 7, 11));
        assert_eq!(civil_from_days(20_702), (2026, 9, 6));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1)); // leap-year boundary (2000 IS a leap year)
        assert_eq!(civil_from_days(11_059), (2000, 4, 12)); // post-Feb-29 in a leap year
    }

    #[test]
    fn rfc3339_now_shape() {
        let ts = rfc3339_now();
        // Not asserting the exact instant, just the shape the log's
        // consumers (and any grepping human) rely on.
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[19..20], ".");
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 24);
    }
}
