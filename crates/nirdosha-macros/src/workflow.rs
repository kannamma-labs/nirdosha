//! `workflow! { .. }` — the dialect's general state-machine substitute
//! for `.nir`'s `workflow { .. }` top-level form (issue #70's "item 4"
//! gap: `wizard!` is one archetype — a linear multi-step form — not a
//! general state/event/transition grammar mirroring `.nir`'s own
//! `WorkflowDecl`). Its main purpose for issue #73: an optional
//! `against = "spec.json"` clause checks the declared states/
//! transitions/data fields against a PRD-extraction spec (the same
//! shape `.nir`'s `extraction_schema::ExtractedWorkflow` parses) at
//! macro-expansion time, via `nirdosha_contract_core::workflow_spec`'s
//! ported `workflow_conformance.rs` algorithm — a real `compile_error!`
//! naming the exact mismatch, not a generic macro error.
//!
//! ```ignore
//! nirdosha_rt::workflow! {
//!     name: Onboarding,
//!     against: "specs/onboarding.json",
//!     data: [ applicant_name: String ],
//!     states: [
//!         state Draft {
//!             on Submit -> Approved,
//!         },
//!         state Approved {
//!             terminal,
//!             on_entry: [notify],
//!         },
//!     ],
//! }
//! ```
//!
//! expands to a plain state enum (`OnboardingState`) and a pure
//! transition function (`advance_onboarding`) validating an event
//! against the declared graph — deliberately no storage, no HTTP
//! routes: unlike `wizard!`, this macro's job is the state graph's
//! *shape*, not a runnable UI archetype (a caller wanting persistence
//! can hold `OnboardingState` in `nirdosha_rt::SharedCell`/
//! `SharedTable` the same way any other value would).
//!
//! `on_entry`/`on_exit` are plain identifier labels, compared by
//! *count* only against a spec — the same scope limit
//! `workflow_conformance.rs`'s own module doc states for `.nir`: no
//! action-name-to-real-call binding is attempted here either.

use crate::util::expect_keyword;
use nirdosha_contract_core::workflow_spec::{self, DeclaredState, DeclaredWorkflow};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, LitStr, Token, Type};

struct FieldDef {
    name: Ident,
    ty: Type,
}

fn parse_data_fields(input: ParseStream) -> syn::Result<Vec<FieldDef>> {
    let content;
    bracketed!(content in input);
    let mut fields = Vec::new();
    while !content.is_empty() {
        let name: Ident = content.parse()?;
        content.parse::<Token![:]>()?;
        let ty: Type = content.parse()?;
        fields.push(FieldDef { name, ty });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(fields)
}

struct StateDef {
    name: Ident,
    terminal: bool,
    on_entry: Vec<Ident>,
    on_exit: Vec<Ident>,
    /// `(event, target)`.
    transitions: Vec<(Ident, Ident)>,
}

fn parse_ident_list(input: ParseStream) -> syn::Result<Vec<Ident>> {
    let content;
    bracketed!(content in input);
    let mut idents = Vec::new();
    while !content.is_empty() {
        idents.push(content.parse()?);
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(idents)
}

impl Parse for StateDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "state")?;
        let name: Ident = input.parse()?;
        let content;
        braced!(content in input);
        let mut state = StateDef {
            name,
            terminal: false,
            on_entry: Vec::new(),
            on_exit: Vec::new(),
            transitions: Vec::new(),
        };
        while !content.is_empty() {
            let keyword: Ident = content.fork().parse()?;
            if keyword == "terminal" {
                content.parse::<Ident>()?;
                state.terminal = true;
            } else if keyword == "on_entry" {
                content.parse::<Ident>()?;
                content.parse::<Token![:]>()?;
                state.on_entry = parse_ident_list(&content)?;
            } else if keyword == "on_exit" {
                content.parse::<Ident>()?;
                content.parse::<Token![:]>()?;
                state.on_exit = parse_ident_list(&content)?;
            } else if keyword == "on" {
                content.parse::<Ident>()?;
                let event: Ident = content.parse()?;
                content.parse::<Token![->]>()?;
                let target: Ident = content.parse()?;
                state.transitions.push((event, target));
            } else {
                return Err(syn::Error::new(
                    keyword.span(),
                    "expected `terminal`, `on_entry: [..]`, `on_exit: [..]`, or `on <Event> -> <Target>`",
                ));
            }
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        Ok(state)
    }
}

struct WorkflowInput {
    name: Ident,
    against: Option<LitStr>,
    data: Vec<FieldDef>,
    states: Vec<StateDef>,
}

