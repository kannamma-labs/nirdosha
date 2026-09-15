//! RFC 0017 "Security Guarantee Manifests" -- Phase 5's chosen failure
//! class (`docs/research/2026-09-pending-verification-differentiation-
//! work.md`'s own framework: "pick one more failure class nobody
//! solves well for LLM-generated code ... build a real minimal
//! version"). The gap this closes is the one named in `docs/research/
//! 2026-09-generated-code-guarantee-evaluation.md`: this project can
//! verify/certify a `.nir` **source** file, but had no checkable
//! artifact that travels with the **generated** code -- a reviewer
//! handed only a binary had no Nirdosha-provided way to ask "does this
//! still satisfy the guarantees the source claimed?"
//!
//! **This is the real minimal version, not the RFC's full design --
//! disclosed, not hidden.** RFC 0017 §3 lists six `guarantee_check`
//! checks; three are built here:
//! 1. Capability ceiling (`check`'s effect-set comparison).
//! 3. Exported contract check (`requires`/`public` match).
//! 5. Vocabulary check (`role_vocabulary`/`authorization_bearing_claims`).
//!
//! Two are real, separate follow-up work, not attempted here:
//! 2. Resource-budget static bounding needs call-graph high-water-mark
//!    analysis this pass doesn't build.
//! 4. Network/file literal-allowlist checking needs codegen-level
//!    visibility into `connect`/`open` call sites this pass doesn't
//!    walk.
//! 6. Import-boundary checking (§7) needs the multi-file loader to
//!    thread a manifest per imported module; this module checks one
//!    program's own manifest only.
//!
//! Runtime enforcement (RFC §5 -- manifest-derived resource ceilings/
//! network policy baked into `runtime-kernels`) is not wired here
//! either: this is the compile-time check + emitted bundle + the
//! `verify-binary` policy check, the "idealize -> build a real minimal
//! version -> wire into existing graph/pack/certificate machinery"
//! first slice, not the full multi-crate runtime story.

use crate::ast::{Effect, FnDecl, Program, Requirement, TypeRegistry};
use std::path::{Path, PathBuf};

fn default_manifest_version() -> String {
    "1".to_string()
}

/// A per-module security guarantee manifest, RFC 0017 §1's JSON shape
/// -- deliberately a real subset of the RFC's full schema (this
/// module's own top doc comment says which fields aren't read yet:
/// `resource_budgets`/`network_policy`/`file_policy`/`imports` are not
/// parsed here at all, so a manifest that includes them is accepted
/// (unknown keys are ignored, RFC §1's own "older tools can read newer
/// manifests" compatibility rule) but those sections have no effect
/// yet.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct GuaranteeManifest {
    #[serde(default = "default_manifest_version")]
    pub manifest_version: String,
    pub module: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub role_vocabulary: Vec<String>,
    #[serde(default)]
    pub authorization_bearing_claims: Vec<String>,
    #[serde(default)]
    pub exports: std::collections::BTreeMap<String, ExportContract>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Capabilities {
    pub allowed_effects: Option<Vec<String>>,
    #[serde(default)]
    pub forbidden_effects: Vec<String>,
}

/// One `exports.<fn>` entry -- RFC 0017 §1. `requires` is the manifest's
/// own compact string form (`"role:hr_staff"` / `"claim:department=
/// cardiology"`), not `Requirement` directly: the manifest is JSON
/// authored by a human reviewer, and this project's existing inline
/// syntax (`requires(role: "hr_staff")`) already has a compact textual
/// form worth mirroring rather than asking for a nested object.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ExportContract {
    pub requires: Option<String>,
    pub public: Option<bool>,
}

