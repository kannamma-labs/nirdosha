//! `nirdosha-guard-macros` — compile-time policy / dataset / relation surface.
//!
//! Syntax front-end for RFC 0023. Emits descriptors into distributed
//! registry slices (`POLICIES`, `CATALOG`, `ROLES`, `APPROVAL_CHAINS`, `WORKFLOWS`, etc.)
//! so `cargo nirdosha verify` can run Gate 2 checks.

use proc_macro::TokenStream;
use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
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

/// `#[dataset(entity = "...", store = "...", maps = { cvv = ["_CCV"], pin = ["PIN_HASH"] })]`.
/// Registers the entity into the Guard catalog and also emits one
/// `LOGGING_FIELD_MAPS` static per `(concept, physical_path)` pair, so
/// the logging policy guard can resolve canonical compliance field names
/// (e.g. `cvv`) to the actual store column names used by this dataset.
#[proc_macro_attribute]
pub fn dataset(args: TokenStream, input: TokenStream) -> TokenStream {
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
            return syn::Error::new_spanned(item, "#[dataset] requires a named item")
                .into_compile_error()
                .into()
        }
    };
    let source = args.to_string();
    let maps = parse_dataset_maps(&source);
    let entity_owned = maps.entity.unwrap_or_else(|| name.to_string());
    let entity = entity_owned.as_str();
    let entity_lit = syn::LitStr::new(entity, proc_macro2::Span::call_site());

    let static_name = format_ident!("__NIRDOSHA_DATASET_{}", name.to_string().to_ascii_uppercase());
    let source_lit = syn::LitStr::new(&source, proc_macro2::Span::call_site());
    let kind_lit = syn::LitStr::new("dataset", proc_macro2::Span::call_site());
    let name_lit = syn::LitStr::new(&name.to_string(), proc_macro2::Span::call_site());

    let mut output = quote! {
        #item
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: #kind_lit,
            name: #name_lit,
            source: #source_lit,
        };
    };

    for (concept, physicals) in &maps.concepts {
        let concept_lit = syn::LitStr::new(concept, proc_macro2::Span::call_site());
        for (idx, physical) in physicals.iter().enumerate() {
            let physical_tokens = vec![syn::LitStr::new(physical, proc_macro2::Span::call_site())];
            let hash = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                entity.hash(&mut hasher);
                concept.hash(&mut hasher);
                physical.hash(&mut hasher);
                idx.hash(&mut hasher);
                hasher.finish()
            };
            let map_static_name = format_ident!("__NIRDOSHA_LOG_FIELD_MAP_{:016X}", hash);
            output.extend(quote! {
                #[used]
                #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::LOGGING_FIELD_MAPS)]
                #[linkme(crate = ::nirdosha_guard_registry::linkme)]
                static #map_static_name: ::nirdosha_guard_registry::FieldMapRegistration = ::nirdosha_guard_registry::FieldMapRegistration {
                    entity: #entity_lit,
                    concept: #concept_lit,
                    physical: &[#(#physical_tokens),*],
                };
            });
        }
    }

    output.into()
}

#[derive(Debug, Default)]
struct DatasetMaps {
    entity: Option<String>,
    concepts: Vec<(String, Vec<String>)>,
}

/// Parse `maps = { cvv = ["_CCV", "card_verification_value"], pin = ["PIN_HASH"] }`
/// plus `entity = "..."` from the attribute token string. Also supports the
/// `fields = [ "_CCV as cvv", "card_verification_value as cvv" ]` spelling.
fn parse_dataset_maps(source: &str) -> DatasetMaps {
    let mut out = DatasetMaps::default();

    // entity = "..."
    if let Some((_key, value)) = find_key_value(source, "entity") {
        if let Some(v) = find_quoted(&value) {
            out.entity = Some(v);
        }
    }

    // maps = { concept = ["a", "b"], concept2 = ["c"] }
    if let Some(body) = find_block_after_key(source, "maps", '{') {
        for (concept, list_body) in split_key_value_list(&body) {
            let physicals: Vec<String> =
                list_body.split(',').filter_map(|s| find_quoted(s.trim())).collect();
            if !physicals.is_empty() {
                out.concepts.push((concept, physicals));
            }
        }
    }

    // fields = [ "_CCV as cvv", "PAN as pan" ]
    if let Some(body) = find_block_after_key(source, "fields", '[') {
        for item in body.split(',') {
            let item = item.trim();
            if let Some((physical, concept)) = parse_as_alias(item) {
                out.concepts.push((concept, vec![physical]));
            }
        }
    }

    out
}

