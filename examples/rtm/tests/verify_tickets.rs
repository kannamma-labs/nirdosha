//! Load-bearing verification for `tickets.md` — the ticket legend that
//! defines every `ticket:T-xx` referenced by `screens.toml` (in `blocked_by`
//! arrays and `notes` lines) and by `menus.toml` (in comments and route notes).
//!
//! Before `tickets.md` existed, the 44 `blocked_by` lines carrying
//! `ticket:T-xx` refs (52 occurrences, 9 distinct tickets) plus the bare
//! `T-xx` mentions in notes/menus pointed at nothing. This test pins the
//! legend to the corpus so the references stay resolvable:
//!
//!   R1  every `T-xx` token appearing anywhere in screens.toml/menus.toml
//!       resolves to a defined, non-reserved ticket section in tickets.md
//!   R2  tickets.md defines exactly T-01..T-14 — contiguous, no duplicates;
//!       every section carries status/size/meaning; screen-id lists are
//!       well-formed and point at real screens with the right evidence
//!   R3  each ticket's `blocked-screens:` set is EXACTLY the set of screens
//!       whose `blocked_by` carries `ticket:<id>` (both directions)
//!   R4  each ticket's `note-mentions:` set is EXACTLY the set of screens
//!       whose `notes` cite the bare ticket id (both directions)
//!   R5  reserved slots (T-05, T-13) carry zero references
//!   R6  the legend's corpus-facts paragraph matches live counts (52 refs on
//!       44 blocked_by lines, 9 stage-gating + 3 note-only + 2 reserved)
//!
//! tickets.md is parsed under a small machine contract documented in its
//! header: `## T-xx — <title>` sections with `- key: value` fields
//! (status/size/meaning/blocked-screens/note-mentions).

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

// ---------------------------------------------------------------------------
// tickets.md parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Ticket {
    id: String,
    title: String,
    status: String,
    size: String,
    meaning: String,
    blocked_screens: Vec<String>,
    note_mentions: Vec<String>,
}

/// `true` iff `s` is `T-` + exactly two ASCII digits.
fn is_ticket_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 4 && b[0] == b'T' && b[1] == b'-' && b[2].is_ascii_digit() && b[3].is_ascii_digit()
}

fn parse_screen_id_list(value: &str, what: &str, ticket: &str) -> Vec<String> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for tok in v.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let ok = {
            let mut parts = tok.split('.');
            let a = parts.next().unwrap_or("");
            let b = parts.next().unwrap_or("");
            parts.next().is_none()
                && !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        };
        assert!(ok, "tickets.md: `{ticket}` has malformed screen id `{tok}` in `{what}` (expected N.N)");
        out.push(tok.to_string());
    }
    out
}

fn parse_tickets_md() -> Vec<Ticket> {
    let src = read("tickets.md");
    let mut tickets: Vec<Ticket> = Vec::new();
    let mut cur: Option<Ticket> = None;
    for line in src.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("## ") {
            // Close current section on ANY heading; open one if it is `## T-xx — ...`.
            if let Some(t) = cur.take() {
                tickets.push(t);
            }
            if let Some(id) = rest.split("—").next().map(str::trim) {
                if is_ticket_id(id) {
                    let title = rest
                        .split_once('—')
                        .map(|(_, t)| t.trim().to_string())
                        .unwrap_or_default();
                    assert!(!title.is_empty(), "tickets.md: `{id}` section has no title");
                    cur = Some(Ticket { id: id.to_string(), title, ..Default::default() });
                }
            }
            continue;
        }
        if line == "---" {
            if let Some(t) = cur.take() {
                tickets.push(t);
            }
            continue;
        }
        let Some(t) = cur.as_mut() else { continue };
        let Some(body) = line.strip_prefix("- ") else { continue };
        let Some((key, value)) = body.split_once(':') else { continue };
        let value = value.trim();
        match key.trim() {
            "status" => t.status = value.to_string(),
            "size" => t.size = value.to_string(),
            "meaning" => t.meaning = value.to_string(),
            "blocked-screens" => t.blocked_screens = parse_screen_id_list(value, "blocked-screens", &t.id),
            "note-mentions" => t.note_mentions = parse_screen_id_list(value, "note-mentions", &t.id),
            _ => {}
        }
    }
    if let Some(t) = cur.take() {
        tickets.push(t);
    }
    assert!(!tickets.is_empty(), "tickets.md parsed to zero ticket sections — header intact?");
    tickets
}

// ---------------------------------------------------------------------------
// corpus scanning
// ---------------------------------------------------------------------------

