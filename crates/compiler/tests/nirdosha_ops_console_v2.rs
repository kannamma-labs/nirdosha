//! End-to-end verification for
//! `examples/features/57_nirdosha_ops_console_server_v2.nir` — the
//! strengthened companion to `56_nirdosha_ops_console_server.nir`: real
//! signed-JWT auth (`mock_issue_token`/`oidc_validate_token`, not a
//! forgeable header), the `Disbursement` workflow wired to a real route,
//! and UPDATE/DELETE/search/pagination. Same "real client sockets
//! against a real long-running compiled server process" pattern
//! `nirdosha_ops_console.rs`'s own server test already established, plus
//! a real mock-webhook listener (that file's own pattern too) since this
//! file's `approve` route really calls one.

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

const SRC: &str = include_str!("../../../examples/features/57_nirdosha_ops_console_server_v2.nir");

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
    p.push(format!("nirdosha_opsconsole_v2_{label}_{}_{}", std::process::id(), unique_suffix()));
    p
}

/// Same real webhook stand-in `nirdosha_ops_console.rs::start_mock_webhook`
/// uses, on this file's own fixed port (`18101`, distinct from `55_...`'s
/// `18099` so both test files can run in the same `cargo test` process).
fn start_mock_webhook(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("mock webhook should bind its fixed port");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                let _ = stream.flush();
            });
        }
    });
}

