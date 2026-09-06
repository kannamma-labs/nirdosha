//! Tests for `tcp`/`connect` (`send`/`recv`/`stop` are reused from
//! `chan`/`sandbox`, not new keywords). Deliberately does *not* depend on
//! any external service (Docker, Neo4j, a real HTTP server) being
//! present -- every test here spins up its own minimal `TcpListener` in
//! the test harness itself, the same "self-contained, not environment-
//! fragile" discipline every other test file in this project follows.
//! `examples/tcp_client.nir` is the illustrative, real-world-facing demo
//! (documented as needing an external service); this file is what
//! actually has to pass in CI.

use nirdosha::ast::Ty;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck, TypeErrorKind};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn parse_ok(src: &str) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    Parser::new(toks).parse_program().expect("parse should succeed")
}

fn first_type_error(src: &str) -> TypeErrorKind {
    let program = parse_ok(src);
    match typecheck(&program) {
        Ok(()) => panic!("expected a type error, but the program type-checked cleanly"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Compiles a `str`-returning `main` (real path, not the interpreter --
/// `tcp` compiles today, `crates/runtime-kernels`'s `nir_tcp_*`) and
/// returns what it printed (`codegen.rs::emit_c_main`'s own convention
/// for a `str` result: printf then exit 0).
fn compile_and_run_str(src: &str) -> String {
    let program = parse_ok(src);
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_tcp_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Binds to an OS-assigned free port and returns it -- avoids any fixed
/// port number that could collide with something else already listening
/// on the machine running these tests.
#[allow(dead_code)]
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").expect("binding a fresh loopback listener should never fail").local_addr().unwrap().port()
}

// ---- basic connect/send/recv/stop, against a real socket -----------------

#[test]
fn a_connected_client_can_send_and_receive_real_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
        stream.write_all(b"pong").unwrap();
    });

    let src = format!(
        r#"
        fn main() {{
            let conn: tcp = connect("127.0.0.1", {port})
            send(conn, "ping")
            let reply: str = recv(conn)
            stop conn
            print(reply)
        }}
    "#
    );
    assert_eq!(compile_and_run_str(&src), "pong\n");
    server.join().unwrap();
}

// ---- static rejections -------------------------------------------------

#[test]
fn connect_requires_a_str_host() {
    let kind = first_type_error(
        r#"
        fn main() -> i64 {
            let conn: tcp = connect(1, 80)
            return 0
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::TypeMismatch { expected: Ty::Str, found: Ty::I64 });
}

#[test]
fn connect_requires_an_i64_port() {
    let kind = first_type_error(
        r#"
        fn main() -> i64 {
            let conn: tcp = connect("localhost", "eighty")
            return 0
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::TypeMismatch { expected: Ty::I64, found: Ty::Str });
}

#[test]
fn send_on_a_tcp_connection_requires_a_str_payload() {
    let kind = first_type_error(
        r#"
        fn main() -> i64 {
            let conn: tcp = connect("localhost", 80)
            send(conn, 42)
            return 0
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::TypeMismatch { expected: Ty::Str, found: Ty::I64 });
}

#[test]
fn stopping_a_tcp_connection_twice_is_a_static_use_after_move() {
    let program = parse_ok(
        r#"
        fn main() -> i64 {
            let conn: tcp = connect("localhost", 80)
            stop conn
            stop conn
            return 0
        }
    "#,
    );
    typecheck(&program).expect("should typecheck cleanly");
    let result = nirdosha::ownership::check_ownership(&program);
    assert!(result.is_err(), "using a tcp connection after `stop`ping it must be a static ownership error");
}
