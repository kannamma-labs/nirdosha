//! `static_embed! { .. }` — embed a checked-in static page (markdown or
//! plain text) into the compiled binary, with its SHA-256 digest pinned
//! in the invocation and verified at macro-expansion time.
//!
//! ```ignore
//! nirdosha_rt::static_embed! {
//!     mount: mount_help,
//!     path: "/help",
//!     title: "Getting Started",
//!     content_file: "files/help/getting-started.md",
//!     sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
//!     access: public,
//! }
//! ```
//!
//! `content_file` resolves relative to the invoking crate's
//! `CARGO_MANIFEST_DIR` (the same convention `app_shell_from_toml!`
//! uses for `menus.toml`); the expansion embeds the file with
//! `include_str!` so the content ships inside the binary.
//!
//! **The pinned digest is load-bearing.** If the file's content no
//! longer hashes to the pinned `sha256:`, expansion is a compile error
//! — the same "lying is a build error" posture the compiler's own
//! contract checks use, and the same content-hash integrity convention
//! the workspace's fixture crates (`nirdosha-scoring-model-onnx`,
//! `nirdosha-screening-list-fixture`) pin their inputs with. The
//! rendered page carries the (verified) digest footer, so a served
//! page discloses exactly which bytes it is.
//!
//! This archetype is presentational by design: there is no `GuardedTable`
//! anywhere in it (nothing for a guard policy to gate at runtime — the
//! same posture RTM's M17 knowledge pages document). The route gate
//! (`access:`) is the whole story: `public`, or `requires role "X"` for
//! a role-proofed route. Menu `guard` references should therefore be
//! omitted for these screens, and the generator classifies them as
//! non-data screens (no `data_binding`, no guard requirement).

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};


enum Access {
    Public,
    Role(LitStr),
}

impl Parse for Access {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = Ident::parse_any(input)?;
        if ident == "public" {
            return Ok(Access::Public);
        }
        if ident == "requires" {
            let role_kw = Ident::parse_any(input)?;
            if role_kw != "role" {
                return Err(syn::Error::new(role_kw.span(), "expected `role`"));
            }
            let role: LitStr = input.parse()?;
            if let Err(e) = nirdosha_contract_core::role::role_ident(&role.value(), role.span()) {
                return Err(e);
            }
            return Ok(Access::Role(role));
        }
        Err(syn::Error::new(ident.span(), "expected `public` or `requires role \"...\"`"))
    }
}

pub struct StaticEmbedInput {
    mount: Ident,
    path: LitStr,
    title: LitStr,
    content_file: LitStr,
    sha256: LitStr,
    access: Access,
}