/// Find `key = ...` returning the raw text after `=` (trimmed).
fn find_key_value(source: &str, key: &str) -> Option<(String, String)> {
    let pattern = format!("{} =", key);
    let start = source.find(&pattern)?;
    let after = &source[start + pattern.len()..];
    let end = after.find(',').or_else(|| Some(after.len())).unwrap();
    let value = &after[..end];
    Some((key.to_string(), value.trim().to_string()))
}

/// Find `key = <delim>...<matching delim>` and return the inner body.
fn find_block_after_key(source: &str, key: &str, delim: char) -> Option<String> {
    let pattern = format!("{} =", key);
    let start = source.find(&pattern)?;
    let after = &source[start + pattern.len()..];
    let open = after.find(delim)?;
    let body_start = open + 1;
    let mut depth = 1;
    for (i, c) in after[body_start..].char_indices() {
        match c {
            c if c == delim => depth += 1,
            '}' if delim == '{' => depth -= 1,
            ']' if delim == '[' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(after[body_start..body_start + i].to_string());
        }
    }
    None
}

fn split_key_value_list(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in body.char_indices() {
        match c {
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                if let Some((k, v)) = split_key_value(&body[start..i]) {
                    out.push((k, v));
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some((k, v)) = split_key_value(&body[start..]) {
        out.push((k, v));
    }
    out
}

fn split_key_value(pair: &str) -> Option<(String, String)> {
    let pair = pair.trim();
    let eq = pair.find('=')?;
    let key = pair[..eq].trim().to_string();
    let value = pair[eq + 1..].trim().to_string();
    Some((key, value))
}

fn find_quoted(s: &str) -> Option<String> {
    let open = s.find('"')?;
    let rest = &s[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

fn parse_as_alias(item: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = item.split_whitespace().collect();
    if parts.len() == 3 && parts[1] == "as" {
        Some((strip_quotes(parts[0]).to_string(), strip_quotes(parts[2]).to_string()))
    } else {
        None
    }
}

fn strip_quotes(s: &str) -> &str {
    s.trim_matches('"')
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

/// Parses an optional `cooling(days = N)` / `cooling(hours = N)` /
/// `cooling(minutes = N)` / `cooling(seconds = N)` clause into
/// milliseconds. Same whitespace-insensitive substring approach as
/// `parse_quorum_clause`. Returns `0` (no cooling) if the clause is
/// absent — every existing chain declared before this clause existed
/// keeps its immediate-on-quorum behavior unchanged.
fn parse_cooling_clause(input_str: &str) -> u64 {
    let compact: String = input_str.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(after_marker) = compact.find("cooling(") else { return 0 };
    let args_start = after_marker + "cooling(".len();
    let rest = &compact[args_start..];
    let Some(args_end) = rest.find(')') else { return 0 };
    let args = &rest[..args_end];
    let Some(eq) = args.find('=') else { return 0 };
    let unit = &args[..eq];
    let Ok(n) = args[eq + 1..].parse::<u64>() else { return 0 };
    match unit {
        "days" => n * 24 * 60 * 60 * 1000,
        "hours" => n * 60 * 60 * 1000,
        "minutes" => n * 60 * 1000,
        "seconds" => n * 1000,
        "ms" => n,
        _ => 0,
    }
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
    let cooling_ms = parse_cooling_clause(&input_str);
    let record_registration = match parse_quorum_clause(&input_str) {
        Some((quorum, approvers)) => quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::APPROVAL_CHAINS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #record_static_name: ::nirdosha_guard_registry::ApprovalChainRegistration = ::nirdosha_guard_registry::ApprovalChainRegistration {
                name: #name,
                quorum: #quorum,
                approvers: &[ #(#approvers),* ],
                cooling_ms: #cooling_ms,
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

/// Parses a `workflow!` machine body's `from -> to -> [to, to, ...];`
/// statements into a flat edge list. Grammar: each `;`-terminated
/// statement is a chain of `->`-separated segments; every segment but
/// possibly the last is a bare state ident, and any segment may instead
/// be a bracketed list of idents (fanning the previous segment's states
/// out to each list member) — `a -> b -> [c, d]` yields edges
/// `(a,b), (b,c), (b,d)`. Consecutive list segments chain generally: the
/// next segment's edges originate from *every* ident the prior segment
/// produced, not just the first.
fn parse_workflow_edges(body_str: &str) -> Vec<(String, String)> {
    let mut edges = Vec::new();
    for stmt in split_top_level(body_str, ';') {
        if stmt.is_empty() {
            continue;
        }
        let segments = split_arrow_top_level(&stmt);
        if segments.len() < 2 {
            continue;
        }
        let mut prev: Vec<String> = parse_state_segment(&segments[0]);
        for seg in &segments[1..] {
            let current = parse_state_segment(seg);
            for from in &prev {
                for to in &current {
                    edges.push((from.clone(), to.clone()));
                }
            }
            prev = current;
        }
    }
    edges
}

/// A single `->`-chain segment: either a bare ident (`open`) or a
/// bracketed list (`[confirmed_fraud, false_positive, escalate]`).
fn parse_state_segment(seg: &str) -> Vec<String> {
    if let Some(inner) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        split_top_level(inner, ',').into_iter().filter(|s| !s.is_empty()).collect()
    } else if seg.is_empty() {
        Vec::new()
    } else {
        vec![seg.to_string()]
    }
}

/// Splits on top-level `->` occurrences (depth 0 across `()[]{}`) — like
/// `split_top_level`, but for the two-character `->` delimiter that
/// delimiter's single-`char` signature can't take.
fn split_arrow_top_level(s: &str) -> Vec<String> {
    let mut depth = 0i32;
    let mut items = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
                i += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
                i += 1;
            }
            '-' if depth == 0 && chars.get(i + 1) == Some(&'>') => {
                items.push(std::mem::take(&mut current));
                i += 2;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

/// `workflow! { machine Name { ... } }`. Emits the existing free-text
/// `CatalogRegistration` (unchanged, still feeds `cargo nirdosha verify`'s
/// dump) plus — new for T-14 — a real `WorkflowRegistration` with the
/// machine's parsed transition graph into `WORKFLOWS`, the same
/// "register both, dual-slice" fix Plan Phase 15 already established for
/// `approval_chain!`/`APPROVAL_CHAINS`.
#[proc_macro]
pub fn workflow(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let static_name = catalog_static_name("WORKFLOW", &input_str);
    let name = extract_named_after(&input_str, "machine", "workflow");
    let catalog_registration = quote! {
        #[used]
        #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::CATALOG)]
        #[linkme(crate = ::nirdosha_guard_registry::linkme)]
        static #static_name: ::nirdosha_guard_registry::CatalogRegistration = ::nirdosha_guard_registry::CatalogRegistration {
            kind: "workflow",
            name: #name,
            source: #input_str,
        };
    };

    let tokens: TokenStream2 = input.into();
    let blocks = top_level_named_blocks(tokens, "machine");
    let edges: Vec<(String, String)> = blocks
        .first()
        .map(|(_, body)| parse_workflow_edges(&compact(&body.to_string())))
        .unwrap_or_default();

    // No parseable edges: register the raw catalog entry only, same
    // honest-gap posture `approval_chain!` takes for a chain with no
    // parseable `quorum(...)` — never a silent empty-but-present record.
    let record_registration = if edges.is_empty() {
        quote! {}
    } else {
        let record_static_name = catalog_static_name("WORKFLOW_RECORD", &input_str);
        let edge_tokens = edges.iter().map(|(from, to)| {
            let from_lit = LitStr::new(from, proc_macro2::Span::call_site());
            let to_lit = LitStr::new(to, proc_macro2::Span::call_site());
            quote! { (#from_lit, #to_lit) }
        });
        quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::WORKFLOWS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #record_static_name: ::nirdosha_guard_registry::WorkflowRegistration = ::nirdosha_guard_registry::WorkflowRegistration {
                name: #name,
                edges: &[ #(#edge_tokens),* ],
            };
        }
    };

    quote! {
        #catalog_registration
        #record_registration
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

// ============================================================================
// RFC 0025 §8.2–8.4/§8.8 — feature windows, model artifacts, screening
// matchers, stream ports, MCP tool servers, and the closed purpose taxonomy.
//
// Parsing approach mirrors `approval_chain!`/`workflow!` above (whitespace-
// stripped `.to_string()` + targeted substring/bracket extraction), not a
// from-scratch `syn::parse::Parse` grammar tree — the same technique already
// accepted there as "real, structured" (see `parse_quorum_clause`). The one
// addition these six need that `approval_chain!`/`workflow!` didn't: several
// of these grammars nest one named/braced block inside another (`server X {
// .. delegation { .. } defaults { .. } }`), and repeat a named block more
// than once per invocation (`port a { .. } port b { .. }`) — plain string
// search over a flattened token string would mis-split those (it can't tell
// a nested `}` from the outer one), so block *splitting* walks the real
// `TokenTree` list tracking brace grouping; only each leaf block's own flat
// `key = value;` lines still go through string extraction.
// ============================================================================

/// Strips all whitespace — safe for these six grammars specifically because
/// none of their string literals contain internal spaces (checked against
/// every real usage in `examples/rtm/roles-N-guard_policy.md`), the same
/// precondition `parse_quorum_clause` already relies on.
fn compact(input_str: &str) -> String {
    input_str.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Index of the bracket matching the opener at `open_idx` (which must point
/// at one of `([{`), tracking nesting depth across all three bracket kinds
/// so a value containing its own brackets (e.g. `outputs = [score: f64,
/// explanation: vec[string]]`) doesn't truncate at the first inner close.
fn matching_bracket(s: &str, open_idx: usize) -> Option<usize> {
    let open = s[open_idx..].chars().next()?;
    let close = match open {
        '[' => ']',
        '(' => ')',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0i32;
    for (i, ch) in s.char_indices().skip(open_idx) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Splits `s` on top-level occurrences of `delim` only (depth 0 across
/// `()[]{}`) — for list items or statements that may contain their own
/// nested brackets (`sum(amount)`, `query_records(A, B, C)`).
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut depth = 0i32;
    let mut items = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            c if c == delim && depth == 0 => {
                items.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

/// `key="literal";` -> the literal's content, unquoted.
fn extract_str_field(compact: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = compact.find(&marker)? + marker.len();
    let end = compact[start..].find('"')? + start;
    Some(compact[start..end].to_string())
}

/// `key"literal";` -> the literal's content, unquoted. For the two
/// `stream_port!` clauses that put the string right after the keyword with
/// no `=` (`bind "card_network.rails";` / `publish "txn.authorized";`),
/// unlike every other clause in these six grammars.
fn extract_str_field_no_eq(compact: &str, key: &str) -> Option<String> {
    let marker = format!("{key}\"");
    let start = compact.find(&marker)? + marker.len();
    let end = compact[start..].find('"')? + start;
    Some(compact[start..end].to_string())
}

/// `key=<raw tokens>;` -> everything up to the next top-level `;`, verbatim.
/// Covers every non-string, non-list scalar clause these grammars have —
/// plain idents (`format=onnx`), numbers (`threshold=0.92`), and mixed-token
/// values with no shared shape (`ttl=30m`, `rate=20/min`).
fn extract_raw_field(compact: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let start = compact.find(&marker)? + marker.len();
    let rest = &compact[start..];
    let end = split_top_level(rest, ';').into_iter().next()?;
    if end.is_empty() || end.starts_with('"') || end.starts_with('[') {
        return None;
    }
    Some(end)
}

/// `key=[a, b, c];` -> the bracket-matched, top-level-comma-split item list.
fn extract_list_field(compact: &str, key: &str) -> Option<Vec<String>> {
    let marker = format!("{key}=[");
    let bracket_start = compact.find(&marker)? + marker.len() - 1;
    let bracket_end = matching_bracket(compact, bracket_start)?;
    let inner = &compact[bracket_start + 1..bracket_end];
    if inner.is_empty() {
        return Some(Vec::new());
    }
    Some(split_top_level(inner, ','))
}

/// Splits `keyword <name> { .. }` blocks out of a token stream's top level
/// (repeated: `port a { .. } port b { .. }`), correctly skipping over each
/// block's own internal braces since it walks real `TokenTree`s rather than
/// searching flattened text.
fn top_level_named_blocks(input: TokenStream2, keyword: &str) -> Vec<(String, TokenStream2)> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            if ident == keyword {
                if let (Some(TokenTree::Ident(name)), Some(TokenTree::Group(group))) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                {
                    if group.delimiter() == Delimiter::Brace {
                        blocks.push((name.to_string(), group.stream()));
                        i += 3;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    blocks
}

/// Finds an unnamed `keyword { .. }` block (no name identifier in between —
/// `mcp_tools!`'s nested `delegation { .. }`/`defaults { .. }`) among a
/// flat `TokenTree` list.
fn find_labeled_block(tokens: &[TokenTree], keyword: &str) -> Option<TokenStream2> {
    for i in 0..tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            if ident == keyword {
                if let Some(TokenTree::Group(group)) = tokens.get(i + 1) {
                    if group.delimiter() == Delimiter::Brace {
                        return Some(group.stream());
                    }
                }
            }
        }
    }
    None
}

fn opt_str_lit(value: &Option<String>) -> proc_macro2::TokenStream {
    match value {
        Some(v) => {
            let lit = LitStr::new(v, proc_macro2::Span::call_site());
            quote!(Some(#lit))
        }
        None => quote!(None),
    }
}

fn str_lits(values: &[String]) -> Vec<LitStr> {
    values.iter().map(|v| LitStr::new(v, proc_macro2::Span::call_site())).collect()
}

/// `purpose_taxonomy! { enum Purpose { Operations, FraudMonitoring, ... } }`
/// — RTM's closed purpose taxonomy (RFC 0025 §1B/§17). Named
/// `purpose_taxonomy`, not `purpose`: that name is already taken by the
/// existing `#[proc_macro_attribute] purpose` (a real, load-bearing,
/// different-grammar macro — `#[nirdosha_rt::purpose(code = ..., basis =
/// ..., review = ...)]`, used by `crates/nirdosha-rt/tests/guard_attributes.rs`).
/// Called fully-qualified as `nirdosha_guard_macros::purpose_taxonomy!` for
/// the identical reason `nirdosha_guard_macros::workflow!` must be
/// fully-qualified: `nirdosha_rt` already re-exports a same-named, unrelated
/// macro (there, the UI screen-flow `workflow!`; here, the attribute
/// `purpose!`) under the plain name.
///
/// The input is literally a valid `enum` item, so this parses it with real
/// `syn`, not string extraction — unlike the other five macros in this
/// section, there's no non-standard grammar here to work around. Emits the
/// real enum (so application code can reference e.g. `Purpose::FraudMonitoring`)
/// plus one `PurposeRegistration` per variant into `PURPOSES`.
#[proc_macro]
pub fn purpose_taxonomy(input: TokenStream) -> TokenStream {
    let item_enum = match syn::parse::<syn::ItemEnum>(input) {
        Ok(item) => item,
        Err(error) => return error.into_compile_error().into(),
    };
    let mut registrations = proc_macro2::TokenStream::new();
    for variant in &item_enum.variants {
        let variant_ident = &variant.ident;
        let code_lit = LitStr::new(&variant_ident.to_string(), variant_ident.span());
        let static_name = format_ident!("__NIRDOSHA_PURPOSE_{}", variant_ident);
        registrations.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::PURPOSES)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::PurposeRegistration =
                ::nirdosha_guard_registry::PurposeRegistration { code: #code_lit };
        });
    }
    quote! {
        #item_enum
        #registrations
    }
    .into()
}

/// `stream_port! { port <name> { bind "..." | publish "..."; [semantics = ident;] [format = "..";] [schema = "..";] } ... }`
/// (RFC 0025 §8.1/§6.5), one or more ports per invocation.
#[proc_macro]
pub fn stream_port(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let blocks = top_level_named_blocks(tokens, "port");
    if blocks.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "stream_port! expects at least one `port <name> { .. }` block",
        )
        .into_compile_error()
        .into();
    }
    let mut output = proc_macro2::TokenStream::new();
    for (name, body) in blocks {
        let body_str = compact(&body.to_string());
        let (direction, target) = match extract_str_field_no_eq(&body_str, "bind") {
            Some(t) => ("bind", t),
            None => match extract_str_field_no_eq(&body_str, "publish") {
                Some(t) => ("publish", t),
                None => {
                    return syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("port `{name}` needs a `bind \"...\"` or `publish \"...\"` clause"),
                    )
                    .into_compile_error()
                    .into();
                }
            },
        };
        let format = extract_str_field(&body_str, "format");
        let schema = extract_str_field(&body_str, "schema");
        let semantics = extract_raw_field(&body_str, "semantics");
        let static_name = catalog_static_name("PORT", &format!("{name}|{body_str}"));
        let name_lit = LitStr::new(&name, proc_macro2::Span::call_site());
        let direction_lit = LitStr::new(direction, proc_macro2::Span::call_site());
        let target_lit = LitStr::new(&target, proc_macro2::Span::call_site());
        let format_expr = opt_str_lit(&format);
        let schema_expr = opt_str_lit(&schema);
        let semantics_expr = opt_str_lit(&semantics);
        output.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::PORTS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::PortRegistration = ::nirdosha_guard_registry::PortRegistration {
                name: #name_lit,
                direction: #direction_lit,
                target: #target_lit,
                format: #format_expr,
                schema: #schema_expr,
                semantics: #semantics_expr,
            };
        });
    }
    output.into()
}

/// `model_artifact! { model <name> { format = ident; inputs = [ident, ...]; outputs = [ident (: type)?, ...]; threshold_alert = <float>; } ... }`
/// (RFC 0025 §8.3), one or more models per invocation.
#[proc_macro]
pub fn model_artifact(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let blocks = top_level_named_blocks(tokens, "model");
    if blocks.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "model_artifact! expects at least one `model <name> { .. }` block",
        )
        .into_compile_error()
        .into();
    }
    let mut output = proc_macro2::TokenStream::new();
    for (name, body) in blocks {
        let body_str = compact(&body.to_string());
        let format = extract_raw_field(&body_str, "format").unwrap_or_default();
        let inputs = extract_list_field(&body_str, "inputs").unwrap_or_default();
        let outputs = extract_list_field(&body_str, "outputs").unwrap_or_default();
        let threshold_alert = extract_raw_field(&body_str, "threshold_alert").and_then(|s| s.parse::<f64>().ok());
        let static_name = catalog_static_name("MODEL", &format!("{name}|{body_str}"));
        let name_lit = LitStr::new(&name, proc_macro2::Span::call_site());
        let version_lit = LitStr::new("", proc_macro2::Span::call_site());
        let format_lit = LitStr::new(&format, proc_macro2::Span::call_site());
        let input_lits = str_lits(&inputs);
        let output_lits = str_lits(&outputs);
        let threshold_expr = match threshold_alert {
            Some(v) => quote!(Some(#v)),
            None => quote!(None),
        };
        output.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::MODELS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::ModelRegistration = ::nirdosha_guard_registry::ModelRegistration {
                name: #name_lit,
                version: #version_lit,
                format: #format_lit,
                inputs: &[#(#input_lits),*],
                outputs: &[#(#output_lits),*],
                threshold_alert: #threshold_expr,
            };
        });
    }
    output.into()
}

/// `matcher! { matcher <name> { algorithm = ident; threshold = <float>; lists = [ident, ...]; } ... }`
/// (RFC 0025 §8.4), one or more matchers per invocation.
#[proc_macro]
pub fn matcher(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let blocks = top_level_named_blocks(tokens, "matcher");
    if blocks.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "matcher! expects at least one `matcher <name> { .. }` block",
        )
        .into_compile_error()
        .into();
    }
    let mut output = proc_macro2::TokenStream::new();
    for (name, body) in blocks {
        let body_str = compact(&body.to_string());
        let algorithm = extract_raw_field(&body_str, "algorithm").unwrap_or_default();
        let threshold: f64 = extract_raw_field(&body_str, "threshold").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let lists = extract_list_field(&body_str, "lists").unwrap_or_default();
        let static_name = catalog_static_name("MATCHER", &format!("{name}|{body_str}"));
        let name_lit = LitStr::new(&name, proc_macro2::Span::call_site());
        let algorithm_lit = LitStr::new(&algorithm, proc_macro2::Span::call_site());
        let list_lits = str_lits(&lists);
        output.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::MATCHERS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::MatcherRegistration = ::nirdosha_guard_registry::MatcherRegistration {
                name: #name_lit,
                algorithm: #algorithm_lit,
                threshold: #threshold,
                lists: &[#(#list_lits),*],
            };
        });
    }
    output.into()
}

/// `feature <name>(<key>) = <kind>( .. );` (RFC 0025 §8.2), one or more
/// statements per invocation, split on top-level `;`. `name`/`key`/`kind`
/// are pulled out structurally; the parenthesized remainder is kept as
/// opaque `spec` source text rather than parsed further — real usages mix
/// plain idents (`keys = [subject_id]`), call-shaped aggregates
/// (`sum(amount)`), and a free-form boolean expression
/// (`geo_speed(geo) > 900 km/h`) with no shared grammar, so there's nothing
/// uniform left to structurally extract once `kind`'s own argument list is
/// reached — the same "structure what's checkable, keep the rest honest as
/// source" split `PolicyRecord.filter_ref` already uses for clauses this
/// lowering pass can name but not fully resolve.
#[proc_macro]
pub fn window(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let body_str = compact(&tokens.to_string());
    let statements = split_top_level(&body_str, ';');
    let mut output = proc_macro2::TokenStream::new();
    let mut found = false;
    for stmt in statements {
        if stmt.is_empty() {
            continue;
        }
        let Some((name, key, kind, spec)) = parse_feature_statement(&stmt) else {
            return syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("window!: cannot parse feature statement `{stmt}` (expected `feature name(key) = kind( .. );`)"),
            )
            .into_compile_error()
            .into();
        };
        found = true;
        let static_name = catalog_static_name("WINDOW", &stmt);
        let name_lit = LitStr::new(&name, proc_macro2::Span::call_site());
        let key_lit = LitStr::new(&key, proc_macro2::Span::call_site());
        let kind_lit = LitStr::new(&kind, proc_macro2::Span::call_site());
        let spec_lit = LitStr::new(&spec, proc_macro2::Span::call_site());
        output.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::WINDOWS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::WindowRegistration = ::nirdosha_guard_registry::WindowRegistration {
                name: #name_lit,
                key: #key_lit,
                kind: #kind_lit,
                spec: #spec_lit,
            };
        });
    }
    if !found {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "window! expects at least one `feature name(key) = kind( .. );` statement",
        )
        .into_compile_error()
        .into();
    }
    output.into()
}

fn parse_feature_statement(stmt: &str) -> Option<(String, String, String, String)> {
    let rest = stmt.strip_prefix("feature")?;
    let paren_open = rest.find('(')?;
    let name = rest[..paren_open].to_string();
    let paren_close = matching_bracket(rest, paren_open)?;
    let key = rest[paren_open + 1..paren_close].to_string();
    let after_paren = &rest[paren_close + 1..];
    let after_eq = after_paren.strip_prefix('=')?;
    let kind_paren = after_eq.find('(')?;
    let kind = after_eq[..kind_paren].to_string();
    let kind_close = matching_bracket(after_eq, kind_paren)?;
    let spec = after_eq[kind_paren + 1..kind_close].to_string();
    Some((name, key, kind, spec))
}

/// `mcp_tools! { server <name> { identity = "..."; tools = [ .. ]; delegation { bind ..; ttl = ..; max_tool_calls = N; rate = ..; } defaults { audit = ident; row_cap = N; destination = ident; } } }`
/// (RFC 0024 §1/§5.3, RFC 0025 §8.8), one or more servers per invocation.
/// `tools` entries — bare idents (`evaluate`) or call-shaped
/// (`query_records(Transaction, Alert, Case, Customer)`) — are each kept as
/// their own rendered token string.
#[proc_macro]
pub fn mcp_tools(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let blocks = top_level_named_blocks(tokens, "server");
    if blocks.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "mcp_tools! expects at least one `server <name> { .. }` block",
        )
        .into_compile_error()
        .into();
    }
    let mut output = proc_macro2::TokenStream::new();
    for (name, body) in blocks {
        let body_str = compact(&body.to_string());
        let identity = extract_str_field(&body_str, "identity").unwrap_or_default();
        let tools = extract_list_field(&body_str, "tools").unwrap_or_default();

        let body_tokens: Vec<TokenTree> = body.into_iter().collect();
        let delegation_str = find_labeled_block(&body_tokens, "delegation").map(|ts| compact(&ts.to_string()));
        let defaults_str = find_labeled_block(&body_tokens, "defaults").map(|ts| compact(&ts.to_string()));

        let ttl = delegation_str.as_deref().and_then(|s| extract_raw_field(s, "ttl"));
        let max_tool_calls = delegation_str
            .as_deref()
            .and_then(|s| extract_raw_field(s, "max_tool_calls"))
            .and_then(|s| s.parse::<u32>().ok());
        let rate = delegation_str.as_deref().and_then(|s| extract_raw_field(s, "rate"));

        let audit_default = defaults_str.as_deref().and_then(|s| extract_raw_field(s, "audit"));
        let row_cap_default = defaults_str
            .as_deref()
            .and_then(|s| extract_raw_field(s, "row_cap"))
            .and_then(|s| s.parse::<u64>().ok());
        let destination_default = defaults_str.as_deref().and_then(|s| extract_raw_field(s, "destination"));

        let static_name = catalog_static_name("MCP_SERVER", &format!("{name}|{body_str}"));
        let name_lit = LitStr::new(&name, proc_macro2::Span::call_site());
        let identity_lit = LitStr::new(&identity, proc_macro2::Span::call_site());
        let tool_lits = str_lits(&tools);
        let ttl_expr = opt_str_lit(&ttl);
        let max_tool_calls_expr = match max_tool_calls {
            Some(v) => quote!(Some(#v)),
            None => quote!(None),
        };
        let rate_expr = opt_str_lit(&rate);
        let audit_default_expr = opt_str_lit(&audit_default);
        let row_cap_default_expr = match row_cap_default {
            Some(v) => quote!(Some(#v)),
            None => quote!(None),
        };
        let destination_default_expr = opt_str_lit(&destination_default);

        output.extend(quote! {
            #[used]
            #[::nirdosha_guard_registry::linkme::distributed_slice(::nirdosha_guard_registry::MCP_SERVERS)]
            #[linkme(crate = ::nirdosha_guard_registry::linkme)]
            static #static_name: ::nirdosha_guard_registry::McpServerRegistration = ::nirdosha_guard_registry::McpServerRegistration {
                name: #name_lit,
                identity: #identity_lit,
                tools: &[#(#tool_lits),*],
                ttl: #ttl_expr,
                max_tool_calls: #max_tool_calls_expr,
                rate: #rate_expr,
                audit_default: #audit_default_expr,
                row_cap_default: #row_cap_default_expr,
                destination_default: #destination_default_expr,
            };
        });
    }
    output.into()
}
