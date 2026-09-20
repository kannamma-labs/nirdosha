//! `nirdosha-guard-macros` — compile-time policy / dataset / relation surface.
//!
//! Syntax front-end for RFC 0023. Emits descriptors into distributed
//! registry slices (`POLICIES`, `CATALOG`, `ROLES`, `APPROVAL_CHAINS`, `WORKFLOWS`, etc.)
//! so `cargo nirdosha verify` can run Gate 2 checks.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    Ident, LitStr, Result, Token,
};

struct PolicyInput {
    effect: Ident,
    id: LitStr,
    subjects: Vec<Ident>,
    actions: Vec<LitStr>,
    resources: Vec<LitStr>,
    purpose: Option<LitStr>,
    clauses: String,
}

/// Parses either `== "value"` (single) or `in ["a", "b", ...]` (list) after
/// a clause keyword (`action`/`resource`) has already been consumed.
/// `in` is a Rust keyword, not an `Ident` — it must be peeked/parsed as
/// `Token![in]`, the same fix `for` needed below.
fn parse_clause_values(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    if input.peek(Token![==]) {
        let _: Token![==] = input.parse()?;
        Ok(vec![input.parse()?])
    } else if input.peek(Token![in]) {
        let _: Token![in] = input.parse()?;
        let content;
        syn::bracketed!(content in input);
        let mut values = Vec::new();
        while !content.is_empty() {
            values.push(content.parse::<LitStr>()?);
            if content.peek(Token![,]) {
                let _: Token![,] = content.parse()?;
            }
        }
        Ok(values)
    } else {
        Err(input.error("expected `==` or `in [...]`"))
    }
}

impl Parse for PolicyInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let effect: Ident = input.parse()?;
        if effect != "allow" && effect != "deny" {
            return Err(syn::Error::new(effect.span(), "expected `allow` or `deny`"));
        }
        let id: LitStr = input.parse()?;

        // Optional `for Role, Role...`. `for` is a Rust keyword, not an
        // `Ident` — peeking `Ident` here never matched it, so this branch
        // was previously dead for every real `for <Role>` policy.
        let mut subjects = Vec::new();
        if input.peek(Token![for]) {
            let _: Token![for] = input.parse()?;
            loop {
                subjects.push(input.parse::<Ident>()?);
                if input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;
                } else {
                    break;
                }
            }
        }

        let when: Ident = input.parse()?;
        if when != "when" {
            return Err(syn::Error::new(when.span(), "expected `when`"));
        }
        let action_name: Ident = input.parse()?;
        if action_name != "action" {
            return Err(syn::Error::new(action_name.span(), "expected `action`"));
        }
        let actions = parse_clause_values(input)?;

        let mut resources = vec![LitStr::new("*", proc_macro2::Span::call_site())];
        let mut purpose = None;

        if input.peek(Token![&&]) {
            let _: Token![&&] = input.parse()?;
            let resource_name: Ident = input.parse()?;
            if resource_name == "resource" {
                resources = parse_clause_values(input)?;
            }
        }

        let mut clauses_tokens = Vec::new();

        // Parse remaining tokens for purpose or details/clauses
        while !input.is_empty() {
            if input.peek(Ident) {
                let kw: Ident = input.parse()?;
                if kw == "purpose" && input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in input);
                    // `content` holds either a string literal (the original
                    // `policy!` convention) or a bare identifier — RTM's
                    // `purpose(FraudMonitoring)` style, an enum-variant-like
                    // reference rather than a wire string. `content.peek`
                    // (not `.parse`) is required here: a *failed* typed
                    // `.parse::<LitStr>()` still records its span in syn's
                    // shared "furthest unexpected token" tracker even when
                    // the `Result::Err` itself is caught and discarded —
                    // and the top-level `syn::parse_str`/`parse2` entry
                    // point surfaces that recorded span as a hard error
                    // regardless of whether `PolicyInput::parse` overall
                    // returns `Ok`. Confirmed by direct reproduction: the
                    // exact same "successfully reached end of parse loop,
                    // returned Ok" path still surfaced as a top-level
                    // "unexpected token" error until the attempt was
                    // changed from parse-and-catch to peek-then-parse.
                    if content.peek(LitStr) {
                        purpose = Some(content.parse::<LitStr>()?);
                    } else {
                        // Bare identifier form: capture as the wire string
                        // (`FraudMonitoring` -> `"FraudMonitoring"`) rather
                        // than silently dropping it — RTM's whole `purpose!`
                        // enum's variant names are exactly these identifiers.
                        let ident: Ident = content.parse()?;
                        purpose = Some(LitStr::new(&ident.to_string(), ident.span()));
                    }
                } else {
                    clauses_tokens.push(kw.to_string());
                }
            } else {
                let tt: proc_macro2::TokenTree = input.parse()?;
                clauses_tokens.push(tt.to_string());
            }
        }

        Ok(Self {
            effect,
            id,
            subjects,
            actions,
            resources,
            purpose,
            clauses: clauses_tokens.join(" "),
        })
    }
}

