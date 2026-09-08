// Builds `../runtime-kernels` — the freestanding, data-dependent linalg
// kernels (`det`/`inv`/`solve`/`rank`/`kf_update_state`/`kf_update_cov`),
// the `tcp`/`tcp_listener`/`file` native I/O kernels, and (as of this
// pass) the `dec128` kernels — into a static library at `nirdosha`'s own
// build time, once, not per user program. `codegen.rs` embeds the
// resulting `.a` via `include_bytes!` and links it into every native
// binary `nirdosha build` produces, alongside the `.ll` file it
// generates — see `runtime-kernels/src/lib.rs`'s module doc for why
// these builtins go through a linked native `call` instead of
// hand-emitted branchy IR.
//
// **`cargo rustc` on a real sub-crate, not a bare `rustc` invocation on a
// loose file** — the change this comment used to argue against, before
// `dec128` needed a real dependency (`rust_decimal`) a dependency-free
// `rustc` call had no way to reach
// (rfcs/0005-plugin-boundary-safety-and-performance.md's own finding).
// `../runtime-kernels` is deliberately its *own* Cargo workspace (its
// `Cargo.toml`'s doc comment), not a member of this repository's root
// workspace: a `cargo build` invoked from *inside* this very build
// script, against the *same* workspace this build script's own `cargo
// build` is already running under, would contend for that workspace's
// `target/` lock — the outer build is waiting on this script to finish,
// so a nested call sharing that lock would deadlock, not just run
// slowly. A separate workspace, built into its own private
// `--target-dir` under `OUT_DIR`, shares no lock with the outer build
// at all.
//
// `cargo rustc ... -- --print=native-static-libs`, not a plain `cargo
// build`, for the same reason the old code used `rustc
// --print=native-static-libs` instead of a plain `rustc` call: it
// forwards that flag straight to the one real `rustc` invocation that
// produces the final artifact, so the OS-level native libraries this
// staticlib's own code transitively needs (`-lm -lpthread ...` on Unix,
// `ws2_32.lib ...` on Windows — `codegen.rs` links this `.a` with a bare
// `clang` invocation, not `rustc`, so `rustc`'s own usual "supply this
// list at final link time" behavior has to be captured explicitly) come
// out of the exact same command that builds the artifact, not a second,
// possibly-divergent one.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR"));
    let kernels_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime-kernels");
    let kernels_manifest = kernels_dir.join("Cargo.toml");
    let kernels_target_dir = out_dir.join("runtime_kernels_target");

    // **Real bug found and fixed this session, not a hypothetical one**:
    // this used to watch only `src/lib.rs`, not the rest of `src/`
    // (`kernel/*.rs`). Cargo has no visibility into this build script's
    // own nested `cargo rustc` shell-out below, so it only re-runs this
    // script (and, critically, only then recompiles `nirdosha` itself
    // against a fresh `include_bytes!`-ed archive) when a path named
    // here actually changes — editing only `kernel/db.rs`/`kernel/
    // http.rs`/`kernel/identity.rs` (adding a new file, in each case)
    // left `lib.rs` itself byte-for-byte unchanged, so this script never
    // re-ran and `nirdosha` kept linking a stale archive missing the new
    // symbols — a real `undefined reference` link failure, reproduced
    // and root-caused via `cargo clean -p nirdosha` making the problem
    // disappear (proving it was a stale-artifact issue, not a codegen
    // bug). Walking the whole `src/` tree, not just `lib.rs`, closes
    // this for every future kernel source file, not just the three that
    // happened to trigger it this time.
    fn watch_dir_recursive(dir: &std::path::Path) {
        println!("cargo::rerun-if-changed={}", dir.display());
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                watch_dir_recursive(&path);
            } else {
                println!("cargo::rerun-if-changed={}", path.display());
            }
        }
    }
    watch_dir_recursive(&kernels_dir.join("src"));
    println!("cargo::rerun-if-changed={}", kernels_manifest.display());

    // `CARGO`, not a bare `"cargo"` on `PATH`: cargo always sets this to
    // the exact `cargo` binary driving the outer build (same toolchain,
    // same version) when it runs a build script — the correct binary to
    // re-invoke, not an assumption that whatever `cargo` happens to
    // resolve first on `PATH` matches.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    let output = Command::new(&cargo)
        .arg("rustc")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&kernels_manifest)
        .arg("--target-dir")
        .arg(&kernels_target_dir)
        .arg("--")
        .arg("--print=native-static-libs")
        .output()
        .expect(
            "failed to invoke `cargo rustc` to build ../runtime-kernels -- \
             a Rust toolchain with `cargo`/`rustc` on PATH is required to build the \
             `nirdosha` compiler itself, same as before this change",
        );

    assert!(
        output.status.success(),
        "cargo rustc failed to build ../runtime-kernels into a staticlib:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // `cargo rustc` (unlike the old dependency-free bare `rustc` call this
    // replaced) gives no way to name the output file explicitly -- it
    // picks the artifact name from the platform's own staticlib
    // convention: `lib<name>.a` everywhere `cargo` treats as
    // GNU-flavored (Linux, macOS, windows-gnu), but `<name>.lib` (no
    // `lib` prefix, MSVC's COFF archive format, not a `.a`) on
    // windows-msvc -- the default host toolchain on GitHub's
    // `windows-latest` runners. Reading `CARGO_CFG_TARGET_ENV` (cargo
    // always sets this for a build script to the *target*'s env, `msvc`/
    // `gnu`/empty) instead of `cfg!(windows)` keeps this correct under
    // cross-compilation too, not just "happens to match on CI".
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let built_lib_name = if target_env == "msvc" {
        "nirdosha_runtime_kernels.lib".to_string()
    } else {
        "libnirdosha_runtime_kernels.a".to_string()
    };
    let built_lib = kernels_target_dir.join("release").join(&built_lib_name);
    let out_lib = out_dir.join("libnirdosha_runtime.a");
    std::fs::copy(&built_lib, &out_lib).unwrap_or_else(|e| {
        panic!(
            "expected cargo rustc to produce {} -- copy failed: {e}",
            built_lib.display()
        )
    });

    // rustc prefixes every note with `note: ` (`note: native-static-libs:
    // ...`), so this looks for the marker as a substring rather than
    // requiring it to start the line.
    let stderr = String::from_utf8_lossy(&output.stderr);
    const MARKER: &str = "native-static-libs: ";
    let native_libs = stderr
        .lines()
        .find_map(|line| line.find(MARKER).map(|i| &line[i + MARKER.len()..]))
        .unwrap_or_else(|| {
            panic!(
                "expected a `native-static-libs:` note in cargo rustc's stderr, found none:\n{stderr}"
            )
        });
    std::fs::write(out_dir.join("native_static_libs.txt"), native_libs)
        .expect("writing native_static_libs.txt into OUT_DIR");

    // Some crates (the `windows`/`windows-sys` family, at least -- found via
    // a real Windows CI failure, `LNK1181: cannot open input file
    // 'windows.0.52.0.lib'`, not anticipated in advance) ship their own
    // private, version-named import lib rather than relying on one the
    // system already provides. `--print=native-static-libs` correctly
    // names these -- they're `dylib`-kind, not `static`, so (unlike e.g.
    // vendored OpenSSL's `static`-kind libs, which get merged straight
    // into `libnirdosha_runtime.a` below and need nothing further) they
    // stay external and must be supplied at final link time, the same as
    // a genuine system lib (`kernel32.lib`, `advapi32.lib`, ...). But
    // unlike a genuine system lib, they don't live on the linker's default
    // search path at all -- only inside this *build's own* private target
    // dir, wherever the owning crate's build script copied it.
    //
    // Referencing that path directly (a `-L` flag into `kernels_target_dir`)
    // would silently break on any machine other than the one that built
    // `nirdosha` itself -- the same "no dependency on the original build
    // machine" reasoning `libnirdosha_runtime.a`'s own `include_bytes!`
    // embedding already rests on. So instead: every native-static-libs
    // token actually found as a real file under this build's own target
    // dir gets copied into `OUT_DIR` and recorded in a generated Rust
    // snippet (`extra_native_libs.rs`, embedding each file's bytes via
    // `include_bytes!`) that `codegen.rs` `include!`s alongside
    // `NATIVE_STATIC_LIBS` -- at actual link time, it writes the bytes
    // back out to a temp file and links that file *by path*, sidestepping
    // the search-path problem entirely (`runtime_lib_path`'s own pattern,
    // just applied to a second kind of embedded artifact). A token with no
    // matching file found (`kernel32.lib`, `advapi32.lib`, ...) is left
    // exactly as it was -- a bare name, assumed to be a genuine,
    // always-present system lib.
    let extra_libs_dir = out_dir.join("extra_native_libs");
    std::fs::create_dir_all(&extra_libs_dir).expect("creating extra_native_libs dir under OUT_DIR");
    let mut extra_libs_rs = String::from(
        "// Generated by build.rs -- see its own doc comment on `extra_native_libs.rs`.\n\
         #[allow(dead_code)]\n\
         pub static EXTRA_NATIVE_LIBS: &[(&str, &[u8])] = &[\n",
    );
    for token in native_libs.split_whitespace() {
        if !token.ends_with(".lib") {
            continue; // only the MSVC-style `name.lib` tokens are ever such a file
        }
        if let Some(found) = find_file_named(&kernels_target_dir, token) {
            let dest = extra_libs_dir.join(token);
            std::fs::copy(&found, &dest).unwrap_or_else(|e| {
                panic!("copying {} to {}: {e}", found.display(), dest.display())
            });
            extra_libs_rs.push_str(&format!(
                "    ({token:?}, include_bytes!({:?})),\n",
                dest.display().to_string()
            ));
        }
    }
    extra_libs_rs.push_str("];\n");
    std::fs::write(out_dir.join("extra_native_libs.rs"), extra_libs_rs)
        .expect("writing extra_native_libs.rs into OUT_DIR");
}

/// Recursively searches `dir` for a file named exactly `name`, returning
/// the first match. Used to locate a native-static-libs token's actual
/// backing file somewhere under this build's own private target dir (see
/// the `extra_native_libs.rs` doc comment above) -- `kernels_target_dir`
/// is a few thousand files at most (one crate graph's worth of build
/// artifacts), so an unindexed walk is cheap relative to the `cargo rustc`
/// invocation that produced them.
fn find_file_named(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}
