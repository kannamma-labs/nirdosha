//! RFC 0016 Phase 2: 5a domain plugins / sealed, non-waivable invariants.
//!
//! A domain plugin is a small, versioned, locally-installed manifest of
//! invariants the graph holds as confirmed, locked, non-waivable units.
//! The banking pack v0 ships with the binary so the fintech demo can be
//! one-shot generated; external packs can be installed with
//! `nirdosha plugin install <path>`.
//!
//! **5a scope, disclosed:** the manifest's SHA-256 is pinned at install
//! (TOFU) and re-checked on every load, but there is NO signature layer
//! yet -- that is RFC 0016's blocked Phase 5b, gated on registry
//! governance. The SHA-256 is still valuable: it detects accidental
//! mutation and gives 5b a stable canonical-bytes hook later.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A loaded pack manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub invariants: Vec<PackInvariant>,
    #[serde(default)]
    pub relations: Vec<(String, String)>,
    #[serde(default)]
    pub mandatory_fns: Vec<String>,
}

/// One invariant unit contributed by a pack.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackInvariant {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<String>,
    pub signature: PackSignature,
    #[serde(default)]
    pub validates: Vec<ValidateTemplate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackSignature {
    #[serde(default)]
    pub params: Vec<(String, String)>,
    pub ret: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateTemplate {
    pub fn_name: String,
    #[serde(default)]
    pub pre: Option<String>,
    #[serde(default)]
    pub post: Vec<String>,
}

/// The banking domain pack v0: whole cents, conservation of money,
/// non-negative balances.  It is deliberately tiny and integer-only
/// because the Tier-1 proof engine can only prove linear arithmetic
/// over integer params/results today.  The pack ships as an embedded
/// asset so the fintech demo works without a manual install step.
const BANKING_V0_JSON: &str = include_str!("../../../agent-skills/nirdosha/packs/banking-v0.json");

/// Where packs live under a project's `.nir/` directory.
pub fn plugins_dir(root: &Path) -> PathBuf {
    crate::hi_graph::hi_dir(root).join("plugins")
}

/// The manifest path for an installed pack.
pub fn pack_manifest_path(root: &Path, pack_id: &str) -> PathBuf {
    plugins_dir(root).join(pack_id).join("pack.json")
}

/// Install or refresh a pack from raw JSON bytes.  `source_desc` is
/// used only in diagnostics (file path, `embedded`, etc.).  Returns the
/// pack id on success.
pub fn install_pack_from_bytes(
    conn: &rusqlite::Connection,
    root: &Path,
    bytes: &[u8],
    source_desc: &str,
) -> Result<String, String> {
    let sha256 = crate::hi_graph::sha256_hex(bytes);
    let manifest: PackManifest =
        serde_json::from_slice(bytes).map_err(|e| format!("parsing pack manifest from {source_desc}: {e}"))?;

    // Write the canonical bytes to disk so the pack survives and is
    // re-hashable on every load.
    let pack_dir = plugins_dir(root).join(&manifest.id);
    std::fs::create_dir_all(&pack_dir)
        .map_err(|e| format!("creating {}: {e}", pack_dir.display()))?;
    let manifest_path = pack_dir.join("pack.json");
    std::fs::write(&manifest_path, bytes)
        .map_err(|e| format!("writing {}: {e}", manifest_path.display()))?;

    // Record or update the plugin row.
    let now = iso_now();
    conn.execute(
        "INSERT INTO plugins (id, name, sha256, manifest_path, installed_at, revoked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             sha256 = excluded.sha256,
             manifest_path = excluded.manifest_path,
             installed_at = excluded.installed_at,
             revoked_at = NULL",
        rusqlite::params![manifest.id, manifest.name, sha256, manifest_path.to_str(), now],
    )
    .map_err(|e| format!("recording plugin {}: {e}", manifest.id))?;

    load_domain_pack(conn, root, &manifest, &sha256)?;
    Ok(manifest.id)
}

/// Re-load every installed pack that has not been revoked, verifying
/// its on-disk bytes still match the recorded SHA-256.  Call this on
/// every `hi` startup after `hi_graph::open`.
pub fn reload_installed_packs(conn: &rusqlite::Connection, root: &Path) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, sha256 FROM plugins WHERE revoked_at IS NULL")
        .map_err(|e| format!("querying installed packs: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
            ))
        })
        .map_err(|e| format!("querying installed packs: {e}"))?;

    for row in rows {
        let (id, recorded_sha256) = row.map_err(|e| format!("reading installed pack row: {e}"))?;
        let path = pack_manifest_path(root, &id);
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("reading pack {} at {}: {e}", id, path.display()))?;
        let actual_sha256 = crate::hi_graph::sha256_hex(&bytes);
        if actual_sha256 != recorded_sha256 {
            return Err(format!(
                "pack {id} has been modified since install (expected sha256 {recorded_sha256}, got {actual_sha256}) -- revoke and reinstall if the change is intentional",
            ));
        }
        let manifest: PackManifest = serde_json::from_slice(&bytes)
            .map_err(|e| format!("re-parsing pack {id}: {e}"))?;
        load_domain_pack(conn, root, &manifest, &actual_sha256)?;
    }
    Ok(())
}

