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
    /// App-wide approval chains (`nirdosha_rt::approval_chain!`), emitted
    /// into bridge.nir and passed to every `GuardedTable` constructor.
    /// An `approval_inbox` screen's source `chain` must resolve to one of
    /// these (the escalation it lists can otherwise never open).
    #[serde(default)]
    approval_chain: Vec<ChainDecl>,
}

#[derive(Deserialize, Clone)]
struct ChainDecl {
    name: String,
    quorum: i64,
    approvers: Vec<String>,
    /// Emit as `cooling(seconds = N)`; absent = no cooling period.
    #[serde(default)]
    cooling_seconds: Option<i64>,
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
    /// `masked = true` (or the `sensitive` alias): the field is
    /// forbidden by every policy synthesized from the screen that
    /// declares it — dropped from reads (genuine absence; the struct
    /// field must therefore be `Option<T>`) and refused on writes
    /// (fail-closed allowed set).
    #[serde(default)]
    masked: bool,
    #[serde(default)]
    sensitive: bool,
    /// `required = true`: the field must be submitted on **create**
    /// policies synthesized from this screen (v1 scope: creates only —
    /// applying `required` to updates would force every partial update
    /// to resend the whole row).
    #[serde(default)]
    required: bool,
}

impl FieldDecl {
    fn is_masked(&self) -> bool {
        self.masked || self.sensitive
    }

    fn is_option(&self) -> bool {
        self.ty.starts_with("Option<")
    }
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
// menus.toml — typed enough to VALIDATE against the screen register; the
// app-shell macro re-reads the file at compile time, but the generator is
// the one that can prove the register pair agrees before emitting code.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MenusFile {
    #[serde(default = "empty_table")]
    meta: toml::Value,
    /// Post-login landing per human role.
    #[serde(default)]
    landing: std::collections::BTreeMap<String, String>,
    /// Nav entries. `[[group]]` blocks are ordering chrome the shell macro
    /// owns; the generator validates entries, not group order.
    #[serde(default)]
    menu: Vec<MenuEntry>,
}

