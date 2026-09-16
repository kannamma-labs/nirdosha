//! The golden corpus contract.
//!
//! Every assertion here pins the **pre-migration** behavior of a
//! translated example under plain cargo. Post-migration, the *same
//! unmodified files* run under the proprietary `nirdosha` compiler
//! must still pass every one of these lines — plus gain the
//! Nirdosha-specific layer (generated UI, durable workflow store,
//! proofs in the certificate). If a migration ever changes a line of
//! observable behavior, this file fails before any user notices.

use std::process::Command;

fn run(bin: &str) -> String {
    let exe = env_bin(bin);
    let out = Command::new(&exe)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {bin}: {e}"));
    assert!(out.status.success(), "{bin} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn env_bin(bin: &str) -> std::path::PathBuf {
    let var = format!("CARGO_BIN_EXE_{bin}").replace('-', "_");
    std::env::var_os(&var)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("{var} not set"))
}

fn expect(stdout: &str, lines: &[&str]) {
    for line in lines {
        assert!(
            stdout.lines().any(|l| l.trim() == *line || l.contains(line.trim())),
            "expected line {line:?} in output:\n{stdout}"
        );
    }
}

#[test]
fn hello() {
    expect(&run("v2_hello_nir"), &["8", "hello, nirdosha"]);
}

#[test]
fn types_and_control_flow() {
    expect(
        &run("v2_02_types_and_control_flow"),
        &["10", "true", "Negative()", "Even()", "Odd()", "3628800"],
    );
}

#[test]
fn data_modeling() {
    expect(
        &run("v2_03_data_modeling"),
        &[
            "5",
            "12.56636",
            "[5, 7, 9]",
            "32",
            "[[1, 3], [2, 4]]",
            "-2",
            "19.99",
            "USD()",
            "Kilogram()",
        ],
    );
}

#[test]
fn ownership_and_concurrency() {
    expect(
        &run("v2_04_ownership_and_concurrency"),
        &["42", "-1"],
    );
}

#[test]
fn platform_services() {
    expect(
        &run("v2_05_platform_services"),
        &[
            "5",
            "9",
            "hello from level 5",
            "ada",
            "nirdosha",
            "connection refused",
            "-1",
        ],
    );
}

#[test]
fn identity_and_declarative_ui() {
    expect(
        &run("v2_06_identity_and_declarative_ui"),
        &[
            "42",
            "500",
            "true",
            "committing 10",
            "compensating -5",
            "approval 1 approved",
            "created",
        ],
    );
}

#[test]
fn validate_contracts() {
    expect(&run("v2_35_validate_contracts"), &["9", "0", "7", "5"]);
}

#[test]
fn transact() {
    expect(
        &run("v2_36_transact"),
        &[
            "committing 10",
            "logged 10 true",
            "compensating -5",
            "logged -5 false",
            "committing 1",
        ],
    );
}

#[test]
fn workflow() {
    expect(
        &run("v2_38_workflow"),
        &["1", "approval 1 for amount 250000", "true"],
    );
}

#[test]
fn screen_ui() {
    expect(&run("v2_39_screen_ui"), &["true", "1"]);
}

#[test]
fn dashboard() {
    expect(&run("v2_40_dashboard"), &["12", "true"]);
}

#[test]
fn enterprise_app() {
    expect(
        &run("v2_enterprise_app"),
        &[
            "true",
            "1",
            "25000",
            "199.99",
            "USD()",
            "0.789",
            "-2", // no cfo role on this token — check_role fails, by design
            "reversing disbursement for purchase order 1",
            "disbursement 1 false",
            "true",
        ],
    );
}
// ---------------------------------------------------------------------------
// The full-corpus goldens (batch 2+)
// ---------------------------------------------------------------------------

#[test]
fn scalar_types_and_literals() {
    expect(
        &run("v2_01_scalar_types"),
        &[
            "100 30000 2000000000 9000000000000000000 200 4000000000 42",
            "3.14159",
            "true",
            "line one",
            "()",
        ],
    );
}

#[test]
fn operators() {
    expect(
        &run("v2_02_operators"),
        &[
            "10",
            "2.3333333333333335",
            "[11, 22, 33]",
            "[-9, -18, -27]",
            "[10, 40, 90]",
            "[0.1, 0.1, 0.1]",
            "true",
            "[[2, 4], [6, 8]]",
            "[[2, 4], [6, 8]]",
            "[3, 7]",
        ],
    );
}

