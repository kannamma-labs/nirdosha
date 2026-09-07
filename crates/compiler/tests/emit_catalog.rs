//! Tests for `nirdosha emit-catalog` (rfcs/0009 Phase 0): the CLI half
//! (`cmd_emit_catalog` in `main.rs`), exercised by spawning the real
//! `nirdosha` binary -- the same `std::process::Command` pattern
//! `crates/compiler/tests/init.rs` already uses for `nirdosha init`.
//! There is no separate "generator half" to test in isolation the way
//! `init.rs` tests `generate_source` directly: `cmd_emit_catalog` only
//! parses and re-serializes a compile-time-embedded file, so the CLI
//! test below *is* the whole behavior.

fn unique_suffix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_emit_catalog_test_{}_{}_{}", std::process::id(), unique_suffix(), name));
    p
}

#[test]
fn prints_valid_json_with_the_expected_top_level_sections() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-catalog"])
        .output()
        .expect("nirdosha emit-catalog should run");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("emit-catalog should print valid JSON");

    // The sections a Phase A/B implementation, or an external reader,
    // would actually key off -- not an exhaustive echo of every key in
    // catalog/std/0.1.json (that would just re-encode the file's own
    // shape as a second, parallel source of truth this test has to keep
    // in sync by hand).
    for section in ["layout", "fieldSpec", "controls", "action", "charts", "panelRenders", "themeTokens", "extensionPoints"] {
        assert!(value.get(section).is_some(), "catalog is missing top-level section `{section}`: {value}");
    }

    // The 7+1 controls crates/compiler/UI_DSL_TODO.md's own tracker
    // counts -- pinned here so a future edit to ui_gen.rs's control
    // inference (build_field) that isn't mirrored into the catalog file
    // fails a test instead of silently drifting.
    let controls = value["controls"].as_object().expect("controls should be an object");
    for control in ["text", "date", "number", "checkbox", "select", "searchable_select", "struct", "readonly"] {
        assert!(controls.contains_key(control), "controls is missing `{control}`");
    }

    // The 4-name chart vocabulary MetricRender/PanelRender share
    // (typeck::check_visual_render_expr's closed set).
    let charts = value["charts"].as_object().expect("charts should be an object");
    for chart in ["bar_chart", "graph", "heatmap", "timeline"] {
        assert!(charts.contains_key(chart), "charts is missing `{chart}`");
    }
}

#[test]
fn rejects_unknown_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-catalog", "--bogus"])
        .output()
        .expect("nirdosha emit-catalog should run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: nirdosha emit-catalog"));
}

#[test]
fn dash_o_writes_the_same_json_to_a_file_instead_of_stdout() {
    let out_path = scratch_path("catalog.json");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(["emit-catalog", "-o"])
        .arg(&out_path)
        .output()
        .expect("nirdosha emit-catalog -o should run");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("wrote"));

    let written = std::fs::read_to_string(&out_path).expect("output file should exist");
    let value: serde_json::Value = serde_json::from_str(&written).expect("written file should be valid JSON");
    assert_eq!(value["$id"], "nirdosha://catalog/std/0.1");

    std::fs::remove_file(&out_path).ok();
}
