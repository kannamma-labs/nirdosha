//! `cargo nirdosha` — the Nirdosha compiler CLI.
//!
//! Usage (as a cargo subcommand):
//!
//! ```text
//! cargo nirdosha build      # Stage 1 scan + Stage 2 (MIR effect lattice), then `cargo build`
//! cargo nirdosha check
//! cargo nirdosha run
//! cargo nirdosha test
//! cargo nirdosha build --fast   # Stage 1 only: sub-second, no nightly/rustc-dev needed
//! cargo nirdosha verify     # verify + emit certificate, no cargo delegation
//! cargo nirdosha fmt        # pass-through: no contract semantics involved
//! ```
//!
//! Behavior:
//! 1. Locate the package the way cargo does (via `cargo metadata`).
//! 2. Scan its `src/` tree: parse every contract (attribute or doc form),
//!    verify `effects(pure)` claims against bodies, enforce dialect
//!    restrictions.
//! 3. Any violation → the build is REFUSED (plain `cargo build` would
//!    have accepted the same code; that difference is the product).
//! 4. Unless `--fast`/`--shallow` was given, the Stage-2 rustc driver
//!    (MIR-level interprocedural effects) rides along as
//!    `RUSTC_WORKSPACE_WRAPPER` — a pure claim that is only locally
//!    clean but reaches an impure call through the call graph is
//!    caught here, not just body-local lies (issue #74: this is the
//!    certifying default now; Stage 1 alone is an opt-in, IDE-speed
//!    hint).
//! 5. Otherwise delegate to the real cargo with the remaining args, and
//!    emit the certificate next to the artifacts:
//!    `<target>/nirdosha/contract-report-<package>.json`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use cargo_nirdosha::{verify_package, ScanSummary, Severity};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // cargo invokes subcommand binaries as `cargo-nirdosha nirdosha <sub>`
    // — the subcommand name rides through as argv[1] (same convention
    // cargo-clippy handles). Strip it; direct invocation is also fine.
    if args.first().map(String::as_str) == Some("nirdosha") {
        args.remove(0);
    }
    let Some(sub) = args.first().cloned() else {
        usage();
        return ExitCode::FAILURE;
    };
    let rest = if args.len() > 1 { &args[1..] } else { &[][..] };

    match sub.as_str() {
        "check-certificate" => check_certificate_cli(rest),
        "keygen" => keygen_cli(rest),
        "verify-certificate" => verify_certificate_cli(rest),
        "ui-proof" => ui_proof_cli(rest),
        "generate-screens" => generate_screens_cli(rest),
        "verify" => {
            if rest.iter().any(|arg| arg == "--guard") {
                return verify_guard_cli(rest);
            }
            let workspace = rest.iter().any(|a| a == "--workspace" || a == "--all");
            // `--provenance` (G6): additionally binds the resolved
            // dependency closure (Cargo.lock) and toolchain into the
            // certificate. Mechanical binding only, not issuer
            // authentication.
            let bind_provenance = rest.iter().any(|a| a == "--provenance");
            // `--sign <key.pk8>` (issue #75 item 2): Ed25519-signs the
            // certificate's binding with `nirdosha keygen`'s private
            // key.
            let sign_key = flag_value(rest, "--sign");
            if workspace {
                return verify_ws_cli(bind_provenance);
            }
            if rest.iter().any(|a| a == "--audit") {
                return audit_cli(rest.iter().find(|a| !a.starts_with("--")).cloned());
            }
            let summary = match verify_cwd(bind_provenance, sign_key.as_deref()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            report(&summary, &[]);
            if summary.violations().is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "bench" => bench_cli(rest),
        s if matches!(s, "build" | "check" | "run" | "test" | "bench" | "doc") => {
            // Stage 2 (the rustc driver, MIR-level interprocedural
            // effects) is the certifying default — see issue #74: a
            // body-local source scan cannot see a pure claim that
            // reaches an impure call three hops down the call graph,
            // and the certificate's headline claim should not be
            // weaker than what the driver can already prove.
            // `--fast`/`--shallow` opts back into Stage-1-only, for
            // sub-second IDE-time feedback with no nightly/rustc-dev
            // requirement. `--deep` is still accepted (and now a
            // no-op) so existing invocations keep working.
            let shallow = rest.iter().any(|a| a == "--fast" || a == "--shallow");
            let deep = !shallow;
            let cargo_args: Vec<String> = rest
                .iter()
                .filter(|a| !matches!(a.as_str(), "--deep" | "--fast" | "--shallow"))
                .cloned()
                .collect();
            // `cargo nirdosha check --workspace` gates every in-dialect
            // crate (strict) before delegating the workspace-wide cargo.
            if cargo_args.iter().any(|a| a == "--workspace" || a == "--all") {
                let exit = verify_ws_cli(false);
                if exit != ExitCode::SUCCESS {
                    eprintln!(
                        "nirdosha: refusing to {s} the workspace — in-dialect crates have violations"
                    );
                    return exit;
                }
                let root = match workspace_root() {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("nirdosha: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                return delegate_with(s, &cargo_args, deep, &root);
            }
            let loc = match locate() {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let summary = match verify_cwd(false, None) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if summary.violations().is_empty() {
                report(&summary, &[]);
                delegate_with(s, &cargo_args, deep, &loc.manifest_dir)
            } else {
                report(&summary, &[s]);
                ExitCode::FAILURE
            }
        }
        s => {
            eprintln!("nirdosha: `{s}` runs as unverified pass-through (no contract semantics)");
            delegate(s, rest)
        }
    }
}

/// `cargo nirdosha verify --guard [--registry-json <path>]`.
///
/// With `--registry-json`: the original, manual mode — read a
/// pre-produced dump straight off disk (CI pipelines that build the dump
/// their own way, or offline inspection of one already written).
///
/// Without it: the dump has to come from *running* the target package —
/// `POLICIES`/`DATASETS`/etc. are `linkme::distributed_slice`s that only
/// populate once the crate declaring them is linked into a live process,
/// so no static analysis of source alone can produce this. Runs `cargo
/// test nirdosha_guard_dump` inside the package at the current directory
/// (via `locate()`, the same "run from inside the package" convention
/// every other subcommand here uses) — the one-line convention documented
/// on `nirdosha_guard_registry::write_dump_from_env` — and reads the JSON
/// it writes. A package with no such test gets a clear error naming the
/// convention, not a confusing "file not found".
fn verify_guard_cli(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--workspace" || a == "--all") {
        return verify_guard_workspace_cli();
    }
    let json = match flag_value(args, "--registry-json") {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("nirdosha: cannot read guard registry {path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => match produce_guard_dump() {
            Ok(value) => value,
            Err(error) => {
                eprintln!("nirdosha: {error}");
                return ExitCode::FAILURE;
            }
        },
    };
    let registry = match nirdosha_guard_verify::RegistryView::from_json(&json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("nirdosha: invalid guard registry: {error}");
            return ExitCode::FAILURE;
        }
    };
    let findings = nirdosha_guard_verify::verify(&registry);
    for finding in &findings {
        eprintln!("{} {:?}: {}", finding.pass, finding.severity, finding.message);
    }
    if findings.is_empty() {
        eprintln!("nirdosha: guard verify — {} polic{} checked, no findings", registry.policies.len(), if registry.policies.len() == 1 { "y" } else { "ies" });
    }
    if findings.iter().any(|finding| finding.severity == nirdosha_guard_verify::Severity::Error) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// `cargo nirdosha verify --guard --workspace`: Gate-3 (Plan Phase 17)
/// cross-crate registry totality. Per-crate `linkme` slices only see what
/// got linked into one compiled binary (`verify_guard_cli`'s own
/// `--registry-json`/`produce_guard_dump` path) — this instead builds the
/// whole workspace through `nirdosha-driver` (`RUSTC_WORKSPACE_WRAPPER`,
/// the same "Stage 2" mechanism `cargo nirdosha build --deep` already
/// uses), with `NIRDOSHA_GATE3_DIR` set so every crate that compiles
/// writes its own real registry fragment (`nirdosha-driver::gate3`,
/// HIR-level, not linker-level), then merges every fragment and reports
/// any policy whose `resource` no `#[dataset]` anywhere in the workspace
/// declares — a gap a single crate's own `linkme` view structurally
/// cannot see, since it never links the crate that would have caught it.
///
/// This checks cross-crate totality specifically, not the full V1–V8
/// pass suite across every package (a distinct, larger undertaking of
/// merging N per-package guard dumps that this mode does not attempt) —
/// stated so the scope isn't misread as broader than it is.
fn verify_guard_workspace_cli() -> ExitCode {
    let Some(driver) = driver_path() else {
        eprintln!(
            "nirdosha: Gate-3 needs the Stage-2 driver next to cargo-nirdosha \
             — run `cargo build -p nirdosha-driver` first (nightly + rustc-dev)"
        );
        return ExitCode::FAILURE;
    };
    let target_dir = match workspace_target_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("nirdosha: {error}");
            return ExitCode::FAILURE;
        }
    };
    let gate3_dir = target_dir.join("nirdosha").join("gate3");
    // Stale fragments from a crate that no longer exists (or no longer
    // declares any policy/dataset) must not linger and be mistaken for
    // current state.
    let _ = std::fs::remove_dir_all(&gate3_dir);
    if let Err(error) = std::fs::create_dir_all(&gate3_dir) {
        eprintln!("nirdosha: cannot create {}: {error}", gate3_dir.display());
        return ExitCode::FAILURE;
    }

    eprintln!("nirdosha: Gate-3 — building the whole workspace through {} to collect per-crate registry fragments", driver.display());
    let status = Command::new("cargo")
        .args(["build", "--workspace"])
        .env("NIRDOSHA_DRIVER", "1")
        .env("RUSTC_WORKSPACE_WRAPPER", &driver)
        .env("NIRDOSHA_GATE3_DIR", &gate3_dir)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("nirdosha: `cargo build --workspace` failed ({status}) — Gate-3 cannot trust a partial fragment set");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("nirdosha: cannot run cargo build --workspace: {error}");
            return ExitCode::FAILURE;
        }
    }

    match gate3_check(&gate3_dir) {
        Ok(findings) => {
            for finding in &findings {
                eprintln!("Gate3 Error: {finding}");
            }
            if findings.is_empty() {
                eprintln!("nirdosha: Gate-3 — no cross-crate registry gaps found");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("nirdosha: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The JSON shape `nirdosha-driver::gate3::Gate3Fragment` writes — kept as
/// an independent reader of that same contract rather than a shared type
/// (see `nirdosha-driver/src/gate3.rs`'s own comment on why: sharing Rust
/// code between a rustc-driver binary and a plain CLI binary would need
/// `nirdosha-driver` to also ship as a library crate for no other reason).
#[derive(serde::Deserialize)]
struct Gate3Fragment {
    crate_name: String,
    #[serde(default)]
    policy_resources: Vec<String>,
    #[serde(default)]
    dataset_entities: Vec<String>,
}

fn gate3_check(dir: &Path) -> Result<Vec<String>, String> {
    let mut all_resources: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_entities: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut resource_owner: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut fragments_read = 0usize;

    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?;
        let fragment: Gate3Fragment = serde_json::from_str(&content).map_err(|e| format!("malformed Gate-3 fragment {}: {e}", entry.path().display()))?;
        fragments_read += 1;
        for resource in &fragment.policy_resources {
            resource_owner.entry(resource.clone()).or_insert_with(|| fragment.crate_name.clone());
        }
        all_resources.extend(fragment.policy_resources);
        all_entities.extend(fragment.dataset_entities);
    }
    eprintln!("nirdosha: Gate-3 — merged {fragments_read} crate fragment(s): {} real policies, {} real datasets", all_resources.len(), all_entities.len());

    Ok(all_resources
        .into_iter()
        .filter(|resource| !all_entities.contains(resource))
        .map(|resource| {
            let owner = resource_owner.get(&resource).map(String::as_str).unwrap_or("?");
            format!("policy in crate {owner:?} references resource {resource:?}, which no #[dataset] anywhere in the workspace declares")
        })
        .collect())
}

/// Runs the target package's `nirdosha_guard_dump` test (see
/// `verify_guard_cli`'s doc comment) and returns the JSON it wrote.
fn produce_guard_dump() -> Result<String, String> {
    let loc = locate()?;
    let dump_dir = loc.target_dir.join("nirdosha");
    std::fs::create_dir_all(&dump_dir).map_err(|e| format!("cannot create {}: {e}", dump_dir.display()))?;
    let dump_path = dump_dir.join("guard-registry.json");
    // A stale file from a previous run must not be mistaken for a fresh
    // one if the test below fails to run at all for some reason other
    // than a clean process exit (e.g. the binary is killed).
    let _ = std::fs::remove_file(&dump_path);

    let manifest = loc.manifest_dir.join("Cargo.toml");
    let status = Command::new("cargo")
        .args(["test", "--manifest-path"])
        .arg(&manifest)
        .args(["nirdosha_guard_dump", "--", "--exact"])
        .env(nirdosha_guard_registry::GUARD_DUMP_PATH_ENV, &dump_path)
        .status()
        .map_err(|e| format!("cannot run `cargo test`: {e}"))?;
    if !status.success() {
        return Err(format!(
            "`cargo test nirdosha_guard_dump` failed in {} — does this package have a\n  #[test] fn nirdosha_guard_dump() {{ nirdosha_guard_registry::write_dump_from_env().unwrap(); }}\nsomewhere in its tests/? (see nirdosha_guard_registry::write_dump_from_env's doc comment)",
            loc.manifest_dir.display()
        ));
    }
    if !dump_path.exists() {
        return Err(format!(
            "cargo test nirdosha_guard_dump ran but {} was never written — is nirdosha_guard_dump actually calling write_dump_from_env()?",
            dump_path.display()
        ));
    }
    std::fs::read_to_string(&dump_path).map_err(|e| format!("cannot read {}: {e}", dump_path.display()))
}

/// Explicit consumer gate, deliberately independent of Cargo metadata.
fn check_certificate_cli(args: &[String]) -> ExitCode {
    let result = (|| -> Result<bool, String> {
        let mut path = None;
        let mut root = None;
        let mut required = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" if root.is_none() => root = Some(PathBuf::from(args.next().ok_or("--root needs a directory")?)),
                "--require" => required.push(args.next().ok_or("--require needs a guarantee")?.clone()),
                value if !value.starts_with('-') && path.is_none() => path = Some(PathBuf::from(value)),
                _ => return Err(format!("unexpected argument `{arg}`")),
            }
        }
        let path = path.ok_or("check-certificate needs a certificate path")?;
        let root = root.ok_or("check-certificate needs --root <package directory>")?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let cert: nirdosha_contract_core::certificate::Certificate =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        nirdosha_contract_core::evidence::check_policy(&cert, &required)?;
        // Refuse paths that could read outside the caller's intended root.
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for source in &cert.sources {
            let relative = Path::new(&source.path);
            if relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err(format!("invalid source path `{}`", source.path));
            }
            let resolved = root.join(relative).canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(&root) { return Err("source escapes package root".into()); }
        }
        if !cert.check_sources(&root).ok() { return Err("listed source files changed or disappeared".into()); }
        // G6: if build_provenance was required, independently re-derive
        // the dependency-closure hash at root and compare — never trust
        // the certificate's own claim without re-checking against the
        // filesystem, same discipline as the source re-hash above.
        let provenance_checked = required.iter().any(|r| r == "build_provenance");
        if provenance_checked {
            nirdosha_contract_core::evidence::check_provenance(&cert, &root)?;
        }
        Ok(provenance_checked)
    })();
    match result {
        Ok(provenance_checked) => {
            if provenance_checked {
                eprintln!("nirdosha: certificate policy passed; listed sources and dependency closure match. Issuer authentication is not established.");
            } else {
                eprintln!("nirdosha: certificate policy passed; listed sources match. Issuer authentication and build provenance are not established.");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nirdosha: certificate policy refused: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `cargo nirdosha keygen [-o <path>]` — issue #75 item 2: generates a
/// real Ed25519 keypair (`nirdosha-audit`'s CSPRNG, not a fixed/test
/// seed — `ring`'s or, under `--features fips`, `aws-lc-rs`'s) for
/// `cargo nirdosha verify --sign`. Writes the private key as raw
/// PKCS#8 DER to `<path>` (default `nirdosha_signing_key.pk8`) —
/// **keep this file secret** — and the base64 public key to
/// `<path>.pub`, the thing you actually distribute/pin. Mirrors the
/// native `.nir` compiler's `nirdosha keygen` (`crates/compiler/src/
/// main.rs::cmd_keygen`) exactly; same backend, same file shapes.
fn keygen_cli(args: &[String]) -> ExitCode {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use nirdosha_audit::crypto_backend::signature::KeyPair;

    let out_path = flag_value(args, "-o").unwrap_or_else(|| "nirdosha_signing_key.pk8".to_string());

    let rng = nirdosha_audit::crypto_backend::rand::SystemRandom::new();
    let pkcs8 = match nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::generate_pkcs8(&rng) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("nirdosha: key generation failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, pkcs8.as_ref()) {
        eprintln!("nirdosha: error writing {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    let keypair = nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .expect("a key this function just generated always parses");
    let public_key_b64 = BASE64_STANDARD.encode(keypair.public_key().as_ref());
    let pub_path = format!("{out_path}.pub");
    if let Err(e) = std::fs::write(&pub_path, format!("{public_key_b64}\n")) {
        eprintln!("nirdosha: error writing {pub_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "private_key_path": out_path,
            "public_key_path": pub_path,
            "public_key": public_key_b64,
            "algorithm": "ed25519",
        }))
        .expect("this JSON value always serializes")
    );
    ExitCode::SUCCESS
}

/// `cargo nirdosha verify-certificate <certificate.json> --public-key
/// <base64>` — issue #75 item 2's verify-side mirror of `verify
/// --sign`: checks the certificate's Ed25519 signature against its own
/// `binding` under the given public key. Says nothing about whether
/// that key is one the caller *should* trust — pinning acceptable keys
/// is the caller's own operational policy, same as the native
/// compiler's `SignedCertificate`.
fn verify_certificate_cli(args: &[String]) -> ExitCode {
    let Some(path) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("nirdosha: usage: cargo nirdosha verify-certificate <certificate.json> --public-key <base64>");
        return ExitCode::FAILURE;
    };
    let Some(public_key) = flag_value(args, "--public-key") else {
        eprintln!("nirdosha: verify-certificate needs --public-key <base64> (see `nirdosha keygen`'s .pub file)");
        return ExitCode::FAILURE;
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("nirdosha: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let cert: nirdosha_contract_core::certificate::Certificate = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nirdosha: {path} is not a valid certificate: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !cert.binding_valid() {
        eprintln!("nirdosha: certificate binding is invalid — this certificate was edited");
        return ExitCode::FAILURE;
    }
    let Some(signature) = &cert.signature else {
        eprintln!("nirdosha: {path} is not signed — missing `signature` (did you mean to run `verify --sign`?)");
        return ExitCode::FAILURE;
    };
    if signature.algorithm != "ed25519" {
        eprintln!("nirdosha: unsupported signature algorithm `{}`", signature.algorithm);
        return ExitCode::FAILURE;
    }
    match nirdosha_audit::signing::verify_bytes(cert.binding.as_bytes(), &public_key, &signature.value) {
        Ok(true) => {
            eprintln!("nirdosha: signature valid — this certificate's binding was signed by the holder of the given public key");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("nirdosha: signature verification FAILED — either a different key signed this certificate, or it was tampered with");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("nirdosha: cannot verify signature: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "cargo-nirdosha — the Nirdosha compiler\n\
         \n\
         usage: cargo nirdosha <build|check|run|test|doc|verify|bench|ui-proof> [args]\n\
         \n\
         Same source, two compilers:\n\
         - plain `cargo build`            compiles and runs Nirdosha code like any Rust program\n\
         - `cargo nirdosha build`         Stage 1 scan + Stage 2 MIR effect lattice; a lie refuses the build\n\
         - `cargo nirdosha build --fast`  Stage 1 only (source scan): sub-second, no nightly/rustc-dev needed\n\
         - `cargo nirdosha verify --workspace`  strict gate over every in-dialect crate + certificate\n\
         - `cargo nirdosha verify --provenance`  also binds Cargo.lock + toolchain into the certificate\n\
         - `cargo nirdosha verify --guard`  runs the package's `nirdosha_guard_dump` test, verifies the result\n\
         - `cargo nirdosha verify --guard --registry-json <path>`  verify a pre-produced dump instead\n\
         - `cargo nirdosha bench`          nfr(latency_ms) CI gate: your test suite is the workload\n\
         - `cargo nirdosha check-certificate <path> --root <package> --require <guarantee>`\n\
         - `cargo nirdosha keygen [-o key.pk8]`   generate an Ed25519 keypair for `verify --sign`\n\
         - `cargo nirdosha verify --sign key.pk8`  sign the certificate (add `--features fips` to build for a CMVP-validatable backend)\n\
         - `cargo nirdosha verify-certificate <path> --public-key <base64>`  check a signed certificate's signature
         - `cargo nirdosha generate-screens <project-dir>`  emit the crate's src/*.nir from screens.toml + menus.toml (v2 register as source of truth)"
    );
}

/// Generate the conservative `.nir/ui-proof.json` skeleton from a project's
/// screen register. This is intentionally inventory-only: it does not invent
/// business transitions or pretend that a declared screen was rendered.
/// `cargo nirdosha generate-screens <project-dir>` — read the project's
/// v2 `screens.toml` + `menus.toml` and emit the crate's `src/*.nir`
/// tree. The screen register becomes a source of truth for code, not an
/// inventory that drifts: every emitted screen is a real macro
/// invocation wired to `GuardedTable`, `guard_policy!` records are
/// synthesized from each guarded screen's own policy block, and the
/// serve binary registers literal routes before `{id}` wildcards.
fn generate_screens_cli(args: &[String]) -> ExitCode {
    let input: Option<String> = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .or_else(|| std::env::var("CARGO_MANIFEST_DIR").ok());
    let Some(project_dir) = input else {
        eprintln!("nirdosha: generate-screens requires a project directory containing screens.toml + menus.toml");
        return ExitCode::FAILURE;
    };
    match cargo_nirdosha::generate::run(Path::new(&project_dir)) {
        Ok(report) => {
            for file in &report.files {
                eprintln!("nirdosha: generated {file}");
            }
            eprintln!(
                "nirdosha: {} screens emitted, {} entities, {} synthesized guard policies",
                report.screens_emitted, report.entities, report.policies
            );
            for skipped in &report.screens_skipped {
                eprintln!("nirdosha: skipped {skipped}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nirdosha: generate-screens refused: {error}");
            ExitCode::FAILURE
        }
    }
}

fn ui_proof_cli(args: &[String]) -> ExitCode {
    let input = flag_value(args, "--register").or_else(|| args.first().filter(|a| !a.starts_with('-')).cloned());
    let Some(input) = input else {
        eprintln!("nirdosha: ui-proof requires --register <screens.toml> [--output <.nir/ui-proof.json>]");
        return ExitCode::FAILURE;
    };
    let source = match std::fs::read_to_string(&input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("nirdosha: cannot read {input}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let spec = match nirdosha_contract_core::ui_assurance::generate_from_screen_register(&source) {
        Ok(spec) => spec,
        Err(error) => {
            eprintln!("nirdosha: cannot generate UI proof: {error}");
            return ExitCode::FAILURE;
        }
    };
    let proof = nirdosha_contract_core::ui_assurance::verify(&spec);
    let output = flag_value(args, "--output").unwrap_or_else(|| ".nir/ui-proof.json".into());
    if let Some(parent) = Path::new(&output).parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("nirdosha: cannot create {}: {error}", parent.display());
            return ExitCode::FAILURE;
        }
    }
    if let Err(error) = std::fs::write(&output, serde_json::to_vec_pretty(&spec).expect("UI proof is serializable")) {
        eprintln!("nirdosha: cannot write {output}: {error}");
        return ExitCode::FAILURE;
    }
    eprintln!("nirdosha: generated conservative UI proof skeleton: {} screens, {} structural states, {} actions → {output}", proof.screens, proof.states, proof.actions);
    eprintln!("nirdosha: this proves inventory structure only; add real transitions, postconditions, and browser traces before release");
    ExitCode::SUCCESS
}

struct Location {
    manifest_dir: PathBuf,
    target_dir: PathBuf,
}

fn locate() -> Result<Location, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed — is this a cargo workspace?".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let target_dir = meta
        .get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or("cargo metadata has no target_directory")?;
    let packages = meta
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or("cargo metadata has no packages")?;
    for package in packages {
        let Some(manifest) = package.get("manifest_path").and_then(|v| v.as_str()) else {
            continue;
        };
        let manifest = Path::new(manifest);
        if manifest.parent() == Some(cwd.as_path()) {
            return Ok(Location {
                manifest_dir: manifest.parent().unwrap().to_path_buf(),
                target_dir,
            });
        }
    }
    Err(format!(
        "cwd {} is not a package root — run cargo-nirdosha from inside the package you want verified",
        cwd.display()
    ))
}

/// Finds `--flag <value>` in `args` and returns `value`, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn verify_cwd(bind_provenance: bool, sign_key: Option<&str>) -> Result<ScanSummary, String> {
    let loc = locate()?;
    let strict = std::env::var_os("NIRDOSHA_STRICT").is_some_and(|v| !v.is_empty());
    let summary = verify_package(&loc.manifest_dir, strict)?;
    let report_path = match sign_key {
        Some(key) => summary.write_report_signed(&loc.target_dir, bind_provenance, key),
        None => summary.write_report(&loc.target_dir, bind_provenance),
    }
    .map_err(|e| format!("cannot write certificate: {e}"))?;
    eprintln!(
        "nirdosha: certificate → {}",
        report_path.display()
    );
    // Issue #75 item 1: the guarantee bundle travels with every build,
    // not just an explicit `verify` invocation — every path here
    // (`build`/`check`/`run`/`test`/`doc`/`bench`/`verify`) funnels
    // through this function.
    let cert_bytes = std::fs::read(&report_path).map_err(|e| format!("cannot read certificate: {e}"))?;
    let cert: nirdosha_contract_core::certificate::Certificate =
        serde_json::from_slice(&cert_bytes).map_err(|e| format!("cannot parse certificate: {e}"))?;
    let bundle_path = summary
        .write_guarantee_bundle(&loc.target_dir, &report_path, &cert.binding)
        .map_err(|e| format!("cannot write guarantee bundle: {e}"))?;
    eprintln!("nirdosha: guarantee bundle → {}", bundle_path.display());
    Ok(summary)
}

/// `cargo nirdosha verify --audit [path]` — re-check a certificate
/// against the sources it attests to: is the binding intact (nobody
/// edited the claims), and do the source hashes still match (nobody
/// edited the code since it was verified)? Exit 0 only when both hold.
fn audit_cli(path: Option<String>) -> ExitCode {
    let loc = match locate() {
        Ok(loc) => loc,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    let path = path.unwrap_or_else(|| {
        loc.target_dir
            .join("nirdosha")
            .join("contract-report-<unknown-package>.json")
            .to_string_lossy()
            .into_owned()
    });
    let path = if path.contains("<unknown-package>") {
        // no explicit path: <target>/nirdosha/contract-report-<pkg>.json,
        // pkg from the manifest in the cwd
        let package = std::fs::read_to_string(loc.manifest_dir.join("Cargo.toml"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| {
                let l = l.trim();
                l.starts_with("name")
                    .then(|| l.split('"').nth(1).map(str::to_string))
                    .flatten()
            }))
            .unwrap_or_default();
        loc.target_dir
            .join("nirdosha")
            .join(format!("contract-report-{package}.json"))
    } else {
        std::path::PathBuf::from(path)
    };
    let cert: nirdosha_contract_core::certificate::Certificate =
        match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(cert) => cert,
            None => {
                eprintln!(
                    "nirdosha: cannot read certificate {} — run `cargo nirdosha verify` first",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        };
    let mut ok = true;
    if cert.binding_valid() {
        eprintln!("nirdosha: audit binding OK — content matches its stored hash ({})", cert.binding);
    } else {
        eprintln!("nirdosha: audit FAILED — binding mismatch; this certificate was edited");
        ok = false;
    }
    let audit = cert.check_sources(&loc.manifest_dir);
    if audit.ok() {
        eprintln!(
            "nirdosha: audit sources OK — {} file(s) match their verified hashes",
            audit.matched
        );
    } else {
        for path in &audit.changed {
            eprintln!("nirdosha: audit FAILED — {path} changed since it was verified");
        }
        for path in &audit.missing {
            eprintln!("nirdosha: audit FAILED — {path} is missing since it was verified");
        }
        ok = false;
    }
    if ok {
        eprintln!("nirdosha: listed source hashes and certificate binding match; this does not authenticate the issuer or establish build provenance");
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "nirdosha: refusing to trust this certificate — plain `cargo build` would have accepted this code; that difference is the product"
        );
        ExitCode::FAILURE
    }
}

fn report(summary: &ScanSummary, refused_for: &[&str]) {
    for finding in &summary.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let at = finding
            .function
            .as_ref()
            .map(|f| format!(" fn `{f}`"))
            .unwrap_or_default();
        eprintln!(
            "nirdosha: {severity} {}:{}{at} — {}",
            finding.file.display(),
            finding.line,
            finding.message
        );
    }
    eprintln!(
        "nirdosha: {} — source_scan: {} files, {} contracts inspected, {} violations",
        summary.package,
        summary.files.len(),
        summary.contracts.len(),
        summary.violations().len()
    );
    if !summary.violations().is_empty() && !refused_for.is_empty() {
        eprintln!(
            "nirdosha: refusing to {} — plain `cargo {}` would have accepted this code; that difference is the product",
            refused_for[0],
            refused_for[0]
        );
    }
}

/// The workspace's target directory (works from the root, unlike
/// `locate`, which demands a package cwd).
fn workspace_target_dir() -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed — is this a cargo workspace?".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    meta.get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata has no target_directory".to_string())
}

/// The workspace root — where `nirdosha-driver`'s own pack-check
/// (issue #76) looks for `.nir/hi.db`/`.nir/plugins` when gating a
/// `--workspace` build, the same convention `nirdosha-hi` uses for a
/// single-project root.
fn workspace_root() -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed — is this a cargo workspace?".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    meta.get("workspace_root")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata has no workspace_root".to_string())
}