#[test]
fn control_flow_and_functions() {
    expect(&run("v2_03_control_flow"), &["1", "odd", "10", "3628800", "true", "false"]);
}

#[test]
fn first_class_functions() {
    expect(&run("v2_04_first_class_functions"), &["42", "20", "25", "18"]);
}

#[test]
fn structs_enums_match() {
    expect(
        &run("v2_05_structs_enums_match"),
        &["3", "4", "5", "12.56636", "12", "0", "4"],
    );
}

#[test]
fn generics() {
    expect(&run("v2_06_generics"), &["5", "1", "9", "true", "41", "7"]);
}

#[test]
fn option_result_prelude() {
    expect(&run("v2_07_option_result"), &["41", "0", "5", "-1", "7"]);
}

#[test]
fn str_boundary_convention() {
    expect(&run("v2_08_str_boundary"), &["hello, nirdosha", "world", "approved"]);
}

#[test]
fn money_and_currency() {
    expect(&run("v2_09_money_currency"), &["19.99", "USD()", "21.59", "true"]);
}

#[test]
fn measure_and_unit() {
    expect(&run("v2_10_measure_unit"), &["5", "Kilogram()", "true"]);
}

#[test]
fn linalg() {
    expect(
        &run("v2_12_linalg"),
        &[
            "32",
            "[-3, 6, -3]",
            "3",
            "6",
            "6",
            "[[1, 3], [2, 4]]",
            "5",
            "-2",
            "[[-2, 1], [1.5, -0.5]]",
            "[0, 0, 0]",
            "[[1, 1], [1, 1]]",
            "[[1, 0, 0], [0, 1, 0], [0, 0, 1]]",
            "[2, 3]",
            "2",
        ],
    );
}

#[test]
fn deterministic_simulation() {
    expect(
        &run("v2_13_simulation"),
        &[
            "0.7415648787718233", // seed-42 first draw, byte-pinned
            "5.594",              // SF->LA Euclidean distance
            "[1, 1, 1, 1]",       // kf predict
        ],
    );
}

#[test]
fn ownership_box() {
    expect(&run("v2_14_box"), &["10", "42", "7", "99"]);
}

#[test]
fn borrowing() {
    expect(&run("v2_15_borrowing"), &["42", "42", "5", "3"]);
}

#[test]
fn effects() {
    expect(
        &run("v2_16_effects"),
        &["5", "0.5665615751722809", "20", "42"], // seed-1 draw, byte-pinned
    );
}

#[test]
fn audited_block() {
    expect(&run("v2_17_audited"), &["20", "0", "20"]);
}

#[test]
fn threads() {
    expect(&run("v2_18_threads"), &["42", "42", "200"]);
}

#[test]
fn channels() {
    expect(&run("v2_19_channels"), &["42", "4", "99"]);
}

#[test]
fn tcp_client() {
    expect(
        &run("v2_22_tcp_client"),
        &["GET / HTTP/1.1", "hello from the server"],
    );
}

#[test]
fn tcp_listener() {
    expect(
        &run("v2_23_tcp_listener"),
        &["hello from client 1|hello from client 2"],
    );
}

#[test]
fn file_io() {
    expect(&run("v2_24_file_io"), &["first line", "second line", "true"]);
}

#[test]
fn requires_public() {
    expect(&run("v2_34_requires_public"), &["true", "1"]);
}

#[test]
fn transact_cross_process() {
    expect(&run("v2_37_transact_cross"), &["committing 42", "true", "txn-"]);
}

#[test]
fn dashboard_visual() {
    expect(&run("v2_41_dashboard_visual"), &["true"]);
}

#[test]
fn workspace_panel() {
    expect(&run("v2_42_workspace_panel"), &["true", "1"]);
}

#[test]
fn layout() {
    expect(&run("v2_43_layout"), &["1"]);
}

#[test]
fn module_nav_grouping() {
    expect(&run("v2_44_module_nav"), &["1", "true"]);
}

#[test]
fn module_namespacing() {
    expect(&run("v2_45_module_namespacing"), &["1", "500", "7", "created", "501"]);
}

#[test]
fn schema_conventions() {
    expect(
        &run("v2_46_schema_conventions"),
        &["Acme", "true", "compliance_officer", "legal-and-compliance"],
    );
}

#[test]
fn external_service_boundary() {
    expect(
        &run("v2_47_external_boundary"),
        &["mysql://user:pass@localhost/orders", "mq_provider_kafka_connect"],
    );
}

