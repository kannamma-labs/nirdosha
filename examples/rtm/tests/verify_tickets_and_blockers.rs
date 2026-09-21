//! Two audits made load-bearing in one file:
//!
//! A) **Ticket invariant** — every `ticket:T-XX` referenced anywhere in
//!    `screens.toml` (`blocked_by` arrays and `notes` lines) must resolve to
//!    a section in `tickets.md`. Adding `ticket:T-15` to the inventory
//!    without defining it turns this red.
//!
//! B) **Stale-blocker invariant**: a `dataset:PG.<name>` blocker is *stale*
//!    when `bridge.nir` already declares a real, running `GuardedTable`
//!    whose `GuardedEntity::RESOURCE` is `<name>` — the dataset the blocker
//!    waits for is wired in the demo's own bridge, so the blocker can only
//!    be historical. The test recomputes the stale pairs from the live
//!    corpus (screens.toml + bridge.nir) and pins the expected set in
//!    `EXPECTED_STALE_PAIRS` / `EXPECTED_PROMOTION_CANDIDATES`: when someone
//!    fixes screens.toml, the pinned list must shrink **in the same commit** —
//!    deliberate friction so this audit cannot silently rot.
//!
//! Promotion candidates (stage=blocked screens whose *every* blocker is a
//! stale dataset — deleting the stale entries promotes them outright) are
//! both pinned and printed as a disclosure, same style as
//! `verify_screen_inventory`.
//!
//! Pinned numbers as of this commit (recomputed from the live corpus, not
//! transcribed from any doc): 34 stale pairs across 34 screens; 21 screens
//! whose every blocker is a stale dataset; 17 of those stage=blocked — the
//! pinned promotion-candidate list. The pinned lists below are the source of
//! truth the test enforces; update them deliberately, never silently.

extern crate rtm;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env!("CARGO_MANIFEST_DIR"))
}

fn read(name: &str) -> String {
    let path = manifest_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {} ({}): {}", name, path.display(), e))
}

fn load_toml(name: &str) -> toml::Value {
    let src = read(name);
    src.parse()
        .unwrap_or_else(|e| panic!("could not parse {}: {}", name, e))
}

// ---------------------------------------------------------------------------
// corpus parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Screen {
    id: String,
    name: String,
    stage: String,
    blocked_by: Vec<String>,
}

fn parse_screens(value: &toml::Value) -> Vec<Screen> {
    value
        .get("screen")
        .and_then(|v| v.as_array())
        .expect("screens.toml must have [[screen]] entries")
        .iter()
        .map(|s| {
            Screen {
                id: s.get("id").and_then(|v| v.as_str()).expect("screen missing id").to_string(),
                name: s.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                stage: s.get("stage").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                blocked_by: s
                    .get("blocked_by")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).map(String::from).collect())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// The `GuardedEntity::RESOURCE` consts declared by `src/bridge.nir` — the
/// datasets the demo's own bridge has already wired as real, guard-enforced
/// tables. Parse, not hardcode, so a new table lands here automatically and
/// the pinned stale set below must grow with it (same-commit friction).
fn wired_resources(bridge_src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in bridge_src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("const RESOURCE: &'static str = \"") {
            let name = rest.split('"').next().expect("RESOURCE line shape");
            assert!(!name.is_empty(), "bridge.nir: empty RESOURCE const");
            out.insert(name.to_string());
        }
    }
    assert!(
        out.len() >= 15,
        "bridge.nir: only {} GuardedEntity::RESOURCE consts parsed ({:?}) — \
         the scanner rot or the table fleet was gutted; fix the audit before trusting it",
        out.len(),
        out
    );
    out
}

/// `true` iff `s` is `T-` + exactly two ASCII digits.
fn is_ticket_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 4 && b[0] == b'T' && b[1] == b'-' && b[2].is_ascii_digit() && b[3].is_ascii_digit()
}