/// One finding from `check`/`check_bundle_against_policy`. Deliberately
/// a real enum (not a bare `String`), matching this codebase's own
/// discipline elsewhere (`typeck::TypeErrorKind`, `contract_check::
/// ContractCheckResult`) of structured diagnostics a caller can match
/// on, not just print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuaranteeError {
    /// A function's own inferred effect set is not a subset of
    /// `allowed_effects` (when declared) or intersects
    /// `forbidden_effects` -- RFC §3 item 1. `offending` names exactly
    /// which effect(s) triggered it, not just "some effect."
    CapabilityCeilingExceeded { fn_name: String, offending: Vec<String> },
    /// `exports.<name>` names a function this program doesn't declare.
    UnknownExport { name: String },
    /// `exports.<name>.requires` doesn't match the source's own
    /// `requires(...)` (or its absence) -- RFC §3 item 3.
    ExportRequiresMismatch { fn_name: String, manifest: String, actual: String },
    /// `exports.<name>.public` doesn't match the source's own
    /// `explicit_public` -- RFC §3 item 3.
    ExportPublicMismatch { fn_name: String, manifest: bool, actual: bool },
    /// A `requires(role: "...")` names a role not listed in
    /// `role_vocabulary` (only checked when that list is non-empty --
    /// an empty vocabulary is "not declared," not "nothing is allowed,"
    /// the same additive-by-default posture the rest of this module
    /// holds) -- RFC §3 item 5.
    UndeclaredRole { fn_name: String, role: String },
    /// Same as `UndeclaredRole`, for `requires(claim: "...", ...)`
    /// against `authorization_bearing_claims`.
    UndeclaredClaim { fn_name: String, claim: String },
    /// A manifest effect name (`allowed_effects`/`forbidden_effects`)
    /// isn't one of this language's real five effect tags -- a real,
    /// surfaced typo/config error rather than a silently-ignored entry
    /// that gives the reviewer false confidence.
    InvalidEffectName { name: String },
}

impl GuaranteeError {
    pub fn describe(&self) -> String {
        match self {
            GuaranteeError::CapabilityCeilingExceeded { fn_name, offending } => {
                format!("`{fn_name}` performs effect(s) [{}] outside this module's guarantee-manifest capability ceiling", offending.join(", "))
            }
            GuaranteeError::UnknownExport { name } => format!("manifest's `exports.{name}` names a function that does not exist in this program"),
            GuaranteeError::ExportRequiresMismatch { fn_name, manifest, actual } => {
                format!("`{fn_name}`: manifest's exported contract demands requires \"{manifest}\", source has {actual}")
            }
            GuaranteeError::ExportPublicMismatch { fn_name, manifest, actual } => {
                format!("`{fn_name}`: manifest's exported contract demands public={manifest}, source's own explicit_public is {actual}")
            }
            GuaranteeError::UndeclaredRole { fn_name, role } => {
                format!("`{fn_name}` requires role \"{role}\", which is not listed in this manifest's role_vocabulary")
            }
            GuaranteeError::UndeclaredClaim { fn_name, claim } => {
                format!("`{fn_name}` requires claim \"{claim}\", which is not listed in this manifest's authorization_bearing_claims")
            }
            GuaranteeError::InvalidEffectName { name } => format!("manifest names unknown effect \"{name}\" (valid: rng, io, concurrent, network, env)"),
        }
    }
}

fn parse_effect_name(s: &str) -> Option<Effect> {
    match s {
        "rng" => Some(Effect::Rng),
        "io" => Some(Effect::Io),
        "concurrent" => Some(Effect::Concurrent),
        "network" => Some(Effect::Network),
        "env" => Some(Effect::Env),
        _ => None,
    }
}

/// Parses the manifest's compact `requires` string form into a real
/// `Requirement` -- `None` for a string that matches neither shape (an
/// author typo), which callers turn into a real diagnostic rather than
/// silently treating as "no requirement."
fn parse_requirement_str(s: &str) -> Option<Requirement> {
    if let Some(role) = s.strip_prefix("role:") {
        return Some(Requirement::Role(role.to_string()));
    }
    if let Some(rest) = s.strip_prefix("claim:") {
        let (k, v) = rest.split_once('=')?;
        return Some(Requirement::Claim(k.to_string(), v.to_string()));
    }
    None
}

/// Where a source file's manifest lives, RFC 0017 §1's convention:
/// `src/payments.nir` -> `src/payments.nir.guarantees.json`.
pub fn manifest_path_for(source_path: &Path) -> PathBuf {
    let mut name = source_path.as_os_str().to_os_string();
    name.push(".guarantees.json");
    PathBuf::from(name)
}

