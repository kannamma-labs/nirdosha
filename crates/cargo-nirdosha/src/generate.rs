//! `cargo nirdosha generate-screens <project-dir>` — the screen-register
//! code generator: reads a project's v2 `screens.toml` + `menus.toml`
//! (the shape `docs/SCREEN_REGISTER_SCHEMA.json` defines) and emits the
//! crate's `src/*.nir` tree: bridge (entities, guard tables, policies,
//! roles), app shell + login, per-module screen files with real macro
//! invocations, and a `bin/serve.nir` with correct route-mount ordering
//! (literal routes before `{id}` wildcards).
//!
//! This is the tool that makes the two TOML files a source of truth for
//! code rather than an inventory that drifts. Design rules it follows:
//!
//! - **Generate macro invocations, not bespoke screens.** Every emitted
//!   screen is one `nirdosha_rt::<archetype>! { .. }` call plus, where
//!   the archetype needs it, small generated helper functions (widget
//!   stats, board move handlers) whose bodies are mechanical reductions
//!   of the register's own `data_binding` declarations — a guarded read
//!   reduced to a count, a group-by, a filtered panel. The register
//!   cannot invent novel business logic; it names sources, and the
//!   generator wires real guarded reads to them.
//! - **Unsupported archetypes are a hard error**, not a silent skip — a
//!   generator that silently skips a declared screen would produce an
//!   app that lies by omission. The error names the archetype so the
//!   register can mark the screen `blocked` (which the generator then
//!   respects and skips, the same disclosed posture RTM uses).
//! - **Route ordering is derived, not hand-maintained**: every
//!   wildcard-`route_path` mount is emitted after every literal-path
//!   mount, because `Router::dispatch` matches in registration order
//!   and a `/{id}` wildcard would otherwise swallow the literal
//!   `new`/`board`/`edit` segments (the exact bug RTM's serve.nir
//!   comment documents from a live curl).
//! - **The guard stays at the data plane.** A screen with
//!   `data_binding.guard` is emitted with `guard: { table, purpose }`
//!   clauses and a real `guard_policy!` record set synthesized from the
//!   screen's own `policy` block (roles × allowed_actions × purpose) —
//!   so generated screens are policy-evaluated from boot, not merely
//!   role-gated.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Register (screens.toml) — typed projection of docs/SCREEN_REGISTER_SCHEMA.json
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterFile {
    metadata: Metadata,
    #[serde(default)]
    screen: Vec<ScreenDecl>,
}

#[derive(Deserialize)]
struct Metadata {
    schema: String,
    register_id: String,
    #[serde(default)]
    version: String,
    total_screens: usize,
    #[serde(default)]
    canonical_order: String,
    #[serde(default)]
    codegen_profile: String,
    #[serde(default)]
    generator_version: String,
}

#[derive(Deserialize)]
#[derive(Clone)]
struct ScreenDecl {
    id: String,
    #[serde(default)]
    name: String,
    module: String,
    archetype: String,
    #[serde(default)]
    stage: String,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
    #[serde(default)]
    codegen: Option<Codegen>,
    #[serde(default)]
    data_binding: Option<DataBinding>,
    #[serde(default)]
    policy: Option<Policy>,
    #[serde(default = "empty_table")]
    parameters: toml::Value,
}

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

#[derive(Deserialize)]
#[derive(Clone)]
struct Codegen {
    #[serde(default)]
    template: String,
    #[serde(default)]
    mount_symbol: Option<String>,
    #[serde(default)]
    route_path: Option<String>,
    #[serde(default)]
    generated_files: Vec<String>,
}

#[derive(Deserialize)]
#[derive(Clone)]
struct DataBinding {
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    primary_key: Option<String>,
    #[serde(default)]
    fields: Vec<FieldDecl>,
    #[serde(default)]
    guard: Option<GuardBinding>,
}

#[derive(Deserialize, Clone)]
struct FieldDecl {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Deserialize, Clone)]
struct GuardBinding {
    table: String,
    purpose: String,
    #[serde(default)]
    row_id: Option<String>,
}

#[derive(Deserialize)]
#[derive(Clone)]
struct Policy {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    allowed_actions: Vec<String>,
}

// ---------------------------------------------------------------------------
// menus.toml — only what the generator itself needs (the app-shell macro
// re-reads the file at compile time; the generator just needs the title).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MenusFile {
    #[serde(default = "empty_table")]
    meta: toml::Value,
}

// ---------------------------------------------------------------------------
// Codegen profile registry. A profile pins the renderer the generator is
// allowed to emit; an unknown profile is a hard error so a register
// written for a future generator can never be silently built by an
// older one.
// ---------------------------------------------------------------------------

const KNOWN_PROFILES: &[&str] = &["web-default"];

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub struct GenerateReport {
    pub files: Vec<String>,
    pub screens_emitted: usize,
    pub screens_skipped: Vec<String>,
    pub entities: usize,
    pub policies: usize,
}

