//! `Router::serve` must survive a client that disconnects before the
//! response is fully written — a real crash reproduced live while
//! screenshotting `57_ui_engine_demo.nir` (Chrome tore down a
//! connection mid-write, `Tcp::send`'s `.expect("tcp send")` turned
//! that broken pipe into a process-ending panic). Forcing an RST close
//! (`SO_LINGER(0)`) reproduces the same failure reliably, in-process.

use nirdosha_rt::{Auth, Response, Router};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

#[test]
fn a_client_that_disconnects_mid_response_does_not_kill_the_server() {
    let port = 8199;
    thread::spawn(move || {
        let router = Router::new(|_| Auth::login("anon", &[])).get("/", "root", |_, _| Response::text(200, "ok"));
        router.serve(port);
    });

    let connect = || TcpStream::connect(("127.0.0.1", port));
    let mut up = false;
    for _ in 0..50 {
        if connect().is_ok() {
            up = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(up, "server never came up");

    // Send a request, then close the connection immediately, before
    // reading anything — enough to reproduce a broken pipe on the
    // server's write side without needing an unstable `SO_LINGER` API.
    for _ in 0..30 {
        let mut stream = connect().unwrap();
        let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        drop(stream);
    }

    thread::sleep(Duration::from_millis(100));

    // The server must still be alive and able to answer a normal request.
    let mut stream = connect().expect("server died after disconnected clients");
    stream.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 200"), "unexpected response after disconnects: {text}");
}
