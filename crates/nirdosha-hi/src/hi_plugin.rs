//! RFC 0016 Phase 2: 5a domain plugins / sealed, non-waivable invariants.
//!
//! A domain plugin is a small, versioned, locally-installed manifest of
//! invariants the graph holds as confirmed, locked, non-waivable units.
//! The banking pack v0 ships with the binary so the fintech demo can be
//! one-shot generated; external packs can be installed with
//! `nirdosha plugin install <path>`.
//!
//! **5a scope, disclosed:** the manifest's SHA-256 is pinned at install
//! (TOFU) and re-checked on every load. This used to be the whole
//! story -- "no signature layer yet, that's blocked Phase 5b" -- but as
//! of RFC 0016 Phase 4 (issue #59) that's stale: the **local** signing
//! mechanics 5b actually needs now exist below (`sign_pack`/
//! `verify_and_install_signed_pack`/`TrustAnchor`, Sigstore-*pattern*:
//! Ed25519 signatures, an operator-configured trust-anchor list, a
//! real hash-chained signing log mirroring Rekor's role). **What
//! remains genuinely blocked, unchanged**: live Fulcio/OIDC short-lived
//! certificate issuance and a public, cross-organization Rekor
//! transparency log -- both need a registry operator this project
//! doesn't have, RFC 0016's own honest position. Plain, unsigned
//! `install_pack_from_bytes` below is still the default path and still
//! works exactly as it always has; signing is additive and opt-in, on
//! both the author and operator sides.
//!
//! **Issue #76 follow-up, closing more of the "pack ID is the law" and
//! "domain_class" gaps 5a/5b left open:**
//! - The pinned/signed identity (`pack_content_id`, both the tamper
//!   pin above and what `sign_pack` actually signs) is now RFC 0016's
//!   own `pack_id = "sha256:" + SHA-256(domsep || JCS-canonical-bytes)`
//!   -- not a plain `SHA-256` over whatever raw bytes were on disk. Two
//!   byte-different encodings of the same logical manifest now share
//!   one identity, and the digest itself is domain-separated so a
//!   pack signature can never be replayed as a signature over
//!   something else.
//! - `PackManifest` now carries `domain_class`/`jurisdictions`
//!   (`DomainClass`'s own doc comment); `verify_and_install_signed_pack`
//!   refuses TOFU outright for `domain_class: regulated` -- this
//!   toolchain's stand-in for "a self-signed cert on a `regulated`
//!   manifest fails verification," since a real registry-issued
//!   certificate chain is still the blocked, registry-dependent piece.
//! - The pack-signing log now rides on `nirdosha_audit::audit_chain`'s
//!   real hash chain (`verify_pack_signing_log`), not a plain
//!   unstructured JSON-Lines append -- tampering with or deleting a
//!   past entry is now detectable, though it remains a *local* log
//!   (RFC 0016's own 5a/5b split: no public inclusion proof without a
//!   registry).
//! - Still genuinely unimplemented, disclosed rather than assumed
//!   solved: `PACK.cert`/registry-issued certificates, live transparency-
//!   log inclusion proofs, break-glass, the deployment-side
//!   `jurisdictions` check, and MIR-level (Stage 2) enforcement of
//!   `mandatory_fns`/`protected_structs` for the Rust v2 dialect
//!   (`cargo-nirdosha`'s own enforcement, wired separately, is
//!   syntactic/Stage 1 today).

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// RFC 0016 "Content addressing — the pack ID is the law": a
/// domain-separation prefix hashed before the canonical payload, so a
/// signature minted over this digest can never be replayed as a
/// signature over some other domain-separated digest that happens to
/// share the same raw bytes. `"nir-plugin/v1\0"`, exactly as specified.
const PACK_ID_DOMAIN_SEPARATOR: &[u8] = b"nir-plugin/v1\0";

/// RFC 0016's `pack_id = "sha256:" + SHA-256(domsep || canonical_bytes)`
/// — the plugin's content-addressed identity. `manifest_bytes` is
/// canonicalized via JCS (RFC 8785, `serde_jcs`) before hashing, so two
/// byte-different encodings of the same logical manifest (different
/// whitespace, different key order) share one identity — unlike a
/// plain `SHA-256` over whatever raw bytes happen to be on disk, which
/// is what this module hashed before this function existed. This is
/// both the value install-time tamper detection pins and the exact
/// bytes `sign_pack`/`verify_and_install_signed_pack` sign — "one
/// hash, one signed message," the RFC's own words for why a second,
/// differently-computed hash for signing would reopen the domain-
/// separation hole this scheme exists to close.
///
/// **Scope, disclosed**: the RFC's second canonicalized member,
/// `pack.invariants.nir`, is not folded into this digest — no
/// installed pack ships `primitives_nir` today (`validate_primitives_
/// nir`'s own doc comment: this toolchain refuses any pack that sets
/// it, since the native `.nir` compiler that would parse/typecheck/
/// prove it no longer lives in this crate), so there is currently
/// nothing else to canonicalize.
pub fn pack_content_id(manifest_bytes: &[u8]) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("pack manifest is not valid JSON: {e}"))?;
    let canonical =
        serde_jcs::to_vec(&value).map_err(|e| format!("canonicalizing pack manifest: {e}"))?;
    let mut payload = Vec::with_capacity(PACK_ID_DOMAIN_SEPARATOR.len() + canonical.len());
    payload.extend_from_slice(PACK_ID_DOMAIN_SEPARATOR);
    payload.extend_from_slice(&canonical);
    Ok(format!("sha256:{}", crate::hi_graph::sha256_hex(&payload)))
}

/// RFC 0016 "Manifest fields": which trust tier a pack's law is held
/// to. `Regulated` packs cannot install via trust-on-first-use — see
/// `verify_and_install_signed_pack`'s own enforcement — mirroring the
/// RFC's "a self-signed cert on a manifest declaring `domain_class:
/// regulated` fails ... verification" rule, adapted to this toolchain's
/// real trust primitive (a configured `TrustAnchor`) rather than the
/// registry-issued certificate chain the full RFC specifies (still
/// blocked on a registry operator this project doesn't have).
/// `#[serde(default)]`'d to `LongTail` on the manifest field below —
/// an unspecified pack gets the *weaker* tier, never silently upgraded
/// to a trust level its author didn't ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainClass {
    #[default]
    LongTail,
    Regulated,
}

