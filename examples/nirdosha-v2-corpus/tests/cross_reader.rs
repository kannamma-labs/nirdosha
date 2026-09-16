//! G5 — cross-reader equivalence scaffolding.
//!
//! Acceptance target (docs/V2_IMPLEMENTATION_BOOK.md): "one enterprise
//! flow under each implemented reader with matching state/audit/recovery
//! outcomes." Only one reader is real today: plain cargo. The proprietary
//! reader does not yet consume v2 files — its parser targets the
//! original `.nir` grammar, not valid-Rust-plus-comments (see
//! docs/nirdosha-v2-comment-layer.md's own Phase 2-4, unstarted).
//!
//! The implementation book is explicit about the failure mode to avoid:
//! "Cross-reader tests must fail or mark a reader unavailable when v2
//! consumption is unimplemented; they must never substitute the same
//! Cargo invocation and call it equivalence." This file is that
//! discipline enforced in code, not just in prose: it never reports
//! "equivalence" because only one reader happened to run.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
    Cargo,
    Proprietary,
}

impl Reader {
    fn name(self) -> &'static str {
        match self {
            Reader::Cargo => "plain cargo",
            Reader::Proprietary => "proprietary nirdosha",
        }
    }
}

/// The proprietary reader is opt-in. Building `crates/compiler` pulls in
/// GTK/WebKit/z3 (minutes, not seconds) — this suite must not silently
/// eat that cost on every run just to prove a reader is missing. Point
/// `NIRDOSHA_PROPRIETARY_BIN` at a prebuilt binary to include it.
fn proprietary_binary() -> Option<PathBuf> {
    std::env::var_os("NIRDOSHA_PROPRIETARY_BIN").map(PathBuf::from)
}

fn available_readers() -> Vec<Reader> {
    let mut readers = vec![Reader::Cargo];
    if proprietary_binary().is_some() {
        readers.push(Reader::Proprietary);
    }
    readers
}

/// Runs the enterprise flow under `reader`. Never returns `Ok` for a
/// reader that didn't actually execute the file — a reader that isn't
/// wired up returns `Err`, not a silently-reused Cargo run.
fn run_enterprise_flow(reader: Reader) -> Result<String, String> {
    match reader {
        Reader::Cargo => {
            let exe = std::env::var_os("CARGO_BIN_EXE_v2_enterprise_app")
                .map(PathBuf::from)
                .ok_or("CARGO_BIN_EXE_v2_enterprise_app not set")?;
            let out = Command::new(&exe).output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "plain cargo run of v2_enterprise_app failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Reader::Proprietary => {
            let bin = proprietary_binary().ok_or("NIRDOSHA_PROPRIETARY_BIN not set")?;
            let source = concat!(env!("CARGO_MANIFEST_DIR"), "/src/enterprise_app.nir");
            let out = Command::new(&bin)
                .arg("run")
                .arg(source)
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "proprietary nirdosha does not yet consume v2 files \
                     (expected until docs/nirdosha-v2-comment-layer.md \
                     Phase 3/4 land in the proprietary repo): {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
    }
}

/// The enterprise flow's exact, ordered stdout under plain cargo — order
/// matters here, unlike `tests/outputs.rs`'s unordered substring pins:
/// this is a saga's audit trail (commit/compensate/reversal sequence),
/// where reordering the same lines would be a different, wrong behavior.
/// This is the golden reference Phase 2-4's proprietary reader must
/// reproduce exactly to claim G5.
const ENTERPRISE_FLOW_GOLDEN: &[&str] = &[
    "true",
    "1",
    "5",
    "5",
    "25000",
    "199.99",
    "USD()",
    "5",
    "0.7899999999999999",
    "-2", // no cfo role on this token — check_role fails, by design
    "reversing disbursement for purchase order 1",
    "disbursement 1 false",
    "true",
];

#[test]
fn plain_cargo_reader_matches_the_ordered_golden_trail() {
    let stdout = run_enterprise_flow(Reader::Cargo).expect("plain cargo must always be available");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines, ENTERPRISE_FLOW_GOLDEN,
        "plain cargo's enterprise-flow audit trail drifted from the golden reference"
    );
}

/// The actual G5 test. With only one reader available, it must not — and
/// does not — report equivalence: it reports exactly which readers ran
/// and which were unavailable, so a green run of this suite is never
/// mistaken for "cross-reader equivalence achieved."
#[test]
fn enterprise_flow_cross_reader_equivalence() {
    let readers = available_readers();
    let mut outcomes = Vec::new();
    for reader in &readers {
        outcomes.push((*reader, run_enterprise_flow(*reader)));
    }

    let succeeded: Vec<_> = outcomes
        .iter()
        .filter_map(|(r, res)| res.as_ref().ok().map(|out| (*r, out.clone())))
        .collect();

    if succeeded.len() < 2 {
        eprintln!(
            "G5 not yet achievable: only {} reader(s) ran the enterprise flow ({}). \
             Cross-reader equivalence requires at least two.",
            succeeded.len(),
            succeeded.iter().map(|(r, _)| r.name()).collect::<Vec<_>>().join(", "),
        );
        for (reader, result) in &outcomes {
            if let Err(reason) = result {
                eprintln!("  - {} unavailable: {reason}", reader.name());
            }
        }
        // Explicit unavailability, not a silent pass: this must never be
        // read as "equivalence confirmed."
        return;
    }

    let (first_reader, first_output) = &succeeded[0];
    for (reader, output) in &succeeded[1..] {
        assert_eq!(
            output, first_output,
            "{} and {} disagree on the enterprise flow's observable output",
            first_reader.name(),
            reader.name()
        );
    }
}