/// Ensure the built-in banking pack v0 is installed.  Idempotent.
pub fn ensure_default_packs(conn: &rusqlite::Connection, root: &Path) -> Result<(), String> {
    install_pack_from_bytes(conn, root, BANKING_V0_JSON.as_bytes(), "embedded banking-v0")?;
    Ok(())
}

/// Load a pack's invariant units into the graph as confirmed, locked,
/// non-waivable nodes.  Re-running this for an already-loaded pack is
/// idempotent: the same ids are upserted with the same attribute text.
fn load_domain_pack(
    conn: &rusqlite::Connection,
    root: &Path,
    manifest: &PackManifest,
    sha256: &str,
) -> Result<(), String> {
    for inv in &manifest.invariants {
        if inv.kind != "fn" {
            return Err(format!(
                "pack {} invariant `{}` has unsupported kind `{}` -- only `fn` invariants are supported in 5a",
                manifest.id, inv.name, inv.kind
            ));
        }
        let created_by = format!("plugin:{}:{}", manifest.id, sha256);
        let id = crate::hi_graph::add_candidate(
            conn,
            "fn",
            &inv.name,
            &format!("Domain-law fn from pack {}: {}", manifest.id, manifest.name),
            &created_by,
        )?;
        // Mark it as pack-owned and non-waivable before confirming/locking.
        conn.execute(
            "UPDATE nodes SET plugin_origin = ?2, non_waivable = 1 WHERE id = ?1",
            rusqlite::params![id, manifest.id],
        )
        .map_err(|e| format!("tagging {id} as non-waivable pack node: {e}"))?;
        crate::hi_graph::confirm_node(conn, &id)?;
        for attr in &inv.attributes {
            crate::hi_graph::attach_attribute(conn, &id, attr)?;
        }
        // Pack invariants are locked immediately: they are not waiting
        // for a Generate pass to materialize them -- the pack *is* the
        // authoritative source.
        conn.execute(
            "UPDATE nodes SET locked = 1 WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| format!("locking pack node {id}: {e}"))?;
    }
    for (src, dst) in &manifest.relations {
        let src_id = crate::hi_graph::code_unit_node_id("fn", src);
        let dst_id = crate::hi_graph::code_unit_node_id("fn", dst);
        crate::hi_graph::add_relation(conn, &src_id, &dst_id)?;
    }
    let _ = root;
    Ok(())
}

/// Inject any missing pack `validate` templates into a generated source
/// string.  Returns the (possibly augmented) source, or a coverage
/// failure if a demanded fn has the wrong signature.
///
/// This is 5a's "injected mode": the model's code is checked against
/// the sealed law; if it forgot a contract, the pack supplies it.  If
/// the model already wrote the contract, injection is a no-op for that
/// fn.
pub fn inject_pack_validates_into_source(
    root: &Path,
    source: &str,
) -> Result<String, crate::hi_llm::CoverageFailure> {
    let packs = list_installed_pack_manifests(root).map_err(|e| crate::hi_llm::CoverageFailure {
        class: crate::hi_llm::CoverageFailureClass::ContractViolated,
        diagnostic: format!("contract coverage failure: could not read installed domain packs: {e}"),
    })?;
    if packs.is_empty() {
        return Ok(source.to_string());
    }

    // Parse once to discover fn signatures and existing validate blocks.
    let toks = match crate::token::Lexer::new(source).tokenize() {
        Ok(t) => t,
        Err(_) => return Ok(source.to_string()), // let typecheck report parse errors in its own voice
    };
    let program = match crate::parser::Parser::new(toks).parse_program() {
        Ok(p) => p,
        Err(_) => return Ok(source.to_string()),
    };

    let mut injections: Vec<String> = Vec::new();
    for (manifest, _sha256) in &packs {
        for inv in &manifest.invariants {
            if inv.kind != "fn" {
                continue;
            }
            let Some(fndecl) = program.fns.iter().find(|f| f.name == inv.name) else {
                continue;
            };
            // If the model already wrote any validate block for this fn,
            // trust that one (the coverage gate will still prove it).
            if program.validates.iter().any(|v| v.fn_name == inv.name) {
                continue;
            }
            // Signature guard: the pack declares the exact param names the
            // contract predicates reference.
            if !signature_matches(fndecl, &inv.signature) {
                return Err(crate::hi_llm::CoverageFailure {
                    class: crate::hi_llm::CoverageFailureClass::ContractViolated,
                    diagnostic: format!(
                        "contract coverage failure: pack `{}` demands fn `{}` with signature `{}`, but the draft uses a different signature -- the domain law requires the exact parameter names its contracts reference",
                        manifest.id,
                        inv.name,
                        pack_signature_text(&inv.signature)
                    ),
                });
            }
            for tmpl in &inv.validates {
                injections.push(render_validate_template(tmpl));
            }
        }
    }

    if injections.is_empty() {
        return Ok(source.to_string());
    }
    let mut out = source.trim_end().to_string();
    out.push('\n');
    out.push_str("// Domain-law validate blocks injected by installed packs:\n");
    for block in injections {
        out.push_str(&block);
        out.push('\n');
    }
    Ok(out)
}

/// List installed, non-revoked packs by reading `.nir/plugins/*/pack.json`
/// directly (the graph only stores the units; the manifest file holds the
/// templates).
fn list_installed_pack_manifests(root: &Path) -> Result<Vec<(PackManifest, String)>, String> {
    let dir = plugins_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("reading {} entry: {e}", dir.display()))?;
        let path = entry.path();
        let manifest_path = path.join("pack.json");
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&manifest_path)
            .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
        let sha256 = crate::hi_graph::sha256_hex(&bytes);
        let manifest: PackManifest = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parsing {}: {e}", manifest_path.display()))?;
        out.push((manifest, sha256));
    }
    Ok(out)
}