/// A loaded pack manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    /// RFC 0016 "Manifest fields" — see [`DomainClass`].
    #[serde(default)]
    pub domain_class: DomainClass,
    /// RFC 0016 "Manifest fields": which jurisdictions this pack's law
    /// covers ("a cross-border system legitimately runs under EU + US
    /// + UK law simultaneously"). Real data, carried and round-tripped
    /// — **the deployment-side check is not wired**: the RFC's loader
    /// rule ("a deployment that declares jurisdiction J must have a
    /// plugin covering J present") needs a place for a *deployment* to
    /// declare which jurisdiction it requires, and no such concept
    /// exists anywhere in this toolchain yet. Disclosed gap, not
    /// silently assumed solved.
    #[serde(default)]
    pub jurisdictions: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<PackInvariant>,
    #[serde(default)]
    pub relations: Vec<(String, String)>,
    #[serde(default)]
    pub mandatory_fns: Vec<String>,
    /// RFC 0016 Phase 3, solution 6: the pack's own certified-primitive
    /// *code*, raw `.nir` source text (the RFC's `pack.invariants.nir`
    /// member) -- real function bodies plus their own `validate` blocks,
    /// not just attribute-carrying templates the model has to satisfy
    /// itself (that's `invariants` above, 5a's own, weaker mechanism:
    /// "the model wires the ledger; it does not write it" needs the
    /// ledger's actual code to come from the pack). Parsed and
    /// typechecked at install (`install_pack_from_bytes`) -- **never
    /// executed**, a pack is data until the graph says otherwise, same
    /// as every other loader-hardening rule this RFC states -- and every
    /// `validate` block inside it must independently PROVE at install
    /// time, not merely parse; a pack whose own certified code doesn't
    /// prove itself refuses to install. `None` for a pack with no
    /// certified primitives at all (banking-v0/fapi-2.0 today -- neither
    /// ships one yet).
    #[serde(default)]
    pub primitives_nir: Option<String>,
    /// RFC 0016's "Compliance profiles" section: a pack may additionally
    /// carry named, versioned compliance profiles (FAPI for fintech,
    /// HIPAA-flavored rules for health, ...) in exactly the four generic
    /// requirement kinds the RFC defines -- "FAPI" is never a keyword in
    /// this struct or anywhere else in `nirdosha` core, only in the data
    /// a pack like `fapi-2.0.json` carries. Empty for a pack that only
    /// declares domain invariants (banking-v0 today) -- profiles are an
    /// independent, optional payload, never implied by a pack's presence.
    #[serde(default)]
    pub compliance_profiles: Vec<ComplianceProfile>,
    /// RFC 0016 Phase 3's `primitive_exclusivity`: struct names this
    /// pack protects -- generated code may not construct a value of any
    /// of these outside a `mandatory_fns` certified primitive
    /// (`contract_check::check_primitive_exclusivity`'s own doc comment
    /// on why "construct" is the complete, not merely narrower, reading
    /// of the RFC's "any write... outside certified primitive units"
    /// for this language). Empty for every pack that doesn't need it
    /// (banking-v0/fapi-2.0 today -- neither declares struct-based state
    /// at all yet).
    #[serde(default)]
    pub protected_structs: Vec<String>,
}