#[derive(Deserialize)]
struct MenuEntry {
    id: String,
    #[serde(default)]
    label_key: String,
    #[serde(default)]
    screen_id: String,
    #[serde(default)]
    route: String,
    #[serde(default)]
    guard: Option<MenuGuard>,
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct MenuGuard {
    action: String,
    resource: String,
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

#[derive(Debug)]
pub struct GenerateReport {
    pub files: Vec<String>,
    pub screens_emitted: usize,
    pub screens_skipped: Vec<String>,
    pub entities: usize,
    pub policies: usize,
    pub invariants_checked: usize,
}

/// Generates a crate's `src/` tree from `<project_dir>/screens.toml` +
/// `menus.toml`. Deterministic: the same register always produces the
/// same output text.
pub fn run(project_dir: &Path) -> Result<GenerateReport, String> {
    let crate_name: String = {
        let cargo_toml_path = project_dir.join("Cargo.toml");
        match std::fs::read_to_string(&cargo_toml_path) {
            Ok(cargo_src) => {
                let cargo: toml::Value = cargo_src
                    .parse()
                    .map_err(|e| format!("{} is invalid: {e}", cargo_toml_path.display()))?;
                cargo
                    .get("package")
                    .and_then(|p| p.get("name"))
                    .and_then(toml::Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| "generated_app".to_string())
            }
            Err(_) => "generated_app".to_string(),
        }
    };

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
    let mut role_wires_by_marker: BTreeMap<String, String> = BTreeMap::new();
    for screen in &register.screen {
        for role in &screen.roles {
            let role = role.split(':').next().unwrap_or(role);
            if role.is_empty() || role == "AllRoles" || role == "AllHuman" {
                continue;
            }
            let marker = pascalize(role);
            if let Some(existing) = role_wires_by_marker.get(&marker) {
                if existing != role {
                    return Err(format!(
                        "role spellings `{existing}` and `{role}` both resolve to marker type `{marker}`; use one wire spelling consistently across screens and modules"
                    ));
                }
            } else {
                role_wires_by_marker.insert(marker, role.to_string());
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
                // A masked field is dropped from reads (true key
                // removal), so its struct slot must deserialize a
                // missing key to `None` — plain `String` here would be
                // a hard deserialize error for exactly the subjects the
                // mask exists to protect. Refused at generation time.
                if f.is_masked() && !f.is_option() {
                    return Err(format!(
                        "screen {} ({}): field `{}` is masked/sensitive, so its type must be `Option<...>` (the read mask removes the key; the row struct must tolerate its absence)",
                        screen.id, screen.archetype, f.name
                    ));
                }
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

    // ---- the register pair must agree (menus vs screens) ----
    // The two registers are one contract: a menu item is a promise that
    // the generated app serves a screen at a route, for roles, gated by
    // a policy the generator synthesized. Each violated promise is a
    // build-time error here, not a dead link or a silent deny at runtime.
    let invariants = validate_menus(&menus, &register, &planned, &entities)?;

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
    let bridge = render_bridge(&register_id, &register, &entities, &planned, &all_roles, &singletons)?;
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
        let rel = Path::new(file)
            .strip_prefix("src/")
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.clone());
        let body = render_screen_file(plans, &entities, &register)?;
        let path = src_dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        files.push(file.clone());
        modules.push((module_name_from_path(&rel), rel));
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
    let serve = render_serve(&crate_name, &planned);
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
        invariants_checked: invariants,
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

    /// Membership check, not a silent String fallback: `type_of` returns
    /// "String" for a *nonexistent* field too, so parameter validation
    /// that wants to typo-proof a field name must use this. The primary
    /// key counts as a declared field (it is one).
    fn has_field(&self, field: &str) -> bool {
        field == self.primary_key || self.fields.iter().any(|f| f.name == field)
    }

    fn field_decl(&self, field: &str) -> Option<&FieldDecl> {
        self.fields.iter().find(|f| f.name == field)
    }

    fn guard_table_fn(&self) -> String {
        self.guard
            .as_ref()
            .map(|g| g.table.clone())
            .unwrap_or_else(|| format!("{}_table", pascal_to_snake(&self.name)))
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

/// Cross-file invariants between `menus.toml` and the (planned) screen
/// register — the generated-app edition of RTM's menus.toml V-probes.
/// Returns the number of invariant checks performed (for the report).
/// Every violation is a hard error: a menu item is a promise the
/// generated app can keep, or the build refuses.
fn validate_menus(
    menus: &MenusFile,
    register: &RegisterFile,
    planned: &[PlannedScreen],
    entities: &BTreeMap<String, EntityDecl>,
) -> Result<usize, String> {
    let mut checks = 0usize;

    // Every register screen by id — including non-built ones, so a menu
    // pointing at a blocked screen gets the V3 message, not V1. The
    // stage check below refuses it either way.
    let screen_by_id: BTreeMap<&str, &ScreenDecl> =
        register.screen.iter().map(|s| (s.id.as_str(), s)).collect();
    // Synthesized policy surface: (resource, action) pairs the generated
    // app can actually enforce.
    let mut policy_surface: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for plan in planned {
        let Some(binding) = plan.decl.data_binding.as_ref() else { continue };
        let Some(entity) = binding.entities.first() else { continue };
        let Some(guard) = binding.guard.as_ref() else { continue };
        let Some(policy) = plan.decl.policy.as_ref() else { continue };
        let resource = entity.to_lowercase();
        for action in &policy.allowed_actions {
            policy_surface.insert((resource.clone(), action.clone()));
            let _ = guard;
        }
    }
    // Human roles the register declares (from every screen's roles).
    let mut human_roles: std::collections::HashSet<String> = std::collections::HashSet::new();
    for screen in &register.screen {
        for role in &screen.roles {
            let name = role.split(':').next().unwrap_or(role).to_string();
            if name != "AllRoles" && name != "AllHuman" {
                human_roles.insert(name);
            }
        }
    }

    // V1 + V2 + V3 + route-agreement + V5-lite, per menu entry.
    let mut seen_routes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in &menus.menu {
        let target = screen_by_id
            .get(entry.screen_id.as_str())
            .ok_or_else(|| format!("menu {} references screen_id `{}` which does not exist in the register (V1)", entry.id, entry.screen_id))?;
        checks += 1;

        let stage = if target.stage.is_empty() { "blocked" } else { target.stage.as_str() };
        if stage != "built" {
            return Err(format!(
                "menu {} references screen {} which is stage `{stage}` — a generated app cannot promise a route for a screen the register does not build (V3); hide it in menus.toml or build the screen",
                entry.id, entry.screen_id
            ));
        }

        // V2: menu.roles ⊆ screen.roles (universal screen roles satisfy
        // every role).
        let screen_role_names: Vec<String> = target
            .roles
            .iter()
            .map(|r| r.split(':').next().unwrap_or(r).to_string())
            .collect();
        let universal = screen_role_names.iter().any(|r| r == "AllRoles" || r == "AllHuman");
        if !universal {
            for role in &entry.roles {
                if !screen_role_names.contains(role) {
                    return Err(format!(
                        "menu {} grants role `{role}` but its target screen {} declares roles {:?} — menu.roles must be a subset of screen.roles (V2)",
                        entry.id, entry.screen_id, target.roles
                    ));
                }
            }
        }
        checks += 1;

        // Route agreement: the menu's route is the route the generator
        // actually emits for that screen — not a related route, not a
        // hand-written spelling of it.
        let emitted_route = target
            .codegen
            .as_ref()
            .and_then(|c| c.route_path.clone())
            .unwrap_or_default();
        if entry.route != emitted_route {
            return Err(format!(
                "menu {} routes to `{}` but screen {} emits `{}` — menu.route must equal codegen.route_path, or the nav link 404s the app it ships in",
                entry.id, entry.route, entry.screen_id, emitted_route
            ));
        }
        if !seen_routes.insert(entry.route.clone()) {
            return Err(format!(
                "menu {} duplicates route `{}` (already used by another menu entry) — nav routes must be unique (V6-lite)",
                entry.id, entry.route
            ));
        }
        checks += 1;

        // V5-lite: a guard reference must resolve to a policy the
        // generator synthesizes. static_embed screens have no data plane,
        // so a guard reference there is always a mistake.
        if let Some(menu_guard) = &entry.guard {
            if target.archetype == "static_embed" || target.archetype == "login" || target.archetype == "app_shell_from_toml" {
                return Err(format!(
                    "menu {} declares guard {{ action = \"{}\", resource = \"{}\" }} but targets {} `{}` — presentational screens have no guard surface; drop the guard reference",
                    entry.id, menu_guard.action, menu_guard.resource, target.archetype, target.name
                ));
            }
            let key = (menu_guard.resource.to_lowercase(), menu_guard.action.clone());
            if !policy_surface.contains(&key) {
                return Err(format!(
                    "menu {} declares guard {{ action = \"{}\", resource = \"{}\" }} but no synthesized policy covers that (resource, action) — add the action to the owning screen's [screen.policy].allowed_actions (V5-lite)",
                    entry.id, menu_guard.action, menu_guard.resource
                ));
            }
            checks += 1;
        }
    }

    // V8-lite: each landing key is a declared human role, and its target
    // is a route the generated app serves for a screen that role may
    // reach.
    for (role, target_route) in &menus.landing {
        if !human_roles.contains(role) {
            return Err(format!(
                "[landing].{role} is not a human role any screen declares (V8-lite): declared roles are {:?}",
                human_roles
            ));
        }
        let reachable = planned.iter().any(|p| {
            p.route_path == *target_route
                && (p.decl.roles.iter().any(|r| {
                    let name = r.split(':').next().unwrap_or(r);
                    name == "AllRoles" || name == "AllHuman" || name == *role
                }))
        });
        if !reachable {
            return Err(format!(
                "[landing].{role} targets `{target_route}` which no built screen at that route offers to role `{role}` (V8-lite: every role lands on a screen it can read)"
            ));
        }
        checks += 1;
    }

    // Guard resources must name real entities (typo-proofing the V5
    // references even before the action check above).
    for entry in &menus.menu {
        if let Some(menu_guard) = &entry.guard {
            if !entities.values().any(|d| d.resource() == menu_guard.resource.to_lowercase()) {
                return Err(format!(
                    "menu {} guard resource \"{}\" matches no entity in the register (expected the entity's lowercase name)",
                    entry.id, menu_guard.resource
                ));
            }
            checks += 1;
        }
    }

    Ok(checks)
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

fn module_name_from_path(rel: &str) -> String {
    let stem = rel
        .strip_suffix(".nir")
        .or_else(|| rel.strip_suffix(".rs"))
        .unwrap_or(rel);
    stem.replace(|c: char| c == '/' || c == '-' || c == '.', "_")
}

fn rust_path_ident(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '_')
        .to_string()
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

/// Register-level access vocabulary -> macro clause. The macros accept
/// only `public` / `requires role "X"`; approval_inbox additionally accepts
/// `or role "Y"...`. Registers may spell the coarse vocabulary
/// `role` / `self_or_role`
/// (mirroring `policy.subject_scope`); those synthesize the `requires
/// role` clause from the screen's own declared roles. Any other value
/// passes through verbatim — registers may already carry the full macro
/// clause (banking/trading style).
fn translate_access(value: &str, roles: &[String], supports_multiple_roles: bool) -> String {
    match value {
        "public" => "public".to_string(),
        "role" | "self_or_role" => {
            let names: Vec<&str> = roles
                .iter()
                .map(|r| r.split(':').next().unwrap_or(r))
                .filter(|r| !r.is_empty() && *r != "AllRoles" && *r != "AllHuman")
                .collect();
            match names.first() {
                None => "public".to_string(),
                Some(first) => {
                    let mut clause = format!("requires role {first:?}");
                    if supports_multiple_roles {
                        for r in &names[1..] {
                            clause.push_str(&format!(" or role {r:?}"));
                        }
                    }
                    clause
                }
            }
        }
        other => other.to_string(),
    }
}

fn access_literal(value: Option<&str>, default: &str, roles: &[String]) -> String {
    match value {
        Some(v) if !v.is_empty() => translate_access(v, roles, false),
        _ => default.to_string(),
    }
}

fn multi_role_access_literal(value: Option<&str>, default: &str, roles: &[String]) -> String {
    match value {
        Some(v) if !v.is_empty() => translate_access(v, roles, true),
        _ => default.to_string(),
    }
}

// ---------------------------------------------------------------------------
// bridge.nir — entities, guard tables, stores, cells, roles, policies
// ---------------------------------------------------------------------------

fn render_bridge(
    register_id: &str,
    register: &RegisterFile,
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
    // The marker ident follows role_ident's PascalCase convention while
    // the wire value remains the register spelling used by Auth and the
    // generated guard policies.
    if !all_roles.is_empty() {
        out.push_str("nirdosha_rt::roles! {\n");
        for role in all_roles {
            // Route gates canonicalize a snake_case wire role to a
            // PascalCase marker type (`kyc_analyst` -> `KycAnalyst`).
            // Keep the wire value unchanged for Auth and guard checks.
            out.push_str(&format!("    {} = {role:?};\n", pascalize(role)));
        }
        out.push_str("}\n\n");
    }

    // approval chains: `nirdosha_rt::approval_chain!` blocks, then the
    // same registry-dump helper RTM's hand-written bridge carries, so
    // every `GuardedTable` constructor below can pass the real chain set
    // (the runtime's escalation/return machinery runs on it). Validated
    // here so a typo'd approver role or a zero quorum is a generation
    // error, not a runtime "chain has no registered definition".
    let chains_declared = !register.approval_chain.is_empty();
    if chains_declared {
        out.push_str("// ---- approval chains ----\n\n");
        for chain in &register.approval_chain {
            if chain.quorum < 1 {
                return Err(format!(
                    "approval chain `{}`: quorum must be at least 1 (got {})",
                    chain.name, chain.quorum
                ));
            }
            let approvers_pascal: Vec<String> = chain.approvers.iter().map(|r| pascalize(r)).collect();
            for approver in &approvers_pascal {
                if !all_roles.iter().any(|r| pascalize(r) == *approver) {
                    return Err(format!(
                        "approval chain `{}` names approver role `{}` which no screen declares — approvers are exact string compares at the data plane",
                        chain.name, approver
                    ));
                }
            }
            let cooling = chain
                .cooling_seconds
                .map(|n| format!("cooling(seconds = {n}); "))
                .unwrap_or_default();
            out.push_str(&format!(
                "nirdosha_rt::approval_chain! {{\n    chain {} {{ quorum({}, of = [{}]); timeout(deny); {} }} }}\n\n",
                chain.name,
                chain.quorum,
                approvers_pascal.join(", "),
                cooling
            ));
        }
        out.push_str(
            "pub fn corpus_approval_chains() -> Vec<nirdosha_guard_core::approval_chain::ApprovalChainDefinition> {\n    nirdosha_guard_registry::dump()\n        .approval_chains\n        .iter()\n        .map(|c| nirdosha_guard_core::approval_chain::ApprovalChainDefinition { name: c.name.clone(), quorum: c.quorum, approver_roles: c.approvers.to_vec(), cooling_period_ms: c.cooling_ms })\n        .collect()\n}\n\n",
        );
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
        // Field policy, synthesized from the screen's OWN declared
        // field flags. No flags → no clause (exact back-compat: an
        // unflagged register synthesizes exactly the policies it did
        // before). With flags:
        //   - masked/sensitive → `forbidden(...)`: dropped from reads
        //     (genuine absence via Drop mask) and refused on writes
        //     (fail-closed allowed set),
        //   - everything else the screen declares → `allowed(...)`, so
        //     a write is fail-closed to the screen's own field shape,
        //   - required flags → `required(...)`, create actions only
        //     (v1 scope; every partial update would otherwise have to
        //     resend the whole row).
        // Per-subject masking falls out of the per-screen subject set:
        // two screens over the same table with different field flags
        // synthesize different policies — each subject matches only the
        // record(s) its roles are named in.
        let screen_fields: Vec<&FieldDecl> = binding.fields.iter().collect();
        let has_flags = screen_fields.iter().any(|f| f.is_masked() || f.required);
        for action in &policy.allowed_actions {
            let id = format!("{}-{}-{}", register_id, screen.id.replace('.', "-"), action);
            // The required() sub-clause is create-scoped (v1): partial
            // updates must not be forced to resend the whole row.
            let action_clause = if has_flags {
                let forbidden: Vec<String> = screen_fields.iter().filter(|f| f.is_masked()).map(|f| f.name.clone()).collect();
                let allowed: Vec<String> = screen_fields.iter().filter(|f| !f.is_masked()).map(|f| f.name.clone()).collect();
                let mut clause = String::from("field_policy {");
                if !allowed.is_empty() {
                    clause.push_str(&format!(" allowed({})", allowed.join(", ")));
                }
                if !forbidden.is_empty() {
                    clause.push_str(&format!(" forbidden({})", forbidden.join(", ")));
                }
                if action == "create" {
                    let required: Vec<String> = screen_fields.iter().filter(|f| f.required).map(|f| f.name.clone()).collect();
                    if !required.is_empty() {
                        clause.push_str(&format!(" required({})", required.join(", ")));
                    }
                }
                clause.push_str(" }");
                clause
            } else {
                String::new()
            };
            let clause_line = if action_clause.is_empty() { String::new() } else { format!("    {action_clause}\n") };
            out.push_str(&format!(
                "nirdosha_rt::guard_policy! {{\n    allow \"{id}\" for {}\n    when action == \"{action}\" && resource == \"{resource}\"\n    purpose({purpose_pascal})\n    filter tenant_scope()\n{clause_line}    cap(row_cap = 200, max_scan_rows = 20_000)\n    obligate audit(sampled)\n}}\n\n",
                subjects.join(", ")
            ));
        }
    }

    // Every table receives the registered chains (empty when none): the
    // escalation/return runtime is a constructor input, not a global.
    let approval_chains_arg = if chains_declared { "corpus_approval_chains()" } else { "Vec::new()" };

    // entities
    for decl in entities.values() {
        let struct_name = decl.struct_name();
        let resource = decl.resource();
        let pk = &decl.primary_key;
        out.push_str(&format!(
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]\npub struct {struct_name} {{\n    pub id: i64,\n    pub {pk}: String,\n    pub tenant_id: String,\n"
        ));
        for f in &decl.fields {
            // The pk is emitted explicitly above; a screen that names it
            // in its own data_binding.fields must not duplicate it.
            if f.name == *pk {
                continue;
            }
            if f.is_option() {
                // Option fields: a dropped (masked) key deserializes to
                // None, and the None is omitted again on re-serialize —
                // true absence survives the round trip, the same
                // convention RTM's bridge rows carry by hand.
                out.push_str(&format!(
                    "    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n    pub {}: {},\n",
                    f.name, f.ty
                ));
            } else {
                out.push_str(&format!("    pub {}: {},\n", f.name, f.ty));
            }
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
            "    TABLE.get_or_init(|| {{\n        nirdosha_guard_screens::GuardedTable::new(\n            nirdosha_guard_mic::MemStoreDriver::new(),\n            nirdosha_guard_registry::candidates(),\n            nirdosha_guard_registry::dump().policies,\n            \"{register_id}-demo\",\n            std::env::temp_dir().join(format!(\"{register_id}-audit-{resource}-{{}}.jsonl\", std::process::id())),\n            HUMAN_ROLES.iter().map(|r| r.to_string()).collect(),\n            DEMO_TENANT,\n            {approval_chains_arg},\n        )\n    }})\n}}\n\n"
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
        out.push_str("/// Boot-time seeding for singleton guarded rows. Idempotent per\n/// process: `std::sync::Once` (the guarded store lives for the process\n/// lifetime, and `system_write` refuses a duplicate create rather than\n/// upserting). Called from the generated `serve` main and from tests\n/// that build a router without running main.\n");
    }
    out.push_str("pub fn seed_singletons() {\n    static ONCE: std::sync::Once = std::sync::Once::new();\n    ONCE.call_once(|| {\n");
    for (entity, row_id) in singletons {
        let Some(decl) = entities.get(entity.as_str()) else { continue };
        let struct_name = decl.struct_name();
        let pk = &decl.primary_key;
        let mut fields_init = String::new();
        for f in &decl.fields {
            if f.name == *pk {
                continue;
            }
            fields_init.push_str(&format!(", {}: Default::default()", f.name));
        }
        out.push_str(&format!(
            "        {table_fn}().system_write(DEMO_TENANT, &{struct_name} {{ id: 0, {pk}: {row_id:?}.into(), tenant_id: DEMO_TENANT.into(){fields_init} }});\n",
            table_fn = decl.guard_table_fn()
        ));
    }
    out.push_str("    });\n}\n");
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
        let users = plan
            .decl
            .parameters
            .get("demo_users")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if users.is_empty() {
            // No fixture users declared: emit production mode (env-var
            // user list), never `mode: demo` with an empty table — the
            // macro refuses that at compile time.
            out.push_str(&format!(
                "nirdosha_rt::login! {{\n    mount: {mount},\n    path: {path:?},\n    mode: production,\n    landing: landing_path,\n}}\n\n"
            ));
            continue;
        }
        out.push_str(&format!("nirdosha_rt::login! {{\n    mount: {mount},\n    path: {path:?},\n    mode: demo,\n"));
        out.push_str("    demo_users: [\n");
        for user in users {
            let username = user.get("username").and_then(toml::Value::as_str).unwrap_or_default();
            let password = user.get("password").and_then(toml::Value::as_str).unwrap_or_default();
            let roles: Vec<String> = user
                .get("roles")
                .and_then(toml::Value::as_array)
                .map(|a| a.iter().filter_map(toml::Value::as_str).map(|s| format!("{s:?}")).collect())
                .unwrap_or_default();
            let avatar = user.get("avatar").and_then(toml::Value::as_str);
            let avatar_clause = avatar.map(|a| format!(", avatar: {a:?}")).unwrap_or_default();
            out.push_str(&format!(
                "        {{ username: {username:?}, password: {password:?}, roles: [{}]{avatar_clause} }},\n",
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

fn render_screen_file(
    plans: &[&PlannedScreen],
    entities: &BTreeMap<String, EntityDecl>,
    register: &RegisterFile,
) -> Result<String, String> {
    // Render every invocation FIRST, then import only the bridge idents
    // the rendered text actually references — no `#[allow(unused)]`, no
    // blanket import: the emitted import list is derived from the same
    // text that needs it.
    let mut body = String::new();
    for plan in plans {
        body.push_str(&render_screen_invocation(plan, entities, register)?);
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

fn render_screen_invocation(
    plan: &PlannedScreen,
    entities: &BTreeMap<String, EntityDecl>,
    register: &RegisterFile,
) -> Result<String, String> {
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
            // Displayed fields = this screen's OWN declared data_binding
            // fields, minus masked ones (a forbidden field is dropped
            // from every read this screen serves, so it must never be a
            // column/search candidate — it could not be searched into
            // existence). Screens that declare no fields of their own
            // fall back to the entity-wide union.
            let own_fields: Vec<FieldDecl> = binding
                .map(|b| b.fields.iter().filter(|f| !f.is_masked()).cloned().collect())
                .unwrap_or_default();
            let displayed: &[FieldDecl] = if own_fields.is_empty() { &decl.fields } else { &own_fields };
            let fields: Vec<String> = displayed.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect();
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::crud_screens! {{\n    mount: {mount},\n    entity: {struct_name},\n    store: {store},\n    path: {path:?},\n    fields: [ {} ],\n",
                fields.join(", ")
            ));
            for (key, default) in [("create", "public"), ("read", "public"), ("update", "public"), ("delete", "public")] {
                let key = format!("access_{key}");
                let access = access_literal(params.get(&key).and_then(toml::Value::as_str), default, &plan.decl.roles);
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
                        let own_masked: Vec<String> = binding
                            .map(|b| b.fields.iter().filter(|f| f.is_masked()).map(|f| f.name.clone()).collect())
                            .unwrap_or_default();
                        let mut typed: Vec<String> = Vec::new();
                        for name in names.iter().filter_map(toml::Value::as_str) {
                            if !decl.has_field(name) {
                                return Err(format!(
                                    "screen {}: {clause} names `{name}` which the entity does not declare — a typo here would emit a field the macro cannot parse; declare it in data_binding.fields or fix the spelling",
                                    plan.id
                                ));
                            }
                            if own_masked.iter().any(|m| m == name) {
                                // masked on this screen = forbidden by
                                // this screen's own synthesized policy —
                                // a write list naming one could never
                                // succeed.
                                return Err(format!(
                                    "screen {}: {clause} includes `{name}` which the screen itself declares masked — the guard forbids submitting it, so the form would always fail",
                                    plan.id
                                ));
                            }
                            typed.push(format!("{}: {}", name, decl.type_of(name)));
                        }
                        // An empty write list must omit the clause
                        // entirely: `[  ]` is not parseable macro input,
                        // and omitting create_fields/update_fields is the
                        // macro's own read-only-by-construction shape.
                        if !typed.is_empty() {
                            out.push_str(&format!(" {}: [ {} ],", clause, typed.join(", ")));
                        }
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
            let post_access = access_literal(params.get("post_access").or_else(|| params.get("access")).and_then(toml::Value::as_str), "public", &plan.decl.roles);
            let read_access = access_literal(params.get("read_access").or_else(|| params.get("access")).and_then(toml::Value::as_str), "public", &plan.decl.roles);
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
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
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
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
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
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
            let title_field = params.get("title_field").and_then(toml::Value::as_str).unwrap_or("title");
            let column_field = params
                .get("column_field")
                .and_then(toml::Value::as_str)
                .ok_or("kanban_board needs parameters.column_field")?
                .to_string();
            if !decl.has_field(&column_field) {
                return Err(format!(
                    "screen {}: kanban column_field `{column_field}` is not a declared field of `{}` — the generated move handler writes through it",
                    plan.id,
                    decl.name
                ));
            }
            if !decl.has_field(title_field) {
                return Err(format!(
                    "screen {}: kanban title_field `{title_field}` is not a declared field of `{}`",
                    plan.id,
                    decl.name
                ));
            }
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
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
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
                let name = entry.split(':').next().unwrap_or(entry);
                if !decl.has_field(name) {
                    return Err(format!(
                        "screen {}: report_builder dimension `{name}` is not a declared field of `{}`",
                        plan.id,
                        decl.name
                    ));
                }
                if !entry.ends_with("String") {
                    return Err(format!("screen {}: report_builder dimensions must be String-typed (got `{entry}`)", plan.id));
                }
            }
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
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
                if !decl.has_field(field) {
                    return Err(format!(
                        "screen {}: tree_view field `{field}` is not a declared field of `{}`",
                        plan.id,
                        decl.name
                    ));
                }
                if decl.type_of(field) != "String" {
                    return Err(format!("screen {}: tree_view fields must be String-typed (`{field}` is `{}`)", plan.id, decl.type_of(field)));
                }
            }
            let access = access_literal(params.get("access").and_then(toml::Value::as_str), "public", &plan.decl.roles);
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::tree_view! {{\n    mount: {mount},\n    entity: {struct_name},\n    table: {table},\n    path: {path:?},\n    title: {:?},\n    purpose: {purpose:?},\n    access: {access},\n    id_field: {id_field},\n    parent_field: {parent_field},\n    label_field: {label_field},\n}}\n",
                plan.decl.name
            ));
            Ok(out)
        }
        "approval_inbox" => {
            let sources = params
                .get("sources")
                .and_then(toml::Value::as_array)
                .ok_or_else(|| format!("screen {}: approval_inbox needs parameters.sources", plan.id))?.as_slice()
                .to_vec();
            if sources.is_empty() {
                return Err(format!("screen {}: approval_inbox needs at least one source", plan.id));
            }
            let mut out = String::new();
            out.push_str(&format!(
                "nirdosha_rt::approval_inbox! {{\n    mount: {mount},\n    path: {path:?},\n"
            ));
            // Access stays vestigial route metadata in guard mode (the
            // macro ignores it for routing when every source is guarded
            // — and the generator only ever emits guard mode, because
            // every entity a generated screen touches must be guarded).
            let access = multi_role_access_literal(params.get("access").and_then(toml::Value::as_str), "", &plan.decl.roles);
            if !access.is_empty() {
                out.push_str(&format!("    access: {access},\n"));
            }
            out.push_str("    sources: [\n");
            for source in &sources {
                let table = source
                    .get("table")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("screen {}: approval_inbox source needs `table`", plan.id))?
                    .to_string();
                let chain = source
                    .get("chain")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("screen {}: approval_inbox source needs `chain`", plan.id))?
                    .to_string();
                let detail_path = source
                    .get("detail_path")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("/{id}")
                    .to_string();
                // Typo-proof the table: it must be some registered
                // entity's generated GuardedTable constructor.
                let decl = entities
                    .values()
                    .find(|d| d.guard_table_fn() == table)
                    .ok_or_else(|| format!(
                        "screen {}: approval_inbox source table `{table}` matches no entity declared in the register (expected `{name}_table` for a registered entity)",
                        plan.id,
                        name = entities
                            .keys()
                            .next()
                            .map(|k| pascal_to_snake(k))
                            .unwrap_or_default()
                    ))?;
                // The worklist read must be guard-gated: every entity a
                // generated screen touches must be guarded, and the
                // inbox's view evaluates that guard per source.
                let purpose = decl
                    .guard
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "screen {}: approval_inbox source entity `{}` has no data_binding.guard anywhere in the register — the worklist read must be guard-gated",
                            plan.id, decl.name
                        )
                    })?
                    .purpose
                    .clone();
                // The return route's {resource} must name the entity the
                // escalation really lives in — derived, not declared, so
                // return routing cannot drift from the table.
                let resource = decl.resource();
                // The chain must be registered in the same register, or
                // the escalations this inbox lists can never open. The
                // chain name string is compared verbatim — it is the
                // registry key both sides use.
                if !register.approval_chain.iter().any(|c| c.name == chain) {
                    return Err(format!(
                        "screen {}: approval_inbox source chain `{chain}` is not declared in any [[approval_chain]] — the escalation it lists could never open",
                        plan.id
                    ));
                }
                out.push_str(&format!(
                    "        {{ table: {table}, chain: {chain:?}, resource: {resource:?}, detail_path: {detail_path:?}, purpose: {purpose:?} }},\n"
                ));
            }
            out.push_str("    ],\n}\n");
            Ok(out)
        }
        "workspace" => {
            let subject = params.get("subject").ok_or_else(|| format!("screen {}: workspace needs parameters.subject", plan.id))?.as_table().ok_or_else(|| format!("screen {}: workspace subject must be a [table]", plan.id))?.clone();
            let subject_entity = subject
                .get("entity")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!(
                    "screen {}: workspace subject needs `entity` (the registered entity the workspace centers on; its guard purpose is derived, not re-declared)",
                    plan.id
                ))?
                .to_string();
            let decl = entities
                .get(&subject_entity)
                .ok_or_else(|| format!("screen {}: workspace subject entity `{subject_entity}` is not declared in the register", plan.id))?;
            let purpose = guard
                .ok_or_else(|| format!("screen {}: workspace reads a GuardedTable and needs data_binding.guard", plan.id))?
                .purpose
                .clone();
            let subject_table = decl.guard_table_fn();
            let struct_name = decl.struct_name();
            let label_fn = format!("{}_workspace_label", pascal_to_snake(&decl.name));
            let label_field = subject
                .get("label_field")
                .and_then(toml::Value::as_str)
                .unwrap_or(&decl.primary_key)
                .to_string();
            if !decl.has_field(&label_field) {
                return Err(format!(
                    "screen {}: workspace subject label_field `{label_field}` is not a declared field of `{}`",
                    plan.id,
                    decl.name
                ));
            }

            // Panel sources: the macro needs real
            // `fn(&Auth, &str) -> Result<Vec<serde_json::Value>, String>`
            // fns. The generator emits them for the inline-table form
            // `{ entity, link_field }` — a real guarded read of the
            // linked entity, filtered in-process to the subject row
            // (masks, caps and audit come from the guard, not from this
            // fn). A string `source` names a hand-written fn the
            // generator cannot emit — refused with that name rather
            // than emitted as a compile error later.
            let panels = params
                .get("panels")
                .and_then(toml::Value::as_array)
                .ok_or_else(|| format!("screen {}: workspace needs parameters.panels", plan.id))?.as_slice()
                .to_vec();
            if panels.is_empty() {
                return Err(format!("screen {}: workspace needs at least one panel", plan.id));
            }
            let mut panel_fns = String::new();
            let mut panel_invocations = Vec::new();
            for panel in &panels {
                let need = panel
                    .get("need")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("screen {}: workspace panel needs `need`", plan.id))?
                    .to_string();
                let title = panel
                    .get("title")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(&need)
                    .to_string();
                let render = panel
                    .get("render")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("table")
                    .to_string();
                let source = panel
                    .get("source")
                    .ok_or_else(|| format!("screen {}: workspace panel `{need}` needs `source`", plan.id))?;
                let source_invocation = match source.as_str() {
                    Some(hand_written) => {
                        return Err(format!(
                            "screen {}: workspace panel `{need}` names hand-written fn `{hand_written}` — the generator cannot emit business-logic fns yet; declare the panel as source = {{ entity = \"X\", link_field = \"y\" }} (a real guarded read filtered to the subject row) or mark the screen blocked",
                            plan.id
                        ));
                    }
                    None => {
                        let source_table = source.as_table().ok_or_else(|| {
                            format!("screen {}: workspace panel `{need}` source must be a string or an {{ entity, link_field }} table", plan.id)
                        })?.clone();
                        let panel_entity = source_table
                            .get("entity")
                            .and_then(toml::Value::as_str)
                            .ok_or_else(|| format!("screen {}: workspace panel `{need}` source needs `entity`", plan.id))?
                            .to_string();
                        let link_field = source_table
                            .get("link_field")
                            .and_then(toml::Value::as_str)
                            .ok_or_else(|| format!("screen {}: workspace panel `{need}` source needs `link_field` (the field joining the panel's rows to the subject row)", plan.id))?
                            .to_string();
                        let panel_decl = entities
                            .get(&panel_entity)
                            .ok_or_else(|| format!(
                                "screen {}: workspace panel `{need}` entity `{panel_entity}` is not declared in the register",
                                plan.id
                            ))?;
                        let panel_purpose = panel_decl
                            .guard
                            .as_ref()
                            .ok_or_else(|| {
                                format!(
                                    "screen {}: workspace panel `{need}` entity `{}` has no data_binding.guard — panel reads must be guard-gated",
                                    plan.id, panel_decl.name
                                )
                            })?
                            .purpose
                            .clone();
                        let link = panel_decl
                            .field_decl(&link_field)
                            .ok_or_else(|| format!(
                                "screen {}: workspace panel `{need}` link_field `{link_field}` is not a declared field of `{}`",
                                plan.id,
                                panel_decl.name
                            ))?;
                        let link_name = link.name.clone();
                        if link.ty != "String" {
                            return Err(format!(
                                "screen {}: workspace panel `{need}` link_field `{link_field}` must be String-typed (the subject id is a String) — got `{}`",
                                plan.id, link.ty
                            ));
                        }
                        let fn_name = format!("__panel_{}", pascal_to_snake(&need));
                        let panel_table = panel_decl.guard_table_fn();
                        let _panel_struct = panel_decl.struct_name();
                        panel_fns.push_str(&format!(
                            "pub fn {fn_name}(auth: &nirdosha_rt::Auth, subject_id: &str) -> Result<Vec<serde_json::Value>, String> {{\n    let rows = {panel_table}().guarded_snapshot(auth, \"{panel_purpose}\").map_err(|e| e.to_string())?;\n    let mut out: Vec<serde_json::Value> = Vec::new();\n    for row in rows {{\n        if row.{link_name} == *subject_id {{\n            out.push(serde_json::to_value(&row).map_err(|e| e.to_string())?);\n        }}\n    }}\n    Ok(out)\n}}\n\n",
                        ));
                        fn_name
                    }
                };
                panel_invocations.push(format!(
                    "        {{ need: {need:?}, title: {title:?}, render: {render:?}, source: {source_invocation} }},\n"
                ));
            }

            let mut out = panel_fns;
            // The subject header label: a generated fn over the entity's
            // declared String field — real code, not a placeholder.
            out.push_str(&format!(
                "pub fn {label_fn}(row: &{struct_name}) -> Vec<(String, String)> {{\n    vec![\n        (\"{}\".to_string(), row.{label_field}.clone()),\n    ]\n}}\n\n",
                decl.name
            ));
            out.push_str(&format!(
                "nirdosha_rt::workspace! {{\n    mount: {mount},\n    path: {path:?},\n    subject: {{ table: {subject_table}, purpose: {purpose:?}, label_fn: {label_fn} }},\n"
            ));
            // The macro's clause grammar requires `budget:` in place;
            // the register may override the defaults but never omit it.
            let budget = params.get("budget");
            let max_rows = budget.and_then(|b| b.get("max_rows")).and_then(toml::Value::as_integer).unwrap_or(500);
            let max_ms = budget.and_then(|b| b.get("max_execution_ms")).and_then(toml::Value::as_integer).unwrap_or(2000);
            out.push_str(&format!("    budget: {{ max_rows: {max_rows}, max_execution_ms: {max_ms} }},\n"));
            out.push_str("    panels: [\n");
            for invocation in panel_invocations {
                out.push_str(&invocation);
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

fn render_serve(crate_name: &str, screens: &[PlannedScreen]) -> String {
    let crate_ident = rust_path_ident(crate_name);
    let mut out = String::new();
    out.push_str("//! Generated by `cargo nirdosha generate-screens` — boot wiring + mount order.\n");
    out.push_str("//! Literal-path mounts are registered before wildcard mounts: `Router::dispatch`\n");
    out.push_str("//! matches in registration order and a `/{id}` wildcard would otherwise swallow\n");
    out.push_str("//! the literal `new`/`board`/`edit` segments.\n\nfn main() {\n");
    out.push_str(&format!(
        "    {crate_ident}::bridge::seed_singletons();\n"
    ));
    out.push_str(&format!(
        "    let router = {crate_ident}::app_shell::mount_app_shell(nirdosha_rt::Router::new(|_req| nirdosha_rt::Auth::login(\"anon\", &[])));\n"
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
            .map(|f| {
                let without_src = f.strip_prefix("src/").unwrap_or(f);
                module_name_from_path(without_src)
            })
            .unwrap_or_else(|| "app_shell".to_string());
        if plan.archetype == "app_shell_from_toml" {
            continue; // mounted above
        }
        if plan.archetype == "login" {
            out.push_str(&format!(
                "    let router = {crate_ident}::app_shell::mount_login(router);\n"
            ));
            continue;
        }
        out.push_str(&format!(
            "    let router = {crate_ident}::{module}::{}(router);\n",
            plan.mount
        ));
        if plan.archetype == "kanban_board" {
            out.push_str(&format!(
                "    let router = {crate_ident}::{module}::{}_move(router);\n",
                plan.mount
            ));
        }
    }
    out.push_str("    println!(\"generated app — listening on http://localhost:8080/\");\n    router.serve(8080);\n}\n");
    out
}



#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid register pair for the temp-dir harness: 2 screens,
    /// 1 entity, 1 role, 1 landing, 1 menu item.
    const SCREENS: &str = r#"
[metadata]
schema = "nirdosha.screen-register/v2"
register_id = "t"
version = "1.0.0"
total_screens = 2
codegen_profile = "web-default"

[[screen]]
id = "1.1"
name = "Login"
module = "M1"
archetype = "login"
roles = ["AllRoles:R"]
stage = "built"
codegen = { template = "login", mount_symbol = "mount_login", route_path = "/login", generated_files = ["src/app_shell.nir"] }
parameters.demo_users = [ { username = "u", password = "p", roles = ["Agent"] } ]

[[screen]]
id = "2.1"
name = "Tickets"
module = "M2"
archetype = "crud_screens"
roles = ["Agent:R/W"]
stage = "built"
codegen = { template = "crud_screens", mount_symbol = "mount_tickets", route_path = "/tickets", generated_files = ["src/screens/m02.nir"] }
data_binding = { entities = ["Ticket"], primary_key = "ticket_id", guard = { table = "ticket_table", purpose = "Operations" }, fields = [ { name = "title", type = "String" }, { name = "internal_notes", type = "Option<String>", masked = true } ] }
policy = { purpose = "Operations", allowed_actions = ["read"] }
"#;

    const MENUS: &str = r#"
[meta]
app_title = "t"
screens_source = "screens.toml"

[landing]
Agent = "/tickets"

[[menu]]
id = "nav.tickets"
screen_id = "2.1"
route = "/tickets"
guard = { action = "read", resource = "ticket" }
roles = ["Agent"]
"#;

    fn write_project(screens: &str, menus: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();
        std::fs::write(dir_path.join("screens.toml"), screens).unwrap();
        std::fs::write(dir_path.join("menus.toml"), menus).unwrap();
        (dir, dir_path)
    }

    #[test]
    fn happy_path_passes_every_invariant() {
        let (_dir, dir) = write_project(SCREENS, MENUS);
        let report = run(&dir).expect("a consistent register pair must generate");
        assert_eq!(report.screens_emitted, 2);
        assert!(report.invariants_checked >= 4, "menu + landing invariants must be counted, got {}", report.invariants_checked);
    }

    #[test]
    fn snake_case_roles_emit_canonical_marker_types_and_wire_names() {
        let screens = SCREENS.replace("Agent:R/W", "kyc_analyst:R/W");
        let menus = MENUS.replace("Agent", "kyc_analyst");
        let (_dir, dir) = write_project(&screens, &menus);
        run(&dir).expect("snake_case register roles must generate");
        let bridge = std::fs::read_to_string(dir.join("src/bridge.nir")).unwrap();
        assert!(
            bridge.contains("KycAnalyst = \"kyc_analyst\";"),
            "role marker and wire name must agree with role_ident(): {bridge}"
        );
    }

    #[test]
    fn coarse_access_and_empty_write_lists_emit_valid_macro_grammar() {
        let screens = SCREENS.replace(
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }",
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\nparameters.access_read = \"role\"\nparameters.create_fields = []\nparameters.update_fields = []",
        );
        let (_dir, dir) = write_project(&screens, MENUS);
        run(&dir).expect("coarse access and read-only CRUD must generate");
        let screen = std::fs::read_to_string(dir.join("src/screens/m02.nir")).unwrap();
        assert!(!screen.contains("access: role"), "coarse access must never leak into macro syntax: {screen}");
        assert!(
            screen.contains("requires role \"Agent\""),
            "single-role macro grammar must receive one concrete route role: {screen}"
        );
        assert!(!screen.contains(" or role "), "crud_screens only accepts one route role: {screen}");
        assert!(!screen.contains("create_fields: [  ]"), "empty create fields must be omitted: {screen}");
        assert!(!screen.contains("update_fields: [  ]"), "empty update fields must be omitted: {screen}");
    }

    #[test]
    fn canonical_role_marker_collisions_are_refused_before_rustc() {
        let screens = SCREENS.replace(
            "roles = [\"Agent:R/W\"]",
            "roles = [\"compliance_officer:R/W\", \"ComplianceOfficer:R\"]",
        );
        let menus = MENUS.replace("Agent", "compliance_officer");
        let (_dir, dir) = write_project(&screens, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("both resolve to marker type `ComplianceOfficer`"), "{err}");
    }

    #[test]
    fn menu_referencing_an_unknown_screen_id_is_refused() {
        let menus = MENUS.replace("screen_id = \"2.1\"", "screen_id = \"9.9\"");
        let (_dir, dir) = write_project(SCREENS, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("which does not exist in the register (V1)"), "{err}");
    }

    #[test]
    fn menu_referencing_a_non_built_screen_is_refused() {
        let screens = SCREENS.replace(
            "[[screen]]\nid = \"2.1\"\nname = \"Tickets\"\nmodule = \"M2\"\narchetype = \"crud_screens\"\nroles = [\"Agent:R/W\"]\nstage = \"built\"",
            "[[screen]]\nid = \"2.1\"\nname = \"Tickets\"\nmodule = \"M2\"\narchetype = \"crud_screens\"\nroles = [\"Agent:R/W\"]\nstage = \"blocked\"\nblocked_by = [\"archetype:stub\"]",
        );
        assert!(screens.contains("stage = \"blocked\"\nblocked_by"), "test setup must actually flip 2.1 to blocked");
        let (_dir, dir) = write_project(&screens, MENUS);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("(V3)"), "non-built target must be refused: {err}");
    }

    #[test]
    fn menu_granting_an_undeclared_role_is_refused() {
        let menus = MENUS.replace("roles = [\"Agent\"]", "roles = [\"Mlro\"]");
        let (_dir, dir) = write_project(SCREENS, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("(V2)"), "{err}");
    }

    #[test]
    fn menu_route_disagreeing_with_codegen_route_is_refused() {
        let menus = MENUS.replace("route = \"/tickets\"", "route = \"/tickets/list\"");
        let (_dir, dir) = write_project(SCREENS, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("menu.route must equal codegen.route_path"), "{err}");
    }

    #[test]
    fn menu_guard_with_no_synthesized_policy_is_refused() {
        let menus = MENUS.replace("action = \"read\"", "action = \"aggregate\"");
        let (_dir, dir) = write_project(SCREENS, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("(V5-lite)"), "{err}");
    }

    #[test]
    fn landing_for_an_undeclared_role_is_refused() {
        let menus = MENUS.replace("[landing]\nAgent = \"/tickets\"", "[landing]\nMlro = \"/tickets\"");
        let (_dir, dir) = write_project(SCREENS, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("V8-lite"), "{err}");
    }

    #[test]
    fn landing_targeting_a_route_the_role_cannot_reach_is_refused() {
        // A route that exists but offers nothing to the landing role:
        // screen 3.1 at /admin declares only Admin.
        let screens = format!(
            "{SCREENS}\n\n[[screen]]\nid = \"3.1\"\nname = \"Admin Only\"\nmodule = \"M3\"\narchetype = \"app_shell_from_toml\"\nroles = [\"Admin:R\"]\nstage = \"built\"\ncodegen = {{ template = \"app_shell_from_toml\", mount_symbol = \"mount_admin\", route_path = \"/admin\", generated_files = [\"src/app_shell.nir\"] }}\n"
        );
        let screens = screens.replace("total_screens = 2", "total_screens = 3");
        let menus = MENUS.replace("Agent = \"/tickets\"", "Agent = \"/admin\"");
        let (_dir, dir) = write_project(&screens, &menus);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("V8-lite"), "{err}");
    }

    #[test]
    fn crud_create_fields_naming_an_undeclared_field_is_refused() {
        let screens = SCREENS.replace(
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }",
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\nparameters.create_fields = [ \"tiitle\" ]",
        );
        let (_dir, dir) = write_project(&screens, MENUS);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("which the entity does not declare"), "{err}");
    }

    #[test]
    fn create_fields_naming_a_masked_field_is_refused() {
        let screens = SCREENS.replace(
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }",
            "policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\nparameters.create_fields = [ \"title\", \"internal_notes\" ]",
        );
        let (_dir, dir) = write_project(&screens, MENUS);
        let err = run(&dir).unwrap_err();
        assert!(err.contains("the screen itself declares masked"), "{err}");
    }
}

#[cfg(test)]
mod workflows_tests {
    use super::*;

    /// A register carrying `[[approval_chain]]` + an approval_inbox and a
    /// workspace, both guarded. Ticket has a `title` field (label_field)
    /// and a `status` field (panel link target).
    const WF: &str = r#"
[metadata]
schema = "nirdosha.screen-register/v2"
register_id = "t"
version = "1.0.0"
total_screens = 3
codegen_profile = "web-default"

[[approval_chain]]
name = "ticket_close"
quorum = 1
approvers = ["Agent"]

[[screen]]
id = "1.1"
name = "Login"
module = "M1"
archetype = "login"
roles = ["AllRoles:R"]
stage = "built"
codegen = { template = "login", mount_symbol = "mount_login", route_path = "/login", generated_files = ["src/app_shell.nir"] }
parameters.demo_users = [ { username = "u", password = "p", roles = ["Agent"] } ]
"#;

    fn write_project(screens: &str, menus: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();
        std::fs::write(dir_path.join("screens.toml"), screens).unwrap();
        std::fs::write(dir_path.join("menus.toml"), menus).unwrap();
        (dir, dir_path)
    }

    fn wf_reg(tail: &str) -> String {
        // One shared Ticket entity + the screen tail.
        format!(
            "{}\n\
             [[screen]]\n\
             id = \"2.1\"\n\
             name = \"Tickets\"\n\
             module = \"M2\"\n\
             archetype = \"crud_screens\"\n\
             roles = [\"Agent:R/W\"]\n\
             stage = \"built\"\n\
             codegen = {{ template = \"crud_screens\", mount_symbol = \"mount_tickets\", route_path = \"/tickets\", generated_files = [\"src/screens/m02.nir\"] }}\n\
             data_binding = {{ entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = {{ table = \"ticket_table\", purpose = \"Operations\" }}, fields = [ {{ name = \"title\", type = \"String\" }}, {{ name = \"status\", type = \"String\" }} ] }}\n\
             policy = {{ purpose = \"Operations\", allowed_actions = [\"read\"] }}\n\
             {tail}\n",
            WF
        )
    }

    fn run_str(screens: &str) -> String {
        let (_d, dir) = write_project(screens, "[meta]\napp_title = \"t\"\n");
        match run(&dir) {
            Ok(_) => String::new(),
            Err(e) => e,
        }
    }

    #[test]
    fn approval_inbox_emits_guard_mode_sources() {
        let screens = wf_reg("\
             [[screen]]\n\
             id = \"3.1\"\n\
             name = \"Close Inbox\"\n\
             module = \"M3\"\n\
             archetype = \"approval_inbox\"\n\
             roles = [\"Agent:R\"]\n\
             stage = \"built\"\n\
             codegen = { template = \"approval_inbox\", mount_symbol = \"mount_close_inbox\", route_path = \"/approvals\", generated_files = [\"src/screens/m03.nir\"] }\n\
             data_binding = { entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = { table = \"ticket_table\", purpose = \"Operations\" }, fields = [] }\n\
             policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\n\
             parameters.sources = [ { table = \"ticket_table\", chain = \"ticket_close\", detail_path = \"/tickets/{id}\" } ]\n");
        let (_d, dir) = write_project(&screens, "[meta]\napp_title = \"t\"\n");
        let out = run(&dir).expect("a guarded approval_inbox with a registered chain must generate");
        let m03 = std::fs::read_to_string(dir.join("src/screens/m03.nir")).unwrap();
        assert!(m03.contains("purpose: \"Operations\""), "per-source guard purpose must be derived from the entity: {m03}");
        assert!(m03.contains("resource: \"ticket\""), "resource must be derived, not declared: {m03}");
        assert!(m03.contains("chain: \"ticket_close\""), "chain must survive verbatim: {m03}");
        assert!(out.screens_emitted == 3, "login + crud + inbox expected, got {}", out.screens_emitted);
        // the app-wide chain reached bridge.nir
        let bridge = std::fs::read_to_string(dir.join("src/bridge.nir")).unwrap();
        assert!(bridge.contains("chain ticket_close"), "chain must be emitted: {bridge}");
        assert!(bridge.contains("corpus_approval_chains()"), "constructor must receive the registry chain set: {bridge}");
    }

    #[test]
    fn approval_inbox_refuses_a_source_whose_chain_is_not_registered() {
        let screens = wf_reg("\
             [[screen]]\n\
             id = \"3.1\"\n\
             name = \"Close Inbox\"\n\
             module = \"M3\"\n\
             archetype = \"approval_inbox\"\n\
             roles = [\"Agent:R\"]\n\
             stage = \"built\"\n\
             codegen = { template = \"approval_inbox\", mount_symbol = \"mount_close_inbox\", route_path = \"/approvals\", generated_files = [\"src/screens/m03.nir\"] }\n\
             data_binding = { entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = { table = \"ticket_table\", purpose = \"Operations\" }, fields = [] }\n\
             policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\n\
             parameters.sources = [ { table = \"ticket_table\", chain = \"no_such_chain\", detail_path = \"/tickets/{id}\" } ]\n");
        let err = run_str(&screens);
        assert!(err.contains("no_such_chain"), "unregistered chain must be a generation error: {err}");
    }

    #[test]
    fn approval_inbox_refuses_a_source_table_that_matches_no_entity() {
        let screens = wf_reg("\
             [[screen]]\n\
             id = \"3.1\"\n\
             name = \"Close Inbox\"\n\
             module = \"M3\"\n\
             archetype = \"approval_inbox\"\n\
             roles = [\"Agent:R\"]\n\
             stage = \"built\"\n\
             codegen = { template = \"approval_inbox\", mount_symbol = \"mount_close_inbox\", route_path = \"/approvals\", generated_files = [\"src/screens/m03.nir\"] }\n\
             data_binding = { entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = { table = \"ticket_table\", purpose = \"Operations\" }, fields = [] }\n\
             policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\n\
             parameters.sources = [ { table = \"ticket_tbl\", chain = \"ticket_close\", detail_path = \"/tickets/{id}\" } ]\n");
        let err = run_str(&screens);
        assert!(err.contains("matches no entity"), "a typo'd table ident must be refused: {err}");
    }

    #[test]
    fn workspace_emits_a_real_guarded_panel_fn() {
        let screens = wf_reg("\
             [[screen]]\n\
             id = \"3.2\"\n\
             name = \"Ticket WS\"\n\
             module = \"M6\"\n\
             archetype = \"workspace\"\n\
             roles = [\"Agent:R\"]\n\
             stage = \"built\"\n\
             codegen = { template = \"workspace\", mount_symbol = \"mount_ticket_ws\", route_path = \"/tickets/{id}/workspace\", generated_files = [\"src/screens/m06.nir\"] }\n\
             data_binding = { entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = { table = \"ticket_table\", purpose = \"Operations\" }, fields = [ { name = \"title\", type = \"String\" } ] }\n\
             policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\n\
             parameters.subject = { entity = \"Ticket\", label_field = \"title\" }\n\
             parameters.panels = [ { need = \"related\", title = \"Related\", render = \"table\", source = { entity = \"Ticket\", link_field = \"status\" } } ]\n");
        let (_d, dir) = write_project(&screens, "[meta]\napp_title = \"t\"\n");
        run(&dir).expect("a workspace with inline panel sources must generate");
        let m06 = std::fs::read_to_string(dir.join("src/screens/m06.nir")).unwrap();
        assert!(m06.contains("guarded_snapshot(auth, \"Operations\")"), "panel fn must be a real guarded read: {m06}");
        assert!(m06.contains("row.status == *subject_id"), "panel fn must filter to the subject row: {m06}");
        assert!(m06.contains("vec!["), "label_fn must return header pairs: {m06}");
        assert!(m06.contains("Ticket\".to_string()"), "label must derive its name from the entity: {m06}");
    }

    #[test]
    fn workspace_refuses_a_hand_written_panel_source() {
        let screens = wf_reg("\
             [[screen]]\n\
             id = \"3.2\"\n\
             name = \"Ticket WS\"\n\
             module = \"M6\"\n\
             archetype = \"workspace\"\n\
             roles = [\"Agent:R\"]\n\
             stage = \"built\"\n\
             codegen = { template = \"workspace\", mount_symbol = \"mount_ticket_ws\", route_path = \"/tickets/{id}/workspace\", generated_files = [\"src/screens/m06.nir\"] }\n\
             data_binding = { entities = [\"Ticket\"], primary_key = \"ticket_id\", guard = { table = \"ticket_table\", purpose = \"Operations\" }, fields = [ { name = \"title\", type = \"String\" } ] }\n\
             policy = { purpose = \"Operations\", allowed_actions = [\"read\"] }\n\
             parameters.subject = { entity = \"Ticket\", label_field = \"title\" }\n\
             parameters.panels = [ { need = \"related\", title = \"Related\", render = \"table\", source = \"related_for_case\" } ]\n");
        let err = run_str(&screens);
        assert!(err.contains("cannot emit business-logic fns"), "a string-source panel the generator can't emit must be refused: {err}");
    }

    #[test]
    fn approval_chain_refuses_a_zero_quorum() {
        // Self-contained register: one login screen + one bad chain. The
        // quorum check runs first, so a zero quorum is a named error even
        // before approver resolution.
        let one = "\n[metadata]\nschema = \"nirdosha.screen-register/v2\"\nregister_id = \"t\"\nversion = \"1.0.0\"\ntotal_screens = 1\ncodegen_profile = \"web-default\"\n\n[[approval_chain]]\nname = \"bad\"\nquorum = 0\napprovers = [\"Agent\"]\n\n[[screen]]\nid = \"1.1\"\nname = \"Login\"\nmodule = \"M1\"\narchetype = \"login\"\nroles = [\"AllRoles:R\"]\nstage = \"built\"\ncodegen = { template = \"login\", mount_symbol = \"mount_login\", route_path = \"/login\", generated_files = [\"src/app_shell.nir\"] }\n\n";

        let one = "\n[metadata]\nschema = \"nirdosha.screen-register/v2\"\nregister_id = \"t\"\nversion = \"1.0.0\"\ntotal_screens = 1\ncodegen_profile = \"web-default\"\n\n[[approval_chain]]\nname = \"bad\"\nquorum = 0\napprovers = [\"Agent\"]\n\n[[screen]]\nid = \"1.1\"\nname = \"Login\"\nmodule = \"M1\"\narchetype = \"login\"\nroles = [\"Agent:R\"]\nstage = \"built\"\ncodegen = { template = \"login\", mount_symbol = \"mount_login\", route_path = \"/login\", generated_files = [\"src/app_shell.nir\"] }\n\n";
        let err = run_str(one);
        assert!(err.contains("quorum must be at least 1"), "{err}");
    }
}
