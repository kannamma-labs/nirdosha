//! `app_shell_from_toml!("menus.toml", title: "...")` — generate the
//! same `app_shell_title` / `app_shell_nav` / `app_shell_nav_for` /
//! `landing_path` / `mount_app_shell` surface that `app_shell!` and
//! `landing!` produce, but driven by `examples/rtm/menus.toml` (and its
//! companion `screens.toml`).
//!
//! This closes the gap where the app's shell/landing were hand-written
//! literals that drifted out of sync with the 152-screen inventory.
//! The macro reads the TOML at compile time, so a missing/malformed
//! menu file is a `rustc` error, not a runtime surprise.
//!
//! ```ignore
//! nirdosha_rt::app_shell_from_toml!(
//!     "menus.toml",
//!     title: "RTM — Real-Time Transaction Monitoring (demo)",
//! );
//! ```
//!
//! The path is resolved relative to `CARGO_MANIFEST_DIR` of the crate
//! invoking the macro. The macro also reads the `screens_source` named
//! in `menus.toml`'s `[meta]` (default `screens.toml`) to skip nav
//! items whose target screen is `blocked` or `delegated`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use std::collections::HashMap;
use syn::{parse::Parse, parse::ParseStream, Ident, LitStr, Token};

struct Input {
    menus_path: LitStr,
    title: LitStr,
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let menus_path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let key: Ident = input.parse()?;
        if key != "title" {
            return Err(syn::Error::new(
                key.span(),
                format!("expected `title:`, found `{key}`"),
            ));
        }
        input.parse::<Token![:]>()?;
        let title: LitStr = input.parse()?;
        let _ = input.parse::<Token![,]>();
        Ok(Input { menus_path, title })
    }
}

fn error_stream(span: proc_macro2::Span, msg: impl Into<String>) -> TokenStream {
    syn::Error::new(span, msg.into()).to_compile_error().into()
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<Input>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).unwrap_or_else(|e| e.into()).into()
}