/// One named, versioned compliance profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceProfile {
    pub name: String,
    pub version: String,
    /// `static_rule`: a lint the toolchain can check directly against
    /// the generated `ast::Program` -- see `check_static_rules` for the
    /// fixed set of kinds actually implemented; an unknown kind fails
    /// pack *load*, never generate (this RFC's own rule for
    /// `wiring_requirement`, applied identically here).
    #[serde(default)]
    pub static_rules: Vec<StaticRule>,
    /// `wiring_requirement`: either real runtime enforcement (today:
    /// only `sender_constrained_tokens`, DPoP -- `compiled_serve`'s own
    /// resource-server-side check) or emitted config for the deployer's
    /// real authorization server/gateway to honor (`par`, `pkce_s256`,
    /// `iss_check`, `par_request_uri_lifetime` -- nirdosha never runs an
    /// AS, so these are declared, not enforced, by design; see
    /// `render_wiring_config`). A pack declaring a kind outside this
    /// fixed set fails to load.
    #[serde(default)]
    pub wiring_requirements: Vec<WiringRequirement>,
    /// `external_conformance`: an attestation slot, not a compile-time
    /// check -- AS-runtime behavior and OIDF certification are legal/
    /// external acts this toolchain cannot verify itself (RFC 0016's own
    /// "NOT provable by us" scoping). Carried here purely as data the
    /// pack's sealed text records; `certify_code` is where an attached
    /// report would eventually be checked against, not this struct.
    #[serde(default)]
    pub external_conformance: Vec<ExternalConformanceRequirement>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticRule {
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WiringRequirement {
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalConformanceRequirement {
    pub kind: String,
    pub description: String,
}

/// `wiring_requirement` kinds this toolchain actually has an emitter
/// for -- checked at pack *load*, per this RFC's own rule ("Plugins
/// declaring unsupported wiring requirements fail at load, not at
/// generate"). `sender_constrained_tokens` is real runtime enforcement
/// (`compiled_serve`'s DPoP check); the rest are config-emission only
/// (`render_wiring_config`) -- nirdosha never runs an authorization
/// server, so PAR/PKCE/`iss`-echo have no resource-server enforcement
/// point, only a declared requirement for the deployer's real AS.
const SUPPORTED_WIRING_REQUIREMENT_KINDS: &[&str] = &[
    "sender_constrained_tokens",
    "par",
    "pkce_s256",
    "iss_check",
    "par_request_uri_lifetime",
];

/// `static_rule` kinds this toolchain actually checks -- same fail-at-
/// load discipline as wiring requirements.
const SUPPORTED_STATIC_RULE_KINDS: &[&str] = &[
    "exposed_mutating_requires_role",
    "deny_by_default_exposure",
    "no_plaintext_secret_params",
];

fn validate_compliance_profiles(manifest: &PackManifest) -> Result<(), String> {
    for profile in &manifest.compliance_profiles {
        for wr in &profile.wiring_requirements {
            if !SUPPORTED_WIRING_REQUIREMENT_KINDS.contains(&wr.kind.as_str()) {
                return Err(format!(
                    "pack `{}` profile `{}` declares wiring_requirement kind `{}`, which this toolchain has no emitter for -- refusing to load rather than silently ignoring it (supported kinds: {})",
                    manifest.id,
                    profile.name,
                    wr.kind,
                    SUPPORTED_WIRING_REQUIREMENT_KINDS.join(", ")
                ));
            }
        }
        for sr in &profile.static_rules {
            if !SUPPORTED_STATIC_RULE_KINDS.contains(&sr.kind.as_str()) {
                return Err(format!(
                    "pack `{}` profile `{}` declares static_rule kind `{}`, which this toolchain does not check -- refusing to load rather than silently ignoring it (supported kinds: {})",
                    manifest.id,
                    profile.name,
                    sr.kind,
                    SUPPORTED_STATIC_RULE_KINDS.join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// RFC 0016 Phase 3's `primitives_nir` (a pack shipping literal native
/// `.nir` source for "certified primitive" functions, Z3-proved at
/// install time) was a native-`.nir`-only concept -- there is no v2
/// equivalent, and this crate no longer depends on the native compiler
/// at all (2026-09-16's `hi` extraction). No installed pack ships
/// `primitives_nir` today (`banking-v0`/`fapi-2.0` both omit it), so
/// this refuses loudly rather than silently accepting an unprovable
/// pack, in the one case that would actually matter: a pack that DOES
/// set it.
fn validate_primitives_nir(manifest: &PackManifest) -> Result<(), String> {
    if manifest.primitives_nir.is_some() {
        return Err(format!(
            "pack `{}` declares `primitives_nir`, but this toolchain no longer has a native `.nir` compiler to parse/typecheck/prove it against (hi was split out of the native compiler crate 2026-09-16) -- ship a pack with no `primitives_nir` (mandatory_fns/protected_structs alone still work), or install it against the native `nirdosha` compiler's own pack tooling if one still exists for it",
            manifest.id
        ));
    }
    Ok(())
}

/// Every active pack's `mandatory_fns` -- the set
/// `contract_check::check_mandatory_primitive_call_sites` gates on, and
/// the set `hi_llm`'s coverage gate demands at least one real call site
/// for. Independent of `wiring_requires_sender_constrained_tokens`'s own
/// active-pack lookup, same underlying `active_pack_manifests` helper.
pub fn active_mandatory_primitive_names(
    conn: &rusqlite::Connection,
    root: &Path,
) -> Result<std::collections::HashSet<String>, String> {
    let packs = active_pack_manifests(conn, root)?;
    Ok(packs
        .iter()
        .flat_map(|m| m.mandatory_fns.iter().cloned())
        .collect())
}

/// Every active pack's `protected_structs` -- the set
/// `contract_check::check_primitive_exclusivity` gates on. Same shape
/// and same underlying `active_pack_manifests` helper as
/// `active_mandatory_primitive_names` just above.
pub fn active_protected_struct_names(
    conn: &rusqlite::Connection,
    root: &Path,
) -> Result<std::collections::HashSet<String>, String> {
    let packs = active_pack_manifests(conn, root)?;
    Ok(packs
        .iter()
        .flat_map(|m| m.protected_structs.iter().cloned())
        .collect())
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

/// The FAPI 2.0 Security Profile compliance pack. **Not** auto-installed
/// by `ensure_default_packs` -- unlike `banking-v0`, a compliance
/// profile is genuinely optional even for a banking-domain project
/// (`compliance_profiles`' own doc comment: "an independent, optional
/// payload, never implied by a pack's presence"). Installed explicitly:
/// `nirdosha plugin install agent-skills/nirdosha/packs/fapi-2.0.json`,
/// or from `hi`'s own Governing Rules panel (`hi_api::handle_pack_
/// install`, rfcs/0014's 2026-09-14 amendment) -- no longer test-only,
/// since that panel needs the same embedded bytes outside a test build.
const FAPI_2_0_JSON: &str = include_str!("../../../agent-skills/nirdosha/packs/fapi-2.0.json");

/// The fixed, known-safe set of packs `hi`'s own UI can offer to
/// install -- deliberately not an arbitrary file path from the webview
/// (this crate has no path-traversal/arbitrary-read surface to expose
/// this way): `(pack_id-as-shown, one-line description, embedded JSON
/// bytes)`. Adding a pack here is the only lever this list needs; the
/// bytes are the same ones `ensure_default_packs`/the CLI install path
/// already trust.
pub fn known_installable_packs() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "banking-v0",
            "Whole-cents money math: conservation, non-negative balances.",
            BANKING_V0_JSON,
        ),
        (
            "fapi-2.0",
            "FAPI 2.0 Security Profile: sender-constrained tokens, PAR, PKCE, role-gated mutations.",
            FAPI_2_0_JSON,
        ),
    ]
}

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
    let sha256 = pack_content_id(bytes)?;
    let manifest: PackManifest = serde_json::from_slice(bytes)
        .map_err(|e| format!("parsing pack manifest from {source_desc}: {e}"))?;
    validate_compliance_profiles(&manifest)?;
    validate_primitives_nir(&manifest)?;

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
        rusqlite::params![
            manifest.id,
            manifest.name,
            sha256,
            manifest_path.to_str(),
            now
        ],
    )
    .map_err(|e| format!("recording plugin {}: {e}", manifest.id))?;

    load_domain_pack(conn, root, &manifest, &sha256)?;
    Ok(manifest.id)
}

/// The `--dry-run` guard the RFC's "fail-closed cuts both ways" section
/// demands: perform the full load, rolling back even on success
/// (`revoke_pack`) so a dry run never persists.
///
/// **Real, disclosed reduction (2026-09-16, the `hi` v2 extraction):**
/// this used to also run the pack's own contracts against a synthesized
/// stub program via this crate's own Z3-backed `contract_check`, before
/// `hi` had no native compiler dependency at all. There is no such
/// proof engine here anymore (nor a v2 equivalent yet), so a dry run
/// today only proves `install_pack_from_bytes`'s own checks (manifest
/// schema, declared compliance-profile kinds) -- a pack whose
/// `invariants[].validates` claims are internally contradictory is no
/// longer caught here. `nirdosha`'s native `verify`/`certify` pipeline
/// (`crates/compiler`) still proves a *generated program's* `validate`
/// blocks; only this pack-authoring-time pre-check is gone.
pub fn dry_run_install(
    conn: &rusqlite::Connection,
    root: &Path,
    bytes: &[u8],
    source_desc: &str,
) -> Result<String, String> {
    let id = install_pack_from_bytes(conn, root, bytes, source_desc)?;
    let _ = revoke_pack(conn, root, &id);
    Ok(id)
}

/// Re-load every installed pack that has not been revoked, verifying
/// its on-disk bytes still match the recorded SHA-256.  Call this on
/// every `hi` startup after `hi_graph::open`.
pub fn reload_installed_packs(conn: &rusqlite::Connection, root: &Path) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, sha256 FROM plugins WHERE revoked_at IS NULL")
        .map_err(|e| format!("querying installed packs: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("querying installed packs: {e}"))?;

    for row in rows {
        let (id, recorded_sha256) = row.map_err(|e| format!("reading installed pack row: {e}"))?;
        let path = pack_manifest_path(root, &id);
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("reading pack {} at {}: {e}", id, path.display()))?;
        let actual_sha256 = pack_content_id(&bytes)?;
        if actual_sha256 != recorded_sha256 {
            return Err(format!(
                "pack {id} has been modified since install (expected sha256 {recorded_sha256}, got {actual_sha256}) -- revoke and reinstall if the change is intentional",
            ));
        }
        let manifest: PackManifest =
            serde_json::from_slice(&bytes).map_err(|e| format!("re-parsing pack {id}: {e}"))?;
        load_domain_pack(conn, root, &manifest, &actual_sha256)?;
    }
    Ok(())
}

/// Ensure the built-in banking pack v0 is installed.  Idempotent.
pub fn ensure_default_packs(conn: &rusqlite::Connection, root: &Path) -> Result<(), String> {
    install_pack_from_bytes(
        conn,
        root,
        BANKING_V0_JSON.as_bytes(),
        "embedded banking-v0",
    )?;
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
        .filter(|u| {
            u.attributes
                .iter()
                .any(|a| crate::hi_llm::demanded_contract(a).is_some())
        })
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
    lines.push(String::from("--- END DOMAIN LAW ---\n"));
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
        conn.execute("DELETE FROM edges WHERE src = ?1 OR dst = ?1", [&id])
            .map_err(|e| format!("deleting edges touching {id}: {e}"))?;
        conn.execute("DELETE FROM nodes WHERE id = ?1", [&id])
            .map_err(|e| format!("deleting node {id}: {e}"))?;
    }
    let _ = root;
    Ok(())
}

/// Print a human-readable list of installed packs to stdout.
pub fn list_packs(conn: &rusqlite::Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, sha256, installed_at, revoked_at FROM plugins ORDER BY installed_at",
        )
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
        let (id, name, sha256, installed_at, revoked_at) =
            row.map_err(|e| format!("reading pack row: {e}"))?;
        any = true;
        let status = if revoked_at.is_some() {
            "REVOKED"
        } else {
            "active"
        };
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

/// Every currently-active (non-revoked) pack's parsed manifest --
/// `list_installed_pack_manifests` reads every pack directory on disk
/// regardless of revocation status; this filters down to the ones the
/// `plugins` table still considers active, the set that actually
/// governs a build.
fn active_pack_manifests(
    conn: &rusqlite::Connection,
    root: &Path,
) -> Result<Vec<PackManifest>, String> {
    let active_ids = installed_pack_ids(conn)?;
    let all = list_installed_pack_manifests(root)?;
    Ok(all
        .into_iter()
        .filter(|(m, _)| active_ids.contains(&m.id))
        .map(|(m, _)| m)
        .collect())
}

/// RFC 0016's FAPI wiring: `true` if any active pack's compliance
/// profile declares the `sender_constrained_tokens` wiring requirement
/// -- `main.rs::cmd_build` reads this to decide whether to pass
/// `require_sender_constrained_tokens: true` into `codegen`'s
/// `ServeCodegenOptions`, which is what actually turns on
/// `compiled_serve`'s real DPoP enforcement at runtime. Never inferred
/// from anything else -- a build with no such pack governing it behaves
/// exactly as before this feature existed.
pub fn wiring_requires_sender_constrained_tokens(
    conn: &rusqlite::Connection,
    root: &Path,
) -> Result<bool, String> {
    let packs = active_pack_manifests(conn, root)?;
    Ok(packs.iter().any(|m| {
        m.compliance_profiles.iter().any(|p| {
            p.wiring_requirements
                .iter()
                .any(|w| w.kind == "sender_constrained_tokens")
        })
    }))
}

/// The config-sidecar half of `wiring_requirement`: every active pack's
/// `par`/`pkce_s256`/`iss_check`/`par_request_uri_lifetime` requirements,
/// rendered as one JSON object -- "rendered into generated config" per
/// this RFC's own "Compliance profiles" table. This is declared
/// configuration for the deployer's *real* authorization server/gateway
/// to read and enforce, not a claim that nirdosha enforces it itself:
/// `compiled_serve` is a resource server and structurally has no PAR/
/// PKCE/`iss`-echo enforcement point (see `sender_constrained_tokens`'s
/// own doc comment above for the one requirement that *is* real
/// runtime enforcement). Written by `main.rs::cmd_build` next to the
/// compiled binary when any active pack declares config-only wiring
/// requirements; `None` when there's nothing to declare, so a plain
/// build writes no sidecar at all.
pub fn render_wiring_config(
    conn: &rusqlite::Connection,
    root: &Path,
) -> Result<Option<serde_json::Value>, String> {
    let packs = active_pack_manifests(conn, root)?;
    let mut profiles_out = Vec::new();
    for pack in &packs {
        for profile in &pack.compliance_profiles {
            let config_only: Vec<&WiringRequirement> = profile
                .wiring_requirements
                .iter()
                .filter(|w| w.kind != "sender_constrained_tokens")
                .collect();
            if config_only.is_empty() {
                continue;
            }
            profiles_out.push(serde_json::json!({
                "pack_id": pack.id,
                "profile": profile.name,
                "version": profile.version,
                "wiring_requirements": config_only.iter().map(|w| serde_json::json!({ "kind": w.kind, "params": w.params })).collect::<Vec<_>>(),
                "external_conformance": profile.external_conformance.iter().map(|e| serde_json::json!({ "kind": e.kind, "description": e.description })).collect::<Vec<_>>(),
                "note": "these requirements bind the client<->authorization-server protocol, which nirdosha's compiled serve (a resource server) does not run -- the deployer's real AS/gateway must enforce them; this file is a declared requirement, not proof that anything enforces it yet.",
            }));
        }
    }
    if profiles_out.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::json!({ "compliance_wiring": profiles_out }),
    ))
}

// ===========================================================================
// RFC 0016 Phase 4 (issue #59, `docs/research/2026-09-competitive-
// verification-and-signing-landscape.md` §5): Sigstore-*pattern* pack
// signing -- the local mechanics 5b actually needs, without waiting on
// "who runs the registry" (RFC 0016's own honest blocker for 5b).
//
// **What this is, and isn't.** Sigstore's real insight, applied here:
// trust shouldn't rest on a bare, unattributed public key ("here is a
// key, trust it because someone pinned it") -- it should rest on a
// *known identity* the key is bound to, recorded in an auditable,
// append-only log. This module gives packs exactly that shape --
// `TrustAnchor` binds a public key to a human-readable identity
// (`hi_plugin::pack_signing::TrustAnchor`, an operator-configured local
// list), and `pack_signing_log()` is an append-only, human-readable
// record of every accepted signature, mirroring Rekor's role. What it
// is **not**: a live Fulcio/OIDC short-lived-certificate issuer or a
// public, cross-organization Rekor transparency log -- those need a
// registry operator this project doesn't have (RFC 0016's own point).
// Standing up live Sigstore infrastructure is real, separate follow-up
// work; the trust-anchor-list shape here is deliberately the same
// shape Sigstore verification itself reduces to once a certificate is
// checked (a public key bound to an identity), so swapping in real
// Fulcio/Rekor later is a matter of how a `TrustAnchor` gets populated,
// not a redesign of anything that consumes it -- the exact "5b swaps
// signatures in without re-architecture" property RFC 0016 asks for.
//
// **Unsigned (5a) installs are completely unchanged** --
// `install_pack_from_bytes` above still exists, still works exactly as
// before, still the default `nirdosha plugin install <path>` path. This
// module is a new, additive, opt-in layer alongside it, not a
// replacement -- signing a pack is something an author chooses to do,
// verifying one is something an operator chooses to require.

/// One signed pack, ready to install or to write to disk -- the pack
/// analogue of the native compiler crate's `verify_pipeline::SignedCertificate`, same shape, same
/// `#[serde(flatten)]`-free plain-fields style (a pack has no existing
/// unsigned JSON shape to stay compatible with the way `Certificate`
/// does, so there's nothing to flatten onto). `manifest_bytes` is the
/// pack's raw JSON exactly as `install_pack_from_bytes` already accepts
/// it -- signed as opaque bytes, not re-derived from a parsed
/// `PackManifest`, so a signature always covers exactly what gets
/// installed, byte for byte.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SignedPackEnvelope {
    /// The pack manifest's raw JSON text, verbatim.
    pub manifest_json: String,
    /// `String`, not `&'static str`: this struct derives `Deserialize`
    /// (`main.rs`'s own install-side parse) -- the native compiler crate's `verify_pipeline::Certificate::
    /// evidence_tier`'s own doc comment is the precedent for exactly
    /// this choice, and its own reasoning applies verbatim here.
    pub signature_algorithm: String,
    pub public_key: String,
    pub signature: String,
    /// A human-readable label for who signed this pack (e.g. `"banking-
    /// domain-experts@kannamma-labs"`) -- carried alongside the raw key
    /// so a trust-anchor list (below) and the transparency log can both
    /// show *who*, not just a base64 blob. Not itself verified against
    /// anything (an OIDC-bound identity, the way Fulcio actually proves
    /// this, is exactly the live-infrastructure piece this module
    /// doesn't have) -- an unauthenticated claim the signer makes about
    /// themselves, same as a git commit's own `Author` line.
    pub signer_identity: String,
}