/// Loads the manifest next to `source_path`, if one exists. `Ok(None)`
/// (not an error) when the file is simply absent -- RFC 0017's own
/// "Compatibility" section: a `.nir` program with no manifest compiles
/// exactly as before, additive only.
pub fn load(source_path: &Path) -> Result<Option<GuaranteeManifest>, String> {
    let path = manifest_path_for(source_path);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let manifest: GuaranteeManifest = serde_json::from_slice(&bytes).map_err(|e| format!("{} is not a valid guarantee manifest: {e}", path.display()))?;
    Ok(Some(manifest))
}

/// RFC 0017 §3's `guarantee_check` pass, real minimal version (this
/// module's own top doc comment names the three checks built here).
/// Takes an already-typechecked `program` (the manifest doesn't
/// replace typeck/ownership/contract_check -- it runs alongside them,
/// same as the RFC's own pipeline diagram).
pub fn check(program: &Program, registry: &TypeRegistry, manifest: &GuaranteeManifest) -> Vec<GuaranteeError> {
    let mut errors = Vec::new();

    // Manifest effect-name validation up front, once, so a typo in
    // `allowed_effects`/`forbidden_effects` is its own diagnostic
    // rather than silently never matching any real function.
    let mut invalid_effect_names = std::collections::BTreeSet::new();
    let allowed: Option<Vec<Effect>> = manifest.capabilities.allowed_effects.as_ref().map(|names| {
        names
            .iter()
            .filter_map(|s| match parse_effect_name(s) {
                Some(e) => Some(e),
                None => {
                    invalid_effect_names.insert(s.clone());
                    None
                }
            })
            .collect()
    });
    let forbidden: Vec<Effect> = manifest
        .capabilities
        .forbidden_effects
        .iter()
        .filter_map(|s| match parse_effect_name(s) {
            Some(e) => Some(e),
            None => {
                invalid_effect_names.insert(s.clone());
                None
            }
        })
        .collect();
    for name in invalid_effect_names {
        errors.push(GuaranteeError::InvalidEffectName { name });
    }

    // 1. Capability ceiling (RFC §3 item 1).
    let effects_map = crate::effects::infer_effects(program, registry);
    for f in &program.fns {
        let Some(fe) = effects_map.get(&f.name) else { continue };
        let mut offending: Vec<String> = Vec::new();
        for e in &fe.inferred {
            let over_ceiling = allowed.as_ref().is_some_and(|allowed| !allowed.contains(e));
            let explicitly_forbidden = forbidden.contains(e);
            if over_ceiling || explicitly_forbidden {
                offending.push(e.name().to_string());
            }
        }
        offending.sort();
        offending.dedup();
        if !offending.is_empty() {
            errors.push(GuaranteeError::CapabilityCeilingExceeded { fn_name: f.name.clone(), offending });
        }
    }

    // 3. Exported contract check (RFC §3 item 3).
    for (name, contract) in &manifest.exports {
        let Some(f): Option<&FnDecl> = program.fns.iter().find(|f| &f.name == name) else {
            errors.push(GuaranteeError::UnknownExport { name: name.clone() });
            continue;
        };
        if let Some(want) = &contract.requires {
            let actual_desc = f.requires.as_ref().map(|r| r.describe()).unwrap_or_else(|| "no requires(...)".to_string());
            match parse_requirement_str(want) {
                Some(req) if f.requires.as_ref() == Some(&req) => {}
                _ => errors.push(GuaranteeError::ExportRequiresMismatch { fn_name: name.clone(), manifest: want.clone(), actual: actual_desc }),
            }
        }
        if let Some(want_public) = contract.public {
            if f.explicit_public != want_public {
                errors.push(GuaranteeError::ExportPublicMismatch { fn_name: name.clone(), manifest: want_public, actual: f.explicit_public });
            }
        }
    }

    // 5. Vocabulary check (RFC §3 item 5) -- every `requires(...)` in
    // the whole program, not just `exports`-listed ones: a role/claim
    // vocabulary is a module-wide capability boundary, not an export-
    // surface-only one. Only enforced when the corresponding vocabulary
    // is non-empty -- an unset vocabulary means "not declared," never
    // "nothing is allowed" (this module's own additive-by-default
    // posture, matching RFC 0017's Compatibility section).
    if !manifest.role_vocabulary.is_empty() || !manifest.authorization_bearing_claims.is_empty() {
        for f in &program.fns {
            match &f.requires {
                Some(Requirement::Role(role)) if !manifest.role_vocabulary.is_empty() && !manifest.role_vocabulary.contains(role) => {
                    errors.push(GuaranteeError::UndeclaredRole { fn_name: f.name.clone(), role: role.clone() });
                }
                Some(Requirement::Claim(claim, _)) if !manifest.authorization_bearing_claims.is_empty() && !manifest.authorization_bearing_claims.contains(claim) => {
                    errors.push(GuaranteeError::UndeclaredClaim { fn_name: f.name.clone(), claim: claim.clone() });
                }
                _ => {}
            }
        }
    }

    errors
}

