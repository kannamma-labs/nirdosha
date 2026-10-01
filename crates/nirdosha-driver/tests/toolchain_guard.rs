//! Issue #72 / #65 item 9: `nirdosha-driver` links `rustc_private`, so
//! it is ABI-compatible only with the exact nightly it was built
//! against. Under `RUSTC_WORKSPACE_WRAPPER`, cargo hands it the
//! *active* toolchain's own `rustc` path -- a mismatch there must fail
//! closed with an actionable message, not silently link against a
//! possibly-incompatible toolchain in-process.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nir-toolchain-guard-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

/// A fake `rustc` that only answers `--version`, reporting a different
/// build than the one `nirdosha-driver` was actually compiled against.
fn fake_rustc(dir: &std::path::Path, version_line: &str) -> PathBuf {
    let path = dir.join("rustc");
    fs::write(
        &path,
        format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '{version_line}'; fi\n"),
    )
    .unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn run_wrapped(rustc_path: &std::path::Path, extra_args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nirdosha-driver"))
        .env("NIRDOSHA_DRIVER", "1")
        .arg(rustc_path)
        .args(extra_args)
        .output()
        .unwrap()
}

#[test]
fn a_mismatched_active_toolchain_is_refused_before_linking() {
    let dir = scratch_dir();
    let rustc = fake_rustc(&dir, "rustc 1.0.0-nightly (deadbeef1 1999-01-01)");
    let out = run_wrapped(&rustc, &["--version"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("toolchain mismatch"), "{stderr}");
    assert!(stderr.contains("rebuild"), "{stderr}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_toolchain_it_was_built_with_passes_the_check() {
    // The real, active `rustc` this test binary was itself built and
    // linked against -- the check must accept it and let the wrapped
    // rustc invocation actually run (a plain `--version` here, so this
    // stays a check of the guard itself, not a real compile).
    let real_rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let out = run_wrapped(std::path::Path::new(&real_rustc), &["--version"]);
    assert!(
        out.status.success(),
        "the guard rejected the toolchain it was built with: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