/// Author-side: signs the pack's content-addressed identity
/// (`pack_content_id`, RFC 0016's `pack_id` — not the raw manifest
/// bytes; "one hash, one signed message") with the Ed25519 private key
/// at `key_path`, reusing `nirdosha_audit::signing::sign_bytes` -- the same primitive
/// `nirdosha certify --sign` uses, one Ed25519 implementation in this
/// crate for both certificates and packs.
pub fn sign_pack(
    manifest_bytes: &[u8],
    key_path: &str,
    signer_identity: String,
) -> Result<SignedPackEnvelope, String> {
    let manifest_json = String::from_utf8(manifest_bytes.to_vec())
        .map_err(|e| format!("pack manifest is not valid UTF-8: {e}"))?;
    // Round-trips through `PackManifest` first so a malformed manifest
    // fails here, at signing time, with a real parse error -- not
    // silently produce a validly-signed envelope around garbage that
    // only fails later, at someone else's install time.
    let _: PackManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| format!("not a valid pack manifest: {e}"))?;
    let pack_id = pack_content_id(manifest_bytes)?;
    let (public_key, signature) =
        nirdosha_audit::signing::sign_bytes(pack_id.as_bytes(), key_path)?;
    Ok(SignedPackEnvelope {
        manifest_json,
        signature_algorithm: "ed25519".to_string(),
        public_key,
        signature,
        signer_identity,
    })
}

/// One operator-accepted signing identity: a public key this
/// deployment trusts, and the human-readable identity it's bound to.
/// This *is* the trust root, in exactly the sense Sigstore's Fulcio
/// certificate is -- "this key speaks for this identity" -- just
/// configured locally by an operator instead of issued by a live CA
/// against a verified OIDC login. Real, disclosed weaker guarantee than
/// live Sigstore (this module's own top doc comment says so); a real,
/// non-zero one compared to today's 5a (no signature layer at all).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustAnchor {
    pub public_key: String,
    pub identity: String,
}