/// Generates a crate's `src/` tree from `<project_dir>/screens.toml` +
/// `menus.toml`. Deterministic: the same register always produces the
/// same output text.
pub fn run(project_dir: &Path) -> Result<GenerateReport, String> {
    let screens_path = project_dir.join("screens.toml");
    let menus_path = project_dir.join("menus.toml");
    let screens_src = std::fs::read_to_string(&screens_path)
        .map_err(|e| format!("cannot read {}: {e}", screens_path.display()))?;
    let menus_src = std::fs::read_to_string(&menus_path)
        .map_err(|e| format!("cannot read {}: {e}", menus_path.display()))?;

    let register: RegisterFile = toml::from_str(&screens_src)
        .map_err(|e| format!("{} is invalid: {e}", screens_path.display()))?;
    let menus: MenusFile =
        toml::from_str(&menus_src).map_err(|e| format!("{} is invalid: {e}", menus_path.display()))?;

    // ---- validation (fail closed, per screen) ----
    if register.metadata.schema != "nirdosha.screen-register/v2" {
        return Err(format!(
            "[metadata].schema must be \"nirdosha.screen-register/v2\", found {:?}",
            register.metadata.schema
        ));
    }
    let profile = register.metadata.codegen_profile.clone();
    if !profile.is_empty() && !KNOWN_PROFILES.contains(&profile.as_str()) {
        return Err(format!(
            "unknown codegen_profile {profile:?}; this generator supports: {}",
            KNOWN_PROFILES.join(", ")
        ));
    }
    if register.screen.len() != register.metadata.total_screens {
        return Err(format!(
            "[metadata].total_screens = {} but the register declares {} screen entries",
            register.metadata.total_screens,
            register.screen.len()
        ));
    }

    let register_id = register.metadata.register_id.clone();
    let app_title = menus
        .meta
        .get("app_title")
        .and_then(toml::Value::as_str)
        .ok_or("menus.toml [meta] must declare app_title (used by app_shell_from_toml!)")?
        .to_string();

    // ---- collect entities + roles across all screens first ----
    let mut entities: BTreeMap<String, EntityDecl> = BTreeMap::new();
    let mut all_roles: Vec<String> = Vec::new();
    for screen in &register.screen {
        for role in &screen.roles {
            let role = role.split(':').next().unwrap_or(role);
            if !role.is_empty() && role != "AllRoles" && role != "AllHuman" && !all_roles.iter().any(|r| r == role) {
                all_roles.push(role.to_string());
            }
        }
        if let Some(binding) = &screen.data_binding {
            let Some(entity) = binding.entities.first() else {
                return Err(format!(
                    "screen {} ({}): data_binding requires at least one entity",
                    screen.id, screen.archetype
                ));
            };
            let Some(primary_key) = &binding.primary_key else {
                return Err(format!(
                    "screen {} ({}): data_binding.primary_key is required to emit the entity struct",
                    screen.id, screen.archetype
                ));
            };
            let decl = entities
                .entry(entity.clone())
                .or_insert_with(|| EntityDecl {
                    name: entity.clone(),
                    primary_key: primary_key.clone(),
                    fields: Vec::new(),
                    guard: None,
                });
            for f in &binding.fields {
                if !decl.fields.iter().any(|x| x.name == f.name) {
                    decl.fields.push(f.clone());
                }
            }
            if decl.guard.is_none() {
                decl.guard = binding.guard.clone();
            }
        }
    }

    // ---- plan screens ----
    let mut planned: Vec<PlannedScreen> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for screen in &register.screen {
        let stage = if screen.stage.is_empty() { "blocked".to_string() } else { screen.stage.clone() };
        if matches!(stage.as_str(), "blocked" | "delegated" | "interim") {
            skipped.push(format!(
                "  - {} ({}) — stage `{stage}`{}",
                screen.id,
                screen.archetype,
                if screen.blocked_by.is_empty() { String::new() } else { format!(", blocked_by {:?}", screen.blocked_by) }
            ));
            continue;
        }
        planned.push(plan_screen(screen)?);
    }

    // ---- the guard is the single data authority ----
    // Every entity a generated screen touches must be guarded: the
    // register cannot describe an unguarded data screen, so the generated
    // app cannot contain one. An entity with no `data_binding.guard`
    // anywhere would silently fall back to `SharedTable`-grade access
    // (no masking, no caps, no audit) — refused here instead.
    for plan in &planned {
        let Some(binding) = plan.decl.data_binding.as_ref() else { continue };
        let Some(entity) = binding.entities.first() else { continue };
        if entities.get(entity).is_some_and(|d| d.guard.is_none()) {
            return Err(format!(
                "screen {} ({}): entity `{entity}` has no `data_binding.guard` anywhere in the register — generated apps route all data through the guard; declare guard = {{ table = ..., purpose = ... }} on the screen that owns it, or mark this screen blocked",
                plan.id, plan.archetype
            ));
        }
    }

    // ---- emit ----
    let src_dir = project_dir.join("src");
    std::fs::create_dir_all(src_dir.join("screens")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(src_dir.join("bin")).map_err(|e| e.to_string())?;

    let mut files: Vec<String> = Vec::new();

    // bridge.nir
    let singletons: Vec<(String, String)> = planned
        .iter()
        .filter_map(|p| {
            let binding = p.decl.data_binding.as_ref()?;
            let entity = binding.entities.first()?.clone();
            let row_id = binding.guard.as_ref()?.row_id.clone()?;
            Some((entity, row_id))
        })
        .collect();
    let bridge = render_bridge(&register_id, &entities, &planned, &all_roles, &singletons)?;;
    std::fs::write(src_dir.join("bridge.nir"), &bridge).map_err(|e| e.to_string())?;
    files.push("src/bridge.nir".into());

    // app_shell.nir: the shell macro + every login screen
    let app_shell = render_app_shell(&app_title, &planned);
    std::fs::write(src_dir.join("app_shell.nir"), &app_shell).map_err(|e| e.to_string())?;
    files.push("src/app_shell.nir".into());

    // Group the remaining planned screens by their declared generated file.
    let mut by_file: BTreeMap<String, Vec<&PlannedScreen>> = BTreeMap::new();
    for plan in &planned {
        if matches!(plan.archetype.as_str(), "login" | "app_shell_from_toml") {
            continue; // rendered into app_shell.nir above
        }
        let file = plan
            .output_file
            .clone()
            .ok_or_else(|| format!("screen {} has no codegen.generated_files entry", plan.id))?;
        by_file.entry(file).or_default().push(plan);
    }
    let mut modules: Vec<(String, String)> = Vec::new(); // (module name, file path rel to src)
    for (file, plans) in &by_file {
        let stem = Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("bad generated_files entry {file:?}"))?
            .to_string();
        let rel = Path::new(file)
            .strip_prefix("src/")
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.clone());
        let body = render_screen_file(plans, &entities)?;
        let path = src_dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        files.push(file.clone());
        modules.push((stem, rel));
    }

    // lib.nir
    let mut lib = String::new();
    lib.push_str("//! Generated by `cargo nirdosha generate-screens` — do not edit by hand;\n");
    lib.push_str("//! edit screens.toml/menus.toml and regenerate.\n\n");
    lib.push_str("#[path = \"bridge.nir\"]\npub mod bridge;\n");
    lib.push_str("#[path = \"app_shell.nir\"]\npub mod app_shell;\n");
    // Screen/guard macros expand to `crate::nirdosha_roles::<Role>`
    // paths (kanban_board!'s move route, app_shell_from_toml!'s role
    // type-existence assertions, ...), and `roles!` was invoked inside
    // the bridge module — re-export the module at the crate root so
    // those paths resolve.
    lib.push_str("pub use crate::bridge::nirdosha_roles;\n\n");
    for (module, file) in &modules {
        lib.push_str(&format!("#[path = \"{file}\"]\npub mod {module};\n"));
    }
    std::fs::write(src_dir.join("lib.nir"), lib).map_err(|e| e.to_string())?;
    files.push("src/lib.nir".into());

    // serve.nir with literal-before-wildcard mount ordering.
    let serve = render_serve(&register_id, &planned);
    std::fs::write(src_dir.join("bin/serve.nir"), serve).map_err(|e| e.to_string())?;
    files.push("src/bin/serve.nir".into());

    let policy_count: usize = planned
        .iter()
        .filter(|p| p.decl.data_binding.as_ref().is_some_and(|b| b.guard.is_some()))
        .filter_map(|p| p.decl.policy.as_ref())
        .map(|p| p.allowed_actions.len())
        .sum();

    Ok(GenerateReport {
        files,
        screens_emitted: planned.len(),
        screens_skipped: skipped,
        entities: entities.len(),
        policies: policy_count,
    })
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

