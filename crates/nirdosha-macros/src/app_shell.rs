//! `app_shell! { .. }` — a declarative app-level shell for the
//! Nirdosha Rust dialect.
//!
//! Defines the title shown by `Router::with_info`, the global top-bar
//! navigation passed to `Router::with_nav`, and (optionally) the
//! post-login landing path. It does **not** wrap individual screen
//! archetype macros; `main()` applies the shell once to the router.
//!
//! ```ignore
//! nirdosha_rt::app_shell! {
//!     mount: mount_app_shell,
//!     title: "My App",
//!     nav: [
//!         { label: "Dashboard", href: "/dashboard" },
//!         { label: "Products",  href: "/products",  role: "admin" },
//!     ],
//!     landing: landing_path,
//! }
//! ```
//!
//! Generates:
//!
//! ```ignore
//! pub fn app_shell_title() -> &'static str { "My App" }
//! pub fn app_shell_nav() -> Vec<NavLink> { ... }           // static, all links
//! pub fn app_shell_nav_for(auth: &Auth) -> Vec<NavLink> { ... } // role-filtered
//! pub fn app_shell_landing(auth: &Auth) -> &'static str { landing_path(auth) }
//! pub fn mount_app_shell(router: Router) -> Router {
//!     router.with_info(app_shell_title(), "0.1.0")
//!           .with_nav_for(app_shell_nav_for)
//! }
//! ```
//!
//! `main()` then mounts the shell before other screens:
//!
//! ```ignore
//! let router = mount_app_shell(nirdosha_rt::Router::new(|_| nirdosha_rt::Auth::login("anon", &[])));
//! let router = mount_login(router);
//! // ... other screens
//! router.serve(8080);
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::bracketed;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

struct NavEntry {
    label: LitStr,
    href: LitStr,
    role: Option<LitStr>,
}

impl Parse for NavEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        let mut label = None;
        let mut href = None;
        let mut role = None;
        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            if key == "label" {
                label = Some(content.parse()?);
            } else if key == "href" {
                href = Some(content.parse()?);
            } else if key == "role" {
                role = Some(content.parse()?);
            } else {
                return Err(syn::Error::new(key.span(), "expected `label`, `href`, or `role`"));
            }
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        let label = label.ok_or_else(|| syn::Error::new(content.span(), "nav entry needs `label`"))?;
        let href = href.ok_or_else(|| syn::Error::new(content.span(), "nav entry needs `href`"))?;
        Ok(NavEntry { label, href, role })
    }
}

struct AppShellInput {
    mount: Ident,
    title: LitStr,
    nav: Vec<NavEntry>,
    landing: Option<Ident>,
}

impl Parse for AppShellInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mount = None;
        let mut title = None;
        let mut nav = Vec::new();
        let mut landing = None;
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            if key == "mount" {
                mount = Some(input.parse()?);
            } else if key == "title" {
                title = Some(input.parse()?);
            } else if key == "nav" {
                let list;
                bracketed!(list in input);
                while !list.is_empty() {
                    nav.push(list.parse()?);
                    if list.peek(Token![,]) {
                        list.parse::<Token![,]>()?;
                    }
                }
            } else if key == "landing" {
                landing = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(key.span(), "expected `mount`, `title`, `nav`, or `landing`"));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        let mount = mount.ok_or_else(|| syn::Error::new(input.span(), "`mount:` is required"))?;
        let title = title.ok_or_else(|| syn::Error::new(input.span(), "`title:` is required"))?;
        Ok(AppShellInput { mount, title, nav, landing })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<AppShellInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: AppShellInput) -> TokenStream2 {
    let mount = &input.mount;
    let title = &input.title;

    let nav_links = input.nav.iter().map(|e| {
        let label = &e.label;
        let href = &e.href;
        quote! {
            ::nirdosha_rt::NavLink { label: #label, href: #href, group: "" }
        }
    });

    let filtered_nav = input.nav.iter().map(|e| {
        let label = &e.label;
        let href = &e.href;
        match &e.role {
            Some(role) => quote! {
                if auth.has_role(#role) {
                    links.push(::nirdosha_rt::NavLink { label: #label, href: #href, group: "" });
                }
            },
            None => quote! {
                links.push(::nirdosha_rt::NavLink { label: #label, href: #href, group: "" });
            },
        }
    });

    let landing_fn = match &input.landing {
        Some(mount) => quote! {
            /// Generated by `nirdosha_rt::app_shell!` — delegates to the
            /// landing mount function named in the macro.
            #[allow(dead_code)]
            pub fn app_shell_landing(auth: &::nirdosha_rt::Auth) -> &'static str {
                #mount(auth)
            }
        },
        None => quote! {
            /// Generated by `nirdosha_rt::app_shell!` — no landing mount
            /// was provided, so every session lands at "/".
            #[allow(dead_code)]
            pub fn app_shell_landing(_auth: &::nirdosha_rt::Auth) -> &'static str {
                "/"
            }
        },
    };

    quote! {
        /// Generated by `nirdosha_rt::app_shell!` — see its own doc comment.
        #[allow(dead_code)]
        pub fn app_shell_title() -> &'static str {
            #title
        }

        /// Generated by `nirdosha_rt::app_shell!` — returns the full
        /// static nav list (role filtering happens in
        /// `app_shell_nav_for`).
        #[allow(dead_code)]
        pub fn app_shell_nav() -> Vec<::nirdosha_rt::NavLink> {
            vec![#(#nav_links),*]
        }

        /// Generated by `nirdosha_rt::app_shell!` — role-filtered nav for
        /// the current session.
        #[allow(dead_code)]
        pub fn app_shell_nav_for(auth: &::nirdosha_rt::Auth) -> Vec<::nirdosha_rt::NavLink> {
            let mut links = Vec::new();
            #(#filtered_nav)*
            links
        }

        /// Generated by `nirdosha_rt::app_shell!` — applies the title
        /// and role-filtered top nav to the router.
        #[allow(dead_code)]
        pub fn #mount(mut router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            router = router.with_info(app_shell_title(), "0.1.0");
            router = router.with_nav_for(app_shell_nav_for);
            router
        }

        #landing_fn
    }
}