/// Where the trust-anchor list lives -- overridable via
/// `NIRDOSHA_PACK_TRUST_ANCHORS` for tests and for an operator who
/// wants it somewhere other than the default, mirroring
/// `hint_cache::cache_path`'s own env-var-override convention.
fn trust_anchors_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("NIRDOSHA_PACK_TRUST_ANCHORS") {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    std::path::PathBuf::from(home)
        .join(".nirdosha")
        .join("pack_trust_anchors.json")
}

/// Loads the configured trust-anchor list -- `Ok(vec![])`, never an
/// error, for a missing file: **no configured trust anchors means
/// trust-on-first-use, not "nothing can ever install,"** the same
/// posture 5a's own sha256 pin already takes for unsigned packs
/// (`hi_plugin.rs`'s own top doc comment: "TOFU -- named as such").
/// A malformed (present but unparseable) file IS a real error, though
/// -- unlike "absent," that's a configuration mistake worth surfacing
/// rather than silently downgrading to TOFU.
fn load_trust_anchors() -> Result<Vec<TrustAnchor>, String> {
    let path = trust_anchors_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a valid trust-anchor list: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

fn pack_signing_log_path() -> std::path::PathBuf {
    trust_anchors_path().with_file_name("pack_signing_log.jsonl")
}

/// Appends one accepted-signature record to a real hash chain
/// (`nirdosha_audit::audit_chain` -- the same primitive
/// `hint_cache.rs`'s self-repair log uses, and the one that module's
/// own doc comment names this call site as the intended second
/// consumer of). Best-effort, exactly like `hint_cache::HintCache::
/// record_success`'s own audit log: a signing decision that couldn't
/// be durably logged must never be the reason a legitimate pack
/// install fails. Unlike a plain JSON-Lines append, tampering with or
/// deleting a past entry is now detectable (`verify_pack_signing_log`)
/// -- this is the local, single-organization analogue of Rekor; see
/// this module's own top doc comment on exactly what that does and
/// doesn't claim (no public, cross-organization inclusion proof).
fn append_pack_signing_log(pack_id: &str, sha256: &str, signer_identity: &str, trust_basis: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let content = serde_json::json!({ "pack_id": pack_id, "sha256": sha256, "signer_identity": signer_identity, "trust_basis": trust_basis });
    nirdosha_audit::audit_chain::append_entry(&pack_signing_log_path(), content, now);
}

/// Verifies the local pack-signing log's hash chain is intact end to
/// end -- an entry silently edited or deleted after the fact is
/// detectable, per `nirdosha_audit::audit_chain::verify_chain`.
/// Returns the number of verified entries. This remains a *local* log
/// (RFC 0016's own 5a/5b split): it proves nobody tampered with this
/// machine's record after the fact, not that the record is complete or
/// that a public, cross-organization observer agrees with it -- that
/// half needs the registry infrastructure RFC 0016 itself says this
/// project doesn't have.
pub fn verify_pack_signing_log() -> Result<usize, String> {
    nirdosha_audit::audit_chain::verify_chain(&pack_signing_log_path()).map_err(|e| e.to_string())
}

/// Install-side: verifies `envelope`'s signature, then delegates to the
/// existing `install_pack_from_bytes` for the actual install mechanics
/// (reused, not duplicated). Three real outcomes, named honestly rather
/// than folded into one boolean:
/// - the signature doesn't verify at all -- refused, regardless of
///   trust anchors (a broken signature is never installable, full stop);
/// - it verifies, and the public key is in the configured trust-anchor
///   list -- installed, `signer_identity` recorded as that anchor's
///   *configured* identity (not the envelope's own self-claimed one --
///   the operator's own record of who that key belongs to is the
///   trustworthy one, an unauthenticated self-claim isn't);
/// - it verifies, but no trust anchors are configured at all --
///   installed anyway, **trust-on-first-use** (`load_trust_anchors`'s
///   own doc comment), `signer_identity` recorded as the envelope's own
///   self-claimed identity, logged plainly as `"tofu"` so an operator
///   auditing the log later can see exactly which installs rested on
///   TOFU versus a real configured anchor.
///
/// **Real, disclosed gap this does NOT cover**: a verified signature
/// from a public key that IS in the trust-anchor list but whose
/// *envelope* self-claims a different identity than the anchor's own
/// configured one -- handled by simply ignoring the envelope's claim in
/// that case (the anchor's own identity always wins when one exists),
/// not by refusing the install. A key present in the trust-anchor list
/// under multiple identities, or removed/rotated mid-flight, is an
/// operator-side list-hygiene question this module doesn't referee.
///
/// **RFC 0016 `domain_class` gating**: a fourth outcome, checked after
/// signature verification succeeds -- a pack that declares
/// `domain_class: regulated` and would otherwise install via TOFU is
/// refused outright. "Self-signed is not enough for regulated domains"
/// (the RFC's own words); the real mechanism here is "a configured
/// `TrustAnchor` is not enough for TOFU," this toolchain's closest
/// available stand-in for the registry-issued certificate the full RFC
/// specifies (still blocked on a registry operator this project
/// doesn't have).
pub fn verify_and_install_signed_pack(
    conn: &rusqlite::Connection,
    root: &Path,
    envelope: &SignedPackEnvelope,
    source_desc: &str,
) -> Result<String, String> {
    let manifest_bytes = envelope.manifest_json.as_bytes();
    let pack_id = pack_content_id(manifest_bytes)?;
    let valid = nirdosha_audit::signing::verify_bytes(
        pack_id.as_bytes(),
        &envelope.public_key,
        &envelope.signature,
    )?;
    if !valid {
        return Err(format!(
            "{source_desc}: signature does not verify against the embedded public key -- refusing to install"
        ));
    }

    let manifest: PackManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("parsing pack manifest from {source_desc}: {e}"))?;

    let anchors = load_trust_anchors()?;
    let (identity, trust_basis) = match anchors.iter().find(|a| a.public_key == envelope.public_key)
    {
        Some(anchor) => (anchor.identity.clone(), "trust_anchor"),
        None => (envelope.signer_identity.clone(), "tofu"),
    };

    if manifest.domain_class == DomainClass::Regulated && trust_basis == "tofu" {
        return Err(format!(
            "{source_desc}: pack `{}` declares domain_class \"regulated\" -- trust-on-first-use is not sufficient for a regulated domain (RFC 0016); configure a trust anchor for public key {} (see NIRDOSHA_PACK_TRUST_ANCHORS) before installing",
            manifest.id, envelope.public_key
        ));
    }

    let installed_id = install_pack_from_bytes(conn, root, manifest_bytes, source_desc)?;
    conn.execute(
        "UPDATE plugins SET signer_identity = ?1 WHERE id = ?2",
        rusqlite::params![identity, installed_id],
    )
    .map_err(|e| format!("recording signer_identity for {installed_id}: {e}"))?;
    append_pack_signing_log(&installed_id, &pack_id, &identity, trust_basis);
    Ok(installed_id)
}

