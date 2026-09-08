//! Tests for `db_connect`/`db_query`/`db_execute` against a real Postgres
//! server, compiled path (`crates/runtime-kernels/src/kernel/db.rs`,
//! `docs/adr/0005-postgres-pooling-and-tls.md`) — layer 2, on top of
//! layer 1's SQLite-only coverage (`codegen.rs`'s
//! `db_connect_execute_query_round_trips_real_sqlite_rows`).
//!
//! **Recreated, not the original file.** A `crates/compiler/tests/postgres.rs`
//! existed before the tree-walking interpreter was deleted
//! (`05a747c`) — every test in it called `nirdosha::run(&src)` and
//! matched on `nirdosha::interpreter::Value`, both gone entirely. This
//! version keeps that file's real, still-accurate conventions (`#[ignore]`
//! by default, `NIRDOSHA_TEST_POSTGRES_URL` with the same
//! `postgres://postgres@127.0.0.1:5432/postgres` local-dev default,
//! one uniquely-named table per test so parallel `cargo test` threads
//! sharing one live server never collide) but replaces the interpreter
//! call with `codegen::build` + running the real compiled binary —
//! `codegen.rs`'s own `compile_and_run` pattern, duplicated here per
//! this repo's existing per-test-file convention (`codegen.rs`/
//! `strings.rs`/`tcp.rs`/... each keep their own small copy, no shared
//! test-support module exists).
//!
//! Run against a real server with:
//! ```text
//! NIRDOSHA_TEST_POSTGRES_URL=postgres://nirdosha:nirdosha@localhost:5432/nirdosha_dev \
//!     cargo test --release --test postgres -- --ignored
//! ```
//! (`docker-compose.dev.yml` at the repo root stands one up locally.)

use std::process::Command;

use nirdosha::ast::Program;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

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

fn compile_and_run(src: &str) -> (String, i32) {
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_pgtest_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed for this program");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status.code().unwrap_or(-1))
}

fn test_db_url() -> String {
    std::env::var("NIRDOSHA_TEST_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5432/postgres".to_string())
}

/// Every test gets its own table name so parallel `cargo test` threads
/// sharing one live server never collide on `CREATE TABLE`.
fn unique_table(label: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("nirdosha_pg_test_{label}_{n}")
}

#[test]
#[ignore]
fn connect_create_insert_and_query_round_trip() {
    let table = unique_table("basic");
    let src = format!(
        r#"
        struct Text {{
            value: str,
        }}
        fn run_all(conn: db) -> Text {{
            let created: i64 = match db_execute(conn, "CREATE TABLE {table} (id BIGINT PRIMARY KEY, name TEXT)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let inserted: i64 = match db_execute(conn, "INSERT INTO {table} (id, name) VALUES (?, ?)", 1, "ada") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let found: Text = match db_query(conn, "SELECT name FROM {table} WHERE id = ?", 1) {{
                Ok(rows) => match json_array_get(rows, 0) {{
                    Ok(row) => match json_get_str(row, "name") {{
                        Ok(n) => Text(n),
                        Err(e) => Text(e),
                    }},
                    Err(e) => Text(e),
                }},
                Err(e) => Text(e),
            }}
            let dropped: i64 = match db_execute(conn, "DROP TABLE {table}") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            stop conn
            return found
        }}
        fn main() {{
            let name: Text = match db_connect("{url}") {{
                Ok(conn) => run_all(conn),
                Err(e) => Text(e),
            }}
            print(name.value)
        }}
    "#,
        url = test_db_url(),
    );
    let (stdout, code) = compile_and_run(&src);
    assert_eq!(code, 0, "program should exit cleanly, stdout: {stdout}");
    assert_eq!(stdout.trim(), "ada");
}