/// Distinct `T-xx` tokens anywhere in `text` (comment, note, or value),
/// with word-boundary checks on both sides.
fn scan_ticket_ids(text: &str) -> BTreeSet<String> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for i in 0..b.len().saturating_sub(3) {
        if b[i] == b'T' && b[i + 1] == b'-' && b[i + 2].is_ascii_digit() && b[i + 3].is_ascii_digit() {
            let before_ok = i == 0
                || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
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

/// Occurrences of a real structured ref (`ticket:T-` + two digits). The
/// register's header carries a format example (`ticket:T-xx`) that is
/// documentation, not a reference — the two-digit filter excludes it.
fn count_structured_refs(text: &str) -> usize {
    let b = text.as_bytes();
    let mut n = 0;
    for i in 0..b.len().saturating_sub(10) {
        if &b[i..i + 9] == b"ticket:T-" && b[i + 9].is_ascii_digit() && b[i + 10].is_ascii_digit() {
            let after_ok = b.get(i + 11).map_or(true, |c| !(c.is_ascii_alphanumeric() || *c == b'_'));
            if after_ok {
                n += 1;
            }
        }
    }
    n
}

/// Lines carrying at least one real structured ref.
fn count_structured_ref_lines(text: &str) -> usize {
    text.lines().filter(|l| count_structured_refs(l) > 0).count()
}

#[derive(Debug, Clone)]
struct Screen {
    id: String,
    stage: String,
    blocked_by: Vec<String>,
    notes: String,
}

fn parse_screens() -> Vec<Screen> {
    let value: toml::Value = read("screens.toml")
        .parse()
        .expect("screens.toml must parse");
    value
        .get("screen")
        .and_then(|v| v.as_array())
        .expect("screens.toml must have [[screen]] entries")
        .iter()
        .map(|s| {
            let id = s.get("id").and_then(|v| v.as_str()).expect("screen missing id").to_string();
            let stage = s.get("stage").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let blocked_by = s
                .get("blocked_by")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).map(String::from).collect())
                .unwrap_or_default();
            let notes = s.get("notes").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Screen { id, stage, blocked_by, notes }
        })
        .collect()
}

fn tickets_by_id<'a>(tickets: &'a [Ticket]) -> BTreeMap<&'a str, &'a Ticket> {
    tickets.iter().map(|t| (t.id.as_str(), t)).collect()
}

fn blocked_by_tickets(screens: &[Screen]) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in screens {
        for b in &s.blocked_by {
            if let Some(t) = b.strip_prefix("ticket:") {
                assert!(is_ticket_id(t), "screens.toml: screen {} has malformed blocker `{b}`", s.id);
                map.entry(t.to_string()).or_default().insert(s.id.clone());
            }
        }
    }
    map
}

