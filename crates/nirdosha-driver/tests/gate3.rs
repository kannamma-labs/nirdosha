//! Plan Phase 17's own stated verification target: "a two-crate fixture
//! ... where a policy in crate A references a dataset only declared in
//! crate B — Gate-3 catches the gap; per-crate V1 cannot." Builds a real,
//! tiny two-crate Cargo workspace (outside this repo's own workspace,
//! under a temp dir) through the real `nirdosha-driver` binary
//! (`RUSTC_WORKSPACE_WRAPPER`, the same mechanism `cargo nirdosha verify
//! --guard --workspace` uses), then proves the merged Gate-3 fragments
//! catch the gap a single crate's own `linkme` dump structurally cannot.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("nir-gate3-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn guard_registry_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("nirdosha-guard-registry")
}

fn guard_macros_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("nirdosha-guard-macros")
}

fn guard_core_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("nirdosha-guard-core")
}

fn nirdosha_audit_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("nirdosha-audit")
}

/// Writes a tiny two-crate Cargo workspace: `crate-a` declares a real
/// `guard_policy!` referencing `orphan_resource`; `crate-b` optionally
/// declares a real `#[dataset(entity = "orphan_resource", ...)]` for it
/// (`with_matching_dataset` controls whether the gap actually exists).
fn write_fixture(root: &Path, with_matching_dataset: bool) {
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[workspace]
resolver = "2"
members = ["crate-a", "crate-b"]
"#
        ),
    )
    .unwrap();

    let dep_lines = |extra: &str| {
        format!(
            "nirdosha-guard-registry = {{ path = {registry:?} }}\nnirdosha-guard-macros = {{ path = {macros:?} }}\nnirdosha-guard-core = {{ path = {core:?} }}\nnirdosha-audit = {{ path = {audit:?} }}\n{extra}",
            registry = guard_registry_dir(),
            macros = guard_macros_dir(),
            core = guard_core_dir(),
            audit = nirdosha_audit_dir(),
        )
    };

    std::fs::create_dir_all(root.join("crate-a/src")).unwrap();
    std::fs::write(
        root.join("crate-a/Cargo.toml"),
        format!("[package]\nname = \"crate-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{}", dep_lines("")),
    )
    .unwrap();
    std::fs::write(
        root.join("crate-a/src/lib.rs"),
        r#"nirdosha_guard_macros::guard_policy! {
    allow "orphan-policy" when action == "read" && resource == "orphan_resource"
}
"#,
    )
    .unwrap();

    std::fs::create_dir_all(root.join("crate-b/src")).unwrap();
    std::fs::write(
        root.join("crate-b/Cargo.toml"),
        format!("[package]\nname = \"crate-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{}", dep_lines("")),
    )
    .unwrap();
    let dataset_item = if with_matching_dataset {
        r#"#[nirdosha_guard_macros::dataset(entity = "orphan_resource", store = "test")]
pub struct OrphanResource { pub id: String }
"#
    } else {
        "// deliberately no #[dataset] here for this variant\n"
    };
    std::fs::write(root.join("crate-b/src/lib.rs"), dataset_item).unwrap();
}

/// Builds the fixture workspace through the real `nirdosha-driver`,
/// collecting Gate-3 fragments into `gate3_dir`.
fn build_through_driver(root: &Path, gate3_dir: &Path) -> std::process::Output {
    std::fs::create_dir_all(gate3_dir).unwrap();
    Command::new("cargo")
        .arg("build")
        .current_dir(root)
        .env("NIRDOSHA_DRIVER", "1")
        .env("RUSTC_WORKSPACE_WRAPPER", env!("CARGO_BIN_EXE_nirdosha-driver"))
        .env("NIRDOSHA_GATE3_DIR", gate3_dir)
        .output()
        .expect("cargo build must run")
}

fn merged_gate3_findings(gate3_dir: &Path) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Fragment {
        crate_name: String,
        #[serde(default)]
        policy_resources: Vec<String>,
        #[serde(default)]
        dataset_entities: Vec<String>,
    }
    let mut resources = std::collections::BTreeSet::new();
    let mut entities = std::collections::BTreeSet::new();
    let mut owner = std::collections::HashMap::new();
    for entry in std::fs::read_dir(gate3_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).unwrap();
        let fragment: Fragment = serde_json::from_str(&content).unwrap_or_else(|e| panic!("malformed fragment {:?}: {e}\n{content}", entry.path()));
        for resource in &fragment.policy_resources {
            owner.entry(resource.clone()).or_insert_with(|| fragment.crate_name.clone());
        }
        resources.extend(fragment.policy_resources);
        entities.extend(fragment.dataset_entities);
    }
    resources
        .into_iter()
        .filter(|resource| !entities.contains(resource))
        .map(|resource| format!("{}: {resource}", owner.get(&resource).cloned().unwrap_or_default()))
        .collect()
}

#[test]
fn gate3_catches_a_policy_resource_with_no_matching_dataset_across_crates() {
    let root = scratch_dir("gap");
    write_fixture(&root, false);
    let gate3_dir = root.join("gate3-fragments");
    let output = build_through_driver(&root, &gate3_dir);
    assert!(output.status.success(), "fixture build must succeed: {}", String::from_utf8_lossy(&output.stderr));

    let findings = merged_gate3_findings(&gate3_dir);
    assert_eq!(findings.len(), 1, "expected exactly one cross-crate gap, got {findings:?}");
    assert!(findings[0].starts_with("crate_a:"), "the gap must be attributed to crate-a, the one that declared the policy: {findings:?}");
    assert!(findings[0].contains("orphan_resource"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gate3_finds_no_gap_once_the_sibling_crate_declares_the_matching_dataset() {
    let root = scratch_dir("nogap");
    write_fixture(&root, true);
    let gate3_dir = root.join("gate3-fragments");
    let output = build_through_driver(&root, &gate3_dir);
    assert!(output.status.success(), "fixture build must succeed: {}", String::from_utf8_lossy(&output.stderr));

    let findings = merged_gate3_findings(&gate3_dir);
    assert!(findings.is_empty(), "crate-b's real #[dataset] must resolve the gap: {findings:?}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_single_crates_own_fragment_alone_cannot_see_the_gap_is_resolved() {
    // The plan's own point: per-crate V1 (a single crate's linkme dump)
    // cannot see this at all — proven here by checking crate-a's
    // fragment ALONE still looks like a gap, even in the
    // dataset-present fixture, because crate-a's own compilation never
    // sees crate-b's registration.
    let root = scratch_dir("percrate");
    write_fixture(&root, true);
    let gate3_dir = root.join("gate3-fragments");
    let output = build_through_driver(&root, &gate3_dir);
    assert!(output.status.success());

    let crate_a_fragment = std::fs::read_to_string(gate3_dir.join("crate_a.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&crate_a_fragment).unwrap();
    assert_eq!(parsed["dataset_entities"].as_array().unwrap().len(), 0, "crate-a's own fragment alone has no dataset_entities at all — it never declared any");
    assert_eq!(parsed["policy_resources"].as_array().unwrap(), &["orphan_resource"]);

    let _ = std::fs::remove_dir_all(&root);
}