/// The signer identity recorded for an installed pack, if it was
/// installed through `verify_and_install_signed_pack` -- `None` for
/// every 5a, unsigned install (this column's own migration default).
/// **Wired into the UI's trust indicator (2026-09-15)**:
/// `hi_api::handle_packs_list`'s `/api/packs` response carries this
/// verbatim, plus a derived `trust_indicator` ("signed"/"unsigned") --
/// `agent-skills/nirdosha/hi_ux_redesign_options.md`'s "trust indicator
/// (who signed it...)" mockup, previously never wired to this function
/// (this doc comment's own prior text said so). the native compiler crate's `verify_pipeline::
/// Certificate::governing_packs` attributing not just *which* packs
/// governed an artifact but who sealed them is still real, separate
/// follow-up work -- this wiring is the UI-facing half, not the
/// certificate-facing one.
pub fn pack_signer_identity(
    conn: &rusqlite::Connection,
    pack_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT signer_identity FROM plugins WHERE id = ?1",
        [pack_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .map_err(|e| format!("reading signer_identity for {pack_id}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "nirdosha_hi_plugin_test_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn pack_with_primitives(id: &str, primitives_nir: &str, mandatory_fns: &[&str]) -> String {
        serde_json::json!({
            "id": id,
            "name": format!("test pack {id}"),
            "mandatory_fns": mandatory_fns,
            "primitives_nir": primitives_nir,
        })
        .to_string()
    }

    fn pack_with_mandatory_fns(id: &str, mandatory_fns: &[&str]) -> String {
        serde_json::json!({
            "id": id,
            "name": format!("test pack {id}"),
            "mandatory_fns": mandatory_fns,
        })
        .to_string()
    }

    #[test]
    fn a_pack_declaring_primitives_nir_refuses_to_install() {
        // `primitives_nir` (native `.nir` source, Z3-proved at install
        // time) has no v2 equivalent -- this crate has no native
        // compiler dependency at all since the 2026-09-16 `hi`
        // extraction. Refusing loudly here (rather than silently
        // accepting an unprovable pack) is the honest behavior; no
        // installed pack ships `primitives_nir` today.
        let dir = scratch_dir("primitives_nir_unsupported");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let bytes = pack_with_primitives(
            "ledger-v0",
            "fn transfer(amount: i64) -> i64 { return amount }",
            &["transfer"],
        );
        let err = install_pack_from_bytes(&conn, &dir, bytes.as_bytes(), "test")
            .expect_err("a pack declaring primitives_nir must be refused");
        assert!(err.contains("primitives_nir"), "got: {err}");
        assert!(installed_pack_ids(&conn).expect("list").is_empty());
    }

    #[test]
    fn active_mandatory_primitive_names_reflects_only_active_packs() {
        let dir = scratch_dir("mandatory_names");
        let conn = crate::hi_graph::open(&dir).expect("open");
        assert!(
            active_mandatory_primitive_names(&conn, &dir)
                .expect("check")
                .is_empty()
        );

        let id = install_pack_from_bytes(
            &conn,
            &dir,
            pack_with_mandatory_fns("ledger-v0-4", &["transfer"]).as_bytes(),
            "test",
        )
        .expect("install");
        let names = active_mandatory_primitive_names(&conn, &dir).expect("check");
        assert!(names.contains("transfer"), "got: {names:?}");

        revoke_pack(&conn, &dir, &id).expect("revoke");
        assert!(
            active_mandatory_primitive_names(&conn, &dir)
                .expect("check")
                .is_empty(),
            "a revoked pack's mandatory_fns must no longer count"
        );
    }

    /// Same shape as `active_mandatory_primitive_names_reflects_only_
    /// active_packs` just above, for the sibling `protected_structs`
    /// field (RFC 0016 Phase 3's `primitive_exclusivity`).
    #[test]
    fn active_protected_struct_names_reflects_only_active_packs() {
        let dir = scratch_dir("protected_struct_names");
        let conn = crate::hi_graph::open(&dir).expect("open");
        assert!(
            active_protected_struct_names(&conn, &dir)
                .expect("check")
                .is_empty()
        );

        let bytes = serde_json::json!({
            "id": "ledger-v0-protected",
            "name": "test pack with a protected struct",
            "mandatory_fns": ["transfer"],
            "protected_structs": ["Account"],
        })
        .to_string();
        let id = install_pack_from_bytes(&conn, &dir, bytes.as_bytes(), "test").expect("install");
        let names = active_protected_struct_names(&conn, &dir).expect("check");
        assert!(names.contains("Account"), "got: {names:?}");

        revoke_pack(&conn, &dir, &id).expect("revoke");
        assert!(
            active_protected_struct_names(&conn, &dir)
                .expect("check")
                .is_empty(),
            "a revoked pack's protected_structs must no longer count"
        );
    }

    #[test]
    fn dry_run_install_installs_the_banking_pack_and_does_not_persist() {
        let dir = scratch_dir("dry_run_banking_v0");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = dry_run_install(&conn, &dir, BANKING_V0_JSON.as_bytes(), "dry-run test")
            .expect("banking-v0 must install");
        assert_eq!(id, "banking-v0");
        assert!(
            installed_pack_ids(&conn).expect("list").is_empty(),
            "a dry run must not persist the pack"
        );
    }

    #[test]
    fn default_banking_manifest_parses() {
        let manifest = default_banking_manifest();
        assert_eq!(manifest.id, "banking-v0");
        assert!(manifest.invariants.iter().any(|i| i.name == "charge_cents"));
    }

    #[test]
    fn fapi_2_0_pack_loads_cleanly_and_its_wiring_requirements_pass_the_load_time_allowlist() {
        let dir = scratch_dir("fapi_install");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = install_pack_from_bytes(&conn, &dir, FAPI_2_0_JSON.as_bytes(), "fapi-2.0 test").expect("the real fapi-2.0.json pack must load -- every wiring_requirement/static_rule kind it declares must be on the supported allowlist");
        assert_eq!(id, "fapi-2.0");
    }

    #[test]
    fn a_pack_declaring_an_unsupported_wiring_requirement_fails_at_load_not_at_generate() {
        let dir = scratch_dir("fapi_unsupported_wiring");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let bogus = r#"{
  "id": "bogus-profile",
  "name": "a pack claiming a wiring kind nirdosha has no emitter for",
  "compliance_profiles": [
    { "name": "bogus", "version": "1", "wiring_requirements": [{ "kind": "quantum_teleportation_binding" }] }
  ]
}"#;
        let err = install_pack_from_bytes(&conn, &dir, bogus.as_bytes(), "bogus test")
            .expect_err("an unsupported wiring_requirement kind must refuse the load");
        assert!(err.contains("quantum_teleportation_binding"), "got:\n{err}");
        assert!(
            installed_pack_ids(&conn).expect("list").is_empty(),
            "a load-time refusal must not partially install anything"
        );
    }

    #[test]
    fn a_pack_declaring_an_unsupported_static_rule_fails_at_load_not_at_generate() {
        let dir = scratch_dir("fapi_unsupported_static_rule");
        let conn = crate::hi_graph::open(&dir).expect("open");
        let bogus = r#"{
  "id": "bogus-profile-2",
  "name": "a pack claiming a static_rule kind nirdosha does not check",
  "compliance_profiles": [
    { "name": "bogus", "version": "1", "static_rules": [{ "kind": "no_time_travel" }] }
  ]
}"#;
        let err = install_pack_from_bytes(&conn, &dir, bogus.as_bytes(), "bogus test")
            .expect_err("an unsupported static_rule kind must refuse the load");
        assert!(err.contains("no_time_travel"), "got:\n{err}");
    }

    #[test]
    fn wiring_requires_sender_constrained_tokens_is_false_until_fapi_is_installed_then_true() {
        let dir = scratch_dir("fapi_wiring_flag");
        let conn = crate::hi_graph::open(&dir).expect("open");
        ensure_default_packs(&conn, &dir).expect("ensure default packs"); // banking-v0 only
        assert!(
            !wiring_requires_sender_constrained_tokens(&conn, &dir).expect("check"),
            "banking-v0 alone must not require DPoP"
        );

        install_pack_from_bytes(&conn, &dir, FAPI_2_0_JSON.as_bytes(), "fapi-2.0 test")
            .expect("install fapi-2.0");
        assert!(
            wiring_requires_sender_constrained_tokens(&conn, &dir).expect("check"),
            "once fapi-2.0 is installed, sender-constrained tokens must be required"
        );
    }

    #[test]
    fn render_wiring_config_declares_par_pkce_and_iss_check_but_not_sender_constrained_tokens() {
        let dir = scratch_dir("fapi_wiring_config");
        let conn = crate::hi_graph::open(&dir).expect("open");
        install_pack_from_bytes(&conn, &dir, FAPI_2_0_JSON.as_bytes(), "fapi-2.0 test")
            .expect("install fapi-2.0");

        let config = render_wiring_config(&conn, &dir)
            .expect("render")
            .expect("fapi-2.0 has config-only requirements to declare");
        let text = config.to_string();
        assert!(text.contains("\"par\""), "got:\n{text}");
        assert!(text.contains("\"pkce_s256\""), "got:\n{text}");
        assert!(text.contains("\"iss_check\""), "got:\n{text}");
        // The one requirement that's real runtime enforcement, not
        // declared config, must not appear in the sidecar -- it has
        // nothing to do with a downstream AS/gateway.
        assert!(
            !text.contains("sender_constrained_tokens"),
            "sender_constrained_tokens is compiled_serve's own enforcement, not declared config:\n{text}"
        );
    }

    #[test]
    fn render_wiring_config_is_none_when_no_installed_pack_declares_config_only_wiring() {
        let dir = scratch_dir("fapi_wiring_config_none");
        let conn = crate::hi_graph::open(&dir).expect("open");
        ensure_default_packs(&conn, &dir).expect("ensure default packs"); // banking-v0 has no compliance profile at all
        assert!(render_wiring_config(&conn, &dir).expect("render").is_none());
    }

    /// A fresh Ed25519 keypair for pack-signing tests, mirroring
    /// `nirdosha keygen`'s own `generate_pkcs8` call -- written to a
    /// real file since `sign_pack`/`nirdosha_audit::signing::sign_bytes` both take a
    /// key *path*, the same interface `nirdosha certify --sign` uses.
    fn generate_test_key(dir: &std::path::Path) -> std::path::PathBuf {
        let rng = nirdosha_audit::crypto_backend::rand::SystemRandom::new();
        let pkcs8 = nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .expect("key generation");
        let path = dir.join("signer.pk8");
        std::fs::write(&path, pkcs8.as_ref()).expect("write key");
        path
    }

    /// `NIRDOSHA_PACK_TRUST_ANCHORS` is a process-global env var, and
    /// three separate tests below each set/read/clear it -- `cargo
    /// test`'s default multi-threaded runner would otherwise let one
    /// test's write race another's read (confirmed the hard way: this
    /// lock was added after `verify_and_install_signed_pack_prefers_
    /// the_configured_anchor_identity_over_the_envelopes_own_claim`
    /// failed under real parallel execution, reading a sibling test's
    /// path instead of its own). Every test that touches the env var
    /// acquires this for its whole body, serializing them against each
    /// other without needing `--test-threads=1` for the whole suite --
    /// the same `TEST_LOCK` pattern `runtime-kernels/src/kernel/
    /// recorder.rs`'s own tests already use for an identical class of
    /// process-global-state hazard.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Scopes `NIRDOSHA_PACK_TRUST_ANCHORS` to a path under `dir` for
    /// the duration of one test body -- same "isolate the global env
    /// var to one test's own scratch dir" discipline
    /// `hint_cache.rs::tests::record_success_then_lookup_round_trips_
    /// through_disk` already established for the same class of problem
    /// (a process-global path resolved from an env var). Callers must
    /// hold `ENV_TEST_LOCK` for their whole test body, not just this call.
    fn point_trust_anchors_at(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("trust_anchors.json");
        unsafe { std::env::set_var("NIRDOSHA_PACK_TRUST_ANCHORS", &path) };
        path
    }

    /// End-to-end: sign the real embedded banking-v0 pack, install it
    /// with no trust anchors configured at all -- must succeed via
    /// trust-on-first-use, recording the envelope's own self-claimed
    /// identity (`load_trust_anchors`'s own doc comment on why absent
    /// is TOFU, not refusal).
    #[test]
    fn verify_and_install_signed_pack_accepts_an_unconfigured_key_via_tofu() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("sign_tofu");
        point_trust_anchors_at(&dir); // points at a path that doesn't exist -- no anchors configured
        let key = generate_test_key(&dir);
        // A long-tail pack (the default `domain_class` when unset) --
        // TOFU is only refused for `domain_class: regulated`, see
        // `regulated_pack_refuses_tofu_install_without_a_configured_
        // trust_anchor` below for that case, now that `banking-v0`
        // itself declares `regulated`.
        let bytes = pack_with_mandatory_fns("ledger-v0-tofu", &["transfer"]);
        let envelope = sign_pack(
            bytes.as_bytes(),
            key.to_str().unwrap(),
            "some-domain-experts@example.org".to_string(),
        )
        .expect("signing must succeed");

        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = verify_and_install_signed_pack(&conn, &dir, &envelope, "test")
            .expect("TOFU install must succeed");
        assert_eq!(id, "ledger-v0-tofu");
        assert_eq!(
            pack_signer_identity(&conn, &id).unwrap(),
            Some("some-domain-experts@example.org".to_string())
        );

        unsafe { std::env::remove_var("NIRDOSHA_PACK_TRUST_ANCHORS") };
    }

    /// RFC 0016: "a self-signed cert on a manifest declaring
    /// `domain_class: regulated` fails ... verification." This
    /// toolchain's stand-in for that registry-issued-certificate
    /// requirement: a regulated pack cannot install via TOFU, full
    /// stop, no matter how valid its signature is -- it needs an
    /// operator-configured `TrustAnchor` for the signing key. Real
    /// `banking-v0` (now `domain_class: "regulated"`), not a synthetic
    /// fixture, so this pins the toolchain's actual shipped demo pack
    /// against the rule it's the RFC's own motivating example for.
    #[test]
    fn regulated_pack_refuses_tofu_install_without_a_configured_trust_anchor() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("sign_regulated_tofu_refused");
        let anchors_path = point_trust_anchors_at(&dir); // no anchors configured yet
        let key = generate_test_key(&dir);
        let envelope = sign_pack(
            BANKING_V0_JSON.as_bytes(),
            key.to_str().unwrap(),
            "banking-domain-experts@kannamma-labs".to_string(),
        )
        .expect("signing must succeed");

        let conn = crate::hi_graph::open(&dir).expect("open");
        let err = verify_and_install_signed_pack(&conn, &dir, &envelope, "test")
            .expect_err("a regulated pack must never install via TOFU");
        assert!(err.contains("regulated"), "got: {err}");
        assert!(err.contains("trust-on-first-use"), "got: {err}");
        assert!(
            installed_pack_ids(&conn).expect("list").is_empty(),
            "a refused install must not partially persist"
        );

        // The same envelope installs cleanly once the signer's key is a
        // configured trust anchor -- regulated packs are not
        // uninstallable, they just can't rest on TOFU.
        std::fs::write(
            &anchors_path,
            serde_json::to_string(&[TrustAnchor {
                public_key: envelope.public_key.clone(),
                identity: "Banking Domain Experts (configured)".to_string(),
            }])
            .unwrap(),
        )
        .unwrap();
        let id = verify_and_install_signed_pack(&conn, &dir, &envelope, "test")
            .expect("install must succeed once the key is a configured trust anchor");
        assert_eq!(id, "banking-v0");

        unsafe { std::env::remove_var("NIRDOSHA_PACK_TRUST_ANCHORS") };
    }

    /// A key present in the configured trust-anchor list installs the
    /// same way, but records the *anchor's own* configured identity,
    /// not the envelope's self-claimed one -- an operator's own record
    /// of who a key belongs to is the trustworthy source, not an
    /// unauthenticated claim inside the envelope itself
    /// (`verify_and_install_signed_pack`'s own doc comment).
    #[test]
    fn verify_and_install_signed_pack_prefers_the_configured_anchor_identity_over_the_envelopes_own_claim()
     {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("sign_trust_anchor");
        let anchors_path = point_trust_anchors_at(&dir);
        let key = generate_test_key(&dir);
        let envelope = sign_pack(
            BANKING_V0_JSON.as_bytes(),
            key.to_str().unwrap(),
            "self-claimed-identity".to_string(),
        )
        .expect("signing must succeed");

        std::fs::write(
            &anchors_path,
            serde_json::to_string(&[TrustAnchor {
                public_key: envelope.public_key.clone(),
                identity: "Configured Anchor Identity".to_string(),
            }])
            .unwrap(),
        )
        .expect("write trust anchors");

        let conn = crate::hi_graph::open(&dir).expect("open");
        let id = verify_and_install_signed_pack(&conn, &dir, &envelope, "test")
            .expect("install under a configured anchor must succeed");
        assert_eq!(
            pack_signer_identity(&conn, &id).unwrap(),
            Some("Configured Anchor Identity".to_string()),
            "the anchor's own configured identity must win over the envelope's self-claim"
        );

        unsafe { std::env::remove_var("NIRDOSHA_PACK_TRUST_ANCHORS") };
    }

    /// A tampered manifest (the signature no longer matches) must be
    /// refused outright, regardless of trust anchors -- a broken
    /// signature is never installable.
    #[test]
    fn verify_and_install_signed_pack_refuses_a_tampered_manifest() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("sign_tampered");
        point_trust_anchors_at(&dir);
        let key = generate_test_key(&dir);
        let mut envelope = sign_pack(
            BANKING_V0_JSON.as_bytes(),
            key.to_str().unwrap(),
            "someone".to_string(),
        )
        .expect("signing must succeed");
        envelope.manifest_json = envelope
            .manifest_json
            .replace("Banking domain law v0", "TAMPERED");

        let conn = crate::hi_graph::open(&dir).expect("open");
        let err = verify_and_install_signed_pack(&conn, &dir, &envelope, "test")
            .expect_err("a tampered manifest must be refused");
        assert!(
            err.contains("does not verify"),
            "the refusal must name the real reason: {err}"
        );

        unsafe { std::env::remove_var("NIRDOSHA_PACK_TRUST_ANCHORS") };
    }

    /// `sign_pack` itself refuses a manifest that doesn't even parse as
    /// a `PackManifest` -- caught at signing time, not left to surprise
    /// whoever tries to install the resulting envelope later
    /// (`sign_pack`'s own doc comment on why).
    #[test]
    fn sign_pack_refuses_a_manifest_that_does_not_parse() {
        let dir = scratch_dir("sign_bad_manifest");
        let key = generate_test_key(&dir);
        let err = sign_pack(
            b"{\"not\": \"a real pack manifest\"}",
            key.to_str().unwrap(),
            "someone".to_string(),
        )
        .expect_err("a non-PackManifest JSON blob must be refused");
        assert!(err.contains("not a valid pack manifest"), "got: {err}");
    }

    /// RFC 0016 "the pack ID is the law": before JCS canonicalization,
    /// this module hashed whatever raw bytes were on disk, so two
    /// byte-different encodings of the identical logical manifest
    /// (here: compact vs. pretty-printed JSON, and a different key
    /// order) minted two different identities/signatures for "the same"
    /// pack. `pack_content_id` must treat them as one.
    #[test]
    fn pack_content_id_is_stable_across_equivalent_json_encodings() {
        let compact = r#"{"id":"ledger-v0","name":"n","mandatory_fns":["transfer"]}"#;
        let pretty = "{\n  \"name\": \"n\",\n  \"id\": \"ledger-v0\",\n  \"mandatory_fns\": [\"transfer\"]\n}\n";
        let id_compact = pack_content_id(compact.as_bytes()).expect("canonicalizes");
        let id_pretty = pack_content_id(pretty.as_bytes()).expect("canonicalizes");
        assert_eq!(
            id_compact, id_pretty,
            "reformatting/reordering keys must not change the pack's identity"
        );
        assert!(id_compact.starts_with("sha256:"), "got: {id_compact}");

        // A real content change (even just whitespace-insignificant vs.
        // an actual different value) must still change the ID.
        let different = r#"{"id":"ledger-v0","name":"n","mandatory_fns":["transfer","refund"]}"#;
        assert_ne!(id_compact, pack_content_id(different.as_bytes()).unwrap());
    }

    /// `sign_pack`/`verify_and_install_signed_pack` sign and verify the
    /// content-addressed `pack_id`, not the raw manifest bytes -- so
    /// re-serializing the same manifest (the shape `manifest_json`
    /// travels as, e.g. pretty vs. compact) must not invalidate an
    /// already-issued signature.
    #[test]
    fn a_signature_survives_re_serialization_of_an_equivalent_manifest() {
        let dir = scratch_dir("sign_reserialize");
        let key = generate_test_key(&dir);
        let compact = pack_with_mandatory_fns("ledger-v0-reserialize", &["transfer"]);
        let mut envelope = sign_pack(
            compact.as_bytes(),
            key.to_str().unwrap(),
            "someone".to_string(),
        )
        .expect("signing must succeed");
        let value: serde_json::Value = serde_json::from_str(&envelope.manifest_json).unwrap();
        envelope.manifest_json = serde_json::to_string_pretty(&value).unwrap();
        assert!(
            nirdosha_audit::signing::verify_bytes(
                pack_content_id(envelope.manifest_json.as_bytes())
                    .unwrap()
                    .as_bytes(),
                &envelope.public_key,
                &envelope.signature
            )
            .unwrap(),
            "a pretty-printed re-serialization of the identical manifest must still verify"
        );
    }

    /// `verify_pack_signing_log`'s own real hash-chain tamper detection
    /// -- an install decision that's been silently rewritten in the
    /// local log file after the fact must be detectable, not just
    /// "append-only by convention."
    #[test]
    fn pack_signing_log_detects_tampering() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("signing_log_tamper");
        point_trust_anchors_at(&dir);
        let key = generate_test_key(&dir);
        let bytes = pack_with_mandatory_fns("ledger-v0-log", &["transfer"]);
        let envelope = sign_pack(
            bytes.as_bytes(),
            key.to_str().unwrap(),
            "someone".to_string(),
        )
        .expect("signing must succeed");
        let conn = crate::hi_graph::open(&dir).expect("open");
        verify_and_install_signed_pack(&conn, &dir, &envelope, "test").expect("install");
        assert_eq!(verify_pack_signing_log().expect("a fresh log verifies"), 1);

        let log_path = pack_signing_log_path();
        let text = std::fs::read_to_string(&log_path).unwrap();
        let tampered = text.replace("\"tofu\"", "\"trust_anchor\"");
        assert_ne!(
            text, tampered,
            "the log must actually contain the trust_basis this test tampers with"
        );
        std::fs::write(&log_path, tampered).unwrap();
        assert!(
            verify_pack_signing_log().is_err(),
            "editing a past entry must be detected"
        );

        unsafe { std::env::remove_var("NIRDOSHA_PACK_TRUST_ANCHORS") };
    }
}

/// Convenience for tests: build the in-memory representation of the
/// default banking pack without touching disk.
pub fn default_banking_manifest() -> PackManifest {
    serde_json::from_str(BANKING_V0_JSON).expect("embedded banking-v0.json parses")
}