#[test]
#[ignore]
fn scalar_types_round_trip_through_pg_row_to_json() {
    let table = unique_table("scalars");
    let src = format!(
        r#"
        struct Row {{
            score: f64,
            active: bool,
            note: str,
        }}
        fn run_all(conn: db) -> Row {{
            let created: i64 = match db_execute(conn, "CREATE TABLE {table} (id BIGINT PRIMARY KEY, score DOUBLE PRECISION, active BOOLEAN, note TEXT)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let inserted: i64 = match db_execute(conn, "INSERT INTO {table} (id, score, active, note) VALUES (?, ?, ?, ?)", 1, 3.5, true, "hello") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let out: Row = match db_query(conn, "SELECT score, active, note FROM {table} WHERE id = ?", 1) {{
                Ok(rows) => match json_array_get(rows, 0) {{
                    Ok(r) => match json_get_f64(r, "score") {{
                        Ok(f) => match json_get_bool(r, "active") {{
                            Ok(b) => match json_get_str(r, "note") {{
                                Ok(s) => Row(f, b, s),
                                Err(e) => Row(-1.0, false, e),
                            }},
                            Err(e) => Row(-1.0, false, e),
                        }},
                        Err(e) => Row(-1.0, false, e),
                    }},
                    Err(e) => Row(-1.0, false, e),
                }},
                Err(e) => Row(-1.0, false, e),
            }}
            let dropped: i64 = match db_execute(conn, "DROP TABLE {table}") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            stop conn
            return out
        }}
        fn main() {{
            let out: Row = match db_connect("{url}") {{
                Ok(conn) => run_all(conn),
                Err(e) => Row(-1.0, false, e),
            }}
            print(out.score)
            print(out.active)
            print(out.note)
        }}
    "#,
        url = test_db_url(),
    );
    let (stdout, code) = compile_and_run(&src);
    assert_eq!(code, 0, "program should exit cleanly, stdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    // `print` on an `f64`/`bool` uses this language's own established
    // formatting (`%f`-style, `0`/`1` not `true`/`false` --
    // `codegen.rs`'s own tests document this same convention), not a
    // Postgres-specific quirk.
    assert_eq!(lines, vec!["3.500000", "1", "hello"]);
}

/// A real, disclosed integer-width mismatch this repo hit and fixed
/// while building this phase: binding an `i64` into an `INTEGER`
/// (`int4`) column used to be a real wire-format error, not a
/// hypothetical one — `PgBindValue::to_sql`
/// (`crates/runtime-kernels/src/kernel/db.rs`) now encodes according to
/// the server's own reported parameter type. This test is the
/// regression guard for exactly that.
#[test]
#[ignore]
fn integer_column_narrower_than_i64_binds_and_reads_back_correctly() {
    let table = unique_table("intwidth");
    let src = format!(
        r#"
        struct Row {{
            rating: i64,
        }}
        fn run_all(conn: db) -> Row {{
            let created: i64 = match db_execute(conn, "CREATE TABLE {table} (id INTEGER PRIMARY KEY, rating INTEGER)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let inserted: i64 = match db_execute(conn, "INSERT INTO {table} (id, rating) VALUES (?, ?)", 1, 5) {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let out: Row = match db_query(conn, "SELECT rating FROM {table} WHERE id = ?", 1) {{
                Ok(rows) => match json_array_get(rows, 0) {{
                    Ok(r) => match json_get_i64(r, "rating") {{
                        Ok(n) => Row(n),
                        Err(e) => Row(-1),
                    }},
                    Err(e) => Row(-1),
                }},
                Err(e) => Row(-1),
            }}
            let dropped: i64 = match db_execute(conn, "DROP TABLE {table}") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            stop conn
            return out
        }}
        fn main() {{
            let out: Row = match db_connect("{url}") {{
                Ok(conn) => run_all(conn),
                Err(e) => Row(-1),
            }}
            print(out.rating)
        }}
    "#,
        url = test_db_url(),
    );
    let (stdout, code) = compile_and_run(&src);
    assert_eq!(code, 0, "program should exit cleanly, stdout: {stdout}");
    assert_eq!(stdout.trim(), "5");
}
