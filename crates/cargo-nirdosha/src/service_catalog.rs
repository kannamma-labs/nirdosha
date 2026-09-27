//! RFC 0029 §7.1 domain service catalog: typed loading and validation for
//! a template's `policy-services.toml` (schema: `docs/SERVICE_CATALOG_SCHEMA.json`).
//!
//! A screen's `[screen.service]` reference may only select an entry from
//! this catalog and set the parameters it declares — the catalog, not the
//! screen and not an LLM, is the source of the security-critical meaning.
//! Everything here is validated at generation time, fail-closed: an
//! unknown service, unsupported policy kind, or unregistered gateway is a
//! hard generation error, never a warning or a silently-skipped screen.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// Enforcement gateways this generator knows how to bind a service entry
/// to. A `gateway` name outside this list fails catalog loading — adding a
/// real gateway means adding it here deliberately, not discovering a typo
/// at codegen time on some unrelated screen.
pub const KNOWN_GATEWAYS: &[&str] = &["transfer_request_gateway_v1", "funds_reserve_gateway_v1"];

/// RFC 0029 §8 policy kinds with published formal artifacts (operational
/// semantics, composition algebra, proof obligations, evidence schema) as
/// of the Phase 0 exit gate (§8.1-§8.4). The other ten kinds are
/// formal-artifact backlog and are rejected by `serde` here (an unknown
/// variant is a parse error), not silently accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PolicyKind {
    Authorization,
    DataPolicy,
    NumericInvariant,
    StateInvariant,
}

/// v1 scope: `FailClosed` is the only representable posture. A single-
/// variant enum still buys the same "unsupported value is a parse error"
/// guarantee a hand-rolled string check would need reimplementing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum FailurePosture {
    FailClosed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceEntry {
    pub resource: String,
    pub effect: String,
    pub policy_kind: PolicyKind,
    pub gateway: String,
    #[serde(default)]
    pub required_facts: Vec<String>,
    #[serde(default)]
    pub allowed_parameters: Vec<String>,
    pub failure_posture: FailurePosture,
}

/// A screen's `[screen.service]` reference into the catalog. Deserialized
/// straight off `ScreenDecl` in `generate.rs`; `parameters` stays raw TOML
/// until `policy_adapter::build_input` validates it against the resolved
/// entry's `allowed_parameters`.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceRef {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "empty_table")]
    pub parameters: toml::Value,
}

fn default_version() -> String {
    "1".to_string()
}

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

#[derive(Debug, Clone, Deserialize)]
struct RawCatalog {
    #[serde(default)]
    service: BTreeMap<String, ServiceEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceCatalog {
    pub services: BTreeMap<String, ServiceEntry>,
}

impl ServiceCatalog {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&ServiceEntry> {
        self.services.get(name)
    }
}

#[derive(Debug)]
pub enum CatalogError {
    /// Covers TOML syntax errors, a duplicate `[service.X]` key (the TOML
    /// spec requires unique keys; `toml::from_str` refuses a duplicate
    /// rather than silently keeping the last one), and an unsupported
    /// `policy_kind`/`failure_posture` value (an unknown enum variant is a
    /// deserialize error, not a defaulted-away typo).
    Parse(String),
    UnknownGateway { service: String, gateway: String },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::Parse(e) => write!(f, "service catalog is invalid: {e}"),
            CatalogError::UnknownGateway { service, gateway } => write!(
                f,
                "service `{service}` names gateway `{gateway}`, which is not in this generator's known-gateway list ({})",
                KNOWN_GATEWAYS.join(", ")
            ),
        }
    }
}

/// Parse and validate a `policy-services.toml` source string. Gateway
/// registration is checked here, at load time, so a screen that later
/// references this service fails fast on the catalog's own defect rather
/// than surfacing a confusing error deep in codegen.
pub fn parse(src: &str) -> Result<ServiceCatalog, CatalogError> {
    let raw: RawCatalog = toml::from_str(src).map_err(|e| CatalogError::Parse(e.to_string()))?;
    for (name, entry) in &raw.service {
        if !KNOWN_GATEWAYS.contains(&entry.gateway.as_str()) {
            return Err(CatalogError::UnknownGateway {
                service: name.clone(),
                gateway: entry.gateway.clone(),
            });
        }
    }
    Ok(ServiceCatalog { services: raw.service })
}

/// Load and validate `<project_dir>/policy-bundle.toml` -- the governed
/// envelope every catalog entry is admitted under. Reuses
/// `nirdosha_guard_rfc0029::PolicyBundle`'s own parser/validator (not a
/// second implementation): the same bundle_hash this computes here is what
/// the generated app's runtime gateway recomputes from its own embedded
/// copy of the file, so the two can never silently drift.
pub fn load_bundle(project_dir: &std::path::Path) -> Result<nirdosha_guard_rfc0029::PolicyBundle, String> {
    let path = project_dir.join("policy-bundle.toml");
    let src = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    nirdosha_guard_rfc0029::PolicyBundle::from_toml_str(&src).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[service.transfer_create]
resource = "TransferRequest"
effect = "transfer.request"
policy_kind = "Authorization"
gateway = "transfer_request_gateway_v1"
required_facts = ["subject_identity"]
allowed_parameters = ["eligible_roles", "jurisdiction"]
failure_posture = "FailClosed"
"#;

    #[test]
    fn parses_valid_catalog() {
        let catalog = parse(VALID).expect("valid catalog");
        let entry = catalog.get("transfer_create").expect("entry present");
        assert_eq!(entry.effect, "transfer.request");
        assert_eq!(entry.policy_kind, PolicyKind::Authorization);
    }

    #[test]
    fn rejects_unknown_gateway() {
        let src = VALID.replace("transfer_request_gateway_v1", "made_up_gateway");
        let err = parse(&src).unwrap_err();
        assert!(matches!(err, CatalogError::UnknownGateway { .. }));
    }

    #[test]
    fn rejects_unsupported_policy_kind() {
        let src = VALID.replace("Authorization", "Sequence");
        let err = parse(&src).unwrap_err();
        assert!(matches!(err, CatalogError::Parse(_)));
    }

    #[test]
    fn rejects_duplicate_service_key() {
        let src = format!("{VALID}\n[service.transfer_create]\nresource = \"X\"\neffect = \"x.y\"\npolicy_kind = \"Authorization\"\ngateway = \"transfer_request_gateway_v1\"\nfailure_posture = \"FailClosed\"\n");
        let err = parse(&src).unwrap_err();
        assert!(matches!(err, CatalogError::Parse(_)));
    }
}