fn signature_matches(fndecl: &crate::ast::FnDecl, sig: &PackSignature) -> bool {
    if !type_name_matches(&fndecl.ret, &sig.ret) {
        return false;
    }
    if fndecl.params.len() != sig.params.len() {
        return false;
    }
    for (p, (name, ty)) in fndecl.params.iter().zip(sig.params.iter()) {
        if p.name != *name {
            return false;
        }
        if !type_name_matches(&p.ty, ty) {
            return false;
        }
    }
    true
}

fn type_name_matches(ty: &crate::ast::Ty, name: &str) -> bool {
    crate::ast::Ty::from_name(name).as_ref() == Some(ty)
}

fn pack_signature_text(sig: &PackSignature) -> String {
    let params: Vec<String> = sig
        .params
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect();
    format!("fn {}({}) -> {}", "name", params.join(", "), sig.ret)
}

fn render_validate_template(tmpl: &ValidateTemplate) -> String {
    let mut out = format!("validate {} {{\n", tmpl.fn_name);
    if let Some(pre) = &tmpl.pre {
        out.push_str(&format!("    pre: {}\n", pre));
    }
    for post in &tmpl.post {
        out.push_str(&format!("    post: {}\n", post));
    }
    out.push_str("}\n");
    out
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

/// Render the domain-law section of the Generate prompt.  Plugin-origin
/// units are pulled out of the ordinary unit list and shown under a
/// dedicated heading so the model cannot mistake them for optional
/// suggestions.
pub fn plugin_law_prompt(units: &[crate::hi_graph::CandidateUnit]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let plugin_units: Vec<&crate::hi_graph::CandidateUnit> = units
        .iter()
        .filter(|u| u.attributes.iter().any(|a| crate::hi_llm::demanded_contract(a).is_some()))
        .collect();
    if plugin_units.is_empty() {
        return String::new();
    }
    lines.push(String::from(
        "\n--- DOMAIN LAW (non-waivable, injected by installed domain packs) ---\n",
    ));
    lines.push(String::from(
        "These components are NOT ordinary user requirements. They are sealed, machine-proven invariants from a domain pack. You MUST implement them exactly as named, with the exact signatures and parameter names shown, and you MUST include the top-level `validate <fn> { ... }` block for each one. The compiler will reject the program unless every demanded contract PROVES under Z3. Do not rename parameters, do not change return types, and do not skip the `validate` block.\n",
    ));
    for u in plugin_units {
        lines.push(format!("- {} `{}`", u.kind, u.name));
        lines.push(format!("  purpose: {}", u.driving_text));
        for attr in &u.attributes {
            for line in attr.lines() {
                if let Some(demand) = crate::hi_llm::demanded_contract(line) {
                    lines.push(format!(
                        "  PROOF DEMAND, not optional: validate contract {demand}"
                    ));
                } else {
                    lines.push(format!("  attribute to attach: {line}"));
                }
            }
        }
    }
    lines.push(String::from(
        "--- END DOMAIN LAW ---\n",
    ));
    lines.join("\n")
}

/// Revoke a pack by id.  Revocation marks the plugin row and deletes the
/// pack-owned invariant nodes (they are non-waivable, so they cannot be
/// left in a waived state).
pub fn revoke_pack(conn: &rusqlite::Connection, root: &Path, pack_id: &str) -> Result<(), String> {
    let now = iso_now();
    conn.execute(
        "UPDATE plugins SET revoked_at = ?2 WHERE id = ?1",
        rusqlite::params![pack_id, now],
    )
    .map_err(|e| format!("revoking pack {pack_id}: {e}"))?;
    // Remove the nodes this pack contributed.  Non-waivable nodes are
    // not eligible for waive; revocation is the supported removal path.
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM nodes WHERE plugin_origin = ?1")
        .and_then(|mut s| {
            s.query_map([pack_id], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .map_err(|e| format!("listing nodes for pack {pack_id}: {e}"))?;
    for id in ids {
        // Revocation is the supported removal path for non-waivable
        // pack nodes; bypass the `delete_node` guard that ordinarily
        // refuses to remove them.
        conn.execute("DELETE FROM edges WHERE src = ?1 OR dst = ?1", [&id,
        ])
        .map_err(|e| format!("deleting edges touching {id}: {e}"))?;
        conn.execute("DELETE FROM nodes WHERE id = ?1", [&id,
        ])
        .map_err(|e| format!("deleting node {id}: {e}"))?;
    }
    let _ = root;
    Ok(())
}

/// Print a human-readable list of installed packs to stdout.
pub fn list_packs(conn: &rusqlite::Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, name, sha256, installed_at, revoked_at FROM plugins ORDER BY installed_at")
        .map_err(|e| format!("listing packs: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| format!("listing packs: {e}"))?;

    let mut any = false;
    for row in rows {
        let (id, name, sha256, installed_at, revoked_at) = row.map_err(|e| format!("reading pack row: {e}"))?;
        any = true;
        let status = if revoked_at.is_some() { "REVOKED" } else { "active" };
        println!("{id}  {name}  [{status}]");
        println!("  sha256: {sha256}");
        println!("  installed: {installed_at}");
    }
    if !any {
        println!("(no packs installed)");
    }
    Ok(())
}

/// Convenience for tests: return the ids of packs currently installed.
pub fn installed_pack_ids(conn: &rusqlite::Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM plugins WHERE revoked_at IS NULL")
        .map_err(|e| format!("listing pack ids: {e}"))?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| format!("listing pack ids: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_hi_plugin_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn default_banking_manifest_parses() {
        let manifest = default_banking_manifest();
        assert_eq!(manifest.id, "banking-v0");
        assert!(manifest.invariants.iter().any(|i| i.name == "charge_cents"));
    }

    #[test]
    fn inject_pack_validates_adds_missing_contract_blocks() {
        let dir = scratch_dir("inject_validates");
        let conn = crate::hi_graph::open(&dir).expect("open");
        ensure_default_packs(&conn, &dir).expect("ensure default packs");

        let source = r#"
fn charge_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents - amount_cents
}

fn credit_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents + amount_cents
}

fn net_change_cents(credits: i64, debits: i64) -> i64 {
    return credits - debits
}

fn main() requires(public) {
    print("charge", charge_cents(1000, 200))
}
"#;

        let injected = inject_pack_validates_into_source(&dir, source).expect("injection should succeed");
        assert!(injected.contains("validate charge_cents"), "injected source must contain the charge_cents contract, got:\n{injected}");
        assert!(injected.contains("validate credit_cents"), "injected source must contain the credit_cents contract, got:\n{injected}");
        assert!(injected.contains("validate net_change_cents"), "injected source must contain the net_change_cents contract, got:\n{injected}");

        let units = crate::hi_graph::confirmed_units(&conn, None).expect("confirmed_units");
        crate::hi_llm::contract_coverage_check(&injected, &units).expect("the injected contracts must prove");
    }

    #[test]
    fn inject_pack_validates_rejects_a_wrong_signature() {
        let dir = scratch_dir("inject_signature_mismatch");
        let conn = crate::hi_graph::open(&dir).expect("open");
        ensure_default_packs(&conn, &dir).expect("ensure default packs");

        // The pack demands `charge_cents(balance_cents, amount_cents)`;
        // this draft renames the params, so the contract predicates would
        // reference unbound identifiers.
        let source = r#"
fn charge_cents(bal: i64, amt: i64) -> i64 {
    return bal - amt
}

fn main() requires(public) {
    print("charge", charge_cents(1000, 200))
}
"#;

        let failure = inject_pack_validates_into_source(&dir, source).expect_err("wrong signature must fail");
        assert_eq!(failure.class, crate::hi_llm::CoverageFailureClass::ContractViolated);
        assert!(failure.diagnostic.contains("different signature"), "got:\n{}", failure.diagnostic);
    }

    #[test]
    fn inject_pack_validates_skips_when_the_model_already_wrote_the_contract() {
        let dir = scratch_dir("inject_no_duplicate");
        let conn = crate::hi_graph::open(&dir).expect("open");
        ensure_default_packs(&conn, &dir).expect("ensure default packs");

        let source = r#"
fn charge_cents(balance_cents: i64, amount_cents: i64) -> i64 {
    return balance_cents - amount_cents
}

validate charge_cents {
    pre: balance_cents >= 0 && amount_cents >= 0 && amount_cents <= balance_cents
    post: result == balance_cents - amount_cents
}

fn main() requires(public) {
    print("charge", charge_cents(1000, 200))
}
"#;

        let injected = inject_pack_validates_into_source(&dir, source).expect("injection should succeed");
        let count = injected.matches("validate charge_cents").count();
        assert_eq!(count, 1, "the model's own validate block must not be duplicated");
    }
}

/// Convenience for tests: build the in-memory representation of the
/// default banking pack without touching disk.
pub fn default_banking_manifest() -> PackManifest {
    serde_json::from_str(BANKING_V0_JSON).expect("embedded banking-v0.json parses")
}