/// Distinct `T-xx` tokens anywhere in `text` (blocked_by, notes, comments),
/// with word-boundary checks on both sides.
fn scan_ticket_ids(text: &str) -> BTreeSet<String> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for i in 0..b.len().saturating_sub(3) {
        if b[i] == b'T' && b[i + 1] == b'-' && b[i + 2].is_ascii_digit() && b[i + 3].is_ascii_digit() {
            let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            let after_ok = b
                .get(i + 4)
                .map_or(true, |c| !(c.is_ascii_alphanumeric() || *c == b'_'));
            if before_ok && after_ok {
                out.insert(String::from_utf8_lossy(&b[i..i + 4]).to_string());
            }
        }
    }
    out
}

/// tickets.md section table: id → status (from the `- status:` field).
fn ticket_sections(tickets_md: &str) -> BTreeMap<String, String> {
    let mut sections: Vec<(String, String)> = Vec::new(); // (id, status)
    let mut cur: Option<(String, String)> = None;
    for line in tickets_md.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("## ") {
            // Any `## ` heading closes the current section; only `## T-xx — …`
            // opens a new one (the file's prose headings are not tickets).
            if let Some(done) = cur.take() {
                sections.push(done);
            }
            if let Some((id_part, _)) = rest.split_once('—') {
                let id = id_part.trim();
                if is_ticket_id(id) {
                    cur = Some((id.to_string(), String::new()));
                }
            }
            continue;
        }
        if line == "---" {
            if let Some(done) = cur.take() {
                sections.push(done);
            }
            continue;
        }
        if let Some((id, status)) = cur.as_mut() {
            if let Some(body) = line.strip_prefix("- status:") {
                *status = body.trim().to_string();
            }
            let _ = id;
        }
    }
    if let Some(done) = cur.take() {
        sections.push(done);
    }
    sections.into_iter().collect()
}

/// Stale blocker pairs recomputed from the live corpus: for every screen,
/// every `dataset:PG.<name>` blocker where `<name>` is a wired
/// `GuardedEntity::RESOURCE` in bridge.nir. (Other dataset namespaces —
/// RD.*/AC.*/K.*/… — are never stale under this rule; `refdata`'s RESOURCE
/// name matches no RD.* blocker literally, which is the deliberate,
/// name-literal shape of the rule.)
fn stale_pairs(screens: &[Screen], wired: &BTreeSet<String>) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for s in screens {
        for b in &s.blocked_by {
            if let Some(ds) = b.strip_prefix("dataset:") {
                if let Some(name) = ds.strip_prefix("PG.") {
                    if wired.contains(name) {
                        out.insert((s.id.clone(), ds.to_string()));
                    }
                }
            }
        }
    }
    out
}

