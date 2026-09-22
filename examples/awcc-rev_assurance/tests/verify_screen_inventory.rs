//! Load-bearing verification for `screens.toml` + `menus.toml`.
//!
//! Enforces the cross-file invariants documented at the top of
//! `menus.toml` (V1–V10) to the extent they can be checked from the
//! compiled policy corpus and the two TOML files.

extern crate awcc_rev_assurance;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env!("CARGO_MANIFEST_DIR"))
}

fn load_toml(name: &str) -> toml::Value {
    let path = manifest_dir().join(name);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {} ({}): {}", name, path.display(), e));
    src.parse()
        .unwrap_or_else(|e| panic!("could not parse {}: {}", name, e))
}

#[derive(Debug, Clone)]
struct Screen {
    id: String,
    stage: String,
    roles: Vec<String>,
}

#[derive(Debug, Clone)]
struct Route {
    path: Option<String>,
    screen_id: Option<String>,
    guard_action: Option<String>,
    guard_resource: Option<String>,
}

#[derive(Debug, Clone)]
struct Menu {
    id: String,
    screen_id: Option<String>,
    route: Option<String>,
    roles: Vec<String>,
    stage_min: Option<String>,
    guard_action: Option<String>,
    guard_resource: Option<String>,
    waive: Vec<String>,
}

fn parse_screens(value: &toml::Value) -> Vec<Screen> {
    value
        .get("screen")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .map(|entry| {
            let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let stage = entry.get("stage").and_then(|v| v.as_str()).unwrap_or("blocked").to_string();
            let roles = entry
                .get("roles")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.split(':').next().unwrap_or(s).to_string())
                        .collect()
                })
                .unwrap_or_default();
            Screen { id, stage, roles }
        })
        .collect()
}

fn parse_routes(value: &toml::Value) -> Vec<Route> {
    value
        .get("route")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .map(|entry| {
            let path = entry.get("path").and_then(|v| v.as_str()).map(String::from);
            let screen_id = entry.get("screen_id").and_then(|v| v.as_str()).map(String::from);
            let (guard_action, guard_resource) = entry
                .get("guard")
                .and_then(|v| v.as_table())
                .map(|t| {
                    (
                        t.get("action").and_then(|v| v.as_str()).map(String::from),
                        t.get("resource").and_then(|v| v.as_str()).map(String::from),
                    )
                })
                .unwrap_or((None, None));
            Route { path, screen_id, guard_action, guard_resource }
        })
        .collect()
}

fn parse_menus(value: &toml::Value) -> Vec<Menu> {
    value
        .get("menu")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .map(|entry| {
            let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let screen_id = entry.get("screen_id").and_then(|v| v.as_str()).map(String::from);
            let route = entry.get("route").and_then(|v| v.as_str()).map(String::from);
            let roles = entry
                .get("roles")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let stage_min = entry.get("stage_min").and_then(|v| v.as_str()).map(String::from);
            let (guard_action, guard_resource) = entry
                .get("guard")
                .and_then(|v| v.as_table())
                .map(|t| {
                    (
                        t.get("action").and_then(|v| v.as_str()).map(String::from),
                        t.get("resource").and_then(|v| v.as_str()).map(String::from),
                    )
                })
                .unwrap_or((None, None));
            let waive = entry
                .get("waive")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            Menu {
                id,
                screen_id,
                route,
                roles,
                stage_min,
                guard_action,
                guard_resource,
                waive,
            }
        })
        .collect()
}

fn policy_guard_pairs() -> HashSet<(String, String)> {
    nirdosha_guard_registry::POLICIES
        .iter()
        .map(|reg| (reg.action.to_string(), reg.resource.to_string()))
        .collect()
}

fn is_universal(role: &str) -> bool {
    role == "AllHuman" || role == "AllRoles" || role == "All"
}

fn stage_order(s: &str) -> i32 {
    match s {
        "built" => 4,
        "emittable" => 3,
        "interim" => 2,
        "blocked" => 1,
        "delegated" => 0,
        _ => -1,
    }
}

