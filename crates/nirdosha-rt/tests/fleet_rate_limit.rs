//! `Router::with_fleet_rate_limit`, proved end-to-end over a real server
//! against a real Redis -- not just `web/fleet_ratelimit.rs`'s own unit
//! tests of the `FleetRateLimiter` type in isolation.
//!
//! Requires a real Redis reachable at `NIRDOSHA_TEST_REDIS_URL`
//! (default `redis://127.0.0.1:6379/`) -- every test here is
//! `#[ignore]`d so the normal suite stays green without one; run
//! explicitly (`cargo test -- --ignored`) with one available. Verified
//! against a real, disposable `redis:alpine` container during this
//! feature's own development.
#![cfg(feature = "fleet-rate-limit")]

use nirdosha_rt::web::{Runtime, ServeConfig};
use nirdosha_rt::{Auth, Response, Router};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn start(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            router.serve_until(listener, &worker_stop, ServeConfig { max_connections: 8.try_into().unwrap(), io_timeout: Duration::from_millis(250) })
        });
        Self { addr, stop, worker: Some(worker) }
    }

    fn request(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}

fn status_of(response: &str) -> u16 {
    response.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn redis_url() -> String {
    std::env::var("NIRDOSHA_TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())
}

// Both scenarios below share Redis state keyed purely by peer IP (the
// same real loopback address every connection in this test process
// uses -- `ratelimit.rs`'s own pre-existing, faithfully-ported design:
// one combined budget across every configured path for a given IP, not
// independent per-path budgets, the same shape a real "these sensitive
// paths share one combined abuse budget" policy wants). Run as one
// `#[test]` function, not two, specifically so they execute
// sequentially against that shared state rather than racing each other
// under `cargo test`'s default parallelism -- two independent `#[test]`
// fns hitting the same real IP's Redis key concurrently would each see
// the other's increments and fail for a reason that has nothing to do
// with either scenario actually being wrong.
#[test]
#[ignore]
fn fleet_rate_limit_denies_past_the_configured_max_and_shares_state_across_independent_servers() {
    let router_a = || {
        Router::new(|_| Auth::login("anon", &[]))
            .with_runtime(Runtime::Sync)
            .get("/limited", "limited", |_, _| Response::text(200, "ok"))
            .get("/unlimited", "unlimited", |_, _| Response::text(200, "ok"))
            .with_fleet_rate_limit(vec!["/limited"], 6, Duration::from_secs(60), &redis_url())
            .expect("a real Redis URL must construct a real limiter")
    };
    let server = Server::start(router_a());

    for _ in 0..3 {
        assert_eq!(status_of(&server.request("/limited")), 200);
    }
    // `/unlimited` was never named in `with_fleet_rate_limit` -- never
    // throttled, no matter how many times the same peer hits it.
    for _ in 0..5 {
        assert_eq!(status_of(&server.request("/unlimited")), 200);
    }

    // A second, independent `Router`/server (standing in for a second
    // fleet instance) pointed at the *same* Redis and the *same*
    // configured limit continues the *same* count -- 3 already spent
    // above, 3 more here reaches the cap of 6, and the next one is
    // denied. This only holds if the two servers really share state,
    // not just their own independent in-memory ones.
    let server_b = Server::start(router_a());
    for _ in 0..3 {
        assert_eq!(status_of(&server_b.request("/limited")), 200);
    }
    assert_eq!(status_of(&server_b.request("/limited")), 429);
    assert_eq!(status_of(&server.request("/limited")), 429, "the original server sees the same denial, sharing the same count");
}
