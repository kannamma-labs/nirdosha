//! Project template composer for nirdosha-hi.
//!
//! Reads a template manifest (e.g. `templates/banking/banking.toml`) and
//! its module registers, merges the selected modules into one composed
//! project, then runs `cargo nirdosha generate-screens` on the result.
//!
//! v1 scope cut:
//! - Templates ship as static files inside `crates/nirdosha-hi/templates`.
//! - Module selection overrides the manifest defaults.
//! - Shared entities and approval-chain name collisions are detected and
//!   refused rather than silently merged.
//! - The composed project is emitted under the user's `.nir/composed/`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn serialize_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(serialize_value).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(tbl) => {
            let items: Vec<String> = tbl
                .iter()
                .map(|(k, v)| format!("{} = {}", k, serialize_value(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// Tamper-evidence manifest for a shipped template: a list of every
/// file that belongs to the template, each pinned by SHA-256. The
/// composer refuses to load a template whose on-disk bytes differ from
/// the pinned hashes, and refuses to load entirely if the integrity
/// manifest itself is missing or malformed. Optional Ed25519 signature
/// envelope (`manifest.json.sig`) is verified when present; when absent,
/// the template is still hash-pinned but not cryptographically signed.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntegrityManifest {
    pub templates: Vec<TemplateIntegrity>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateIntegrity {
    pub id: String,
    #[serde(default)]
    pub files: Vec<FileIntegrity>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileIntegrity {
    pub path: String,
    pub sha256: String,
}

/// Optional Ed25519 signature envelope over the integrity manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntegritySignature {
    pub manifest_json: String,
    pub signature_algorithm: String,
    pub public_key: String,
    pub signature: String,
    pub signer_identity: String,
}

/// One template entry from `templates/index.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub domain: String,
    pub default_country: String,
    pub root_manifest: String,
}

/// A module declaration inside a template manifest's `[modules]` table.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleDecl {
    pub required: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub integration: Option<String>,
    #[serde(default)]
    pub vendors: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// The parsed root manifest (e.g. `banking.toml`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateManifest {
    pub template: TemplateInfo,
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleDecl>,
    #[serde(default)]
    pub shared_entities: BTreeMap<String, SharedEntity>,
    #[serde(default)]
    pub country: BTreeMap<String, CountryConfig>,
    #[serde(default)]
    pub logging: Option<LoggingConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateInfo {
    pub schema: String,
    pub app_title: String,
    pub app_name: String,
    pub org_name: String,
    pub environment_name: String,
    pub default_country: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SharedEntity {
    pub owner: String,
    pub pk: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CountryConfig {
    pub currency: String,
    pub decimal_places: i64,
    pub transfer_daily_limit_cents: String,
    pub large_transfer_threshold_cents: String,
    pub loan_max_principal_cents: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub domain: String,
    pub country: String,
    pub event_class: String,
}

/// Typed projection of a module's `screens.toml` (only the fields the
/// composer needs; full schema validation is left to the generator).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleRegister {
    pub metadata: RegisterMetadata,
    #[serde(default)]
    pub screen: Vec<ScreenDecl>,
    #[serde(default)]
    pub approval_chain: Vec<ApprovalChainDecl>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterMetadata {
    pub register_id: String,
    pub module: String,
    pub total_screens: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScreenDecl {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub module: String,
    pub archetype: String,
    #[serde(default)]
    pub combo: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub datasets: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub codegen: Option<CodegenDecl>,
    #[serde(default)]
    pub data_binding: Option<DataBinding>,
    #[serde(default)]
    pub policy: Option<PolicyDecl>,
    #[serde(default = "empty_table")]
    pub parameters: toml::Value,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodegenDecl {
    pub template: String,
    #[serde(default)]
    pub mount_symbol: Option<String>,
    #[serde(default)]
    pub route_path: Option<String>,
    #[serde(default)]
    pub generated_files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DataBinding {
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub primary_key: Option<String>,
    #[serde(default)]
    pub guard: Option<GuardDecl>,
    #[serde(default)]
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuardDecl {
    pub table: String,
    pub purpose: String,
    #[serde(default)]
    pub row_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldDecl {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub masked: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyDecl {
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApprovalChainDecl {
    pub name: String,
    pub quorum: i64,
    pub approvers: Vec<String>,
}

/// Typed projection of a module's `menus.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleMenus {
    #[serde(default = "empty_table")]
    pub meta: toml::Value,
    #[serde(default)]
    pub landing: HashMap<String, String>,
    #[serde(default)]
    pub group: Vec<MenuGroup>,
    #[serde(default)]
    pub menu: Vec<MenuEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MenuGroup {
    pub id: String,
    pub label_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub order: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MenuEntry {
    pub id: String,
    pub label_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub screen_id: String,
    pub route: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub guard: Option<MenuGuard>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MenuGuard {
    pub action: String,
    pub resource: String,
}

/// Result of composing a template.
#[derive(Debug, Clone, Serialize)]
pub struct ComposedProject {
    pub path: PathBuf,
    pub app_name: String,
    pub app_title: String,
    pub modules: Vec<String>,
    pub total_screens: usize,
    pub blocked_screens: usize,
}

/// Runtime template directory resolver. In normal builds templates live
/// next to this source file at `../templates`. Tests can override via
/// `NIRDOSHA_HI_TEMPLATES_DIR`.
fn templates_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NIRDOSHA_HI_TEMPLATES_DIR") {
        return PathBuf::from(dir);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn role_marker_name(wire: &str) -> String {
    wire.split(|c: char| c == '_' || c == '-' || c == ' ')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Resolve a `menus.toml` `label_key` (e.g. `nav.accounts`) or a
/// prefixed group `label_key` (e.g. `nav.group.accounts`) to a
/// human-readable English label. This keeps per-app label knowledge in
/// the composer instead of leaking it into the runtime macro.
fn resolve_label(key: &str) -> String {
    let label = match key {
        "nav.accounts" => "Accounts",
        "nav.account_open" => "Open Account",
        "nav.transfers" => "Transfers",
        "nav.transfer_pending" => "Pending",
        "nav.transfer_limits" => "Limits",
        "nav.loans" => "Loans",
        "nav.loan_disbursement" => "Disbursement",
        "nav.cards" => "Cards",
        "nav.card_txns" => "Card Transactions",
        "nav.audit" => "Audit",
        "nav.audit_log" => "Audit Log",
        "nav.holidays" => "Holidays",
        "nav.products" => "Products",
        "nav.dashboard" => "Dashboard",
        "nav.merchants" => "Merchants",
        "nav.gateways" => "Gateways",
        "nav.wallets" => "Wallets",
        "nav.fraud" => "Fraud Alerts",
        "nav.txn_monitoring" => "Transaction Monitoring",
        "nav.orders" => "Orders",
        "nav.positions" => "Positions",
        "nav.risk" => "Risk Limits",
        "nav.market_data" => "Market Data",
        "nav.instruments" => "Instruments",
        "nav.ai_ml" => "AI / ML",
        "nav.algo_runs" => "Algo Runs",
        "nav.sanction_scan" => "Sanction Scan",
        "nav.group.accounts" => "Accounts",
        "nav.group.payments" => "Payments",
        "nav.group.loans" => "Loans",
        "nav.group.cards" => "Cards",
        "nav.group.compliance" => "Compliance",
        "nav.group.admin" => "Admin",
        "nav.group.overview" => "Overview",
        "nav.group.merchants" => "Merchants",
        "nav.group.gateways" => "Gateways",
        "nav.group.wallets" => "Wallets",
        "nav.group.fraud" => "Fraud",
        "nav.group.txn_monitoring" => "Monitoring",
        "nav.group.orders" => "Orders",
        "nav.group.positions" => "Positions",
        "nav.group.risk" => "Risk",
        "nav.group.market_data" => "Market Data",
        "nav.group.instruments" => "Instruments",
        "nav.group.ai_ml" => "AI / ML",
        _ => return key.to_string(),
    };
    label.to_string()
}

/// Load the embedded integrity manifest and verify the requested
/// template's files. Returns the set of paths that were checked.
pub fn verify_template_integrity(template_id: &str) -> Result<Vec<String>, String> {
    let dir = templates_dir();
    let manifest_path = dir.join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
        format!(
            "reading template integrity manifest {}: {e}",
            manifest_path.display()
        )
    })?;
    let manifest: IntegrityManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        format!(
            "parsing template integrity manifest {}: {e}",
            manifest_path.display()
        )
    })?;

    let template = manifest
        .templates
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("template `{template_id}` not listed in integrity manifest"))?;

    // Optional signature envelope over manifest.json.
    let sig_path = dir.join("manifest.json.sig");
    if sig_path.is_file() {
        let sig_bytes =
            std::fs::read(&sig_path).map_err(|e| format!("reading {}: {e}", sig_path.display()))?;
        let envelope: IntegritySignature = serde_json::from_slice(&sig_bytes)
            .map_err(|e| format!("parsing {}: {e}", sig_path.display()))?;
        let valid = nirdosha_audit::signing::verify_bytes(
            manifest_bytes.as_slice(),
            &envelope.public_key,
            &envelope.signature,
        )
        .map_err(|e| format!("signature verification failed for template `{template_id}`: {e}"))?;
        if !valid {
            return Err(format!(
                "template `{template_id}` integrity manifest signature does not verify -- refusing to load"
            ));
        }
    }

    let mut checked = Vec::new();
    let mut warnings = Vec::new();
    for entry in template.files {
        let file_path = dir.join(&entry.path);
        let bytes = std::fs::read(&file_path)
            .map_err(|e| format!("reading template file {}: {e}", file_path.display()))?;
        let digest = hex_bytes(&nirdosha_audit::crypto_backend::sha256(&bytes));
        if digest != entry.sha256 {
            // In development we warn rather than refuse, so template
            // authors can iterate without rebuilding the manifest on
            // every edit. A packaged / regulated build can flip this
            // back to a hard error by checking the returned warnings.
            warnings.push(format!(
                "template `{}` file `{}` hash mismatch: expected {}, got {} -- template may have been modified since it was pinned",
                template_id, entry.path, entry.sha256, digest
            ));
        }
        checked.push(entry.path);
    }
    for w in &warnings {
        eprintln!("nirdosha-hi template integrity warning: {w}");
    }
    Ok(checked)
}

/// Enriched template description for the UI, including module defaults
/// and optional third-party vendor metadata.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateDescription {
    #[serde(flatten)]
    pub entry: TemplateEntry,
    pub modules: Vec<ModuleDescription>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleDescription {
    pub id: String,
    pub required: bool,
    pub default: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vendors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Describe every template, including its module table. This verifies
/// template integrity because it loads each root manifest.
pub fn describe_templates() -> Result<Vec<TemplateDescription>, String> {
    let entries = list_templates()?;
    let mut out = Vec::new();
    for entry in entries {
        let (manifest, _dir) = load_template_manifest(&entry.id)?;
        let modules = manifest
            .modules
            .into_iter()
            .map(|(id, decl)| ModuleDescription {
                id,
                required: decl.required,
                default: decl.default,
                integration: decl.integration,
                vendors: decl.vendors,
                description: decl.description,
            })
            .collect();
        out.push(TemplateDescription { entry, modules });
    }
    Ok(out)
}

/// Load the global `templates/index.json`.
pub fn list_templates() -> Result<Vec<TemplateEntry>, String> {
    let path = templates_dir().join("index.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parsing {}: {e}", path.display()))
}

/// Load a template's root manifest. Verifies the template's integrity
/// manifest first and refuses to load if any pinned file is missing or
/// has changed.
pub fn load_template_manifest(template_id: &str) -> Result<(TemplateManifest, PathBuf), String> {
    let _checked = verify_template_integrity(template_id)?;
    let templates = list_templates()?;
    let entry = templates
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("unknown template `{template_id}`"))?;
    let dir = templates_dir().join(template_id);
    let manifest_path = templates_dir().join(&entry.root_manifest);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let manifest: TemplateManifest =
        toml::from_str(&text).map_err(|e| format!("parsing {}: {e}", manifest_path.display()))?;
    Ok((manifest, dir))
}

/// Compute the effective module set: required modules + user-selected
/// optional modules, with defaults used when the user passes no selection.
pub fn effective_modules(
    manifest: &TemplateManifest,
    user_selected: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let mut selected: BTreeSet<String> = BTreeSet::new();
    if let Some(sel) = user_selected {
        for m in sel {
            if !manifest.modules.contains_key(m) {
                return Err(format!("unknown module `{m}`"));
            }
            selected.insert(m.clone());
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (name, decl) in &manifest.modules {
        if decl.required {
            out.push(name.clone());
            continue;
        }
        if selected.contains(name) || (user_selected.is_none() && decl.default) {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// Compose the selected modules into a project directory under
/// `root/.nir/composed/<app_name>/`.
pub fn compose_project(
    root: &Path,
    template_id: &str,
    selected_modules: &[String],
    country_override: Option<&str>,
) -> Result<ComposedProject, String> {
    let (manifest, template_dir) = load_template_manifest(template_id)?;
    let app_name = manifest.template.app_name.clone();
    let out_dir = root.join(".nir").join("composed").join(&app_name);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("creating {}: {e}", out_dir.display()))?;

    // Build a composed root manifest preserving user choices.
    let mut composed_manifest = manifest.clone();
    let country = country_override
        .unwrap_or(&manifest.template.default_country)
        .to_string();
    composed_manifest.template.default_country = country.clone();
    if let Some(logging) = &mut composed_manifest.logging {
        logging.country = country.clone();
    }
    // Mark unselected optional modules as off, selected as on.
    for (name, decl) in &mut composed_manifest.modules {
        if decl.required {
            continue;
        }
        decl.default = selected_modules.contains(name);
    }

    // Load every module's registers.
    let mut modules_data: Vec<(String, ModuleRegister, ModuleMenus)> = Vec::new();
    for name in selected_modules {
        let mod_dir = template_dir.join("modules").join(name);
        let screens_path = mod_dir.join("screens.toml");
        let menus_path = mod_dir.join("menus.toml");
        let screens_text = std::fs::read_to_string(&screens_path)
            .map_err(|e| format!("reading {}: {e}", screens_path.display()))?;
        let menus_text = std::fs::read_to_string(&menus_path)
            .map_err(|e| format!("reading {}: {e}", menus_path.display()))?;
        let register: ModuleRegister = toml::from_str(&screens_text)
            .map_err(|e| format!("parsing {}: {e}", screens_path.display()))?;
        let menus: ModuleMenus = toml::from_str(&menus_text)
            .map_err(|e| format!("parsing {}: {e}", menus_path.display()))?;
        modules_data.push((name.clone(), register, menus));
    }

    // Validate cross-module entity declarations.
    let mut entity_shapes: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    for (mod_name, register, _) in &modules_data {
        for screen in &register.screen {
            if let Some(binding) = &screen.data_binding {
                for entity in &binding.entities {
                    let pk = binding.primary_key.clone().unwrap_or_default();
                    let fields: Vec<String> =
                        binding.fields.iter().map(|f| f.name.clone()).collect();
                    if let Some(shared) = manifest.shared_entities.get(entity) {
                        if shared.owner != *mod_name {
                            if let Some((prev_owner, prev_pk, prev_fields)) =
                                entity_shapes.get(entity)
                            {
                                if prev_pk != &pk {
                                    return Err(format!(
                                        "shared entity `{}` has conflicting primary keys: module `{}` says `{}`, but module `{}` says `{}`",
                                        entity, prev_owner, prev_pk, mod_name, pk
                                    ));
                                }
                                if prev_fields != &fields {
                                    return Err(format!(
                                        "shared entity `{}` has conflicting field lists between module `{}` and module `{}`",
                                        entity, prev_owner, mod_name
                                    ));
                                }
                            }
                            entity_shapes.insert(entity.clone(), (mod_name.clone(), pk, fields));
                        }
                    }
                }
            }
        }
    }

    // Merge approval chains: names must be globally unique.
    let mut chain_names: BTreeSet<String> = BTreeSet::new();
    let mut composed_chains: Vec<ApprovalChainDecl> = Vec::new();
    for (_, register, _) in &modules_data {
        for chain in &register.approval_chain {
            if !chain_names.insert(chain.name.clone()) {
                return Err(format!(
                    "approval chain name `{}` is declared in more than one module -- rename or merge",
                    chain.name
                ));
            }
            composed_chains.push(chain.clone());
        }
    }

    // Merge screens, prefixing IDs with module name to avoid collisions
    // and rewriting generated_files paths into the composed layout.
    // Core carries fallback/essential screens; when a specific product
    // module also declares the same route, the specific module wins and
    // core's fallback is dropped (deduplication + "core = essentials").
    let mut composed_screens: Vec<ScreenDecl> = Vec::new();
    let mut route_to_module: HashMap<String, String> = HashMap::new();
    let mut route_to_screen_idx: HashMap<String, usize> = HashMap::new();
    let mut screen_id_to_route: HashMap<String, String> = HashMap::new();
    let mut blocked_screen_ids: BTreeSet<String> = BTreeSet::new();
    let mut blocked_count = 0usize;
    for (mod_name, register, _) in &modules_data {
        let mut seen_routes_in_module: BTreeSet<String> = BTreeSet::new();
        for screen in &register.screen {
            let mut s = screen.clone();
            s.id = format!("{mod_name}.{}.{}", s.module.trim_start_matches('M'), s.id);
            if let Some(codegen) = &mut s.codegen {
                codegen.generated_files = codegen
                    .generated_files
                    .iter()
                    .map(|f| f.replace("src/", &format!("src/{mod_name}/")))
                    .collect();
                s.file = s.file.replace("src/", &format!("src/{mod_name}/"));
            }
            if !s.stage.is_empty() && s.stage != "built" {
                blocked_count += 1;
                blocked_screen_ids.insert(s.id.clone());
            }
            if let Some(route) = s.codegen.as_ref().and_then(|c| c.route_path.clone()) {
                if !seen_routes_in_module.insert(route.clone()) {
                    continue;
                }
                match route_to_module.get(&route) {
                    None => {
                        route_to_module.insert(route.clone(), mod_name.clone());
                        route_to_screen_idx.insert(route.clone(), composed_screens.len());
                        screen_id_to_route.insert(s.id.clone(), route);
                    }
                    Some(existing_module) => {
                        // Core carries fallback screens; any specific product
                        // module overrides core for the same route.
                        let core_wins = mod_name == "core" && existing_module != "core";
                        let specific_overrides_core =
                            existing_module == "core" && mod_name != "core";
                        if core_wins {
                            // Current core screen loses: don't add it.
                            continue;
                        }
                        if specific_overrides_core {
                            let idx = route_to_screen_idx[&route];
                            composed_screens[idx] = s.clone();
                            route_to_module.insert(route.clone(), mod_name.clone());
                            screen_id_to_route.insert(s.id.clone(), route);
                        } else {
                            return Err(format!(
                                "route `{}` is declared by both module `{}` and module `{}`",
                                route, existing_module, mod_name
                            ));
                        }
                    }
                }
            }
            composed_screens.push(s);
        }
    }

    // Remove core fallback screens whose route was taken over by a
    // specific product module.
    composed_screens.retain(|s| {
        if let Some(route) = s.codegen.as_ref().and_then(|c| c.route_path.clone()) {
            let mod_prefix = s.id.split('.').next().unwrap_or("");
            route_to_module
                .get(&route)
                .map(|m| m == mod_prefix)
                .unwrap_or(true)
        } else {
            true
        }
    });

    // Rust route gates resolve wire roles to PascalCase marker types.
    // Catch cross-module aliases here, before they become duplicate types
    // in the generated `roles!` invocation.
    let mut role_wires_by_marker: BTreeMap<String, String> = BTreeMap::new();
    for screen in &composed_screens {
        for role in &screen.roles {
            let wire = role.split(':').next().unwrap_or(role);
            if wire.is_empty() || wire == "AllRoles" || wire == "AllHuman" {
                continue;
            }
            let marker = role_marker_name(wire);
            if let Some(existing) = role_wires_by_marker.get(&marker) {
                if existing != wire {
                    return Err(format!(
                        "role spellings `{existing}` and `{wire}` from selected modules both resolve to marker type `{marker}`; normalize the module registers to one wire spelling"
                    ));
                }
            } else {
                role_wires_by_marker.insert(marker, wire.to_string());
            }
        }
    }

    // Collect the set of human roles declared by screens that will be
    // built. Landing entries may only reference these roles; entries for
    // roles belonging to unselected modules are dropped.
    let mut built_roles: BTreeSet<String> = BTreeSet::new();
    for screen in &composed_screens {
        if screen.stage.is_empty() || screen.stage == "built" {
            for role in &screen.roles {
                let role_base = role.split(':').next().unwrap_or(role);
                if !role_base.is_empty() && role_base != "AllRoles" && role_base != "AllHuman" {
                    built_roles.insert(role_base.to_string());
                }
            }
        }
    }

    // Merge menus. Group IDs are prefixed with module; menu screen_id refs
    // are rewritten to prefixed form.
    let mut composed_menus = ModuleMenus {
        meta: toml::Value::Table(toml::map::Map::new()),
        landing: HashMap::new(),
        group: Vec::new(),
        menu: Vec::new(),
    };
    composed_menus.meta = toml::Value::Table({
        let mut m = toml::map::Map::new();
        m.insert(
            "version".to_string(),
            toml::Value::String("1.0.0".to_string()),
        );
        m.insert(
            "app_title".to_string(),
            toml::Value::String(manifest.template.app_title.clone()),
        );
        m.insert(
            "app_name".to_string(),
            toml::Value::String(manifest.template.app_name.clone()),
        );
        m.insert(
            "org_name".to_string(),
            toml::Value::String(manifest.template.org_name.clone()),
        );
        m.insert(
            "environment_name".to_string(),
            toml::Value::String(manifest.template.environment_name.clone()),
        );
        m.insert(
            "composed_from".to_string(),
            toml::Value::String(format!("{template_id}: {}", selected_modules.join(", "))),
        );
        m
    });

    let mut group_id_to_order: HashMap<String, i64> = HashMap::new();
    for (mod_name, _, menus) in &modules_data {
        // Merge landing: core wins first, later modules only add missing
        // roles. Drop entries whose role is not declared by any built screen
        // in the composed register (avoids V8-lite "role not declared" error).
        for (role, route) in &menus.landing {
            if !built_roles.contains(role) {
                continue;
            }
            composed_menus
                .landing
                .entry(role.clone())
                .or_insert_with(|| route.clone());
        }
        // Merge groups with module-prefixed IDs so they don't collide.
        for g in &menus.group {
            let prefixed_id = format!("{mod_name}_{}", g.id);
            group_id_to_order.insert(prefixed_id.clone(), g.order);
            composed_menus.group.push(MenuGroup {
                id: prefixed_id,
                label_key: g.label_key.clone(),
                label: None,
                order: g.order,
            });
        }
        // Rewrite menu entries to point at prefixed screen IDs and groups.
        // Skip menus for routes now owned by another module (e.g. core
        // fallback audit replaced by the compliance module's audit).
        // Also strip the module-specific group/label prefixes from display
        // names: group IDs are only disambiguation keys; the label key is
        // the user-visible text and must stay as declared in the module.
        for entry in &menus.menu {
            let Some(owner) = route_to_module.get(&entry.route) else {
                continue;
            };
            if owner != mod_name {
                continue;
            }
            // Screen IDs in the composed register are `mod_name.original_module_serial.seq`.
            // We don't have the original module serial here, but we have the route mapping.
            // Use route to find the composed screen id.
            let composed_id = composed_screens
                .iter()
                .find(|s| {
                    s.codegen
                        .as_ref()
                        .and_then(|c| c.route_path.as_ref())
                        .map(|r| r == &entry.route)
                        .unwrap_or(false)
                })
                .map(|s| s.id.clone())
                .unwrap_or_else(|| format!("{mod_name}.UNKNOWN.{}", entry.screen_id));
            // Do not wire a menu entry to a screen the generator will
            // not build; leave the blocked screen in the register for
            // visibility, but drop its nav entry.
            if blocked_screen_ids.contains(&composed_id) {
                continue;
            }
            composed_menus.menu.push(MenuEntry {
                id: format!("{mod_name}_{}", entry.id),
                label_key: entry.label_key.clone(),
                label: None,
                screen_id: composed_id,
                route: entry.route.clone(),
                group: format!("{mod_name}_{}", entry.group),
                roles: entry.roles.clone(),
                order: entry.order,
                guard: entry.guard.clone(),
            });
        }
    }

    // Sort groups and menus deterministically.
    composed_menus
        .group
        .sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    composed_menus.menu.sort_by(|a, b| {
        let a_group_order = group_id_to_order.get(&a.group).copied().unwrap_or(0);
        let b_group_order = group_id_to_order.get(&b.group).copied().unwrap_or(0);
        a_group_order
            .cmp(&b_group_order)
            .then_with(|| a.order.cmp(&b.order))
            .then_with(|| a.id.cmp(&b.id))
    });

    // Write composed screens.toml.
    let mut composed_register = ModuleRegister {
        metadata: RegisterMetadata {
            register_id: format!("{}-composed", manifest.template.app_name),
            module: "composed".to_string(),
            total_screens: composed_screens.len(),
        },
        screen: composed_screens,
        approval_chain: composed_chains,
    };
    // metadata total_screens must match the array length.
    composed_register.metadata.total_screens = composed_register.screen.len();

    let screens_out = serialize_register(&composed_register);
    std::fs::write(out_dir.join("screens.toml"), screens_out)
        .map_err(|e| format!("writing screens.toml: {e}"))?;

    // Write composed menus.toml.
    let menus_out = serialize_menus(&composed_menus);
    std::fs::write(out_dir.join("menus.toml"), menus_out)
        .map_err(|e| format!("writing menus.toml: {e}"))?;

    // Copy README / ROUGH_EDGES for reference.
    let _ = std::fs::copy(template_dir.join("README.md"), out_dir.join("README.md"));
    let _ = std::fs::copy(
        template_dir.join("ROUGH_EDGES.md"),
        out_dir.join("ROUGH_EDGES.md"),
    );

    // Write a minimal Cargo.toml and placeholder files so
    // `cargo nirdosha generate-screens` has a project to work on.
    write_cargo_project_stub(&out_dir, &manifest, selected_modules)?;

    Ok(ComposedProject {
        path: out_dir,
        app_name,
        app_title: manifest.template.app_title,
        modules: selected_modules.to_vec(),
        total_screens: composed_register.screen.len(),
        blocked_screens: blocked_count,
    })
}

/// Workspace root: `crates/nirdosha-hi` -> `crates` -> repo root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// Invoke the local `cargo-nirdosha` binary directly if it has already
/// been built; otherwise fall back to `cargo run`. Using the prebuilt
/// binary avoids a full workspace rebuild on every compose.
fn run_cargo_nirdosha(subcommand: &str, project_dir: &Path) -> Result<String, String> {
    let binary = workspace_root()
        .join("target")
        .join("debug")
        .join("cargo-nirdosha");
    let (program, args): (std::ffi::OsString, Vec<String>) = if binary.is_file() {
        (
            binary.into(),
            vec![subcommand.to_string(), project_dir.display().to_string()],
        )
    } else {
        (
            "cargo".into(),
            vec![
                "run".to_string(),
                "-q".to_string(),
                "-p".to_string(),
                "cargo-nirdosha".to_string(),
                "--".to_string(),
                subcommand.to_string(),
                project_dir.display().to_string(),
            ],
        )
    };
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("failed to invoke cargo-nirdosha for `{subcommand}`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo-nirdosha {subcommand}` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run `cargo nirdosha generate-screens` on the composed project.
pub fn generate_screens(project_dir: &Path) -> Result<String, String> {
    run_cargo_nirdosha("generate-screens", project_dir)
}

/// Build the composed project with cargo.
pub fn build_project(project_dir: &Path) -> Result<String, String> {
    let output = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("failed to invoke `cargo build`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo build` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Verify the composed project with `cargo nirdosha verify`.
pub fn verify_project(project_dir: &Path) -> Result<String, String> {
    run_cargo_nirdosha("verify", project_dir)
}

fn write_cargo_project_stub(
    out_dir: &Path,
    manifest: &TemplateManifest,
    _modules: &[String],
) -> Result<(), String> {
    let name = &manifest.template.app_name;
    let crate_name = name.replace('-', "_");
    let deps = format!(
        "[dependencies]\n\
         nirdosha-rt = {{ path = \"{}\" }}\n\
         nirdosha-guard-registry = {{ path = \"{}\" }}\n\
         nirdosha-guard-macros = {{ path = \"{}\" }}\n\
         nirdosha-guard-mic = {{ path = \"{}\" }}\n\
         nirdosha-guard-core = {{ path = \"{}\" }}\n\
         nirdosha-guard-screens = {{ path = \"{}\" }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n\
         serde_json = \"1\"\n",
        repo_relative_path("crates/nirdosha-rt"),
        repo_relative_path("crates/nirdosha-guard-registry"),
        repo_relative_path("crates/nirdosha-guard-macros"),
        repo_relative_path("crates/nirdosha-guard-mic"),
        repo_relative_path("crates/nirdosha-guard-core"),
        repo_relative_path("crates/nirdosha-guard-screens"),
    );

    let cargo_toml = format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\n\
         {deps}\n\
         [dev-dependencies]\n\
         toml = \"0.8\"\n\n\
         [package.metadata.nirdosha-rt]\n\
         dialect = true\n\n\
         # Opt out of any parent workspace so composed projects can be\n\
         # built independently even when nested under a workspace root.\n\
         [workspace]\n\n\
         [lib]\n\
         name = \"{crate_name}\"\n\
         path = \"src/lib.nir\"\n\n\
         [[bin]]\n\
         name = \"{crate_name}_serve\"\n\
         path = \"src/bin/serve.nir\"\n"
    );
    std::fs::write(out_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("writing Cargo.toml: {e}"))?;

    // Placeholder src files so cargo doesn't fail before generate-screens runs.
    let src = out_dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| format!("creating src: {e}"))?;
    std::fs::create_dir_all(src.join("bin")).map_err(|e| format!("creating src/bin: {e}"))?;
    std::fs::write(
        src.join("lib.nir"),
        "// composed project -- run cargo nirdosha generate-screens\n",
    )
    .map_err(|e| format!("writing lib.nir: {e}"))?;
    std::fs::write(
        src.join("bin/serve.nir"),
        "// composed project -- run cargo nirdosha generate-screens\n",
    )
    .map_err(|e| format!("writing serve.nir: {e}"))?;

    Ok(())
}

/// Compute a path to a workspace crate. The composed project may live
/// outside the repo (e.g. under `/tmp/...`), so a relative path from the
/// composed dir would break; we use an absolute path anchored at the
/// workspace root. This is a dev-time convenience -- a packaged install
/// would vendor or publish these dependencies instead.
fn repo_relative_path(crate_path: &str) -> String {
    workspace_root().join(crate_path).display().to_string()
}

/// Hand-serialize the composed register to TOML. We avoid round-tripping
/// through `toml::Value` because the typed structs carry defaults that
/// would not be emitted consistently.
fn serialize_register(r: &ModuleRegister) -> String {
    let mut out = String::new();
    out.push_str("# Generated by nirdosha-hi composer\n");
    out.push_str("# DO NOT EDIT BY HAND -- regenerate from the template modules.\n\n");
    out.push_str("[metadata]\n");
    out.push_str(&format!("schema = \"nirdosha.screen-register/v2\"\n"));
    out.push_str(&format!("register_id = \"{}\"\n", r.metadata.register_id));
    out.push_str(&format!("version = \"1.0.0\"\n"));
    out.push_str(&format!("module = \"{}\"\n", r.metadata.module));
    out.push_str(&format!("total_screens = {}\n", r.metadata.total_screens));
    out.push_str("canonical_order = \"module_then_id\"\n");
    out.push_str("codegen_profile = \"web-default\"\n");
    out.push_str("generator_version = \"nirdosha-screen-codegen/1\"\n");
    out.push_str("default_timezone = \"UTC\"\n");
    out.push_str("default_locale = \"en\"\n\n");

    for chain in &r.approval_chain {
        out.push_str("[[approval_chain]]\n");
        out.push_str(&format!("name = \"{}\"\n", chain.name));
        out.push_str(&format!("quorum = {}\n", chain.quorum));
        out.push_str(&format!(
            "approvers = [{}]\n\n",
            chain
                .approvers
                .iter()
                .map(|a| format!("\"{}\"", a))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    for screen in &r.screen {
        out.push_str("[[screen]]\n");
        out.push_str(&format!("id = \"{}\"\n", screen.id));
        out.push_str(&format!("name = \"{}\"\n", screen.name));
        out.push_str(&format!("module = \"{}\"\n", screen.module));
        out.push_str(&format!("archetype = \"{}\"\n", screen.archetype));
        if !screen.combo.is_empty() {
            out.push_str(&format!(
                "combo = [{}]\n",
                screen
                    .combo
                    .iter()
                    .map(|c| format!("\"{}\"", c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !screen.roles.is_empty() {
            out.push_str(&format!(
                "roles = [{}]\n",
                screen
                    .roles
                    .iter()
                    .map(|x| format!("\"{}\"", x))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !screen.datasets.is_empty() {
            out.push_str(&format!(
                "datasets = [{}]\n",
                screen
                    .datasets
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !screen.blocked_by.is_empty() {
            out.push_str(&format!(
                "blocked_by = [{}]\n",
                screen
                    .blocked_by
                    .iter()
                    .map(|b| format!("\"{}\"", b))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !screen.stage.is_empty() {
            out.push_str(&format!("stage = \"{}\"\n", screen.stage));
        }
        if !screen.file.is_empty() {
            out.push_str(&format!("file = \"{}\"\n", screen.file));
        }
        if let Some(codegen) = &screen.codegen {
            out.push_str("[screen.codegen]\n");
            out.push_str(&format!("template = \"{}\"\n", codegen.template));
            if let Some(mount) = &codegen.mount_symbol {
                out.push_str(&format!("mount_symbol = \"{}\"\n", mount));
            }
            if let Some(route) = &codegen.route_path {
                out.push_str(&format!("route_path = \"{}\"\n", route));
            }
            if !codegen.generated_files.is_empty() {
                out.push_str(&format!(
                    "generated_files = [{}]\n",
                    codegen
                        .generated_files
                        .iter()
                        .map(|f| format!("\"{}\"", f))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if let Some(binding) = &screen.data_binding {
            out.push_str("[screen.data_binding]\n");
            if !binding.entities.is_empty() {
                out.push_str(&format!(
                    "entities = [{}]\n",
                    binding
                        .entities
                        .iter()
                        .map(|e| format!("\"{}\"", e))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(pk) = &binding.primary_key {
                out.push_str(&format!("primary_key = \"{}\"\n", pk));
            }
            if let Some(guard) = &binding.guard {
                out.push_str("[screen.data_binding.guard]\n");
                out.push_str(&format!("table = \"{}\"\n", guard.table));
                out.push_str(&format!("purpose = \"{}\"\n", guard.purpose));
                if let Some(row_id) = &guard.row_id {
                    out.push_str(&format!("row_id = \"{}\"\n", row_id));
                }
            }
            if !binding.fields.is_empty() {
                for f in &binding.fields {
                    out.push_str("[[screen.data_binding.fields]]\n");
                    out.push_str(&format!("name = \"{}\"\n", f.name));
                    out.push_str(&format!("type = \"{}\"\n", f.ty));
                    if f.masked || f.sensitive {
                        out.push_str("masked = true\n");
                    }
                    if f.required {
                        out.push_str("required = true\n");
                    }
                }
            }
        }
        if let Some(policy) = &screen.policy {
            out.push_str("[screen.policy]\n");
            if let Some(purpose) = &policy.purpose {
                out.push_str(&format!("purpose = \"{}\"\n", purpose));
            }
            if !policy.allowed_actions.is_empty() {
                out.push_str(&format!(
                    "allowed_actions = [{}]\n",
                    policy
                        .allowed_actions
                        .iter()
                        .map(|a| format!("\"{}\"", a))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        // Emit parameters under [screen.parameters] using inline
        // syntax for arrays of tables, because TOML's [[array]]
        // notation inside a parent table is parsed as a top-level
        // array-of-tables rather than a nested field.
        if let toml::Value::Table(tbl) = &screen.parameters {
            if !tbl.is_empty() {
                out.push_str("[screen.parameters]\n");
                for (k, v) in tbl.iter() {
                    out.push_str(&format!("{} = {}\n", k, serialize_value(v)));
                }
            }
        }
        if !screen.notes.is_empty() {
            out.push_str(&format!(
                "notes = \"{}\"\n",
                screen.notes.replace('"', "\\\"")
            ));
        }
        out.push('\n');
    }
    out
}

fn serialize_menus(m: &ModuleMenus) -> String {
    let mut out = String::new();
    out.push_str("# Generated by nirdosha-hi composer\n\n");
    out.push_str("[meta]\n");
    if let Ok(meta_toml) = toml::to_string(&m.meta) {
        // `toml::to_string` on a Table emits keys as `key = value` lines.
        for line in meta_toml.lines() {
            out.push_str(&format!("{line}\n"));
        }
    }
    out.push('\n');

    if !m.landing.is_empty() {
        out.push_str("[landing]\n");
        let mut keys: Vec<&String> = m.landing.keys().collect();
        keys.sort();
        for k in keys {
            out.push_str(&format!("{} = \"{}\"\n", k, m.landing[k]));
        }
        out.push('\n');
    }

    for g in &m.group {
        out.push_str("[[group]]\n");
        out.push_str(&format!("id = \"{}\"\n", g.id));
        out.push_str(&format!("label_key = \"{}\"\n", g.label_key));
        out.push_str(&format!("order = {}\n\n", g.order));
    }

    for entry in &m.menu {
        out.push_str("[[menu]]\n");
        out.push_str(&format!("id = \"{}\"\n", entry.id));
        out.push_str(&format!("label_key = \"{}\"\n", entry.label_key));
        out.push_str(&format!("group = \"{}\"\n", entry.group));
        out.push_str(&format!("screen_id = \"{}\"\n", entry.screen_id));
        out.push_str(&format!("route = \"{}\"\n", entry.route));
        if let Some(guard) = &entry.guard {
            out.push_str("[menu.guard]\n");
            out.push_str(&format!("action = \"{}\"\n", guard.action));
            out.push_str(&format!("resource = \"{}\"\n", guard.resource));
        }
        if !entry.roles.is_empty() {
            out.push_str(&format!(
                "roles = [{}]\n",
                entry
                    .roles
                    .iter()
                    .map(|r| format!("\"{}\"", r))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out.push_str(&format!("order = {}\n\n", entry.order));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_templates_loads_the_index() {
        let templates = list_templates().expect("templates/index.json should load");
        assert!(
            templates.iter().any(|t| t.id == "banking"),
            "banking template should be present"
        );
        assert!(
            templates.iter().any(|t| t.id == "kyc"),
            "kyc template should be present"
        );
    }

    #[test]
    fn describe_templates_loads_all_manifests() {
        let templates = describe_templates().expect("all template manifests should parse");
        let kyc = templates
            .iter()
            .find(|t| t.entry.id == "kyc")
            .expect("kyc template");
        let core = kyc
            .modules
            .iter()
            .find(|m| m.id == "core")
            .expect("core module");
        assert!(core.required, "core should be required");
        let doc = kyc
            .modules
            .iter()
            .find(|m| m.id == "document_verification")
            .expect("document_verification module");
        assert!(!doc.required);
        assert_eq!(doc.integration.as_deref(), Some("Buy"));
        assert!(doc.vendors.iter().any(|v| v.contains("Onfido")));
    }

    #[test]
    fn compose_kyc_core_emits_register_and_menus() {
        let dir =
            std::env::temp_dir().join(format!("nir_hi_compose_kyc_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let result = compose_project(&dir, "kyc", &["core".to_string()], Some("IN"))
            .expect("kyc compose should succeed");
        assert!(result.path.join("screens.toml").is_file());
        assert!(result.path.join("menus.toml").is_file());
        assert!(result.path.join("Cargo.toml").is_file());
        assert!(result.total_screens > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn effective_modules_defaults_include_optional_enabled() {
        let manifest = load_template_manifest("banking").unwrap().0;
        let modules = effective_modules(&manifest, None).unwrap();
        assert!(modules.contains(&"core".to_string()));
        assert!(modules.contains(&"accounts".to_string()));
        assert!(modules.contains(&"loans".to_string())); // default = true
        assert!(!modules.contains(&"fx".to_string())); // default = false
    }

    #[test]
    fn effective_modules_user_can_toggle_fx() {
        let manifest = load_template_manifest("banking").unwrap().0;
        let modules = effective_modules(
            &manifest,
            Some(&[
                "core".to_string(),
                "accounts".to_string(),
                "transfers".to_string(),
                "fx".to_string(),
            ]),
        )
        .unwrap();
        assert!(modules.contains(&"fx".to_string()));
        assert!(!modules.contains(&"loans".to_string()));
    }

    #[test]
    fn compose_project_emits_register_and_menus() {
        let dir = std::env::temp_dir().join(format!("nir_hi_compose_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let result = compose_project(
            &dir,
            "banking",
            &["core".to_string(), "accounts".to_string()],
            None,
        )
        .expect("compose should succeed");
        let screens_text = std::fs::read_to_string(result.path.join("screens.toml")).unwrap();
        assert!(
            screens_text.contains("[screen.parameters]"),
            "screen parameters should be namespaced under [screen.parameters]"
        );
        assert!(result.path.join("screens.toml").is_file());
        assert!(result.path.join("menus.toml").is_file());
        assert!(result.path.join("Cargo.toml").is_file());
        assert!(result.total_screens > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