/// The emitted guarantee bundle (RFC §4), real minimal version: the
/// fields this module can actually populate from a live `Program`
/// (`inferred_effects`, `gated_exports`, `public_exports`,
/// `source_hash`) -- `proven_contracts`/`resource_budgets`/
/// `network_policy`/`governing_packs` are the RFC's own richer example
/// and are real, disclosed follow-up (they need the certificate/
/// contract-check/pack machinery threaded in here, not just `Program`
/// + `TypeRegistry`; `mcp_tools::build_certificate` already computes
/// several of them for the *source*-side `Certificate`, so wiring them
/// into this bundle too is a real, bounded next step, not a redesign).
pub fn build_bundle(program: &Program, registry: &TypeRegistry, source_hash: &str, manifest: Option<&GuaranteeManifest>) -> serde_json::Value {
    let effects_map = crate::effects::infer_effects(program, registry);
    let mut inferred_effects = serde_json::Map::new();
    let mut gated_exports = serde_json::Map::new();
    let mut public_exports: Vec<String> = Vec::new();
    for f in &program.fns {
        if let Some(fe) = effects_map.get(&f.name) {
            let names: Vec<&str> = fe.inferred.iter().map(|e| e.name()).collect();
            inferred_effects.insert(f.name.clone(), serde_json::json!(names));
        }
        if let Some(req) = &f.requires {
            gated_exports.insert(f.name.clone(), serde_json::json!(req.describe()));
        } else if f.explicit_public {
            public_exports.push(f.name.clone());
        }
    }
    serde_json::json!({
        "bundle_version": "1",
        "module": manifest.and_then(|m| m.module.clone()),
        "source_hash": source_hash,
        "inferred_effects": inferred_effects,
        "gated_exports": gated_exports,
        "public_exports": public_exports,
    })
}

