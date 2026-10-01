//! Issue #75 item 3 ("self-contained binary"): a crate's own embedded
//! default assets (`include_str!`/`include_bytes!`) must live inside
//! its own crate directory. An embed that reaches outside it (e.g.
//! into a sibling example) means the crate cannot be built, vendored,
//! or published on its own — the exact "UI assets alongside the
//! binary rather than inside it" problem the issue names, just for a
//! *compile-time* embed instead of a runtime sidecar file.
//!
//! This walks every `.rs` file under `src/`, finds each `include_str!`/
//! `include_bytes!` call, and resolves its path argument (relative to
//! the file that calls it, exactly as rustc resolves it) against this
//! crate's own root.

use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `include_str!("...")` / `include_bytes!("...")` argument found
/// in `file`, in source order.
fn embed_paths(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for macro_name in ["include_str!", "include_bytes!"] {
        let mut rest = source;
        while let Some(at) = rest.find(macro_name) {
            rest = &rest[at + macro_name.len()..];
            let Some(open_quote) = rest.find('"') else { continue };
            let after_open = &rest[open_quote + 1..];
            let Some(close_quote) = after_open.find('"') else { continue };
            out.push(after_open[..close_quote].to_string());
            rest = &after_open[close_quote + 1..];
        }
    }
    out
}

#[test]
fn embedded_assets_never_reach_outside_the_crate() {
    let root = crate_root();
    let src = root.join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty(), "expected to find source files under {}", src.display());

    let mut checked = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for embed in embed_paths(&text) {
            // `env!`/`concat!`-built paths aren't plain string literals
            // this scan can resolve; skip rather than false-positive.
            if embed.contains('{') || embed.starts_with('/') {
                continue;
            }
            let resolved = file.parent().unwrap().join(&embed);
            let canonical = resolved.canonicalize().unwrap_or_else(|e| {
                panic!(
                    "{}: include!'s target `{embed}` does not exist ({e}) — resolved to {}",
                    file.display(),
                    resolved.display()
                )
            });
            let crate_root_canonical = root.canonicalize().unwrap();
            assert!(
                canonical.starts_with(&crate_root_canonical),
                "{}: embeds `{embed}`, which resolves outside this crate ({}) — \
                 a self-contained crate's default assets must live inside its own \
                 directory, not reach into a sibling crate/example",
                file.display(),
                canonical.display()
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "expected at least one include_str!/include_bytes! to check (is the theme embed still there?)");
}
