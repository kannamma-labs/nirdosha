//! End-to-end verification for `examples/features/55_nirdosha_ops_console.nir`
//! (Phase 7, "build everything for real" — see that file's own header
//! comment for the honest scope: verified by running the compiled
//! binary directly, not by serving it and clicking through a browser).
//!
//! Real infrastructure throughout, no mocks except the one thing this
//! environment genuinely can't provide (a real payment gateway): a
//! plain `TcpListener` standing in for the webhook `disburse`'s
//! `network` slot calls, on the file's own fixed `127.0.0.1:18099`.
//! Postgres is real (`docker-compose.dev.yml`'s local dev instance, or
//! whatever's already listening on `127.0.0.1:5432` — this file's own
//! hardcoded connection string, same default `postgres.rs::test_db_url`
//! uses, but *not* itself overridable via `NIRDOSHA_TEST_POSTGRES_URL`
//! since the `.nir` source embeds it as a literal — a real, disclosed
//! limitation of a self-contained demo binary, not an oversight).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;

use nirdosha::ast::Program;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

const SRC: &str = include_str!("../../../examples/features/55_nirdosha_ops_console.nir");

fn parse_checked(src: &str) -> Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    program
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn unique_temp_path(label: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_opsconsole_{label}_{}_{}", std::process::id(), unique_suffix()));
    p
}

fn build_binary(src: &str) -> std::path::PathBuf {
    let program = parse_checked(src);
    let report = analyze(&program);
    let out_path = unique_temp_path("bin");
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed for this program");
    out_path
}

/// A minimal, real webhook stand-in — one thread, accepts forever
/// (dies with the test process), answers every request `200 OK` with a
/// tiny fixed body. Real bytes over a real socket, `kernel::http`'s own
/// pooled client on the other end — not a mock of the HTTP layer
/// itself, only of the payment gateway this environment doesn't have.
fn start_mock_webhook(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("mock webhook should bind its fixed port");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf); // don't care about the request itself
                let _ = handle_one(&mut stream);
            });
        }
    });
}

fn handle_one(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")?;
    stream.flush()
}

fn wait_for_port(port: u16) {
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("mock webhook on port {port} never became reachable");
}

/// Real, but requires a real local Postgres at `postgres://postgres@127.0.0.1:5432/postgres`
/// (`docker-compose.dev.yml`) — `#[ignore]`-gated, same convention
/// `postgres.rs` already established, so a clean checkout/CI (no
/// Postgres service container, `docs/ROADMAP.md`'s own note on why)
/// never needs one. Run locally with:
/// `cargo test --release --test nirdosha_ops_console -- --ignored`
#[test]
#[ignore]
fn nirdosha_ops_console_runs_the_full_scenario_against_real_postgres_and_http() {
    start_mock_webhook(18099);
    wait_for_port(18099);

    let bin = build_binary(SRC);
    let output = Command::new(&bin).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stdout was:\n{stdout}\nstderr was:\n{stderr}");

    let lines: Vec<&str> = stdout.lines().filter(|l| !l.starts_with(' ') && *l != "nirdosha kernel flight recorder:").collect();
    assert_eq!(
        lines,
        vec![
            "1",     // is_admin(admin_identity())
            "0",     // is_finance(admin_identity())
            "0",     // is_admin(finance_identity())
            "1",     // is_finance(finance_identity())
            "1",     // create_as_finance_is_denied() -- real RBAC denial, not just a role-string check
            "Acme Robotics",
            "Globex Supplies",
            "1", // stat_purchase_order_count() >= 2
            "1", // list_purchase_order()'s row count >= 2
            "1", // instance_id > 0
            // `disburse_on_approval`'s own `print("disbursement", instance_id, ok)`
            // -- a real workflow `on_entry` action ran, printed here as
            // three separate lines (this language's `print(a, b, c)`
            // prints each argument on its own line, not space-joined).
            "disbursement",
            "1", // instance_id
            "1", // ok (disburse succeeded)
            "1", // advance_disbursement_now(..., Approve()) itself returned true
        ],
        "unexpected stdout:\n{stdout}"
    );

    // Real admission telemetry: `db`/`http` both show real grants (5
    // db_connects across create/list/count/disburse's own commit, 1
    // real http_post), and the new `Domain::ServeHttp` line is present
    // (held at zero -- nothing served yet, `crates/compiled-serve`
    // isn't wired to this binary -- but the domain itself is real).
    // `nir_kernel_flight_recorder_dump` prints to stderr, not stdout
    // (`runtime-kernels/src/lib.rs`'s own doc comment: "always visible
    // after a run without needing a file to manage").
    assert!(stderr.contains("db: held=0 grants=5"), "stderr was:\n{stderr}");
    assert!(stderr.contains("http: held=0 grants=1"), "stderr was:\n{stderr}");
    assert!(stderr.contains("serve_http: held=0 grants=0"), "stderr was:\n{stderr}");
}