/// `nirdosha verify-binary`'s check (RFC §6): the dual of `check` --
/// reads an already-emitted bundle against an operator-supplied policy
/// (itself a `GuaranteeManifest`, reusing the same schema rather than
/// inventing a second one) **without recompiling**, the real
/// "give me the generated code and I'll tell you whether it satisfies
/// these guarantees" answer for the *binary* side.
pub fn check_bundle_against_policy(bundle: &serde_json::Value, policy: &GuaranteeManifest) -> Vec<GuaranteeError> {
    let mut errors = Vec::new();
    let mut invalid_effect_names = std::collections::BTreeSet::new();
    let allowed: Option<Vec<Effect>> = policy.capabilities.allowed_effects.as_ref().map(|names| {
        names
            .iter()
            .filter_map(|s| match parse_effect_name(s) {
                Some(e) => Some(e),
                None => {
                    invalid_effect_names.insert(s.clone());
                    None
                }
            })
            .collect()
    });
    let forbidden: Vec<Effect> = policy
        .capabilities
        .forbidden_effects
        .iter()
        .filter_map(|s| match parse_effect_name(s) {
            Some(e) => Some(e),
            None => {
                invalid_effect_names.insert(s.clone());
                None
            }
        })
        .collect();
    for name in invalid_effect_names {
        errors.push(GuaranteeError::InvalidEffectName { name });
    }

    if let Some(inferred_effects) = bundle.get("inferred_effects").and_then(|v| v.as_object()) {
        for (fn_name, effs) in inferred_effects {
            let Some(effs) = effs.as_array() else { continue };
            let mut offending = Vec::new();
            for e in effs {
                let Some(name) = e.as_str() else { continue };
                let Some(effect) = parse_effect_name(name) else { continue };
                let over_ceiling = allowed.as_ref().is_some_and(|allowed| !allowed.contains(&effect));
                if over_ceiling || forbidden.contains(&effect) {
                    offending.push(name.to_string());
                }
            }
            offending.sort();
            offending.dedup();
            if !offending.is_empty() {
                errors.push(GuaranteeError::CapabilityCeilingExceeded { fn_name: fn_name.clone(), offending });
            }
        }
    }

    for (name, contract) in &policy.exports {
        if let Some(want) = &contract.requires {
            let actual = bundle.get("gated_exports").and_then(|g| g.get(name)).and_then(|v| v.as_str());
            let matches = actual.is_some_and(|actual| parse_requirement_str(want).is_some_and(|req| req.describe() == actual));
            if !matches {
                errors.push(GuaranteeError::ExportRequiresMismatch {
                    fn_name: name.clone(),
                    manifest: want.clone(),
                    actual: actual.map(str::to_string).unwrap_or_else(|| "no requires(...)".to_string()),
                });
            }
        }
        if contract.public == Some(true) {
            let is_public = bundle.get("public_exports").and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name.as_str())));
            if !is_public {
                errors.push(GuaranteeError::ExportPublicMismatch { fn_name: name.clone(), manifest: true, actual: false });
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    // Returns just `Program` -- `TypeRegistry<'a>` borrows from it, so
    // a helper can't hand back both from behind its own stack frame;
    // every call site below builds its own registry with `TypeRegistry::
    // build(&program)` right after calling this.
    fn program_from(src: &str) -> Program {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("nir_guarantee_manifest_test_src_{}_{n}.nir", std::process::id()));
        std::fs::write(&path, src).expect("writing the test source file");
        let (program, _src) = crate::loader::load_program(path.to_str().expect("temp path is valid utf-8")).expect("load_program");
        crate::typeck::typecheck(&program).expect("typecheck");
        let _ = std::fs::remove_file(&path);
        program
    }

    #[test]
    fn a_program_with_no_manifest_has_nothing_to_load() {
        let path = std::env::temp_dir().join(format!("nir_guarantee_manifest_test_missing_{}.nir", std::process::id()));
        assert_eq!(load(&path).expect("Ok(None) for a missing manifest file"), None);
    }

    #[test]
    fn manifest_round_trips_through_json_and_load() {
        let manifest = GuaranteeManifest {
            manifest_version: "1".to_string(),
            module: Some("payments".to_string()),
            capabilities: Capabilities { allowed_effects: Some(vec!["io".to_string()]), forbidden_effects: vec!["network".to_string()] },
            role_vocabulary: vec!["admin".to_string()],
            authorization_bearing_claims: vec![],
            exports: Default::default(),
        };
        let source_path = std::env::temp_dir().join(format!("nir_guarantee_manifest_test_roundtrip_{}.nir", std::process::id()));
        let manifest_path = manifest_path_for(&source_path);
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let loaded = load(&source_path).expect("load must succeed").expect("a manifest file was written");
        assert_eq!(loaded.module, Some("payments".to_string()));
        assert_eq!(loaded.capabilities.forbidden_effects, vec!["network".to_string()]);
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn capability_ceiling_flags_a_forbidden_effect_a_function_actually_performs() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let manifest = GuaranteeManifest { capabilities: Capabilities { forbidden_effects: vec!["io".to_string()], ..Default::default() }, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(
            errors.iter().any(|e| matches!(e, GuaranteeError::CapabilityCeilingExceeded { fn_name, offending } if fn_name == "main" && offending.iter().any(|o| o == "io"))),
            "expected a capability-ceiling error for main's real io effect: {errors:?}"
        );
    }

    #[test]
    fn capability_ceiling_is_silent_when_the_only_effect_is_allowed() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let manifest = GuaranteeManifest { capabilities: Capabilities { allowed_effects: Some(vec!["io".to_string()]), ..Default::default() }, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.is_empty(), "an allowed effect must not be flagged: {errors:?}");
    }

    #[test]
    fn capability_ceiling_flags_an_effect_outside_the_allowed_set() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let manifest = GuaranteeManifest { capabilities: Capabilities { allowed_effects: Some(vec!["network".to_string()]), ..Default::default() }, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::CapabilityCeilingExceeded { .. })), "io isn't in the allowed set, so this must be flagged: {errors:?}");
    }

    #[test]
    fn unknown_export_is_flagged() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let mut exports = std::collections::BTreeMap::new();
        exports.insert("does_not_exist".to_string(), ExportContract::default());
        let manifest = GuaranteeManifest { exports, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::UnknownExport { name } if name == "does_not_exist")));
    }

    #[test]
    fn export_public_mismatch_is_flagged_when_source_is_not_public() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let mut exports = std::collections::BTreeMap::new();
        exports.insert("main".to_string(), ExportContract { requires: None, public: Some(true) });
        let manifest = GuaranteeManifest { exports, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::ExportPublicMismatch { fn_name, .. } if fn_name == "main")));
    }

    #[test]
    fn export_public_match_is_silent() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let mut exports = std::collections::BTreeMap::new();
        exports.insert("main".to_string(), ExportContract { requires: None, public: Some(false) });
        let manifest = GuaranteeManifest { exports, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.is_empty(), "main's real explicit_public is false, matching the manifest: {errors:?}");
    }

    #[test]
    fn invalid_effect_name_in_the_manifest_is_its_own_diagnostic() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let manifest = GuaranteeManifest { capabilities: Capabilities { forbidden_effects: vec!["not_a_real_effect".to_string()], ..Default::default() }, ..Default::default() };
        let errors = check(&program, &registry, &manifest);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::InvalidEffectName { name } if name == "not_a_real_effect")));
    }

    #[test]
    fn check_bundle_against_policy_flags_a_forbidden_effect_in_the_bundle() {
        let bundle = serde_json::json!({
            "inferred_effects": { "main": ["io", "network"] },
            "gated_exports": {},
            "public_exports": [],
        });
        let policy = GuaranteeManifest { capabilities: Capabilities { forbidden_effects: vec!["network".to_string()], ..Default::default() }, ..Default::default() };
        let errors = check_bundle_against_policy(&bundle, &policy);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::CapabilityCeilingExceeded { fn_name, offending } if fn_name == "main" && offending.iter().any(|o| o == "network"))));
    }

    #[test]
    fn check_bundle_against_policy_is_clean_when_the_bundle_satisfies_the_policy() {
        let bundle = serde_json::json!({
            "inferred_effects": { "main": ["io"] },
            "gated_exports": { "get_employee": "requires role \"hr_staff\"" },
            "public_exports": ["public_status"],
        });
        let mut exports = std::collections::BTreeMap::new();
        exports.insert("get_employee".to_string(), ExportContract { requires: Some("role:hr_staff".to_string()), public: None });
        exports.insert("public_status".to_string(), ExportContract { requires: None, public: Some(true) });
        let policy = GuaranteeManifest { capabilities: Capabilities { allowed_effects: Some(vec!["io".to_string()]), ..Default::default() }, exports, ..Default::default() };
        let errors = check_bundle_against_policy(&bundle, &policy);
        assert!(errors.is_empty(), "bundle satisfies the policy exactly: {errors:?}");
    }

    #[test]
    fn check_bundle_against_policy_flags_a_missing_required_gate() {
        let bundle = serde_json::json!({
            "inferred_effects": {},
            "gated_exports": {},
            "public_exports": [],
        });
        let mut exports = std::collections::BTreeMap::new();
        exports.insert("get_employee".to_string(), ExportContract { requires: Some("role:hr_staff".to_string()), public: None });
        let policy = GuaranteeManifest { exports, ..Default::default() };
        let errors = check_bundle_against_policy(&bundle, &policy);
        assert!(errors.iter().any(|e| matches!(e, GuaranteeError::ExportRequiresMismatch { fn_name, .. } if fn_name == "get_employee")));
    }

    #[test]
    fn build_bundle_carries_source_hash_and_inferred_effects() {
        let program = program_from("fn main() {\n    print(\"hi\")\n}\n");
        let registry = TypeRegistry::build(&program);
        let bundle = build_bundle(&program, &registry, "sha256:deadbeef", None);
        assert_eq!(bundle["source_hash"], "sha256:deadbeef");
        assert_eq!(bundle["inferred_effects"]["main"], serde_json::json!(["io"]));
    }
}
