//! The real, no-hand-assembly proof for `ui_plugin::discover_components`
//! (rfcs/0009 Phase B's Cargo-metadata-driven discovery): a scratch
//! project whose own `Cargo.toml` depends on `crates/ui-plugin-example-
//! sparkline` by path -- no Rust code written for the occasion, no
//! `[dev-dependencies]` edge from `crates/compiler` itself -- fed
//! straight to the real, compiled `nirdosha emit-ui` binary. Same
//! "spawn the real CLI, don't call library functions directly" posture
//! `tests/init.rs`'s CLI tests already use, since this is specifically
//! proving the CLI's own `--manifest-path`/auto-detection wiring, not
//! just the library function `tests/ui_plugin_examples.rs` already
//! covers directly.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // `crates/compiler` -> up two levels -> the repo root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn unique_suffix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

fn scratch_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_ui_plugin_discovery_test_{}_{}_{}", std::process::id(), unique_suffix(), name));
    p
}

/// A scratch project depending on the real sparkline crate by an
/// absolute path -- `cargo metadata` (what `discover_components`
/// actually shells out to) resolves this exactly like it would resolve
/// a real crates.io/registry dependency, no network access needed.
fn write_scratch_project(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).expect("create scratch dir");
    let sparkline_crate_path = workspace_root().join("crates/ui-plugin-example-sparkline");
    let cargo_toml = format!(
        r#"[package]
name = "scratch-app"
version = "0.1.0"
edition = "2021"

[dependencies]
nirdosha-ui-plugin-example-sparkline = {{ path = {:?} }}
"#,
        sparkline_crate_path.canonicalize().expect("sparkline crate path should exist").display().to_string()
    );
    std::fs::write(dir.join("Cargo.toml"), cargo_toml).expect("write scratch Cargo.toml");
    // `cargo metadata` validates the manifest fully, including that it
    // has a real target -- an app author's actual project would have a
    // real `src/`; this scratch one just needs `cargo metadata` to
    // accept it at all. Nothing here is ever built, only resolved.
    std::fs::create_dir_all(dir.join("src")).expect("create scratch src dir");
    std::fs::write(dir.join("src/lib.rs"), "").expect("write scratch src/lib.rs");

    let nir_src = r#"
        struct Report {
            id: i64,
        }
        fn recent_sales_totals() -> Result(json, i64) requires(public) {
            return match json_parse("[]") { Ok(v) => Ok(v), Err(e) => Err(0), }
        }
        fn list_report() -> Result(json, i64) requires(public) {
            return match json_parse("[]") { Ok(v) => Ok(v), Err(e) => Err(0), }
        }

        screen Report {
            layout {
                column {
                    sparkline { source: recent_sales_totals field: "amount" }
                }
            }
        }

        fn main() {}
    "#;
    let nir_path = dir.join("app.nir");
    std::fs::write(&nir_path, nir_src).expect("write scratch .nir file");
    nir_path
}

#[test]
fn emit_ui_auto_detects_cargo_toml_next_to_the_nir_file_and_links_the_real_component() {
    let dir = scratch_dir("auto_detect");
    let nir_path = write_scratch_project(&dir);
    let out_path = dir.join("out.html");

    // No `--manifest-path` at all -- relies entirely on
    // `resolve_ui_component_manifest`'s auto-detection of `Cargo.toml`
    // sitting next to `app.nir`.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-ui"])
        .arg(&nir_path)
        .arg("-o")
        .arg(&out_path)
        .output()
        .expect("nirdosha emit-ui should run");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let html = std::fs::read_to_string(&out_path).expect("output file should exist");
    assert!(html.contains("function render_sparkline(node)"), "the real crate's JS should have been discovered and spliced in");
    assert!(html.contains(r#"WIDGET_RENDERERS["sparkline"] = render_sparkline;"#));
    assert!(html.contains(r#""kind":"sparkline""#));
    assert!(html.contains(r#""source":"recent_sales_totals""#));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emit_ui_manifest_path_flag_works_when_the_nir_file_lives_elsewhere() {
    let project_dir = scratch_dir("explicit_manifest_project");
    let nir_dir = scratch_dir("explicit_manifest_nir");
    std::fs::create_dir_all(&nir_dir).expect("create nir dir");
    write_scratch_project(&project_dir);
    // Move just the .nir file away from its Cargo.toml -- proves
    // `--manifest-path` itself does the linking, not proximity.
    let nir_path = nir_dir.join("app.nir");
    std::fs::rename(project_dir.join("app.nir"), &nir_path).expect("move .nir file");
    let out_path = nir_dir.join("out.html");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-ui"])
        .arg(&nir_path)
        .arg("-o")
        .arg(&out_path)
        .arg("--manifest-path")
        .arg(project_dir.join("Cargo.toml"))
        .output()
        .expect("nirdosha emit-ui should run");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let html = std::fs::read_to_string(&out_path).expect("output file should exist");
    assert!(html.contains("function render_sparkline(node)"));

    std::fs::remove_dir_all(&project_dir).ok();
    std::fs::remove_dir_all(&nir_dir).ok();
}

#[test]
fn without_a_manifest_at_all_sparkline_is_rejected_exactly_as_before_this_rfc() {
    // No Cargo.toml anywhere near the .nir file, no --manifest-path --
    // `sparkline` is still just an unknown widget kind, same
    // UnknownRenderValue a plain `nirdosha` build would give before
    // any of rfcs/0009 Phase B existed.
    let dir = scratch_dir("no_manifest");
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let nir_path = dir.join("app.nir");
    std::fs::write(
        &nir_path,
        r#"
        struct Report { id: i64 }
        screen Report {
            layout {
                column {
                    sparkline { field: "amount" }
                }
            }
        }
        fn main() {}
    "#,
    )
    .expect("write .nir file");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-ui"])
        .arg(&nir_path)
        .output()
        .expect("nirdosha emit-ui should run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("sparkline"));

    std::fs::remove_dir_all(&dir).ok();
}