/// Promotion candidates: stage=blocked screens whose *every* blocker is a
/// stale dataset blocker — deleting the stale entries promotes them
/// outright. Interim screens that merely still carry stale blockers are
/// already shippable and deliberately excluded.
fn promotion_candidates(screens: &[Screen], wired: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in screens {
        if s.stage != "blocked" || s.blocked_by.is_empty() {
            continue;
        }
        let all_stale = s.blocked_by.iter().all(|b| {
            b.strip_prefix("dataset:")
                .and_then(|ds| ds.strip_prefix("PG."))
                .map_or(false, |name| wired.contains(name))
        });
        if all_stale {
            out.insert(s.id.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// pinned expected sets — the deliberate friction: fixing screens.toml must
// shrink these in the same commit, or the audit goes red.
// ---------------------------------------------------------------------------

/// Every stale `dataset:` blocker pair in the corpus. Sorted; kept exhaustive
/// (not just a count) so a stale blocker removed from screens.toml without a
/// same-commit shrink fails loudly, and a new stale blocker must be
/// acknowledged here by name.
const EXPECTED_STALE_PAIRS: &[(&str, &str)] = &[
    ("10.1", "PG.screening_hit"),
    ("10.2", "PG.screening_hit"),
    ("11.1", "PG.payment"),
    ("11.2", "PG.payment"),
    ("11.3", "PG.payment"),
    ("11.5", "PG.payment"),
    ("12.1", "PG.sar_bundle"),
    ("12.10", "PG.sar_bundle"),
    ("12.6", "PG.sar_bundle"),
    ("12.9", "PG.sar_bundle"),
    ("14.1", "PG.qa_review"),
    ("14.2", "PG.qa_review"),
    ("14.3", "PG.qa_review"),
    ("14.4", "PG.qa_review"),
    ("14.5", "PG.qa_review"),
    ("16.1", "PG.notification"),
    ("16.5", "PG.user_profile"),
    ("18.1", "PG.user_role"),
    ("18.9", "PG.customer"),
    ("2.4", "PG.sar_bundle"),
    ("21.1", "PG.user_role"),
    ("21.2", "PG.sar_bundle"),
    ("21.3", "PG.payment"),
    ("21.4", "PG.customer"),
    ("22.1", "PG.user_profile"),
    ("22.2", "PG.support_ticket"),
    ("4.2", "PG.sar_bundle"),
    ("5.1", "PG.customer"),
    ("5.10", "PG.customer"),
    ("5.2", "PG.customer"),
    ("5.4", "PG.customer"),
    ("5.5", "PG.customer"),
    ("5.6", "PG.customer"),
    ("5.8", "PG.customer"),
];

/// The pinned promotion-candidate list — the actionable face of the audit.
/// When someone fixes screens.toml, this list must shrink in the same
/// commit, or the audit goes red.
const EXPECTED_PROMOTION_CANDIDATES: &[&str] = &[
    "10.1", // Screening Hit Queue — PG.screening_hit
    "10.2", // Hit Review & Disposition — PG.screening_hit
    "11.5", // Hold Outcomes Log & Stats — PG.payment
    "12.1", // SAR Workbench Queue — PG.sar_bundle
    "12.9", // Continuing Activity SAR — PG.sar_bundle
    "14.1", // QA Sampling Queue — PG.qa_review
    "14.2", // QA Scorecard — PG.qa_review
    "14.3", // Analyst Scorecards — PG.qa_review
    "16.1", // Notification Center — PG.notification
    "16.5", // Notification Preferences — PG.user_profile
    "18.1", // User Management — PG.user_role
    "2.4",  // MLRO Dashboard — PG.sar_bundle
    "21.3", // CS Transaction Status Lookup — PG.payment
    "21.4", // RM/Business Customer Status View — PG.customer
    "22.1", // My Profile & Preferences — PG.user_profile
    "22.2", // Help & Support — PG.support_ticket
    "5.5",  // Expected vs Actual Activity — PG.customer
];

fn assert_pinned_set<T: Ord + Clone + std::fmt::Debug>(
    actual: &BTreeSet<T>,
    expected: &BTreeSet<T>,
    what: &str,
    fix_hint: &str,
) {
    let removed: Vec<_> = expected.difference(actual).collect();
    let added: Vec<_> = actual.difference(expected).collect();
    if removed.is_empty() && added.is_empty() {
        return;
    }
    let mut msg = format!("{what} drifted from the pinned expected set.\n");
    if !removed.is_empty() {
        msg.push_str(&format!("  no longer true (shrink the pin in the same commit as the screens.toml fix):\n"));
        for r in &removed {
            msg.push_str(&format!("    - {r:?}\n"));
        }
    }
    if !added.is_empty() {
        msg.push_str("  newly true (acknowledge here by name — do not let the audit rot):\n");
        for a in &added {
            msg.push_str(&format!("    + {a:?}\n"));
        }
    }
    msg.push_str(fix_hint);
    panic!("{}", msg);
}

// ---------------------------------------------------------------------------
// Audit A — ticket invariant
// ---------------------------------------------------------------------------

#[test]
fn every_ticket_reference_resolves_to_a_tickets_md_section() {
    let tickets_md = read("tickets.md");
    let sections = ticket_sections(&tickets_md);
    assert!(
        sections.contains_key("T-01"),
        "tickets.md lost its T-01 section — the legend is the definition site for `ticket:T-xx` refs"
    );

    let screens_src = read("screens.toml");
    let referenced = scan_ticket_ids(&screens_src);
    assert!(
        !referenced.is_empty(),
        "scanner rot: screens.toml parsed to zero ticket references"
    );
    for id in &referenced {
        let status = sections.get(id).unwrap_or_else(|| {
            panic!(
                "dangling ticket reference `{id}` in screens.toml — define a section for it in tickets.md \
                 (status/size/meaning/gates) in the same commit"
            )
        });
        assert_ne!(
            status, "reserved",
            "ticket `{id}` is referenced by screens.toml but marked reserved in tickets.md — promote its section"
        );
    }

    // Structured `ticket:` blockers must be well-formed ids (the register
    // header's `ticket:T-xx` format example lives outside blocked_by and is
    // deliberately not a reference).
    let screens = parse_screens(&load_toml("screens.toml"));
    for s in &screens {
        for b in &s.blocked_by {
            if let Some(id) = b.strip_prefix("ticket:") {
                assert!(
                    is_ticket_id(id),
                    "screens.toml: screen {} has malformed ticket blocker `{b}`",
                    s.id
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Audit B — stale dataset blockers, pinned
// ---------------------------------------------------------------------------

#[test]
fn stale_dataset_blockers_match_the_pinned_set() {
    let screens = parse_screens(&load_toml("screens.toml"));
    let wired = wired_resources(&read("src/bridge.nir"));

    let live = stale_pairs(&screens, &wired);
    let expected_pairs: BTreeSet<(String, String)> = EXPECTED_STALE_PAIRS
        .iter()
        .map(|(s, d)| (s.to_string(), d.to_string()))
        .collect();
    assert_pinned_set(
        &live,
        &expected_pairs,
        "stale dataset:blocker pairs",
        "Rule: `dataset:PG.<name>` is stale iff bridge.nir declares a GuardedTable with RESOURCE == <name>. \
         If you unblocked screens in screens.toml, delete their pairs above in the same commit; \
         if a new stale pair is intentional, add it here with a comment saying why it stays.",
    );

    let promo = promotion_candidates(&screens, &wired);
    let expected_promo: BTreeSet<String> =
        EXPECTED_PROMOTION_CANDIDATES.iter().map(|s| s.to_string()).collect();
    assert_pinned_set(
        &promo,
        &expected_promo,
        "promotion candidates (stage=blocked screens whose every blocker is a stale dataset)",
        "Same rule as the pair pin: shrink in the same commit as the screens.toml fix.",
    );

    // Disclosure — same print style as verify_screen_inventory's disclosed gaps.
    let by_id: BTreeMap<String, &Screen> = screens.iter().map(|s| (s.id.clone(), s)).collect();
    println!(
        "verify_tickets_and_blockers: {} screens, {} wired GuardedTables (bridge.nir), {} stale dataset:blocker pairs, {} promotion candidates",
        screens.len(),
        wired.len(),
        live.len(),
        promo.len()
    );
    for id in &promo {
        let s = by_id[id];
        let stale: Vec<String> = s
            .blocked_by
            .iter()
            .filter(|b| {
                b.strip_prefix("dataset:")
                    .and_then(|ds| ds.strip_prefix("PG."))
                    .map_or(false, |name| wired.contains(name))
            })
            .cloned()
            .collect();
        println!(
            "verify_tickets_and_blockers: promotion candidate {} `{}` — every blocker is a stale dataset {:?} (backed by bridge.nir tables)",
            s.id, s.name, stale
        );
    }
    println!(
        "verify_tickets_and_blockers: deleting the stale blockers above promotes all {} candidates; \
         anything beyond that is a screens.toml+bridge.nir change that must re-pin this test in the same commit",
        promo.len()
    );
}