fn policy_impl(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<PolicyInput>(input) {
        Ok(value) => value,
        Err(error) => return error.into_compile_error().into(),
    };
    let id = parsed.id.value();
    let sanitized_id: String = id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect();
    let effect = if parsed.effect == "allow" {
        quote!(::nirdosha_guard_registry::Effect::Allow)
    } else {
        quote!(::nirdosha_guard_registry::Effect::Deny)
    };
    let id_lit = parsed.id;
    let purpose_expr = match parsed.purpose {
        Some(p) => quote!(Some(#p)),
        None => quote!(None),
    };
    let clauses_expr = if parsed.clauses.is_empty() {
        quote!(None)
    } else {
        let clauses = parsed.clauses;
        quote!(Some(#clauses))
    };
    // `parsed.subjects` are `Ident`s; render each as its own string literal.
    let subject_lits: Vec<LitStr> = parsed
        .subjects
        .iter()
        .map(|ident| LitStr::new(&ident.to_string(), ident.span()))
        .collect();
    let subjects_expr = quote!(&[#(#subject_lits),*]);

    // A policy with `action in [...]` and/or `resource in [...]` is a
    // shorthand for one registration per (action, resource) pair — the
    // downstream IR (`PolicyRecord`/`PolicyCandidate`) is single-action,
    // single-resource by design, so the fan-out happens here, once, at
    // macro-expansion time.
    let mut output = proc_macro2::TokenStream::new();
    for (a_idx, action) in parsed.actions.iter().enumerate() {
        for (r_idx, resource) in parsed.resources.iter().enumerate() {
            let static_name =
                format_ident!("__NIRDOSHA_POLICY_{}_{}_{}", sanitized_id, a_idx, r_idx);
            output.extend(quote! {
                #[used]
                #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::POLICIES)]
                #[linkme(crate = ::nirdosha_guard_registry::linkme)]
                static #static_name: ::nirdosha_guard_registry::PolicyRegistration = ::nirdosha_guard_registry::PolicyRegistration {
                    id: #id_lit,
                    effect: #effect,
                    subjects: #subjects_expr,
                    action: #action,
                    resource: #resource,
                    purpose: #purpose_expr,
                    clauses_json: #clauses_expr,
                    source: module_path!(),
                    line: line!(),
                };
            });
        }
    }
    output.into()
}

/// `policy! { allow "name" when ... }` — canonical policy surface.
#[proc_macro]
pub fn policy(input: TokenStream) -> TokenStream {
    policy_impl(input)
}

/// Alias for `policy!`.
#[proc_macro]
pub fn guard_policy(input: TokenStream) -> TokenStream {
    policy_impl(input)
}

fn attribute_impl(kind: &'static str, args: TokenStream, input: TokenStream) -> TokenStream {
    let item = match syn::parse::<syn::Item>(input.clone()) {
        Ok(item) => item,
        Err(error) => return error.into_compile_error().into(),
    };
    let name = match &item {
        syn::Item::Const(item) => item.ident.clone(),
        syn::Item::Enum(item) => item.ident.clone(),
        syn::Item::Fn(item) => item.sig.ident.clone(),
        syn::Item::Struct(item) => item.ident.clone(),
        syn::Item::Type(item) => item.ident.clone(),
        _ => {
            return syn::Error::new_spanned(item, "guard attribute requires a named item")
                .into_compile_error()
                .into()
        }
    };
    let source = args.to_string();
    let static_name = format_ident!("__NIRDOSHA_{}_{}", kind.to_ascii_uppercase(), name);
    let kind_lit = kind;
    let source_lit = source;
    quote! {
        #item
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: #kind_lit,
            name: stringify!(#name),
            source: #source_lit,
        };
    }
    .into()
}

/// `#[dataset(entity = "...", store = "...")]` — binds an entity to a store.
#[proc_macro_attribute]
pub fn dataset(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("dataset", args, input)
}

/// `#[relation(source = "...", cardinality = N, ttl = ...)]`.
#[proc_macro_attribute]
pub fn relation(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("relation", args, input)
}

/// `#[classify(level = ...)]` — marks a type's classification.
#[proc_macro_attribute]
pub fn classify(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("classify", args, input)
}

/// `#[mask_transform(name = ...)]` — registers a mask transform.
#[proc_macro_attribute]
pub fn mask_transform(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("mask_transform", args, input)
}

/// `#[materialize(relation = ...)]` — Tier 2 materialized column.
#[proc_macro_attribute]
pub fn materialize(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("materialize", args, input)
}

/// `#[reference(field = ..., store = ..., missing = ...)]`.
#[proc_macro_attribute]
pub fn reference(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("reference", args, input)
}

/// `#[invariant(name = ..., schema = ...)]` — pure, schema-typed invariant.
#[proc_macro_attribute]
pub fn invariant(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("invariant", args, input)
}

/// `#[purpose(code = ..., basis = ..., review = ...)]` — tenant-extensible purpose.
#[proc_macro_attribute]
pub fn purpose(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_impl("purpose", args, input)
}

/// Collision-free static name for a `CATALOG`-registered item.
///
/// Every macro below previously used a single hardcoded static name
/// (`__NIRDOSHA_APPROVAL_CHAIN_REG` etc.), so a *second* invocation in the
/// same module was a hard `E0428` "defined multiple times" error — unlike
/// `guard_policy!`, none of them derived a name from their own content.
/// Confirmed by compiling `examples/rtm/roles-N-guard_policy.md` verbatim:
/// it calls `approval_chain!` seven times in one file (`00_core.nir`) and
/// `workflow!` five times in another (`10_domains.nir`); every invocation
/// past the first failed to compile. Hashing the invocation's own token
/// text gives a name that's unique per distinct declaration and stable
/// across rebuilds (no dependence on line/column, which are unstable
/// across nightly toolchains for function-like macros).
fn catalog_static_name(kind: &str, content: &str) -> Ident {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format_ident!("__NIRDOSHA_{}_{:016X}", kind.to_ascii_uppercase(), hasher.finish())
}

/// Best-effort human-readable catalog name: the identifier immediately
/// following `marker` in the token stream (e.g. `chain sar_release` ->
/// `"sar_release"`), falling back to `kind` when the grammar has no
/// reliable leading name token to key off (`break_glass!`, `enumerate!`,
/// the two `audit_*!` macros — none of RTM's real usages give one).
fn extract_named_after(input_str: &str, marker: &str, kind: &'static str) -> String {
    let tokens: Vec<&str> = input_str.split_whitespace().collect();
    tokens
        .iter()
        .position(|t| *t == marker)
        .and_then(|i| tokens.get(i + 1))
        .map(|s| s.to_string())
        .unwrap_or_else(|| kind.to_string())
}

/// `guard_roles! { RoleA, RoleB, ... }`.
#[proc_macro]
pub fn guard_roles(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("ROLES", &input_str);
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "roles",
            name: "guard_roles",
            source: #input_str,
        };
    }
    .into()
}

/// Parses `quorum(N, of = [Role, Role, ...])` out of a `TokenStream`'s own
/// `.to_string()` rendering. Whitespace-insensitive by construction
/// (strips every whitespace character before searching) rather than
/// hardcoding a specific spacing convention: `proc_macro::TokenStream`
/// (what this function actually receives, via `input.to_string()`) and
/// `proc_macro2::TokenStream` were confirmed, by direct compilation while
/// developing this function, to *not* format identically (`proc_macro2`
/// inserts a space before `(` and after `,`; the compiler's own
/// `proc_macro::TokenStream::to_string()` does not) — the whitespace-
/// stripping approach is correct under either, and isn't tied to an
/// internal formatting detail that could shift between compiler versions.
/// Returns `None` if no `quorum(...)` clause is present, or if it has no
/// eligible role in `of = [...]` — Plan Phase 15's own
/// `ApprovalChainRuntime::open` refuses a chain with an empty role list
/// (`ChainHasNoEligibleRoles`) for the identical reason: a chain that can
/// never reach quorum shouldn't be silently registered as usable.
fn parse_quorum_clause(input_str: &str) -> Option<(u8, Vec<String>)> {
    let compact: String = input_str.chars().filter(|c| !c.is_whitespace()).collect();
    let after_marker = compact.find("quorum(")?;
    let args_start = after_marker + "quorum(".len();
    let rest = &compact[args_start..];
    let args_end = rest.find(')')?;
    let args = &rest[..args_end];
    let comma = args.find(',')?;
    let quorum: u8 = args[..comma].parse().ok()?;
    let of_part = &args[comma + 1..];
    let bracket_start = of_part.find('[')?;
    let bracket_end = of_part.find(']')?;
    let roles: Vec<String> = of_part[bracket_start + 1..bracket_end].split(',').map(str::to_string).filter(|role| !role.is_empty()).collect();
    if roles.is_empty() {
        return None;
    }
    Some((quorum, roles))
}

/// `approval_chain! { chain name { quorum(N, of = [Role, ...]); timeout(deny); } }`.
///
/// Plan Phase 15: previously this only registered the block's raw source
/// text into `CATALOG` (useful for `cargo nirdosha verify`'s dump, not for
/// anything that needs the chain's actual quorum/role requirements) — the
/// separately-declared `APPROVAL_CHAINS` slice existed but nothing ever
/// populated it, so `RegistryDump.approval_chains` was silently empty
/// regardless of how many chains a crate declared, and
/// `Decision::Escalate { to: EscalateTarget::Approval { chain } }` had no
/// real chain definition to resolve against
/// (`nirdosha_guard_core::approval_chain::ApprovalChainRuntime`, new this
/// phase). Now emits both: the existing `CatalogRegistration` (unchanged,
/// still feeds the dump/verify path) and a real `ApprovalChainRecord`
/// with the parsed quorum/roles, into `APPROVAL_CHAINS`.
#[proc_macro]
pub fn approval_chain(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("APPROVAL_CHAIN", &input_str);
    let record_static_name = catalog_static_name("APPROVAL_CHAIN_RECORD", &input_str);
    let name = extract_named_after(&input_str, "chain", "approval_chain");
    let catalog_registration = quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "approval_chain",
            name: #name,
            source: #input_str,
        };
    };
    let record_registration = match parse_quorum_clause(&input_str) {
        Some((quorum, approvers)) => quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::APPROVAL_CHAINS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #record_static_name: ::nirdosha_guard_registry::ApprovalChainRegistration = ::nirdosha_guard_registry::ApprovalChainRegistration {
                name: #name,
                quorum: #quorum,
                approvers: &[ #(#approvers),* ],
            };
        },
        // No parseable quorum(...) clause: register the raw catalog entry
        // only, same as before this phase — an honest gap (nothing to
        // resolve a chain with no real quorum requirement against), not a
        // silent zero-quorum record that would trivially "approve" with
        // no approvals at all.
        None => quote! {},
    };
    quote! {
        #catalog_registration
        #record_registration
    }
    .into()
}