struct EntityDecl {
    name: String,
    primary_key: String,
    fields: Vec<FieldDecl>,
    guard: Option<GuardBinding>,
}

impl EntityDecl {
    fn struct_name(&self) -> String {
        format!("{}Row", self.name)
    }

    fn resource(&self) -> String {
        self.name.to_lowercase()
    }

    fn type_of(&self, field: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| f.ty.clone())
            .unwrap_or_else(|| "String".to_string())
    }

    fn guard_table_fn(&self) -> String {
        format!("{}_table", pascal_to_snake(&self.name))
    }

    fn store_fn(&self) -> String {
        format!("{}_store", pascal_to_snake(&self.name))
    }

    fn cell_fn(&self) -> String {
        format!("{}_cell", pascal_to_snake(&self.name))
    }
}

struct PlannedScreen {
    id: String,
    archetype: String,
    module: String,
    mount: String,
    route_path: String,
    output_file: Option<String>,
    decl: ScreenDecl,
}

fn plan_screen(screen: &ScreenDecl) -> Result<PlannedScreen, String> {
    let codegen = screen.codegen.as_ref().ok_or("missing [screen.codegen]")?;
    let mount = codegen
        .mount_symbol
        .clone()
        .ok_or("codegen.mount_symbol is required")?;
    let output_file = codegen.generated_files.first().cloned();
    let route_path = codegen.route_path.clone().unwrap_or_default();

    let supported = matches!(
        screen.archetype.as_str(),
        "login"
            | "app_shell_from_toml"
            | "crud_screens"
            | "dashboard"
            | "kanban_board"
            | "approval_inbox"
            | "communication_feed"
            | "settings_screen"
            | "wizard"
            | "workspace"
            | "static_embed"
            | "report_builder"
            | "tree_view"
    );
    if !supported {
        return Err(format!(
            "archetype `{}` has no generator template yet — mark this screen stage = \"blocked\" with blocked_by = [\"archetype:{}\"] until the macro exists",
            screen.archetype, screen.archetype
        ));
    }

    let needs_data = matches!(
        screen.archetype.as_str(),
        "crud_screens"
            | "kanban_board"
            | "communication_feed"
            | "settings_screen"
            | "wizard"
            | "workspace"
            | "approval_inbox"
            | "dashboard"
            | "report_builder"
            | "tree_view"
    );
    if needs_data && screen.data_binding.is_none() {
        return Err(format!(
            "archetype `{}` reads data and must declare [screen.data_binding]",
            screen.archetype
        ));
    }
    if needs_data && screen.data_binding.as_ref().is_some_and(|b| b.primary_key.is_none()) {
        return Err("data_binding.primary_key is required to emit the entity struct".to_string());
    }

    Ok(PlannedScreen {
        id: screen.id.clone(),
        archetype: screen.archetype.clone(),
        module: screen.module.clone(),
        mount,
        route_path,
        output_file,
        decl: screen.clone(),
    })
}

// ---------------------------------------------------------------------------
// shared name helpers
// ---------------------------------------------------------------------------