/// Real, but requires a real local Postgres at
/// `postgres://postgres@127.0.0.1:5432/postgres` -- `#[ignore]`-gated,
/// same convention `nirdosha_ops_console.rs` already established. Run
/// locally with:
/// `cargo test --release --test nirdosha_ops_console_v2 -- --ignored`
#[test]
#[ignore]
fn nirdosha_ops_console_v2_answers_real_http_requests_with_real_auth_workflow_and_crud() {
    start_mock_webhook(18101);

    let program = parse_checked(SRC);
    let report = analyze(&program);
    let bin = unique_temp_path("server_bin");
    codegen::build(&program, &report, &bin, codegen::OptLevel::O2).expect("codegen::build should succeed for this program");
    let mut child = Command::new(&bin).spawn().expect("compiled server should start");

    let request = |method: &str, path: &str, headers: &str, body: &str| -> String {
        let mut attempt = 0;
        let mut conn = loop {
            match TcpStream::connect(("127.0.0.1", 8091)) {
                Ok(s) => break s,
                Err(_) if attempt < 100 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("could not connect to the compiled server: {e}"),
            }
        };
        let req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len());
        conn.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    };

    let json_body = |response: &str| -> serde_json::Value {
        let sep = response.find("\r\n\r\n").expect("response should have a header/body separator");
        serde_json::from_str(&response[sep + 4..]).unwrap_or_else(|e| panic!("response body should be valid JSON: {e}\nresponse: {response}"))
    };

    // Real signed-JWT login, both roles.
    let admin_login = request("POST", "/api/login", "", "{\"role\":\"admin\"}");
    let admin_token = json_body(&admin_login)["value"].as_str().expect("admin login should return a real token").to_string();
    assert!(admin_token.split('.').count() == 3, "should look like a real JWT (header.payload.signature): {admin_token}");

    let finance_login = request("POST", "/api/login", "", "{\"role\":\"finance\"}");
    let finance_token = json_body(&finance_login)["value"].as_str().expect("finance login should return a real token").to_string();

    // No bearer token at all -- real rejection, not a silent pass-through.
    let no_auth = request("POST", "/api/create", "", "{\"vendor\":\"No Auth Co\",\"amount_cents\":1}");
    let no_auth_body = json_body(&no_auth);
    assert!(
        no_auth_body["value"].as_str().unwrap_or("").contains("missing"),
        "a request with no bearer token should be refused: {no_auth_body}"
    );

    // Real RBAC: a finance token is a real, verified identity, but still
    // denied by `check_role`/`acquire` for an admin-gated route.
    let finance_denied = request(
        "POST",
        "/api/create",
        &format!("Authorization: Bearer {finance_token}\r\n"),
        "{\"vendor\":\"Denied Co\",\"amount_cents\":1}",
    );
    let finance_denied_body = json_body(&finance_denied);
    assert!(
        !finance_denied_body["value"].as_str().unwrap_or("").contains("Denied Co"),
        "a finance-role token must not be able to create a purchase order: {finance_denied_body}"
    );

    // Real admin create.
    let created = request(
        "POST",
        "/api/create",
        &format!("Authorization: Bearer {admin_token}\r\n"),
        "{\"vendor\":\"Browser Test Vendor\",\"amount_cents\":31337}",
    );
    let created_body = json_body(&created);
    assert_eq!(created_body["value"], "Browser Test Vendor", "create response: {created_body}");

    // Search finds it; an unrelated query doesn't.
    let search_hit = request("POST", "/api/purchase_orders/list", "", "{\"q\":\"Browser Test\",\"offset\":0,\"limit\":10}");
    let search_hit_rows = json_body(&search_hit);
    assert!(
        search_hit_rows.as_array().is_some_and(|rows| rows.iter().any(|r| r["vendor"] == "Browser Test Vendor")),
        "search for a real substring should find the row: {search_hit_rows}"
    );
    let search_miss = request("POST", "/api/purchase_orders/list", "", "{\"q\":\"Definitely Not A Vendor\",\"offset\":0,\"limit\":10}");
    let search_miss_rows = json_body(&search_miss);
    assert!(search_miss_rows.as_array().is_some_and(|rows| rows.is_empty()), "search for a nonexistent vendor should find nothing: {search_miss_rows}");

    let id = search_hit_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["vendor"] == "Browser Test Vendor")
        .expect("just asserted this exists")["id"]
        .as_i64()
        .expect("row should have a real integer id");

    // Real UPDATE.
    let updated = request(
        "POST",
        "/api/purchase_orders/update",
        &format!("Authorization: Bearer {admin_token}\r\n"),
        &format!("{{\"id\":{id},\"vendor\":\"Renamed Vendor\",\"amount_cents\":40000}}"),
    );
    let updated_body = json_body(&updated);
    assert_eq!(updated_body["value"], "Renamed Vendor", "update response: {updated_body}");
    let after_update = request("POST", "/api/purchase_orders/list", "", &format!("{{\"q\":\"Renamed\",\"offset\":0,\"limit\":10}}"));
    let after_update_rows = json_body(&after_update);
    assert!(
        after_update_rows.as_array().is_some_and(|rows| rows.iter().any(|r| r["amount_cents"] == 40000)),
        "the update should really have changed the row: {after_update_rows}"
    );

    // The `Disbursement` workflow, wired to a real route: starts a real
    // instance, advances Draft -> Approved, which really calls the mock
    // webhook and really writes a durable `disbursements` row. This PO
    // is below the escalation threshold, so a plain admin token suffices.
    let approved = request("POST", "/api/purchase_orders/approve", &format!("Authorization: Bearer {admin_token}\r\n"), &format!("{{\"id\":{id}}}"));
    let approved_body = json_body(&approved);
    assert_eq!(approved_body["value"], "approved", "approve response: {approved_body}");

    // Real dual control: a purchase order at/above the escalation
    // threshold refuses a plain admin token and only accepts a real
    // finance_director one -- a structurally different `acquire`d
    // callable, not an `if` inside the same function.
    let large_created = request(
        "POST",
        "/api/create",
        &format!("Authorization: Bearer {admin_token}\r\n"),
        "{\"vendor\":\"Big Ticket Vendor\",\"amount_cents\":15000000}",
    );
    let large_created_body = json_body(&large_created);
    assert_eq!(large_created_body["value"], "Big Ticket Vendor", "large create response: {large_created_body}");
    let large_list = request("POST", "/api/purchase_orders/list", "", "{\"q\":\"Big Ticket\",\"offset\":0,\"limit\":10}");
    let large_id = json_body(&large_list).as_array().unwrap()[0]["id"].as_i64().expect("large row should have a real id");

    let admin_denied_large = request(
        "POST",
        "/api/purchase_orders/approve",
        &format!("Authorization: Bearer {admin_token}\r\n"),
        &format!("{{\"id\":{large_id}}}"),
    );
    let admin_denied_large_body = json_body(&admin_denied_large);
    assert!(
        admin_denied_large_body["value"].as_str().unwrap_or("").contains("finance_director"),
        "a plain admin token must not be able to approve a large PO: {admin_denied_large_body}"
    );

    let director_login = request("POST", "/api/login", "", "{\"role\":\"finance_director\"}");
    let director_token = json_body(&director_login)["value"].as_str().expect("director login should return a real token").to_string();
    let director_approved_large = request(
        "POST",
        "/api/purchase_orders/approve",
        &format!("Authorization: Bearer {director_token}\r\n"),
        &format!("{{\"id\":{large_id}}}"),
    );
    let director_approved_large_body = json_body(&director_approved_large);
    assert_eq!(director_approved_large_body["value"], "approved", "director approve response: {director_approved_large_body}");

    // Real REJECT: the workflow's other real branch out of Draft.
    let rejected = request("POST", "/api/purchase_orders/reject", &format!("Authorization: Bearer {admin_token}\r\n"), "{}");
    let rejected_body = json_body(&rejected);
    assert_eq!(rejected_body["value"], "rejected", "reject response: {rejected_body}");

    // Real overdue detection (`list_disbursement_overdue`, driven by
    // `Draft`'s own `sla_seconds`) -- nothing should be stale yet at
    // test speed, but the route itself must answer with a real JSON
    // array, not an error.
    let overdue = request("GET", "/api/disbursements/overdue", "", "");
    let overdue_rows = json_body(&overdue);
    assert!(overdue_rows.is_array(), "overdue response should be a real JSON array: {overdue_rows}");

    // Real DELETE.
    let deleted = request(
        "POST",
        "/api/purchase_orders/delete",
        &format!("Authorization: Bearer {admin_token}\r\n"),
        &format!("{{\"id\":{id}}}"),
    );
    let deleted_body = json_body(&deleted);
    assert_eq!(deleted_body["value"], "true", "delete response: {deleted_body}");
    let after_delete = request("POST", "/api/purchase_orders/list", "", &format!("{{\"q\":\"Renamed\",\"offset\":0,\"limit\":10}}"));
    let after_delete_rows = json_body(&after_delete);
    assert!(after_delete_rows.as_array().is_some_and(|rows| rows.is_empty()), "the deleted row should really be gone: {after_delete_rows}");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
}