/// `workflow! { machine Name { ... } }`.
#[proc_macro]
pub fn workflow(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("WORKFLOW", &input_str);
    let name = extract_named_after(&input_str, "machine", "workflow");
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "workflow",
            name: #name,
            source: #input_str,
        };
    }
    .into()
}

/// `break_glass! { scope(...) ttl(...) dual_approve ... }`.
#[proc_macro]
pub fn break_glass(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("BREAK_GLASS", &input_str);
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "break_glass",
            name: "break_glass",
            source: #input_str,
        };
    }
    .into()
}

/// `audit_sampling! { ... }` — explicit classification → rate table.
#[proc_macro]
pub fn audit_sampling(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("AUDIT_SAMPLING", &input_str);
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "audit_sampling",
            name: "audit_sampling",
            source: #input_str,
        };
    }
    .into()
}

/// `audit_rules! { always_full: [...] }`.
#[proc_macro]
pub fn audit_rules(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("AUDIT_RULES", &input_str);
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "audit_rules",
            name: "audit_rules",
            source: #input_str,
        };
    }
    .into()
}

/// `enumerate! { options from ... }` — scope-injected enumeration.
#[proc_macro]
pub fn enumerate(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("ENUMERATE", &input_str);
    quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "enumerate",
            name: "enumerate",
            source: #input_str,
        };
    }
    .into()
}