fn pascal_to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn pascalize(s: &str) -> String {
    s.split(|c: char| c == '_' || c == '-' || c == ' ')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn access_literal(value: Option<&str>, default: &str) -> String {
    match value {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => default.to_string(),
    }
}

// ---------------------------------------------------------------------------
// bridge.nir — entities, guard tables, stores, cells, roles, policies
// ---------------------------------------------------------------------------

fn render_bridge(
    register_id: &str,
    entities: &BTreeMap<String, EntityDecl>,
    screens: &[PlannedScreen],
    all_roles: &[String],
    singletons: &[(String, String)],
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str(&format!(
        "//! Generated by `cargo nirdosha generate-screens` from {register_id}'s screens.toml.\n\
         //! Entities, `GuardedEntity` impls, `GuardedTable` constructors, managed stores,\n\
         //! `roles!`, and one real `guard_policy!` per (guarded screen, allowed action) —\n\
         //! synthesized from each guarded screen's own `policy` block. Do not edit by\n\
         //! hand; edit screens.toml and regenerate.\n\n"
    ));

    // roles!
    // Wire name == the PascalCase ident string: guard_policy! subjects
    // (`for Agent`) are stored as the ident's own string, demo_users
    // login role strings must match verbatim, and `Auth::has_role` is
    // an exact string compare — a snake_case wire name here would
    // silently deny every policy.
    if !all_roles.is_empty() {
        out.push_str("nirdosha_rt::roles! {\n");
        for role in all_roles {
            out.push_str(&format!("    {role} = \"{role}\";\n"));
        }
        out.push_str("}\n\n");
    }

    // guard_policy! records: one per (guarded screen, allowed action).
    out.push_str("// ---- guard policies, synthesized from each guarded screen's policy block ----\n\n");
    for screen in screens {
        let Some(binding) = &screen.decl.data_binding else { continue };
        let Some(entity) = binding.entities.first() else { continue };
        let Some(guard) = &binding.guard else { continue };
        let Some(policy) = &screen.decl.policy else {
            return Err(format!(
                "screen {} declares data_binding.guard but no [screen.policy] block — the generator needs allowed_actions + purpose to synthesize real guard_policy! records",
                screen.id
            ));
        };
        let resource = entity.to_lowercase();
        let purpose_pascal = policy.purpose.clone().unwrap_or_else(|| pascalize(&guard.purpose));
        let subjects: Vec<String> = screen
            .decl
            .roles
            .iter()
            .map(|r| r.split(':').next().unwrap_or(r).to_string())
            .filter(|r| r != "AllRoles" && r != "AllHuman")
            .collect();
        if subjects.is_empty() {
            return Err(format!(
                "screen {}: guarded screens need concrete roles in screen.roles to synthesize guard policies (universal roles like AllRoles cannot be policy subjects)",
                screen.id
            ));
        }
        for action in &policy.allowed_actions {
            let id = format!("{}-{}-{}", register_id, screen.id.replace('.', "-"), action);
            out.push_str(&format!(
                "nirdosha_rt::guard_policy! {{\n    allow \"{id}\" for {}\n    when action == \"{action}\" && resource == \"{resource}\"\n    purpose({purpose_pascal})\n    filter tenant_scope()\n    cap(row_cap = 200, max_scan_rows = 20_000)\n    obligate audit(sampled)\n}}\n\n",
                subjects.join(", ")
            ));
        }
    }

    // entities
    for decl in entities.values() {
        let struct_name = decl.struct_name();
        let resource = decl.resource();
        let pk = &decl.primary_key;
        out.push_str(&format!(
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]\npub struct {struct_name} {{\n    pub id: i64,\n    pub {pk}: String,\n    pub tenant_id: String,\n"
        ));
        for f in &decl.fields {
            out.push_str(&format!("    pub {}: {},\n", f.name, f.ty));
        }
        out.push_str("}\n\n");
        out.push_str(&format!(
            "impl nirdosha_guard_screens::GuardedEntity for {struct_name} {{\n    const RESOURCE: &'static str = \"{resource}\";\n    fn row_id(&self) -> String {{\n        self.{pk}.clone()\n    }}\n}}\n\n"
        ));

        // guarded table
        let table_fn = decl.guard_table_fn();
        out.push_str(&format!(
            "pub fn {table_fn}() -> &'static nirdosha_guard_screens::GuardedTable<nirdosha_guard_mic::MemStoreDriver, {struct_name}> {{\n"
        ));
        out.push_str(&format!(
            "    static TABLE: std::sync::OnceLock<nirdosha_guard_screens::GuardedTable<nirdosha_guard_mic::MemStoreDriver, {struct_name}>> = std::sync::OnceLock::new();\n"
        ));
        out.push_str(&format!(
            "    TABLE.get_or_init(|| {{\n        nirdosha_guard_screens::GuardedTable::new(\n            nirdosha_guard_mic::MemStoreDriver::new(),\n            nirdosha_guard_registry::candidates(),\n            nirdosha_guard_registry::dump().policies,\n            \"{register_id}-demo\",\n            std::env::temp_dir().join(format!(\"{register_id}-audit-{resource}-{{}}.jsonl\", std::process::id())),\n            HUMAN_ROLES.iter().map(|r| r.to_string()).collect(),\n            DEMO_TENANT,\n            Vec::new(),\n        )\n    }})\n}}\n\n"
        ));

        // unguarded keyed store (macros like wizard!/kanban_board! require one)
        let store_fn = decl.store_fn();
        out.push_str(&format!(
            "pub fn {store_fn}() -> &'static nirdosha_rt::prelude::SharedTable<i64, {struct_name}> {{\n    static STORE: std::sync::OnceLock<nirdosha_rt::prelude::SharedTable<i64, {struct_name}>> = std::sync::OnceLock::new();\n    STORE.get_or_init(nirdosha_rt::prelude::SharedTable::new)\n}}\n\n"
        ));

        // append-only cell (communication_feed!'s store shape)
        let cell_fn = decl.cell_fn();
        out.push_str(&format!(
            "pub fn {cell_fn}() -> &'static nirdosha_rt::prelude::SharedCell<Vec<{struct_name}>> {{\n    static CELL: std::sync::OnceLock<nirdosha_rt::prelude::SharedCell<Vec<{struct_name}>>> = std::sync::OnceLock::new();\n    CELL.get_or_init(|| nirdosha_rt::prelude::SharedCell::new(Vec::new()))\n}}\n\n"
        ));
    }

    // shared constants
    let roles_lit: Vec<String> = all_roles.iter().map(|r| format!("    \"{r}\",")).collect();
    out.push_str(&format!(
        "pub const HUMAN_ROLES: &[&str] = &[\n{}\n];\n\npub const DEMO_TENANT: &str = \"default\";\n",
        roles_lit.join("\n")
    ));

    // boot-time singleton seeding: a `row_id`-bound guard on an empty
    // table would 404 its own singleton forever, so each singleton
    // entity gets one idempotent `system_write` at boot (storage is
    // keyed `<type>:<row-id>`, so rewriting the same row_id replaces —
    // genuinely idempotent). This is bridge-level, not a screen route:
    // `system_write` is the one legitimate non-screen write path (same
    // posture as RTM's notify hook).
    if !singletons.is_empty() {
        out.push_str("/// Boot-time seeding for singleton guarded rows. Idempotent per\n/// process: `std::sync::Once` (the guarded store lives for the process\n/// lifetime, and `system_write` refuses a duplicate create rather than\n/// upserting). Called from the generated `serve` main and from tests\n/// that build a router without running main.\npub fn seed_singletons() {\n    static ONCE: std::sync::Once = std::sync::Once::new();\n    ONCE.call_once(|| {\n");
        for (entity, row_id) in singletons {
            let Some(decl) = entities.get(entity.as_str()) else { continue };
            let struct_name = decl.struct_name();
            let pk = &decl.primary_key;
            let mut fields_init = String::new();
            for f in &decl.fields {
                fields_init.push_str(&format!(", {}: Default::default()", f.name));
            }
            out.push_str(&format!(
                "        {table_fn}().system_write(DEMO_TENANT, &{struct_name} {{ id: 0, {pk}: {row_id:?}.into(), tenant_id: DEMO_TENANT.into(){fields_init} }});\n",
                table_fn = decl.guard_table_fn()
            ));
        }
        out.push_str("    });\n}\n");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// app_shell.nir — shell macro + login screen
// ---------------------------------------------------------------------------

fn render_app_shell(title: &str, screens: &[PlannedScreen]) -> String {
    let mut out = String::new();
    out.push_str("//! Generated app shell + landing (menus.toml, read at compile time) + login.\n\n");
    out.push_str(&format!(
        "nirdosha_rt::app_shell_from_toml!(\n    \"menus.toml\",\n    title: {title:?},\n);\n\n"
    ));
    for plan in screens.iter().filter(|p| p.archetype == "login") {
        let mount = &plan.mount;
        let path = &plan.route_path;
        out.push_str(&format!("nirdosha_rt::login! {{\n    mount: {mount},\n    path: {path:?},\n    mode: demo,\n"));
        let users = plan
            .decl
            .parameters
            .get("demo_users")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        out.push_str("    demo_users: [\n");
        for user in users {
            let username = user.get("username").and_then(toml::Value::as_str).unwrap_or_default();
            let password = user.get("password").and_then(toml::Value::as_str).unwrap_or_default();
            let roles: Vec<String> = user
                .get("roles")
                .and_then(toml::Value::as_array)
                .map(|a| a.iter().filter_map(toml::Value::as_str).map(|s| format!("{s:?}")).collect())
                .unwrap_or_default();
            out.push_str(&format!(
                "        {{ username: {username:?}, password: {password:?}, roles: [{}] }},\n",
                roles.join(", ")
            ));
        }
        out.push_str("    ],\n    landing: landing_path,\n}\n\n");
    }
    out
}

// ---------------------------------------------------------------------------
// per-module screen files
// ---------------------------------------------------------------------------

fn render_screen_file(plans: &[&PlannedScreen], entities: &BTreeMap<String, EntityDecl>) -> Result<String, String> {
    // Render every invocation FIRST, then import only the bridge idents
    // the rendered text actually references — no `#[allow(unused)]`, no
    // blanket import: the emitted import list is derived from the same
    // text that needs it.
    let mut body = String::new();
    for plan in plans {
        body.push_str(&render_screen_invocation(plan, entities)?);
        body.push('\n');
    }
    let mut out = String::new();
    out.push_str("//! Generated by `cargo nirdosha generate-screens` — edit screens.toml, not this file.\n\n");
    let mut used_entities: Vec<String> = Vec::new();
    for (entity, decl) in entities {
        let referenced = [decl.struct_name(), decl.guard_table_fn(), decl.store_fn(), decl.cell_fn()]
            .iter()
            .any(|ident| body.contains(ident));
        if referenced {
            used_entities.push(entity.clone());
        }
    }
    if !used_entities.is_empty() {
        out.push_str("// The macro grammar requires `entity:`/`store:` keys even where the\n// guard-mode expansion never references them — keep the imports that\n// match the invocation text and silence the residue.\n#[allow(unused_imports)]\nuse crate::bridge::{\n");
        for entity in &used_entities {
            let decl = entities
                .get(entity)
                .ok_or_else(|| format!("entity {entity} missing from bridge plan"))?;
            let idents: Vec<String> = [
                decl.struct_name(),
                decl.guard_table_fn(),
                decl.store_fn(),
                decl.cell_fn(),
            ]
            .into_iter()
            .filter(|ident| body.contains(ident))
            .collect();
            out.push_str(&format!("    {},\n", idents.join(", ")));
        }
        out.push_str("};\n\n");
    }
    out.push_str(&body);
    Ok(out)
}

fn render_screen_invocation(plan: &PlannedScreen, entities: &BTreeMap<String, EntityDecl>) -> Result<String, String> {
    let binding = plan.decl.data_binding.as_ref();
    // The guard comes from the ENTITY (declared on whichever screen owns
    // it), not from this screen's own binding: a kanban board reading a
    // Ticket entity is guarded by Ticket's guard whether or not the
    // board screen re-declares it. `plan_screen` + the post-plan check
    // have already proven every data-backed screen's entity is guarded.
    let guard = binding
        .and_then(|b| b.entities.first())
        .and_then(|e| entities.get(e))
        .and_then(|d| d.guard.as_ref());
    let params = &plan.decl.parameters;
    let mount = &plan.mount;
    let path = &plan.route_path;

    match plan.archetype.as_str() {
        "crud_screens" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("crud_screens needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let store = decl.store_fn();
            let fields: Vec<String> = decl.fields.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect();
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::crud_screens! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {store},\n    path: {path:?},\n    fields: [ {} ],\n",
                fields.join(", ")
            ));
            for (key, default) in [("create", "public"), ("read", "public"), ("update", "public"), ("delete", "public")] {
                let key = format!("access_{key}");
                let access = access_literal(params.get(&key).and_then(toml::Value::as_str), default);
                out.push_str(&format!("    {}: {},\n", key.strip_prefix("access_").unwrap_or(&key), access));
            }
            if let Some(refresh) = params.get("refresh_seconds").and_then(toml::Value::as_integer) {
                out.push_str(&format!("    refresh_seconds: {refresh},\n"));
            }
            if let Some(sort_by) = params.get("sort_by").and_then(toml::Value::as_str) {
                out.push_str(&format!("    sort_by: {sort_by},\n"));
            }
            if let Some(countdown) = params.get("countdown_field").and_then(toml::Value::as_str) {
                out.push_str(&format!("    countdown_field: {countdown},\n"));
            }
            if let Some(guard) = guard {
                out.push_str(&format!(
                    "    guard: {{ table: {}, purpose: {:?},",
                    guard.table, guard.purpose
                ));
                // create_fields/update_fields are guard-clause keys: the
                // macro's trailing-clause loop only knows guard/
                // refresh_seconds/sort_by/countdown_field, and the guard
                // owns which submitted fields are acceptable — so they
                // live inside the guard block, not as top-level keys.
                for clause in ["update_fields", "create_fields"] {
                    if let Some(names) = params.get(clause).and_then(toml::Value::as_array) {
                        let typed: Vec<String> = names
                            .iter()
                            .filter_map(toml::Value::as_str)
                            .map(|n| format!("{}: {}", n, decl.type_of(n)))
                            .collect();
                        out.push_str(&format!(" {}: [ {} ],", clause, typed.join(", ")));
                    }
                }
                out.push_str(" },\n");
            }
            out.push_str("}\n");
            Ok(out)
        }
        "dashboard" => {
            let mut out = String::new();
            let widgets = params
                .get("widgets")
                .and_then(toml::Value::as_array)
                .cloned()
                .unwrap_or_default();
            for widget in &widgets {
                out.push_str(&render_widget_fn(widget, entities)
                    .map_err(|e| format!("screen {}: {e}", plan.id))?);
                out.push('\n');
            }
            let title = params
                .get("title")
                .and_then(toml::Value::as_str)
                .unwrap_or(&plan.decl.name);
            out.push_str(&format!(
                "nirdosha_rt::dashboard! {{\n    mount: {mount},\n    path: {path:?},\n    title: {title:?},\n"
            ));
            if let Some(refresh) = params.get("refresh_seconds").and_then(toml::Value::as_integer) {
                out.push_str(&format!("    refresh_seconds: {refresh},\n"));
            }
            out.push_str("    widgets {\n");
            for widget in &widgets {
                let kind = widget.get("kind").and_then(toml::Value::as_str).unwrap_or("Metric");
                let label = widget.get("label").and_then(toml::Value::as_str).unwrap_or("");
                let fn_name = widget.get("fn").and_then(toml::Value::as_str).unwrap_or("");
                let guarded = widget.get("source").is_some();
                match kind {
                    "Metric" => {
                        let target = widget.get("target").and_then(toml::Value::as_integer).unwrap_or(0);
                        let alert_below = widget.get("alert_below").and_then(toml::Value::as_bool).unwrap_or(false);
                        out.push_str(&format!(
                            "        Metric {{ label: {label:?}, fn: {fn_name}, target: {target}, alert_below: {alert_below}, guarded: {guarded} }},\n"
                        ));
                    }
                    "Chart" => {
                        let mark = widget.get("mark").and_then(toml::Value::as_str).unwrap_or("bar");
                        out.push_str(&format!(
                            "        Chart {{ label: {label:?}, fn: {fn_name}, mark: {mark}, guarded: {guarded} }},\n"
                        ));
                    }
                    other => return Err(format!("screen {}: unknown widget kind `{other}`", plan.id)),
                }
            }
            out.push_str("    }\n}\n");
            Ok(out)
        }
        "communication_feed" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("communication_feed needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let cell = decl.cell_fn();
            let fields: Vec<String> = decl.fields.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect();
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::communication_feed! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {cell},\n    path: {path:?},\n    fields: [ {} ],\n",
                fields.join(", ")
            ));
            let post_access = access_literal(params.get("post_access").and_then(toml::Value::as_str), "public");
            let read_access = access_literal(params.get("read_access").and_then(toml::Value::as_str), "public");
            out.push_str(&format!("    post_access: {post_access},\n    read_access: {read_access},\n"));
            // refresh/long-poll must precede the guard clause: the macro
            // parses the optional poll clause before the guard loop.
            if let Some(refresh) = params.get("refresh_seconds").and_then(toml::Value::as_integer) {
                out.push_str(&format!("    refresh_seconds: {refresh},\n"));
            }
            if let Some(poll) = params.get("long_poll_seconds").and_then(toml::Value::as_integer) {
                out.push_str(&format!("    long_poll_seconds: {poll},\n"));
            }
            if let Some(guard) = guard {
                out.push_str(&format!(
                    "    guard: {{ table: {}, purpose: {:?} }},\n",
                    guard.table, guard.purpose
                ));
            }
            out.push_str("}\n");
            Ok(out)
        }
        "settings_screen" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("settings_screen needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let store = decl.store_fn();
            let fields: Vec<String> = decl.fields.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect();
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::settings_screen! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {store},\n    path: {path:?},\n    fields: [ {} ],\n    access: {access},\n",
                fields.join(", ")
            ));
            if let Some(guard) = guard {
                let row_id = guard.row_id.clone().unwrap_or_else(|| "singleton".to_string());
                out.push_str(&format!(
                    "    guard: {{ table: {}, purpose: {:?}, row_id: {:?} }},\n",
                    guard.table, guard.purpose, row_id
                ));
            }
            out.push_str("}\n");
            Ok(out)
        }
        "wizard" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("wizard needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let store = decl.store_fn();
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::wizard! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {store},\n    path: {path:?},\n    access: {access},\n    steps: [\n"
            ));
            if let Some(steps) = params.get("steps").and_then(toml::Value::as_array) {
                for step in steps {
                    let name = step.get("name").and_then(toml::Value::as_str).unwrap_or("");
                    let fields: Vec<String> = step
                        .get("fields")
                        .and_then(toml::Value::as_array)
                        .map(|a| {
                            a.iter()
                                .map(|f| {
                                    format!(
                                        "{}: {}",
                                        f.get("name").and_then(toml::Value::as_str).unwrap_or(""),
                                        f.get("type").and_then(toml::Value::as_str).unwrap_or("String")
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    out.push_str(&format!("        {{ name: {name:?}, fields: [ {} ] }},\n", fields.join(", ")));
                }
            }
            out.push_str("    ],\n");
            if let Some(guard) = guard {
                out.push_str(&format!(
                    "    guard: {{ table: {}, purpose: {:?} }},\n",
                    guard.table, guard.purpose
                ));
            }
            out.push_str("}\n");
            Ok(out)
        }
        "kanban_board" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("kanban_board needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let entity_name = entity.to_string();
            let store = decl.store_fn();
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let title_field = params.get("title_field").and_then(toml::Value::as_str).unwrap_or("title");
            let column_field = params
                .get("column_field")
                .and_then(toml::Value::as_str)
                .ok_or("kanban_board needs parameters.column_field")?
                .to_string();
            let columns: Vec<String> = params
                .get("columns")
                .and_then(toml::Value::as_array)
                .map(|a| a.iter().filter_map(toml::Value::as_str).map(|s| format!("{s:?}")).collect())
                .unwrap_or_default();
            let move_path = params
                .get("move_path")
                .and_then(toml::Value::as_str)
                .ok_or("kanban_board needs parameters.move_path (with {id} and {to})")?
                .to_string();
            if !move_path.contains("{id}") || !move_path.contains("{to}") {
                return Err(format!("screen {}: kanban move_path must contain {{id}} and {{to}}", plan.id));
            }
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::kanban_board! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {store},\n    path: {path:?},\n    access: {access},\n    title_field: {title_field},\n    column_field: {column_field},\n    columns: [ {} ],\n    move_path: {move_path:?},\n}}\n\n",
                columns.join(", ")
            ));
            // The macro is presentational: `board_js` serves JS pointing at
            // `move_path`, but the move route itself is hand-wired in RTM.
            // Here it is generated — guarded when the screen is guarded.
            let base = move_path.split("/{id}").next().unwrap_or(path.as_str()).to_string();
            if let Some(guard) = guard {
                let entity_name = entity_name.to_string();
                let table = entities
                    .get(entity_name.as_str())
                    .map(|d| d.guard_table_fn().clone())
                    .ok_or_else(|| format!("kanban screen {}: entity `{}` not found", plan.id, entity_name))?;
                let purpose = &guard.purpose;
                out.push_str(&format!(
                    "pub fn {mount}_move(router: nirdosha_rt::Router) -> nirdosha_rt::Router {{\n    router.post_with_auth({move_path:?}, \"Move card\", |_req, params, auth| {{\n        let Some(id) = params.get(\"id\") else {{ return nirdosha_rt::Response::bad_request(\"id required\") }};\n        let Some(to) = params.get(\"to\") else {{ return nirdosha_rt::Response::bad_request(\"to required\") }};\n        let changed: std::collections::HashSet<String> = [{column_field:?}].into_iter().map(|s| s.to_string()).collect();\n        match {table}().guarded_update(auth, {purpose:?}, id, &changed, move |e| {{ e.{column_field} = to.to_string(); }}) {{\n            Ok(_entity) => nirdosha_rt::Response::redirect({base:?}),\n            Err(e) => nirdosha_guard_screens::guard_error_response(e),\n        }}\n    }})\n}}\n"
                ));
            } else {
                out.push_str(&format!(
                    "pub fn {mount}_move(router: nirdosha_rt::Router) -> nirdosha_rt::Router {{\n    router.post({move_path:?}, \"Move card\", |_req, params| {{\n        let Some(id) = params.get(\"id\").and_then(|s| s.parse::<i64>().ok()) else {{ return nirdosha_rt::Response::bad_request(\"id must be an integer\") }};\n        let Some(to) = params.get(\"to\") else {{ return nirdosha_rt::Response::bad_request(\"to required\") }};\n        let to = to.to_string();\n        {store}().update(&id, |e| match e {{\n            Some(e) => {{ e.{column_field} = to.clone(); Ok(e.clone()) }}\n            None => Err(\"not found\".to_string()),\n        }});\n        nirdosha_rt::Response::redirect({base:?})\n    }})\n}}\n"
                ));
            }
            Ok(out)
        }
        "static_embed" => {
            // Presentational: the pinned-digest integrity check happens
            // in the macro at expansion time; the generator just refuses
            // an invocation missing its inputs, so a register that
            // forgot the pin fails at generation, not at compile.
            let title = params.get("title").and_then(toml::Value::as_str).unwrap_or(&plan.decl.name);
            let content_file = params
                .get("content_file")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("screen {}: static_embed needs parameters.content_file (project-root-relative)", plan.id))?;
            let sha256 = params
                .get("sha256")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("screen {}: static_embed needs parameters.sha256 (pin the content file's SHA-256)", plan.id))?;
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::static_embed! {{\n    mount: {mount},\n    path: {path:?},\n    title: {title:?},\n    content_file: {content_file:?},\n    sha256: {sha256:?},\n    access: {access},\n}}\n"
            ));
            Ok(out)
        }
        "report_builder" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("report_builder needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let table = decl.guard_table_fn();
            let purpose = guard
                .ok_or_else(|| format!("screen {}: report_builder reads a GuardedTable and needs data_binding.guard", plan.id))?
                .purpose
                .clone();
            let dimensions: Vec<String> = params
                .get("dimensions")
                .and_then(toml::Value::as_array)
                .ok_or_else(|| format!("screen {}: report_builder needs parameters.dimensions", plan.id))?
                .iter()
                .filter_map(toml::Value::as_str)
                .map(|n| format!("{}: {}", n, decl.type_of(n)))
                .collect();
            if dimensions.is_empty() {
                return Err(format!("screen {}: report_builder needs at least one dimension", plan.id));
            }
            for entry in &dimensions {
                if !entry.ends_with("String") {
                    return Err(format!("screen {}: report_builder dimensions must be String-typed (got `{entry}`)", plan.id));
                }
            }
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::report_builder! {{\n    mount: {mount},\n    entity: {struct_name},\n    table: {table},\n    path: {path:?},\n    title: {:?},\n    purpose: {purpose:?},\n    access: {access},\n    dimensions: [ {} ],\n}}\n",
                plan.decl.name,
                dimensions.join(", ")
            ));
            Ok(out)
        }
        "tree_view" => {
            let entity = binding.and_then(|b| b.entities.first()).ok_or("tree_view needs an entity")?;
            let decl = entities.get(entity).ok_or("entity missing from bridge plan")?;
            let struct_name = decl.struct_name();
            let table = decl.guard_table_fn();
            let purpose = guard
                .ok_or_else(|| format!("screen {}: tree_view reads a GuardedTable and needs data_binding.guard", plan.id))?
                .purpose
                .clone();
            let id_field = params
                .get("id_field")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("screen {}: tree_view needs parameters.id_field", plan.id))?;
            let parent_field = params
                .get("parent_field")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("screen {}: tree_view needs parameters.parent_field", plan.id))?;
            let label_field = params
                .get("label_field")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("screen {}: tree_view needs parameters.label_field", plan.id))?;
            for field in [id_field, parent_field, label_field] {
                if decl.type_of(field) != "String" {
                    return Err(format!("screen {}: tree_view fields must be String-typed (`{field}` is `{}`)", plan.id, decl.type_of(field)));
                }
            }
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::tree_view! {{\n    mount: {mount},\n    entity: {struct_name},\n    table: {table},\n    path: {path:?},\n    title: {:?},\n    purpose: {purpose:?},\n    access: {access},\n    id_field: {id_field},\n    parent_field: {parent_field},\n    label_field: {label_field},\n}}\n",
                plan.decl.name
            ));
            Ok(out)
        }
        "approval_inbox" => {
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::approval_inbox! {{\n    mount: {mount},\n    path: {path:?},\n    access: {access},\n    sources: [\n"
            ));
            if let Some(sources) = params.get("sources").and_then(toml::Value::as_array) {
                for source in sources {
                    let table = source.get("table").and_then(toml::Value::as_str).unwrap_or("");
                    let chain = source.get("chain").and_then(toml::Value::as_str).unwrap_or("");
                    let resource = source.get("resource").and_then(toml::Value::as_str).unwrap_or("");
                    let detail_path = source.get("detail_path").and_then(toml::Value::as_str).unwrap_or("");
                    out.push_str(&format!(
                        "        {{ table: {table}, chain: {chain:?}, resource: {resource:?}, detail_path: {detail_path:?} }},\n"
                    ));
                }
            }
            out.push_str("    ],\n}\n");
            Ok(out)
        }
        "workspace" => {
            let subject = params.get("subject").ok_or("workspace needs parameters.subject")?;
            let subject_table = subject.get("table").and_then(toml::Value::as_str).unwrap_or("");
            let subject_purpose = subject.get("purpose").and_then(toml::Value::as_str).unwrap_or("");
            let label_fn = subject.get("label_fn").and_then(toml::Value::as_str).unwrap_or("");
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::workspace! {{\n    mount: {mount},\n    path: {path:?},\n    subject: {{ table: {subject_table}, purpose: {subject_purpose:?}, label_fn: {label_fn} }},\n"
            ));
            if let Some(budget) = params.get("budget") {
                let max_rows = budget.get("max_rows").and_then(toml::Value::as_integer).unwrap_or(500);
                let max_ms = budget.get("max_execution_ms").and_then(toml::Value::as_integer).unwrap_or(2000);
                out.push_str(&format!("    budget: {{ max_rows: {max_rows}, max_execution_ms: {max_ms} }},\n"));
            }
            out.push_str("    panels: [\n");
            if let Some(panels) = params.get("panels").and_then(toml::Value::as_array) {
                for panel in panels {
                    let need = panel.get("need").and_then(toml::Value::as_str).unwrap_or("");
                    let title = panel.get("title").and_then(toml::Value::as_str).unwrap_or("");
                    let render = panel.get("render").and_then(toml::Value::as_str).unwrap_or("table");
                    let source = panel.get("source").and_then(toml::Value::as_str).unwrap_or("");
                    out.push_str(&format!(
                        "        {{ need: {need:?}, title: {title:?}, render: {render:?}, source: {source} }},\n"
                    ));
                }
            }
            out.push_str("    ],\n}\n");
            Ok(out)
        }
        "login" | "app_shell_from_toml" => Ok(String::new()), // rendered into app_shell.nir
        other => Err(format!("archetype `{other}` reached emission without a template")),
    }
}