/// The mid-transact-crash-and-replay scenario, run for real against the
/// actual demo app -- the general mechanism (bounded retry, a durable
/// `commit_pending` row on exhaustion, a compiler-synthesized replay
/// trampoline dispatching it from a *later* process's own `main`
/// prologue) is already deterministically proven by
/// `crates/compiler/tests/codegen.rs`'s
/// `transact_replay_finishes_a_commit_pending_row_left_by_a_prior_process`
/// against the exact same codegen path this file's own `disburse`
/// `transact` block compiles through -- this test's own job is to prove
/// *this specific program's* disbursement survives a real crash, not to
/// re-derive `transact`'s own general durability proof a second time.
///
/// Two real, separately-compiled binaries, not two processes of the
/// same one: a "bad" variant whose `record_disbursement` connects to an
/// unreachable Postgres port (`127.0.0.1:1`, connection-refused
/// immediately) -- everything else in the program, including every
/// *other* `db_connect` call site, is untouched and still points at the
/// real local server, so `main`'s own CRUD/workflow steps before
/// `disburse`'s own `commit` still run for real. Run first, its
/// `commit` exhausts its retry budget against the bad port and leaves a
/// real `commit_pending` row. Run second: the real, unmodified binary,
/// against the *same* durability log -- its own `main` prologue replays
/// that row before its own `main` body ever executes, this time
/// succeeding (the real port, real credentials), proving replay across
/// a genuine binary-version change, not just a re-run of the identical
/// process.
#[test]
#[ignore]
fn nirdosha_ops_console_disbursement_survives_a_crash_and_replays_on_a_later_binary() {
    start_mock_webhook(18100);
    wait_for_port(18100);

    // Same program, `notify_webhook`'s port bumped to this test's own
    // (`18100`, distinct from the other test's `18099` so the two can
    // run concurrently) and `record_disbursement`'s own connect target
    // swapped to an unreachable port -- this exact multi-line anchor is
    // unique in the file, so every *other* `db_connect("postgres://postgres@127.0.0.1:5432/postgres")`
    // call site (create/list/count) is untouched and still hits the
    // real server.
    let src_this_test = SRC.replace("18099", "18100");
    let good_src = src_this_test.clone();
    let bad_anchor = "fn record_disbursement(txn_id: str, po_id: i64, amount_cents: i64) -> Result(i64, ErrMsg) {\n    return match db_connect(\"postgres://postgres@127.0.0.1:5432/postgres\") {";
    assert_eq!(src_this_test.matches(bad_anchor).count(), 1, "the anchor must be unique in the source");
    let bad_replacement = "fn record_disbursement(txn_id: str, po_id: i64, amount_cents: i64) -> Result(i64, ErrMsg) {\n    return match db_connect(\"postgres://postgres@127.0.0.1:1/postgres\") {";
    let bad_src = src_this_test.replace(bad_anchor, bad_replacement);
    assert_ne!(bad_src, good_src, "the bad variant must actually differ from the good one");

    let log_path = unique_temp_path("crash_replay_log");
    let envs = [("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())];

    let bad_bin = build_binary(&bad_src);
    let run1 = Command::new(&bad_bin).envs(envs).output().expect("first run should execute");
    let _ = std::fs::remove_file(&bad_bin);
    assert_eq!(run1.status.code(), Some(0), "stderr was:\n{}", String::from_utf8_lossy(&run1.stderr));
    {
        let log_conn = rusqlite::Connection::open(&log_path).expect("durability log should be openable");
        let pending: i64 = log_conn.query_row("SELECT COUNT(*) FROM nirdosha_transact_log WHERE state = 'commit_pending'", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 1, "the first run's disbursement commit must exhaust its retry budget against the bad port and leave exactly one row pending");
    }
    let disbursements_before: i64 = pg_count("disbursements");

    let good_bin = build_binary(&good_src);
    let run2 = Command::new(&good_bin).envs(envs).output().expect("second run should execute");
    let _ = std::fs::remove_file(&good_bin);
    assert_eq!(run2.status.code(), Some(0), "stderr was:\n{}", String::from_utf8_lossy(&run2.stderr));

    let log_conn = rusqlite::Connection::open(&log_path).expect("durability log should be openable");
    let pending: i64 = log_conn.query_row("SELECT COUNT(*) FROM nirdosha_transact_log WHERE state = 'commit_pending'", [], |r| r.get(0)).unwrap();
    assert_eq!(pending, 0, "the second (real) binary's own `main` prologue must replay the row the first run left pending");

    // Two new disbursement rows landed for real: the one replay just
    // finished (from run 1) plus run 2's own fresh, independent
    // disbursement (its own `main` runs the whole scenario again).
    let disbursements_after: i64 = pg_count("disbursements");
    assert_eq!(disbursements_after, disbursements_before + 2, "replay must have inserted the stuck row's own disbursement, not silently dropped it");

    let _ = std::fs::remove_file(&log_path);
}

fn pg_count(table: &str) -> i64 {
    let mut client = postgres::Client::connect("postgres://postgres@127.0.0.1:5432/postgres", postgres::NoTls).expect("should connect to the real local Postgres");
    let row = client.query_one(&format!("SELECT COUNT(*) FROM {table}"), &[]).expect("count query should succeed");
    row.get(0)
}

/// `nirdosha emit-ui` against the same program produces a real
/// `LANDING` data table — "each role lands on their own screen" is
/// checked, real *data* a served bundle would use, even without a live
/// browser click-through (this file's own header comment has the full
/// disclosure on why that's the honest bar this test can actually
/// clear today). No Postgres/HTTP needed -- `emit-ui` never runs the
/// program, only reads its declarations -- so this test is *not*
/// `#[ignore]`-gated.
#[test]
fn nirdosha_ops_console_emit_ui_produces_a_real_per_role_landing_table() {
    let program = {
        let toks = Lexer::new(SRC).tokenize().expect("lex should succeed");
        let program = Parser::new(toks).parse_program().expect("parse should succeed");
        nirdosha::typeck::typecheck_optional_main(&program).expect("should typecheck cleanly");
        program
    };
    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let effects = nirdosha::effects::infer_effects(&program, &registry);
    let bundle = nirdosha::ui_gen::generate(&program, &effects, None, false, false, false, None);

    let marker = "const LANDING = ";
    let start = bundle.find(marker).expect("bundle should contain the LANDING table") + marker.len();
    let end = bundle[start..].find(';').expect("LANDING assignment should be terminated") + start;
    let landing_json = &bundle[start..end];

    let parsed: serde_json::Value = serde_json::from_str(landing_json).expect("LANDING should be valid JSON");
    let rules = parsed.as_array().expect("LANDING should be a JSON array");
    assert_eq!(rules.len(), 3, "landing_json was: {landing_json}");
    assert_eq!(rules[0]["role"], "admin");
    assert_eq!(rules[0]["target"], "admin_home");
    assert_eq!(rules[1]["role"], "finance");
    assert_eq!(rules[1]["target"], "finance_queue");
    assert_eq!(rules[2]["default"], true);
    assert_eq!(rules[2]["target"], "home_screen");
}