#[test]
fn froze() {
    expect(&run("v2_48_froze"), &["21", "42", "63"]);
}

#[test]
fn nfr() {
    expect(&run("v2_49_nfr"), &["5", "5", "-1"]);
}

#[test]
fn field_masking_and_check_role() {
    expect(
        &run("v2_50_field_masking"),
        &["10", "150000", "0", "Ada Lovelace", "-3", "no_access"],
    );
}

#[test]
fn compiled_workflow_state_machine() {
    expect(
        &run("v2_52_compiled_workflow"),
        &[
            "ticket 1 opened",
            "1",
            "ticket 1 leaving Open",
            "ticket 1 now InProgress",
            "ticket 1 closed",
            "true",
        ],
    );
}

#[test]
fn compiled_workflow_escalation() {
    expect(
        &run("v2_54_compiled_escalation"),
        &[
            "incident 1 opened",
            "1",
            "[]",
            "true",
            "instance_id",
            "incident 1 escalated to manager",
            "incident 1 resolved",
        ],
    );
}

#[test]
fn bench_dot() {
    expect(&run("v2_bench_dot"), &["352531173.3352983"]);
}

#[test]
fn bench_matmul() {
    expect(&run("v2_bench_matmul"), &["187065526.66763383"]);
}

#[test]
fn bench_det() {
    expect(&run("v2_bench_det"), &["742528617.5592908"]);
}

#[test]
fn bench_fib() {
    expect(&run("v2_bench_fib"), &["9227465"]);
}

#[test]
fn bench_floatloop() {
    expect(&run("v2_bench_floatloop"), &["1499998.4998404514"]);
}

#[test]
fn bench_kalman() {
    expect(
        &run("v2_bench_kalman"),
        &["1999.989999998326", "999.994999999163"],
    );
}

/// 51 runs an accept loop forever by design — the golden test plays the
/// original's `curl` walkthrough against it with a real client socket,
/// then kills it.
#[test]
fn compiled_serve_serves_real_requests() {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let exe = env_bin("v2_51_compiled_serve");
    let mut child = Command::new(&exe).spawn().expect("serve bin");

    // poll until the listener is up
    let mut stream = None;
    for _ in 0..100 {
        if let Ok(s) = TcpStream::connect("127.0.0.1:8080") {
            stream = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut stream = stream.expect("serve never came up");

    stream
        .write_all(b"GET /api/hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).unwrap();
    let hello = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert!(hello.contains("hello from /api/hello"), "got: {hello}");

    let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap();
    stream
        .write_all(b"GET /api/echo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let n = stream.read(&mut buf).unwrap();
    let echo = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert!(echo.contains("hello from /api/echo"), "got: {echo}");

    let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap();
    stream
        .write_all(b"GET /api/unknown HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let n = stream.read(&mut buf).unwrap();
    let missing = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert!(missing.contains("404 Not Found"), "got: {missing}");

    let _ = child.kill();
    let _ = child.wait();
}

/// 53 needs a reachable Redis, like the original — the golden test
/// stands one up (a +PONG responder), then the three REAL provider
/// POSTs flow to the example's own listener.
#[test]
fn compiled_workflow_notifications_post_for_real() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // the fake Redis: PING -> +PONG, PUBLISH -> :1
    let redis = TcpListener::bind("127.0.0.1:6379");
    let redis_handle = match redis {
        Ok(l) => Some(std::thread::spawn(move || {
            for conn in l.incoming().flatten() {
                let mut conn = conn;
                let mut buf = [0u8; 128];
                if let Ok(n) = conn.read(&mut buf) {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if text.starts_with("PING") {
                        let _ = conn.write_all(b"+PONG\r\n");
                    } else if text.starts_with("PUBLISH") {
                        let _ = conn.write_all(b":1\r\n");
                    }
                }
            }
        })),
        // a real Redis is already here — the bin will use it instead
        Err(_e) => None,
    };

    let out = run("v2_53_compiled_notifications");
    expect(
        &out,
        &[
            "2",                       // 3 CREATEs (0) + 2 INSERTs (1 each)
            "POST /send/email",        // a real request left the process
            "Bearer email-secret-key", // the real authenticated shape
            "POST /send/sms",
            "Bearer sms-secret-key",
            "push not configured",     // the real not-configured path
            "1",
        ],
    );
    drop(redis_handle);
}