/// Dashboard widget function generation: a real, guarded read reduced to
/// a metric count or a group-by chart. `filter` narrows rows
/// (`field == equals`); `group_by` buckets rows into `{name, count}`
/// pairs for Chart widgets.
fn render_widget_fn(widget: &toml::Value, entities: &BTreeMap<String, EntityDecl>) -> Result<String, String> {
    let kind = widget.get("kind").and_then(toml::Value::as_str).unwrap_or("Metric");
    let fn_name = widget
        .get("fn")
        .and_then(toml::Value::as_str)
        .ok_or("widget needs fn")?
        .to_string();
    let source = widget.get("source").ok_or("widget needs a source { table, purpose } block")?;
    let table_ident = source.get("table").and_then(toml::Value::as_str).unwrap_or("").to_string();
    let purpose = source.get("purpose").and_then(toml::Value::as_str).unwrap_or("").to_string();
    let action = source.get("action").and_then(toml::Value::as_str).unwrap_or("read");
    // Resolve the entity this table belongs to: the entity -> table
    // naming convention is generated, so it is derivable here. The
    // lookup also proves the widget's table is a guarded one (guard
    // table fns only exist for entities, and every entity a generated
    // app ships must be guarded).
    let _decl = entities
        .values()
        .find(|d| d.guard_table_fn() == table_ident)
        .ok_or_else(|| format!("widget `{fn_name}` references table `{table_ident}` which no data_binding declares"))?;
    let read_call = if action == "aggregate" {
        format!("{table_ident}().guarded_aggregate(auth, {purpose:?})")
    } else {
        format!("{table_ident}().guarded_snapshot(auth, {purpose:?})")
    };
    let filter_expr = widget.get("filter").map(|filter| {
        let field = filter.get("field").and_then(toml::Value::as_str).unwrap_or("").to_string();
        let equals = filter.get("equals").and_then(toml::Value::as_str).unwrap_or("").to_string();
        format!(".filter(|r| r.{field} == {equals:?})")
    });
    if kind == "Metric" {
        let filter = filter_expr.unwrap_or_default();
        // i64, not u64: IntoMetricValue is implemented for i64/f64 only
        // (see dashboard.rs) — a u64 return is a compile error in the
        // generated code.
        Ok(format!(
            "pub fn {fn_name}(auth: &nirdosha_rt::Auth) -> i64 {{\n    match {read_call} {{\n        Ok(rows) => rows.iter(){filter}.count() as i64,\n        Err(_) => 0,\n    }}\n}}\n"
        ))
    } else {
        let group_by = widget
            .get("group_by")
            .and_then(toml::Value::as_str)
            .ok_or("Chart widget needs group_by")?
            .to_string();
        let filter = filter_expr.unwrap_or_default();
        Ok(format!(
            "pub fn {fn_name}(auth: &nirdosha_rt::Auth) -> String {{\n    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();\n    if let Ok(rows) = {read_call} {{\n        for r in rows.iter(){filter} {{\n            *counts.entry(r.{group_by}.clone()).or_insert(0) += 1;\n        }}\n    }}\n    let points: Vec<serde_json::Value> = counts.into_iter().map(|(name, count)| serde_json::json!({{ \"name\": name, \"count\": count }})).collect();\n    serde_json::to_string(&points).unwrap_or_else(|_| \"[]\".to_string())\n}}\n"
        ))
    }
}

