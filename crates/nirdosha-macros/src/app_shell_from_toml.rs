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
                links.push(::nirdosha_rt::NavLink { label: #label, href: #href });
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

    let title = &input.title;

    Ok(quote! {
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