fn expand_parsed(input: Input) -> Result<TokenStream2, TokenStream> {
    let menus_span = input.menus_path.span();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let base = std::path::Path::new(&manifest);
    let menus_file = base.join(input.menus_path.value());
    let menus_dir = menus_file.parent().unwrap_or(base).to_path_buf();

    let menus_src = std::fs::read_to_string(&menus_file).map_err(|e| {
        error_stream(menus_span, format!("could not read menus.toml ({}): {e}", menus_file.display()))
    })?;
    let menus: toml::Value = menus_src.parse().map_err(|e| {
        error_stream(menus_span, format!("could not parse menus.toml: {e}"))
    })?;

    // ----------------------------------------------------------------
    // Load companion screens.toml for stage gating.
    // ----------------------------------------------------------------
    let screens_source = menus
        .get("meta")
        .and_then(|m| m.get("screens_source"))
        .and_then(|v| v.as_str())
        .unwrap_or("screens.toml");
    let screens_file = menus_dir.join(screens_source);
    let screens_src = std::fs::read_to_string(&screens_file).map_err(|e| {
        error_stream(menus_span, format!("could not read {screens_source} ({}): {e}", screens_file.display()))
    })?;
    let screens: toml::Value = screens_src.parse().map_err(|e| {
        error_stream(menus_span, format!("could not parse {screens_source}: {e}"))
    })?;

    let mut screen_stage: HashMap<String, String> = HashMap::new();
    if let Some(arr) = screens.get("screen").and_then(|v| v.as_array()) {
        for entry in arr {
            let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let stage = entry.get("stage").and_then(|v| v.as_str()).unwrap_or("blocked").to_string();
            if !id.is_empty() {
                screen_stage.insert(id, stage);
            }
        }
    }

    // ----------------------------------------------------------------
    // Groups (ordering)
    // ----------------------------------------------------------------
    let mut group_order: HashMap<String, i64> = HashMap::new();
    if let Some(groups) = menus.get("group").and_then(|v| v.as_array()) {
        for g in groups {
            let id = g.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let order = g.get("order").and_then(|v| v.as_integer()).unwrap_or(0);
            if !id.is_empty() {
                group_order.insert(id, order);
            }
        }
    }

    // ----------------------------------------------------------------
    // Menus -> nav entries
    // ----------------------------------------------------------------
    #[derive(Clone)]
    struct NavEntry {
        label: String,
        href: String,
        roles: Vec<String>,
        unconditional: bool,
        group_order: i64,
        order: i64,
    }

    let mut nav_entries: Vec<NavEntry> = Vec::new();
    if let Some(menus_arr) = menus.get("menu").and_then(|v| v.as_array()) {
        for m in menus_arr {
            let Some(screen_id) = m.get("screen_id").and_then(|v| v.as_str()) else { continue };
            let stage = screen_stage.get(screen_id).map(String::as_str).unwrap_or("blocked");
            // Hide screens that are not yet reachable. The inventory explicitly
            // marks `blocked` / `delegated` screens as not shippable.
            if stage == "blocked" || stage == "delegated" {
                continue;
            }

            let group = m.get("group").and_then(|v| v.as_str()).unwrap_or("work").to_string();
            let g_order = *group_order.get(&group).unwrap_or(&0);
            let order = m.get("order").and_then(|v| v.as_integer()).unwrap_or(0);
            let label = m.get("label_key").and_then(|v| v.as_str()).unwrap_or(screen_id).to_string();
            let href = m.get("route").and_then(|v| v.as_str()).unwrap_or("/").to_string();

            let roles: Vec<String> = m
                .get("roles")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            let unconditional = roles.iter().any(|r| r == "AllHuman" || r == "AllRoles");
            let roles = if unconditional { Vec::new() } else { roles };

            nav_entries.push(NavEntry {
                label,
                href,
                roles,
                unconditional,
                group_order: g_order,
                order,
            });
        }
    }

    nav_entries.sort_by(|a, b| a.group_order.cmp(&b.group_order).then(a.order.cmp(&b.order)));

    let nav_links = nav_entries.iter().map(|e| {
        let label = &e.label;
        let href = &e.href;
        quote! { ::nirdosha_rt::NavLink { label: #label, href: #href } }
    });

    let nav_for_pushes = nav_entries.iter().map(|e| {
        let label = &e.label;
        let href = &e.href;
        if e.unconditional {
            quote! {
                if auth.has_any_role() {
                    links.push(::nirdosha_rt::NavLink { label: #label, href: #href });
                }
            }
        } else {
            let checks = e.roles.iter().map(|r| quote! { auth.has_role(#r) });
            quote! {
                if #(#checks)||* {
                    links.push(::nirdosha_rt::NavLink { label: #label, href: #href });
                }
            }
        }
    });

    // ----------------------------------------------------------------
    // Landing rules from [landing]
    // ----------------------------------------------------------------
    let mut landing_rules: Vec<(String, String)> = Vec::new();
    let mut default_target = String::from("/");
    if let Some(landing) = menus.get("landing").and_then(|v| v.as_table()) {
        for (role, target_val) in landing.iter() {
            let target = target_val.as_str().unwrap_or("/").to_string();
            if role == "default" {
                default_target = target;
            } else {
                landing_rules.push((role.clone(), target));
            }
        }
    }

    let landing_arms = landing_rules.iter().map(|(role, target)| {
        quote! { if auth.has_role(#role) { return #target; } }
    });

    // ----------------------------------------------------------------
    // T-10: role-ident canonicalization. menus.toml writes logical
    // roles PascalCase ("ComplianceLead", "Mlro") free-text — until
    // now nothing checked that a role actually resolves to a
    // `nirdosha_rt::roles! { role X; }` declaration, so a typo'd or
    // stale role silently produced unreachable nav (never a build
    // error). Every role this macro consumes (nav guards + landing
    // rules) is validated the same way `crud_screens!`'s `requires
    // role "..."` grammar already validates its own role literals
    // (`nirdosha_contract_core::role::role_ident`), then re-emitted as
    // a dead type-alias referencing `crate::nirdosha_roles::<Ident>` —
    // forcing rustc to resolve the path, so an undeclared role is a
    // named "cannot find type" compile error, not a runtime surprise.
    // ----------------------------------------------------------------
    let mut role_assertions: Vec<TokenStream2> = Vec::new();
    let mut seen_roles: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut assert_role = |name: &str| -> Result<(), TokenStream> {
        if !seen_roles.insert(name.to_string()) {
            return Ok(());
        }
        let ident = nirdosha_contract_core::role::role_ident(name, menus_span)
            .map_err(|e| TokenStream::from(e.to_compile_error()))?;
        let alias = quote::format_ident!("__AssertRoleDeclared_{}", ident);
        role_assertions.push(quote! {
            #[allow(dead_code, non_camel_case_types)]
            type #alias = crate::nirdosha_roles::#ident;
        });
        Ok(())
    };
    for e in &nav_entries {
        for r in &e.roles {
            assert_role(r)?;
        }
    }
    for (role, _) in &landing_rules {
        assert_role(role)?;
    }

    let title = &input.title;

    Ok(quote! {
        #(#role_assertions)*

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — application title.
        #[allow(dead_code)]
        pub fn app_shell_title() -> &'static str {
            #title
        }

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — full static nav list.
        #[allow(dead_code)]
        pub fn app_shell_nav() -> Vec<::nirdosha_rt::NavLink> {
            vec![#(#nav_links),*]
        }

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — role-filtered nav.
        #[allow(dead_code)]
        pub fn app_shell_nav_for(auth: &::nirdosha_rt::Auth) -> Vec<::nirdosha_rt::NavLink> {
            let mut links = Vec::new();
            #(#nav_for_pushes)*
            links
        }

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — delegates to the
        /// TOML-derived landing rules below.
        #[allow(dead_code)]
        pub fn app_shell_landing(auth: &::nirdosha_rt::Auth) -> &'static str {
            landing_path(auth)
        }

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — first-match-wins
        /// post-login redirect from `menus.toml` `[landing]`.
        #[allow(dead_code)]
        pub fn landing_path(auth: &::nirdosha_rt::Auth) -> &'static str {
            #(#landing_arms)*
            #default_target
        }

        /// Generated by `nirdosha_rt::app_shell_from_toml!` — apply title and
        /// role-filtered nav to the router.
        #[allow(dead_code)]
        pub fn mount_app_shell(mut router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router = router.with_info(app_shell_title(), "0.1.0");
            router = router.with_nav_for(app_shell_nav_for);
            router
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{expand_parsed, Input};

    /// Writes a minimal menus.toml + screens.toml fixture pair to a fresh
    /// temp dir and returns the `Input` needed to drive `expand_parsed`
    /// against it (menus_path is absolute, so `expand_parsed`'s
    /// `base.join(..)` resolves it as-is regardless of CARGO_MANIFEST_DIR).
    fn fixture(role: &str) -> (Input, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "nirdosha_t10_test_{role}_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("screens.toml"),
            "[[screen]]\nid = \"1.1\"\nstage = \"built\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("menus.toml"),
            format!(
                "[[menu]]\nscreen_id = \"1.1\"\nlabel_key = \"Home\"\nroute = \"/\"\nroles = [\"{role}\"]\n"
            ),
        )
        .unwrap();
        let menus_path = dir.join("menus.toml").to_string_lossy().into_owned();
        let input: Input =
            syn::parse_str(&format!("\"{menus_path}\", title: \"Test\"")).unwrap();
        (input, dir)
    }

    /// `expand_parsed`'s Err path converts through real
    /// `proc_macro::TokenStream` (via `syn::Error::to_compile_error()`),
    /// which panics ("procedural macro API is used outside of a
    /// procedural macro") outside an actual macro expansion — a
    /// pre-existing constraint of this file's error plumbing, not
    /// specific to this change. So the shape-rejection half of
    /// `assert_role` (malformed role names, e.g. `"not a valid role!"`)
    /// is exercised at its source instead:
    /// `nirdosha_contract_core::role::role_ident`'s own test module
    /// (`still_rejects_junk_that_is_neither_snake_nor_pascal`) already
    /// covers exactly that input space, and `assert_role` above does
    /// nothing but call it and propagate. What's new here — the
    /// type-existence assertion — has no such conversion in its
    /// success path, so it's directly testable below.

    /// A syntactically valid but *undeclared* PascalCase role (e.g. a
    /// typo'd or stale menus.toml entry with no matching `role X;` in
    /// `roles!`) cannot be caught here — `expand_parsed` has no view of
    /// the target crate's module tree. What it MUST do is emit a dead
    /// type alias referencing `crate::nirdosha_roles::<Role>`, so the
    /// *consuming* crate's own compiler resolves that path and fails
    /// with a named "cannot find type" error the moment it's undeclared
    /// — this is the same mechanism `crud_screens!`'s `requires role
    /// "..."` already relies on (via `#[contract(requires(role = ..))]`
    /// resolving to the same module). Assert the alias is really wired.
    #[test]
    fn every_consumed_role_gets_a_type_existence_assertion() {
        let (input, dir) = fixture("SomeTypoedRole");
        let tokens = expand_parsed(input).expect("well-shaped role name must expand");
        let src = tokens.to_string();
        assert!(
            src.contains("__AssertRoleDeclared_SomeTypoedRole")
                && src.contains("nirdosha_roles :: SomeTypoedRole"),
            "expansion must assert the role resolves to a declared `nirdosha_roles` type, got: {src}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Positive-path parity: a genuinely declared RTM role (`Analyst`)
    /// gets the exact same assertion — the mechanism isn't special-cased
    /// away for "known good" roles. examples/rtm's own `cargo build -p
    /// rtm` is the real end-to-end proof this then compiles clean for
    /// all 11 of its declared roles.
    #[test]
    fn declared_role_gets_the_same_assertion_and_expands_cleanly() {
        let (input, dir) = fixture("Analyst");
        let tokens = expand_parsed(input).expect("a genuinely declared role must not be rejected");
        assert!(tokens.to_string().contains("__AssertRoleDeclared_Analyst"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