// ---------------------------------------------------------------------------
// serve.nir — boot wiring + mount order (literal before wildcard)
// ---------------------------------------------------------------------------

fn render_serve(register_id: &str, screens: &[PlannedScreen]) -> String {
    let mut out = String::new();
    out.push_str("//! Generated by `cargo nirdosha generate-screens` — boot wiring + mount order.\n");
    out.push_str("//! Literal-path mounts are registered before wildcard mounts: `Router::dispatch`\n");
    out.push_str("//! matches in registration order and a `/{id}` wildcard would otherwise swallow\n");
    out.push_str("//! the literal `new`/`board`/`edit` segments.\n\nfn main() {\n");
    out.push_str(&format!(
        "    {register_id}::bridge::seed_singletons();\n"
    ));
    out.push_str(&format!(
        "    let router = {register_id}::app_shell::mount_app_shell(nirdosha_rt::Router::new(|_req| nirdosha_rt::Auth::login(\"anon\", &[])));\n"
    ));
    // Literal routes must register before wildcards (`dispatch` matches
    // in registration order). crud_screens is the one archetype whose
    // DECLARED route_path carries no braces but whose own macro always
    // registers `/{id}` wildcard routes (list is literal, detail is
    // `/path/{id}`) — so classify by archetype, not just the declared
    // path, or `GET /tickets/board` would be swallowed by crud's
    // `GET /tickets/{id}` (id="board" → 404).
    let (literal, wildcard): (Vec<&PlannedScreen>, Vec<&PlannedScreen>) = screens
        .iter()
        .partition(|s| s.archetype != "crud_screens" && !s.route_path.contains('{'));
    for plan in literal.into_iter().chain(wildcard.into_iter()) {
        let module = plan
            .output_file
            .as_deref()
            .and_then(|f| Path::new(f).file_stem().and_then(|s| s.to_str()))
            .unwrap_or("app_shell");
        if plan.archetype == "app_shell_from_toml" {
            continue; // mounted above
        }
        if plan.archetype == "login" {
            out.push_str(&format!(
                "    let router = {register_id}::app_shell::mount_login(router);\n"
            ));
            continue;
        }
        out.push_str(&format!(
            "    let router = {register_id}::{module}::{}(router);\n",
            plan.mount
        ));
        if plan.archetype == "kanban_board" {
            out.push_str(&format!(
                "    let router = {register_id}::{module}::{}_move(router);\n",
                plan.mount
            ));
        }
    }
    out.push_str("    println!(\"generated app — listening on http://localhost:8080/\");\n    router.serve(8080);\n}\n");
    out
}