#[test]
fn screens_toml_and_menus_toml_are_internally_consistent() {
    let screens_toml = load_toml("screens.toml");
    let menus_toml = load_toml("menus.toml");

    let screens = parse_screens(&screens_toml);
    let menus = parse_menus(&menus_toml);
    let routes = parse_routes(&menus_toml);

    let by_id: HashMap<String, Screen> = screens.iter().map(|s| (s.id.clone(), s.clone())).collect();
    let guard_pairs = policy_guard_pairs();

    let mut errors: Vec<String> = Vec::new();
    let mut v5_blocked_exempt: Vec<(String, String, String)> = Vec::new();

    for menu in &menus {
        let Some(screen_id) = &menu.screen_id else {
            continue;
        };
        let Some(screen) = by_id.get(screen_id) else {
            errors.push(format!(
                "V1: menu `{}` references unknown screen_id `{}`",
                menu.id, screen_id
            ));
            continue;
        };

        if screen.stage == "delegated" {
            errors.push(format!(
                "V10: menu `{}` references delegated screen `{}`",
                menu.id, screen_id
            ));
        }

        if let Some(min) = &menu.stage_min {
            if stage_order(&screen.stage) < stage_order(min) {
                continue;
            }
        }

        if !menu.waive.contains(&"V2".to_string()) {
            for role in &menu.roles {
                if is_universal(role) {
                    continue;
                }
                if !screen.roles.iter().any(|sr| sr == role) {
                    errors.push(format!(
                        "V2: menu `{}` (roles {:?}) includes role `{}` not in screen `{}` roles {:?}",
                        menu.id, menu.roles, role, screen_id, screen.roles
                    ));
                }
            }
        }

        if !menu.waive.contains(&"V5".to_string()) {
            if let (Some(action), Some(resource)) = (&menu.guard_action, &menu.guard_resource) {
                let covered = guard_pairs.contains(&(action.clone(), resource.clone()));
                if !covered {
                    if screen.stage == "blocked" || screen.stage == "delegated" {
                        v5_blocked_exempt.push((menu.id.clone(), action.clone(), resource.clone()));
                    } else {
                        errors.push(format!(
                            "V5: menu `{}` guard `{} / {}` has no matching guard_policy! record (screen `{}` is `{}`)`",
                            menu.id, action, resource, screen_id, screen.stage
                        ));
                    }
                }
            }
        }
    }

    let mut seen_paths: HashMap<String, String> = HashMap::new();
    for menu in &menus {
        if let Some(path) = &menu.route {
            if let Some(prev) = seen_paths.insert(path.clone(), format!("menu `{}`", menu.id)) {
                errors.push(format!(
                    "V6: route path `{}` declared twice ({prev} and menu `{}`)",
                    path, menu.id
                ));
            }
        }
    }
    for route in &routes {
        if let Some(sid) = &route.screen_id {
            if !by_id.contains_key(sid) {
                errors.push(format!(
                    "V1: route `{}` references unknown screen_id `{}`",
                    route.path.as_deref().unwrap_or("?"),
                    sid
                ));
            }
        }
        if let (Some(action), Some(resource)) = (&route.guard_action, &route.guard_resource) {
            if !guard_pairs.contains(&(action.clone(), resource.clone())) {
                let stage = route
                    .screen_id
                    .as_deref()
                    .and_then(|sid| by_id.get(sid))
                    .map(|s| s.stage.as_str())
                    .unwrap_or("built");
                if stage == "blocked" || stage == "delegated" {
                    v5_blocked_exempt.push((
                        format!("route {}", route.path.as_deref().unwrap_or("?")),
                        action.clone(),
                        resource.clone(),
                    ));
                } else {
                    errors.push(format!(
                        "V5: route `{}` guard `{} / {}` has no matching guard_policy! record (screen `{}` is `{}`)",
                        route.path.as_deref().unwrap_or("?"),
                        action,
                        resource,
                        route.screen_id.as_deref().unwrap_or("?"),
                        stage
                    ));
                }
            }
        }
        if let Some(path) = &route.path {
            if let Some(prev) = seen_paths.insert(path.clone(), format!("route `{}`", path)) {
                errors.push(format!(
                    "V6: route path `{}` declared twice ({prev} and route `{}`)",
                    path, path
                ));
            }
        }
    }

    println!("verify_screen_inventory: {} menus, {} screens", menus.len(), screens.len());
    println!(
        "verify_screen_inventory: {} unique guard pairs covered by guard_policy! records",
        guard_pairs.len()
    );
    if !v5_blocked_exempt.is_empty() {
        println!(
            "verify_screen_inventory: {} menu/route guards on blocked/delegated screens are intentionally not backed by a policy yet (disclosed gaps):",
            v5_blocked_exempt.len()
        );
        for (id, action, resource) in &v5_blocked_exempt {
            println!("  - {}: {} {}", id, action, resource);
        }
    }

    if !errors.is_empty() {
        for e in &errors {
            println!("{}", e);
        }
        panic!(
            "screens.toml / menus.toml invariant violations: {} error(s)",
            errors.len()
        );
    }
}
