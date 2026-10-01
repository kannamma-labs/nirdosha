//! Issue #75 item 2: "the dialect certificate's `signature` field is
//! ... unimplemented. `cargo-nirdosha` has no signing path at all
//! today." This test fails against that prior behavior (`verify
//! --sign`/`verify-certificate`/`keygen` didn't exist as subcommands at
//! all) and passes once certificate signing is wired through
//! `nirdosha-audit`, the same Ed25519/SHA-256 backend the native `.nir`
//! compiler's certificate/pack signing already uses.
//!
//! **Isolation note**: every test that calls `cargo-nirdosha verify` writes a
//! certificate into the project's `.nir/` directory. To prevent the two tests
//! in this file from racing on that shared state when run in parallel, each
//! test gets its own isolated copy of the `rt-payroll` example via
//! `isolated_workdir()`. The copy excludes the `.nir/` sub-directory so the
//! signing test cannot "infect" the unsigned test's working tree.

use std::path::PathBuf;
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-nirdosha"))
}

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("rt-payroll")
}

/// Copy the rt-payroll example into a unique temp directory (excluding any
/// pre-existing `.nir/` output directory) so that parallel test runs can each
/// call `cargo-nirdosha verify` without sharing certificate state.
fn isolated_workdir() -> PathBuf {
    let dest = std::env::temp_dir().join(format!(
        "nir-cert-test-{}-{}",
        std::process::id(),
        // Unique counter so two tests in the same process don't collide.
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            N.fetch_add(1, Ordering::Relaxed)
        }
    ));
    copy_dir_except_nir(&example_dir(), &dest);
    dest
}

fn copy_dir_except_nir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create dest dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name();
        if name == ".nir" {
            continue; // never copy stale certificate state
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            copy_dir_except_nir(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).expect("copy file");
        }
    }
}

fn keygen(dir: &std::path::Path, name: &str) -> (PathBuf, String) {
    let key_path = dir.join(name);
    let out = bin().arg("keygen").arg("-o").arg(&key_path).output().unwrap();
    assert!(out.status.success(), "keygen failed: {}", String::from_utf8_lossy(&out.stderr));
    let pub_path = dir.join(format!("{name}.pub"));
    assert!(key_path.is_file(), "private key was not written");
    assert!(pub_path.is_file(), "public key was not written");
    let public_key = std::fs::read_to_string(&pub_path).unwrap().trim().to_string();
    (key_path, public_key)
}

fn verify_certificate(cert_path: &str, public_key: &str) -> Output {
    bin()
        .arg("verify-certificate")
        .arg(cert_path)
        .arg("--public-key")
        .arg(public_key)
        .output()
        .unwrap()
}

#[test]
fn sign_then_verify_round_trips_and_rejects_the_wrong_key() {
    let workdir = isolated_workdir();
    let tmp = std::env::temp_dir().join(format!("nir-sign-keys-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (key_path, public_key) = keygen(&tmp, "key.pk8");
    let (_other_key_path, other_public_key) = keygen(&tmp, "other_key.pk8");

    // `verify --sign` mints a signed certificate.
    let out = bin()
        .current_dir(&workdir)
        .arg("verify")
        .arg("--sign")
        .arg(&key_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "signed verify failed: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let cert_path = stderr
        .lines()
        .find(|l| l.contains("certificate →"))
        .and_then(|l| l.rsplit("→ ").next())
        .expect("certificate path printed")
        .trim()
        .to_string();

    let cert: serde_json::Value = serde_json::from_slice(&std::fs::read(&cert_path).unwrap()).unwrap();
    assert_eq!(cert["signature"]["algorithm"], "ed25519");
    assert_eq!(cert["signature"]["key_id"], public_key);
    assert!(cert["signature"]["value"].as_str().is_some_and(|v| !v.is_empty()));

    // The right key verifies.
    let result = verify_certificate(&cert_path, &public_key);
    assert!(result.status.success(), "verification with the signing key must pass: {}", String::from_utf8_lossy(&result.stderr));

    // A different, unrelated key must not verify.
    let result = verify_certificate(&cert_path, &other_public_key);
    assert!(!result.status.success(), "verification with the wrong key must fail");
    assert!(String::from_utf8_lossy(&result.stderr).contains("FAILED"));

    std::fs::remove_dir_all(&tmp).ok();
    std::fs::remove_dir_all(&workdir).ok();
}

#[test]
fn an_unsigned_certificate_has_no_signature_to_verify() {
    // Run in an isolated copy so a concurrently-running sign test cannot
    // leave a signed certificate in the shared rt-payroll/.nir/ directory
    // and cause this test to observe a cert that already has a signature.
    let workdir = isolated_workdir();

    let out = bin().current_dir(&workdir).arg("verify").output().unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let cert_path = stderr
        .lines()
        .find(|l| l.contains("certificate →"))
        .and_then(|l| l.rsplit("→ ").next())
        .expect("certificate path printed")
        .trim()
        .to_string();

    let result = verify_certificate(&cert_path, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("not signed"),
        "expected 'not signed' in stderr, got: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    std::fs::remove_dir_all(&workdir).ok();
}