/// `cargo nirdosha verify --workspace`: verify every in-dialect crate,
/// strict by default, plus the aggregate certificate.
fn verify_ws_cli(bind_provenance: bool) -> ExitCode {
    match cargo_nirdosha::verify_workspace(true) {
        Ok(ws) => {
            let target_dir = match workspace_target_dir() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match ws.write_reports(&target_dir, bind_provenance) {
                Ok(path) => eprintln!("nirdosha: workspace certificate → {}", path.display()),
                Err(e) => {
                    eprintln!("nirdosha: cannot write certificate: {e}");
                    return ExitCode::FAILURE;
                }
            }
            for summary in &ws.packages {
                for finding in &summary.findings {
                    let severity = match finding.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    };
                    eprintln!(
                        "nirdosha: {severity} [{}] {}:{} — {}",
                        summary.package,
                        finding.file.display(),
                        finding.line,
                        finding.message
                    );
                }
            }
            eprintln!(
                "nirdosha: workspace source_scan — {} in-dialect package(s) scanned, {} contracts, {} violations",
                ws.packages.len(),
                ws.contracts(),
                ws.violations()
            );
            if ws.violations() == 0 {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "nirdosha: plain `cargo build` would have accepted all of it; that difference is the product"
                );
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("nirdosha: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cargo nirdosha bench [test args…]`: the nfr(latency_ms) CI gate.
/// Verifies first, then runs the package's own test suite as the load
/// generator, with the flight recorder's file sink pointed at
/// target/nirdosha/. The p95 per guarded function is compared against
/// the declared limit. Exit nonzero on any breach — or on any verify
/// failure, so a lying program never gets a bench verdict.
fn bench_cli(rest: &[String]) -> ExitCode {
    let summary = match verify_cwd(false, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !summary.violations().is_empty() {
        report(&summary, &["bench"]);
        return ExitCode::FAILURE;
    }
    let loc = match locate() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    let dir = loc.target_dir.join("nirdosha");
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("nirdosha: cannot create {}", dir.display());
        return ExitCode::FAILURE;
    }
    let sink = dir.join(format!("nfr-events-{}.ndjson", summary.package));
    let _ = std::fs::remove_file(&sink);

    eprintln!(
        "nirdosha: bench — running `cargo test` as the workload (flight recorder → {})",
        sink.display()
    );
    let status = Command::new("cargo")
        .arg("test")
        .args(rest)
        .env("NIRDOSHA_NFR_LOG_FILE", &sink)
        .status();
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo test: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !status.success() {
        return ExitCode::from(status.code().unwrap_or(1) as u8);
    }

    let events = cargo_nirdosha::read_bench_events(&sink);
    let verdicts = cargo_nirdosha::evaluate_bench(&events, &summary.contracts);
    let mut breaches = 0;
    let mut unmeasured = 0;
    for v in &verdicts {
        let Some(limit) = v.limit_ms else { continue };
        let verdict = if v.ok { "PASS" } else { "FAIL" };
        if !v.ok {
            breaches += 1;
        }
        eprintln!(
            "nirdosha: bench {verdict} fn `{}` — {} call(s), p50 {:.3}ms, p95 {:.3}ms, max {:.3}ms (limit {}ms)",
            v.function, v.calls, v.p50_ms, v.p95_ms, v.max_ms, limit
        );
    }
    for contract in &summary.contracts {
        if contract.contract.nfr.is_some_and(|n| n.latency_ms.is_some())
            && verdicts
                .iter()
                .find(|v| v.function == contract.function)
                .is_some_and(|v| v.calls == 0)
        {
            unmeasured += 1;
            eprintln!(
                "nirdosha: bench WARN fn `{}` declares nfr(latency_ms) but the workload never called it",
                contract.function
            );
        }
    }
    let report = serde_json::json!({
        "tool": "cargo-nirdosha",
        "dialect": "nirdosha-rt",
        "mode": "bench",
        "package": summary.package,
        "events": events.len(),
        "verdicts": verdicts,
    });
    let path = dir.join(format!("bench-report-{}.json", summary.package));
    if let Ok(json) = serde_json::to_vec_pretty(&report) {
        let _ = std::fs::write(&path, json);
        eprintln!("nirdosha: bench report → {}", path.display());
    }
    if breaches > 0 {
        eprintln!("nirdosha: bench — {breaches} SLA breach(es); refusing");
        ExitCode::FAILURE
    } else {
        eprintln!(
            "nirdosha: bench — {} SLA(s) held, {unmeasured} unmeasured",
            verdicts
                .iter()
                .filter(|v| v.limit_ms.is_some())
                .count()
        );
        if unmeasured > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

/// The Stage-2 driver binary, expected next to this executable (same
/// target dir). Build it with `cargo build -p nirdosha-driver`.
fn driver_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("nirdosha-driver");
    dir.is_file().then_some(dir)
}

/// Delegate to cargo, optionally with the Stage-2 rustc driver attached
/// as RUSTC_WORKSPACE_WRAPPER. `deep` is true unless `--fast`/`--shallow`
/// was given (issue #74: Stage 2 is the certifying default). `package_root`
/// is where the driver's own pack-check (issue #76) looks for
/// `.nir/hi.db`/`.nir/plugins` — the same root `cargo-nirdosha`'s own
/// Stage 1 pack wiring (`check_pack_invariants`) already uses.
fn delegate_with(sub: &str, rest: &[String], deep: bool, package_root: &Path) -> ExitCode {
    let mut command = Command::new("cargo");
    command.arg(sub).args(rest);
    if deep {
        let Some(driver) = driver_path() else {
            eprintln!(
                "nirdosha: the Stage-2 driver is not built next to cargo-nirdosha \
                 — run `cargo build -p nirdosha-driver` first (nightly + rustc-dev), \
                 or pass `--fast`/`--shallow` for the weaker, Stage-1-only source scan"
            );
            return ExitCode::FAILURE;
        };
        eprintln!(
            "nirdosha: deep verification active — MIR effects lattice via {}",
            driver.display()
        );
        command.env("NIRDOSHA_DRIVER", "1");
        command.env("RUSTC_WORKSPACE_WRAPPER", &driver);
        command.env("NIRDOSHA_PACKAGE_ROOT", package_root);
    }
    match command.status() {
        Ok(status) => status
            .code()
            .map(|code| ExitCode::from(code as u8))
            .unwrap_or(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo {sub}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn delegate(sub: &str, rest: &[String]) -> ExitCode {
    match Command::new("cargo").arg(sub).args(rest).status() {
        Ok(status) => status
            .code()
            .map(|code| ExitCode::from(code as u8))
            .unwrap_or(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo {sub}: {e}");
            ExitCode::FAILURE
        }
    }
}