impl Parse for StaticEmbedInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mount: Option<Ident> = None;
        let mut path: Option<LitStr> = None;
        let mut title: Option<LitStr> = None;
        let mut content_file: Option<LitStr> = None;
        let mut sha256: Option<LitStr> = None;
        let mut access: Option<Access> = None;
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "mount" => mount = Some(input.parse()?),
                "path" => path = Some(input.parse()?),
                "title" => title = Some(input.parse()?),
                "content_file" => content_file = Some(input.parse()?),
                "sha256" => sha256 = Some(input.parse()?),
                "access" => access = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("expected `mount`, `path`, `title`, `content_file`, `sha256`, or `access`, found `{other}`"),
                    ))
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        let mount = mount.ok_or_else(|| syn::Error::new(input.span(), "`mount:` is required"))?;
        let path = path.ok_or_else(|| syn::Error::new(input.span(), "`path:` is required"))?;
        let title = title.ok_or_else(|| syn::Error::new(input.span(), "`title:` is required"))?;
        let content_file = content_file.ok_or_else(|| syn::Error::new(input.span(), "`content_file:` is required (project-root-relative)"))?;
        let sha256 = sha256.ok_or_else(|| syn::Error::new(input.span(), "`sha256:` is required — pin the content file's SHA-256 (the expansion verifies it)"))?;
        let access = access.unwrap_or(Access::Public);
        let pin = sha256.value();
        if pin.len() != 64 || !pin.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(syn::Error::new(sha256.span(), "`sha256:` must be a 64-char hex digest"));
        }
        Ok(StaticEmbedInput { mount, path, title, content_file, sha256, access })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<StaticEmbedInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    // Expansion-time integrity check: read the file now, hash it, and
    // refuse the build if it doesn't match the pin. (The expansion also
    // embeds via `include_str!` at the invoking crate's compile time —
    // same file, same invocation, so the checked bytes are the served
    // bytes.)
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let file_path = std::path::Path::new(&manifest).join(parsed.content_file.value());
    let bytes = match std::fs::read(&file_path) {
        Ok(b) => b,
        Err(e) => {
            return syn::Error::new(
                parsed.content_file.span(),
                format!("static_embed! cannot read {}: {e}", file_path.display()),
            )
            .to_compile_error()
            .into()
        }
    };
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let computed: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if computed != parsed.sha256.value() {
        return syn::Error::new(
            parsed.sha256.span(),
            format!(
                "static_embed! digest mismatch for {}: pinned sha256 {}, but the file hashes to {} — if the edit is intended, re-pin the register (screens.toml) and regenerate",
                file_path.display(),
                parsed.sha256.value(),
                computed
            ),
        )
        .to_compile_error()
        .into();
    }

    let mount = &parsed.mount;
    let path = &parsed.path;
    let title = &parsed.title;
    let content_file = parsed.content_file.value();
    let digest = parsed.sha256.value();
    let route = match &parsed.access {
        Access::Public => quote! {
            router
                .get(#path, #title, |_req, _params| {
                    let content = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #content_file));
                    nirdosha_rt::Response::html(200, nirdosha_rt::screens::static_page_html(#title, content, #digest))
                })
        },
        Access::Role(role) => {
            let role_ident = nirdosha_contract_core::role::role_ident(&role.value(), role.span())
                .expect("role name already validated at parse time");
            quote! {
                router
                    .get_gated::<crate::nirdosha_roles::#role_ident>(#path, #title, |_req, _params, _proof| {
                        let content = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #content_file));
                        nirdosha_rt::Response::html(200, nirdosha_rt::screens::static_page_html(#title, content, #digest))
                    })
            }
        }
    };

    quote! {
        pub fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            #route
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_invocation_parses() {
        let ok = syn::parse_str::<StaticEmbedInput>(
            "mount: mount_help, path: \"/help\", title: \"Getting Started\", \
             content_file: \"files/help/getting-started.md\", \
             sha256: \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\", \
             access: public",
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn access_defaults_to_public() {
        let ok = syn::parse_str::<StaticEmbedInput>(
            "mount: mount_help, path: \"/help\", title: \"Help\", \
             content_file: \"f.md\", \
             sha256: \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"",
        );
        assert!(ok.is_ok());

        let role = syn::parse_str::<StaticEmbedInput>(
            "mount: mount_help, path: \"/help\", title: \"Help\", \
             content_file: \"f.md\", \
             sha256: \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\", \
             access: requires role \"Analyst\"",
        );
        assert!(role.is_ok());
    }

    #[test]
    fn digest_must_be_hex64() {
        let bad = syn::parse_str::<StaticEmbedInput>(
            "mount: mount_help, path: \"/help\", title: \"Help\", \
             content_file: \"f.md\", sha256: \"deadbeef\"",
        );
        assert!(bad.is_err(), "a short digest must be refused at parse time");
    }

    #[test]
    fn unknown_key_is_a_parse_error() {
        let bad = syn::parse_str::<StaticEmbedInput>(
            "mount: mount_help, path: \"/help\", title: \"Help\", \
             content_file: \"f.md\", \
             sha256: \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\", \
             surprise: 1",
        );
        assert!(bad.is_err());
    }
}