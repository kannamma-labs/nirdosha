// Builds ../../../../crates/runtime-kernels the same way
// crates/compiler/build.rs does, and links this benchmark binary
// against the resulting staticlib -- so this harness measures the
// exact `nir_tcp_*`/`nir_file_*` functions a real `nirdosha build`
// binary calls, not a reimplementation of their logic. See
// crates/compiler/build.rs's own doc comment for why `cargo rustc`
// with a private `--target-dir` (not a plain `cargo build`, not a bare
// `rustc` call) is the right invocation here too.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR"));
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let kernels_dir = manifest_dir.join("../../../../crates/runtime-kernels");
    let kernels_manifest = kernels_dir.join("Cargo.toml");
    let kernels_target_dir = out_dir.join("runtime_kernels_target");

    println!("cargo::rerun-if-changed={}", kernels_dir.join("src/lib.rs").display());
    println!("cargo::rerun-if-changed={}", kernels_manifest.display());

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
            "failed to invoke `cargo rustc` to build ../../../../crates/runtime-kernels -- \
             a Rust toolchain with cargo/rustc on PATH is required",
        );

    assert!(
        output.status.success(),
        "cargo rustc failed to build ../../../../crates/runtime-kernels into a staticlib:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Same MSVC-vs-GNU artifact-naming split crates/compiler/build.rs
    // handles, kept here for the same reason even though this harness
    // is currently only exercised on Linux.
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let built_lib_name = if target_env == "msvc" {
        "nirdosha_runtime_kernels.lib".to_string()
    } else {
        "libnirdosha_runtime_kernels.a".to_string()
    };
    let built_lib = kernels_target_dir.join("release").join(&built_lib_name);
    let out_lib = out_dir.join(&built_lib_name);
    std::fs::copy(&built_lib, &out_lib).unwrap_or_else(|e| {
        panic!(
            "expected cargo rustc to produce {} -- copy failed: {e}",
            built_lib.display()
        )
    });

    println!("cargo::rustc-link-search=native={}", out_dir.display());
    println!("cargo::rustc-link-lib=static=nirdosha_runtime_kernels");

    // The staticlib bundles its own complete copy of `std` by design
    // (a real `nirdosha build` binary has no Rust runtime of its own --
    // this crate's Cargo.toml doc comment) -- unlike that real path,
    // this harness's own `main.rs` needs std too (threads, Instant,
    // TcpStream), so the final binary here ends up with two copies of
    // std's internals (`rust_eh_personality`, panic machinery, argv
    // init) and the linker rejects the duplicate definitions. Both
    // copies come from the same toolchain invocation moments apart, so
    // they're ABI-identical -- telling the linker to keep one instead
    // of erroring is safe specifically because of that, not safe in
    // general for two arbitrary std copies.
    println!("cargo::rustc-link-arg=-Wl,--allow-multiple-definition");

    // Forward the OS-level native libs this staticlib's own code
    // transitively needs (-lpthread -lm ... on Unix) -- same reason
    // crates/compiler/build.rs captures this from `cargo rustc`'s own
    // stderr instead of hand-listing them.
    let stderr = String::from_utf8_lossy(&output.stderr);
    const MARKER: &str = "native-static-libs: ";
    let native_libs = stderr
        .lines()
        .find_map(|line| line.find(MARKER).map(|i| &line[i + MARKER.len()..]))
        .unwrap_or_else(|| {
            panic!("expected a `native-static-libs:` note in cargo rustc's stderr, found none:\n{stderr}")
        });

    for tok in native_libs.split_whitespace() {
        if let Some(name) = tok.strip_prefix("-l") {
            println!("cargo::rustc-link-lib={name}");
        }
        // Other token shapes (e.g. `-framework Foo` on macOS)
        // intentionally unhandled -- this harness targets Linux, this
        // session's own platform.
    }
}