impl Parse for WorkflowInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "name")?;
        input.parse::<Token![:]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        let against = if input.fork().parse::<Ident>().is_ok_and(|i| i == "against") {
            input.parse::<Ident>()?;
            input.parse::<Token![:]>()?;
            let path: LitStr = input.parse()?;
            input.parse::<Token![,]>()?;
            Some(path)
        } else {
            None
        };

        expect_keyword(input, "data")?;
        input.parse::<Token![:]>()?;
        let data = parse_data_fields(input)?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "states")?;
        input.parse::<Token![:]>()?;
        let states_content;
        bracketed!(states_content in input);
        let mut states = Vec::new();
        while !states_content.is_empty() {
            states.push(states_content.parse::<StateDef>()?);
            if states_content.peek(Token![,]) {
                states_content.parse::<Token![,]>()?;
            }
        }
        if states.is_empty() {
            return Err(syn::Error::new(name.span(), "workflow! needs at least one `state ..` entry"));
        }
        let _ = input.parse::<Token![,]>();

        Ok(WorkflowInput { name, against, data, states })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<WorkflowInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: WorkflowInput) -> TokenStream2 {
    // Every `on <Event> -> <Target>` names a state by identifier; a
    // target with no matching `state ..` entry is exactly the kind of
    // typo a bespoke parser would silently accept and a checked
    // declaration must not.
    let declared_names: std::collections::HashSet<String> =
        input.states.iter().map(|s| s.name.to_string()).collect();
    for state in &input.states {
        for (_, target) in &state.transitions {
            if !declared_names.contains(&target.to_string()) {
                return syn::Error::new(
                    target.span(),
                    format!("`{target}` is not a declared state in this workflow!"),
                )
                .to_compile_error();
            }
        }
    }

    if let Some(against) = &input.against {
        let declared = DeclaredWorkflow {
            data: input
                .data
                .iter()
                .map(|f| {
                    let ty = &f.ty;
                    (f.name.to_string(), quote!(#ty).to_string().replace(' ', ""))
                })
                .collect(),
            states: input
                .states
                .iter()
                .map(|s| DeclaredState {
                    name: s.name.to_string(),
                    terminal: s.terminal,
                    on_entry: s.on_entry.len(),
                    on_exit: s.on_exit.len(),
                })
                .collect(),
            transitions: input
                .states
                .iter()
                .flat_map(|s| {
                    s.transitions
                        .iter()
                        .map(move |(event, target)| (s.name.to_string(), event.to_string(), target.to_string(), false))
                })
                .collect(),
        };
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let full_path = std::path::Path::new(&manifest_dir).join(against.value());
        let json = match std::fs::read_to_string(&full_path) {
            Ok(j) => j,
            Err(e) => {
                return syn::Error::new(
                    against.span(),
                    format!("workflow! against(\"{}\") — cannot read {}: {e}", against.value(), full_path.display()),
                )
                .to_compile_error()
            }
        };
        let spec = match workflow_spec::parse_spec(&json) {
            Ok(s) => s,
            Err(e) => {
                return syn::Error::new(
                    against.span(),
                    format!("workflow! against(\"{}\") — malformed spec JSON: {e}", against.value()),
                )
                .to_compile_error()
            }
        };
        let mismatches = workflow_spec::check_conformance(&spec, &declared);
        if !mismatches.is_empty() {
            let details = mismatches.iter().map(|m| format!("  - {m}")).collect::<Vec<_>>().join("\n");
            return syn::Error::new(
                against.span(),
                format!(
                    "workflow! `{}` does not conform to `{}`:\n{details}",
                    input.name,
                    against.value()
                ),
            )
            .to_compile_error();
        }
    }

    let enum_name = format_ident!("{}State", input.name);
    let variants = input.states.iter().map(|s| &s.name);
    let terminal_arms = input.states.iter().map(|s| {
        let name = &s.name;
        let terminal = s.terminal;
        quote! { #enum_name::#name => #terminal }
    });
    let advance_fn = format_ident!("advance_{}", snake_case(&input.name.to_string()));
    let is_terminal_fn = format_ident!("{}_is_terminal", snake_case(&input.name.to_string()));

    let transition_arms = input.states.iter().flat_map(|s| {
        let from = &s.name;
        let enum_name = enum_name.clone();
        s.transitions.iter().map(move |(event, target)| {
            let event_str = event.to_string();
            quote! { (#enum_name::#from, #event_str) => Ok(#enum_name::#target) }
        })
    });

    let workflow_name_str = input.name.to_string();
    quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #enum_name {
            #(#variants),*
        }

        /// Validates `event` against `#workflow_name_str`'s declared
        /// transition graph, returning the resulting state or a plain
        /// `Err` naming what was actually attempted -- never a panic,
        /// never a silent no-op.
        #[allow(dead_code)]
        pub fn #advance_fn(state: #enum_name, event: &str) -> Result<#enum_name, String> {
            match (state, event) {
                #(#transition_arms,)*
                (state, event) => Err(format!(
                    "{} has no `{event}` transition out of {state:?}",
                    #workflow_name_str
                )),
            }
        }

        #[allow(dead_code)]
        pub fn #is_terminal_fn(state: #enum_name) -> bool {
            match state {
                #(#terminal_arms),*
            }
        }
    }
}

fn snake_case(pascal: &str) -> String {
    let mut out = String::new();
    for (i, c) in pascal.chars().enumerate() {
        if c.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
