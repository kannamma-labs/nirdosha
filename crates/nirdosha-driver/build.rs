//! The driver links `rustc_private` crates, which (in rustup toolchains)
//! live next to `libLLVM-<version>.so` in the sysroot's own `lib/`
//! directory — a search path rustc does not add for normal crates. This
//! adds it, so `cargo build -p nirdosha-driver` works with no manual
//! RUSTFLAGS.

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let output = std::process::Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("nirdosha-driver build.rs: cannot run rustc to find the sysroot");
    let sysroot = String::from_utf8(output.stdout)
        .expect("sysroot path is UTF-8")
        .trim()
        .to_string();
    println!("cargo:rustc-link-search=native={sysroot}/lib");
    // Cargo test supplies a loader search path, but direct invocation and
    // RUSTC_WORKSPACE_WRAPPER must also find this exact nightly's driver.
    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib");
    }
    let version = std::process::Command::new(&rustc)
        .arg("--version")
        .output()
        .expect("rustc version");
    println!(
        "cargo:rustc-env=NIRDOSHA_RUSTC_VERSION={}",
        String::from_utf8(version.stdout).unwrap().trim()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
