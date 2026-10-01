//! Reads active (installed, non-revoked) domain packs — issue #76's
//! "wire what already exists": `nirdosha-hi::hi_plugin`'s real Ed25519
//! signing + trust-anchor + install machinery
//! (`verify_and_install_signed_pack`/`install_pack_from_bytes`) already
//! writes a trusted pack's manifest to `<root>/.nir/plugins/<id>/
//! pack.json` and marks it active in `<root>/.nir/hi.db`'s `plugins`
//! table (`revoked_at IS NULL`) — but until now, every *real* compile
//! path (`cargo nirdosha build`, `nirdosha build`/`certify`) was blind
//! to that state entirely; only `nirdosha-hi`'s own LLM-generation
//! guidance loop ever read it.
//!
//! This module is a read-only mirror of that same on-disk convention —
//! same directory, same table, same "non-revoked wins" rule — so a
//! pack a project already installed and trusted becomes visible to a
//! real build too, **without** this crate (or any of its callers,
//! `cargo-nirdosha`/`crates/compiler`) depending on the rest of
//! `nirdosha-hi` (its GUI window, LLM client, MCP server...). It never
//! installs, signs, or revokes anything — only asks "what does this
//! project already trust, right now?" The trust decision itself was
//! already made, once, at install time, by code this module never
//! re-implements.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The narrow slice of `nirdosha-hi::hi_plugin::PackManifest`'s schema
/// this reader actually needs. Deliberately **not**
/// `#[serde(deny_unknown_fields)]`: the owning schema (`hi_plugin.rs`)
/// already validates a pack fully at install time; every other RFC
/// 0016 field (`invariants`, `relations`, `primitives_nir`,
/// `compliance_profiles`, ...) is real and used elsewhere, just not by
/// the two enforcement checks this module exists to feed.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PackManifest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mandatory_fns: Vec<String>,
    #[serde(default)]
    pub protected_structs: Vec<String>,
}

fn plugins_dir(root: &Path) -> PathBuf {
    root.join(".nir").join("plugins")
}

fn db_path(root: &Path) -> PathBuf {
    root.join(".nir").join("hi.db")
}

/// Every currently-active (non-revoked) pack's manifest for the
/// project rooted at `root`. `Ok(vec![])` — never an error — when no
/// `.nir/hi.db` exists at all: "no pack was ever installed for this
/// project" is the overwhelmingly common case and must stay a silent
/// no-op, the same "additive only" rule every other pack-aware surface
/// in this codebase follows.
pub fn load_active_packs(root: &Path) -> Result<Vec<PackManifest>, String> {
    let db_path = db_path(root);
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let mut stmt = conn
        .prepare("SELECT id FROM plugins WHERE revoked_at IS NULL")
        .map_err(|e| format!("reading active packs from {}: {e}", db_path.display()))?;
    let active_ids: HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| format!("reading active packs from {}: {e}", db_path.display()))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| format!("reading active packs from {}: {e}", db_path.display()))?;

    let dir = plugins_dir(root);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("reading {} entry: {e}", dir.display()))?;
        let manifest_path = entry.path().join("pack.json");
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&manifest_path)
            .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
        let manifest: PackManifest = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parsing {}: {e}", manifest_path.display()))?;
        if active_ids.contains(&manifest.id) {
            out.push(manifest);
        }
    }
    // Deterministic order: findings that name a pack-derived rule must
    // not depend on directory iteration order.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Every active pack's `mandatory_fns`, unioned.
pub fn active_mandatory_fns(root: &Path) -> Result<HashSet<String>, String> {
    Ok(load_active_packs(root)?.into_iter().flat_map(|m| m.mandatory_fns).collect())
}

/// Every active pack's `protected_structs`, unioned.
pub fn active_protected_structs(root: &Path) -> Result<HashSet<String>, String> {
    Ok(load_active_packs(root)?.into_iter().flat_map(|m| m.protected_structs).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn tmp_project() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nir-pack-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn install(root: &Path, id: &str, mandatory_fns: &[&str], protected_structs: &[&str], revoked: bool) {
        std::fs::create_dir_all(root.join(".nir").join("plugins").join(id)).unwrap();
        std::fs::write(
            root.join(".nir").join("plugins").join(id).join("pack.json"),
            serde_json::json!({
                "id": id,
                "name": id,
                "mandatory_fns": mandatory_fns,
                "protected_structs": protected_structs,
            })
            .to_string(),
        )
        .unwrap();
        let conn = rusqlite::Connection::open(root.join(".nir").join("hi.db")).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS plugins (id TEXT PRIMARY KEY, name TEXT, sha256 TEXT, manifest_path TEXT, installed_at TEXT, revoked_at TEXT)",
            [],
        )
        .unwrap();
        let revoked_at: Option<&str> = if revoked { Some("2026-01-01") } else { None };
        conn.execute(
            "INSERT INTO plugins (id, name, sha256, revoked_at) VALUES (?1, ?1, 'x', ?2)",
            rusqlite::params![id, revoked_at],
        )
        .unwrap();
    }

    #[test]
    fn no_hi_db_is_a_silent_no_op() {
        let root = tmp_project();
        assert!(load_active_packs(&root).unwrap().is_empty());
    }

    #[test]
    fn an_installed_active_pack_is_returned() {
        let root = tmp_project();
        install(&root, "banking-v0", &["debit_ledger"], &["Account"], false);
        let packs = load_active_packs(&root).unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "banking-v0");
        assert_eq!(active_mandatory_fns(&root).unwrap(), HashSet::from(["debit_ledger".to_string()]));
        assert_eq!(active_protected_structs(&root).unwrap(), HashSet::from(["Account".to_string()]));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_revoked_pack_no_longer_counts() {
        let root = tmp_project();
        install(&root, "banking-v0", &["debit_ledger"], &["Account"], true);
        assert!(load_active_packs(&root).unwrap().is_empty());
        assert!(active_mandatory_fns(&root).unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