fn note_mentions(screens: &[Screen]) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in screens {
        for t in scan_ticket_ids(&s.notes) {
            map.entry(t).or_default().insert(s.id.clone());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// R1: every corpus reference resolves
// ---------------------------------------------------------------------------

#[test]
fn every_ticket_reference_resolves_to_a_defined_active_ticket() {
    let legend = parse_tickets_md();
    let tickets = tickets_by_id(&legend);
    let screens_src = read("screens.toml");
    let menus_src = read("menus.toml");

    let mut checked = 0;
    for id in scan_ticket_ids(&screens_src).union(&scan_ticket_ids(&menus_src)).cloned().collect::<BTreeSet<_>>() {
        let t = tickets.get(id.as_str()).unwrap_or_else(|| {
            panic!("dangling ticket reference `{id}` in screens.toml/menus.toml — define it in tickets.md");
        });
        assert_ne!(t.status, "reserved", "reserved slot `{id}` is referenced by the corpus");
        checked += 1;
    }
    // The 11 referenced slots: T-01..T-04, T-06..T-12 (T-05/T-13 reserved;
    // T-14 closed its only gate and is no longer referenced anywhere).
    assert_eq!(checked, 11, "expected exactly 11 distinct referenced ticket ids");

    // Every structured blocker in a parsed `blocked_by` array is well-formed
    // (`ticket:T-xx`). The register header's format example (`ticket:T-xx`)
    // lives outside blocked_by and is deliberately not a reference.
    for s in parse_screens() {
        for b in &s.blocked_by {
            if let Some(id) = b.strip_prefix("ticket:") {
                assert!(is_ticket_id(id), "screens.toml: screen {} has malformed ticket blocker `{b}`", s.id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// R2: legend is well-formed and internally sound
// ---------------------------------------------------------------------------

#[test]
fn legend_defines_exactly_t01_through_t14_with_required_fields() {
    let tickets = parse_tickets_md();
    let by_id = tickets_by_id(&tickets);

    // Contiguous T-01..T-14, no duplicates.
    let ids: BTreeSet<String> = tickets.iter().map(|t| t.id.clone()).collect();
    assert_eq!(tickets.len(), 14, "tickets.md must define exactly 14 sections");
    let expected: BTreeSet<String> = (1..=14).map(|i| format!("T-{i:02}")).collect();
    assert_eq!(ids, expected, "tickets.md ids must be exactly T-01..T-14");
    assert_eq!(by_id.len(), 14, "duplicate ticket sections in tickets.md");

    for t in &tickets {
        assert!(
            t.status == "active" || t.status == "reserved",
            "tickets.md: `{}` has invalid status `{}`", t.id, t.status
        );
        assert!(!t.meaning.is_empty(), "tickets.md: `{}` missing meaning", t.id);
        assert!(
            t.meaning.chars().count() <= 140,
            "tickets.md: `{}` meaning should stay a single line ({} chars)", t.id, t.meaning.chars().count()
        );
        if t.status == "reserved" {
            assert_eq!(t.size, "unassigned", "tickets.md: reserved `{}` must be size unassigned", t.id);
            assert!(t.blocked_screens.is_empty() && t.note_mentions.is_empty(),
                "tickets.md: reserved `{}` must gate nothing", t.id);
        } else {
            assert!(["S", "M", "L"].contains(&t.size.as_str()),
                "tickets.md: `{}` has invalid size `{}`", t.id, t.size);
        }
    }
}

#[test]
fn legend_screen_ids_exist_and_carry_the_right_evidence() {
    let tickets = parse_tickets_md();
    let by_id = tickets_by_id(&tickets);
    let screens = parse_screens();
    let screen_ids: BTreeSet<String> = screens.iter().map(|s| s.id.clone()).collect();

    for t in &tickets {
        for sid in &t.blocked_screens {
            let s = screens.iter().find(|s| &s.id == sid).unwrap_or_else(|| {
                panic!("tickets.md: `{}` blocked-screens lists nonexistent screen `{sid}`", t.id)
            });
            assert!(
                s.blocked_by.iter().any(|b| b == &format!("ticket:{}", t.id)),
                "tickets.md: `{}` lists screen `{sid}` as blocked, but screens.toml `blocked_by` does not carry ticket:{}",
                t.id, t.id
            );
        }
        for sid in &t.note_mentions {
            let s = screens.iter().find(|s| &s.id == sid).unwrap_or_else(|| {
                panic!("tickets.md: `{}` note-mentions lists nonexistent screen `{sid}`", t.id)
            });
            assert!(
                scan_ticket_ids(&s.notes).contains(t.id.as_str()),
                "tickets.md: `{}` note-mentions screen `{sid}`, but its screens.toml `notes` never cites {}",
                t.id, t.id
            );
        }
        // Guard against copy-paste slips between the two fields.
        assert!(
            t.blocked_screens.iter().all(|s| !t.note_mentions.contains(s) || by_id[t.id.as_str()].status == "active"),
            "tickets.md: `{}` — a screen may appear in both fields only when active (documented gate vs note)",
            t.id
        );
    }
    let _ = screen_ids;
}

// ---------------------------------------------------------------------------
// R3: blocked-screens sets are exactly the screens.toml reality
// ---------------------------------------------------------------------------

#[test]
fn blocked_screens_match_screens_toml_blocked_by_exactly() {
    let tickets = parse_tickets_md();
    let screens = parse_screens();

    let from_toml = blocked_by_tickets(&screens);
    let mut from_legend: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in &tickets {
        if t.status != "active" || t.blocked_screens.is_empty() {
            continue;
        }
        from_legend.insert(t.id.clone(), t.blocked_screens.iter().cloned().collect());
    }

    assert_eq!(from_legend, from_toml,
        "tickets.md blocked-screens sets must equal screens.toml `ticket:` blocked_by sets (both directions)");

    // The register deliberately allows built/interim/emittable screens to
    // still carry ticket blockers (full-fidelity gates on shipped screens —
    // e.g. 3.3 Disposition Panel is built, its T-03 vocabulary is pending),
    // so no stage rule is asserted here beyond the set equality above.
}

// ---------------------------------------------------------------------------
// R4: note-mentions sets are exactly the screens.toml reality
// ---------------------------------------------------------------------------

#[test]
fn note_mentions_match_screens_toml_notes_exactly() {
    let tickets = parse_tickets_md();
    let screens = parse_screens();

    let from_toml = note_mentions(&screens);
    let mut from_legend: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in &tickets {
        if t.status != "active" || t.note_mentions.is_empty() {
            continue;
        }
        from_legend.insert(t.id.clone(), t.note_mentions.iter().cloned().collect());
    }

    assert_eq!(from_legend, from_toml,
        "tickets.md note-mentions must equal the set of screens whose `notes` cite each ticket (both directions)");
}

// ---------------------------------------------------------------------------
// R5: reserved slots stay unreferenced
// ---------------------------------------------------------------------------

#[test]
fn reserved_slots_carry_zero_references() {
    let tickets = parse_tickets_md();
    let screens_src = read("screens.toml");
    let menus_src = read("menus.toml");

    let corpus: BTreeSet<String> = scan_ticket_ids(&screens_src)
        .union(&scan_ticket_ids(&menus_src))
        .cloned()
        .collect();

    for t in tickets.iter().filter(|t| t.status == "reserved") {
        assert!(
            !corpus.contains(t.id.as_str()),
            "tickets.md: `{}` is marked reserved but is now referenced in screens.toml/menus.toml — promote its section (fill status/size/meaning/gates in the same commit)",
            t.id
        );
    }
}

// ---------------------------------------------------------------------------
// R6: the legend's corpus-facts paragraph matches live counts
// ---------------------------------------------------------------------------

#[test]
fn legend_corpus_facts_match_live_counts() {
    let tickets = parse_tickets_md();
    let screens = parse_screens();
    let screens_src = read("screens.toml");

    // total_screens metadata and actual [[screen]] count agree at 152.
    let value: toml::Value = screens_src.parse().expect("screens.toml must parse");
    let total = value.get("metadata").and_then(|m| m.get("total_screens")).and_then(|v| v.as_integer());
    assert_eq!(total, Some(152), "screens.toml metadata.total_screens drifted");
    assert_eq!(screens.len(), 152, "screens.toml holds {} screens, not 152", screens.len());

    // 39 `ticket:T-…` refs on 33 blocked_by lines (the header's `ticket:T-xx`
    // format example is documentation and excluded by the two-digit filter).
    let occurrences = count_structured_refs(&screens_src);
    let lines = count_structured_ref_lines(&screens_src);
    assert_eq!(occurrences, 39, "screens.toml now carries {occurrences} ticket refs (legend says 39)");
    assert_eq!(lines, 33, "screens.toml now has {lines} ticket-bearing blocked_by lines (legend says 33)");

    // 6 stage-gating tickets, 5 note-only, 2 reserved — and the legend's
    // corpus-facts paragraph still names exactly those sets. (T-03 closed
    // out its stage-gating blockers on 3.3/4.1/4.10/11.2 — its remaining
    // trace is 7.1's historical note-mention, so it moved to note-only.
    // T-14 closed 4.1's drag-graying blocker — its only gate — and dropped
    // out of both sets entirely. T-01 (B7) closed all 5 of its
    // stage-gating blockers — its remaining trace is menus.toml's
    // nav.exports/sar-export route notes, so it moved to note-only too.)
    let blocked = blocked_by_tickets(&screens);
    let stage_gating: Vec<&String> = blocked.keys().collect();
    assert_eq!(stage_gating.len(), 6, "stage-gating tickets drifted: {stage_gating:?}");

    let active = tickets.iter().filter(|t| t.status == "active").count();
    let reserved = tickets.iter().filter(|t| t.status == "reserved").count();
    assert_eq!(active, 12);
    assert_eq!(reserved, 2);
    let referenced: BTreeSet<String> = blocked.keys().cloned()
        .chain(scan_ticket_ids(&screens_src))
        .chain(scan_ticket_ids(&read("menus.toml")))
        .collect();
    assert_eq!(referenced.len(), 11, "referenced ticket ids drifted: {referenced:?}");
    let note_only: Vec<String> = referenced.iter().filter(|t| !blocked.contains_key(*t)).cloned().collect();
    assert_eq!(note_only, vec!["T-01", "T-02", "T-03", "T-10", "T-12"], "note-only tickets drifted");

    let src = read("tickets.md");
    let facts = src
        .split("**Corpus facts")
        .nth(1)
        .expect("tickets.md lost its `**Corpus facts**` paragraph")
        .split("---")
        .next()
        .unwrap();
    for token in ["39", "33", "152", "11 of the 14", "6 stage-gating", "T-01, T-02, T-03, T-10, T-12", "T-05 and T-13"] {
        assert!(facts.contains(token), "tickets.md corpus-facts paragraph no longer states `{token}`");
    }
    for tid in stage_gating {
        assert!(facts.contains(tid.as_str()), "corpus-facts paragraph omits stage-gating ticket {tid}");
    }
}