//! Real sockets exercise worker isolation, bounded admission and shutdown.
use nirdosha_rt::web::ServeConfig;
use nirdosha_rt::{Auth, Response, Router};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn start(router: Router, max_connections: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            router.serve_until(
                listener,
                &worker_stop,
                ServeConfig {
                    max_connections: max_connections.try_into().unwrap(),
                    io_timeout: Duration::from_millis(250),
                },
            )
        });
        Self {
            addr,
            stop,
            worker: Some(worker),
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }

    fn request(&self, path: &str) -> String {
        let mut stream = self.connect();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}

fn router() -> Router {
    Router::new(|_| Auth::login("anon", &[])).get("/", "root", |_, _| Response::text(200, "ok"))
}

#[test]
fn disconnects_and_handler_panics_do_not_kill_server() {
    let server = Server::start(
        router().get("/panic", "panic", |_, _| panic!("handler failure")),
        64,
    );
    for _ in 0..20 {
        let mut stream = server.connect();
        let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    }
    assert!(server.request("/panic").is_empty());
    assert!(server.request("/").starts_with("HTTP/1.1 200"));
}

#[test]
fn idle_client_does_not_block_a_request_and_shutdown_drains_it() {
    let server = Server::start(router(), 2);
    let _idle = server.connect();
    assert!(server.request("/").ends_with("ok"));
    // Keep the idle peer open while Drop joins its timed-out worker.
    drop(server);
}

#[test]
fn slow_handler_does_not_block_other_routes_and_capacity_is_bounded() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let server = Server::start(
        router().get("/wait", "held open", move |_, _| {
            entered_tx.send(()).unwrap();
            let (lock, wake) = &*worker_gate;
            let (released, _) = wake
                .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(3), |v| !*v)
                .unwrap();
            assert!(*released, "test did not release handler");
            Response::text(200, "released")
        }),
        2,
    );
    let mut first = server.connect();
    first
        .write_all(b"GET /wait HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(server.request("/").ends_with("ok"));
    thread::sleep(Duration::from_millis(20));
    let mut second = server.connect();
    second
        .write_all(b"GET /wait HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut excess = server.connect();
    let mut byte = [0];
    match excess.read(&mut byte) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("excess connection was not closed: {other:?}"),
    }
    server.stop.store(true, Ordering::Release);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    for mut stream in [first, second] {
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("released"));
    }
}

/// Real sockets, a real `serve_until` accept loop, real repeated
/// connections from the same peer (`127.0.0.1`, this test process
/// itself) -- proving `with_rate_limit` actually throttles through the
/// live server, not just at the `ratelimit::RateLimiter` unit level.
/// `router.dispatch(&req)` (no real socket, no peer) never rate-limits
/// at all -- disclosed, not a gap -- so this has to go through a real
/// `Server`, the same reason every other test in this file does.
#[test]
fn rate_limit_denies_past_the_configured_max_from_the_same_peer_and_leaves_other_paths_untouched() {
    let server = Server::start(
        Router::new(|_| Auth::login("anon", &[]))
            .get("/limited", "limited", |_, _| Response::text(200, "ok"))
            .get("/unlimited", "unlimited", |_, _| Response::text(200, "ok"))
            .with_rate_limit(vec!["/limited"], 3, Duration::from_secs(60)),
        64,
    );
    for _ in 0..3 {
        assert!(server.request("/limited").starts_with("HTTP/1.1 200"));
    }
    assert!(server.request("/limited").starts_with("HTTP/1.1 429"));
    // A path never named in `with_rate_limit` is never throttled, no
    // matter how many times the same peer hits it.
    for _ in 0..5 {
        assert!(server.request("/unlimited").starts_with("HTTP/1.1 200"));
    }
}

/// `Router::new` defaults to `Runtime::Async` (unit-tested in
/// `web.rs` itself); this proves the `Runtime::Sync` opt-out is a real,
/// working transport over real sockets too, not just a type that
/// exists -- the same rate-limit scenario the async-default test above
/// already covers, run again with the sync accept loop explicitly
/// selected.
#[test]
fn with_runtime_sync_opt_out_still_serves_real_requests_over_real_sockets() {
    use nirdosha_rt::web::Runtime;
    let server = Server::start(
        Router::new(|_| Auth::login("anon", &[]))
            .get("/limited", "limited", |_, _| Response::text(200, "ok"))
            .with_rate_limit(vec!["/limited"], 2, Duration::from_secs(60))
            .with_runtime(Runtime::Sync),
        64,
    );
    assert!(server.request("/limited").starts_with("HTTP/1.1 200"));
    assert!(server.request("/limited").starts_with("HTTP/1.1 200"));
    assert!(server.request("/limited").starts_with("HTTP/1.1 429"));
}

#[test]
fn concurrent_sessions_keep_roles_and_logout_separate() {
    use nirdosha_rt::{NavLink, Request, Role};
    struct Member;
    impl Role for Member {
        const NAME: &'static str = "member";
    }
    let router = Arc::new(
        router()
            .with_login("/login", |name, password| {
                (name == password).then(|| vec!["member".into()])
            })
            .with_nav(vec![NavLink {
                label: "home",
                href: "/",
                group: "",
            }])
            .get_gated::<Member>("/private", "private", |_, _, _| {
                Response::text(200, "allowed")
            }),
    );
    let workers: Vec<_> = (0..16)
        .map(|i| {
            let router = router.clone();
            thread::spawn(move || {
                let mut req = Request::parse(&format!(
                    "POST /login HTTP/1.1\r\n\r\nusername=user{i}&password=user{i}"
                ))
                .unwrap();
                let login = router.dispatch(&req);
                assert_eq!(login.status, 302);
                let cookie = login
                    .extra_headers
                    .iter()
                    .find(|(k, _)| k == "Set-Cookie")
                    .unwrap()
                    .1
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string();
                req.method = "GET".into();
                req.path = "/private".into();
                assert_eq!(router.dispatch(&req).status, 403);
                req.headers.insert("cookie".into(), cookie);
                assert_eq!(router.dispatch(&req).status, 200);
                req.path = "/login".into();
                assert!(router.dispatch(&req).body.contains(&format!("user{i}")));
                req.path = "/openapi.json".into();
                assert!(router.dispatch(&req).body.contains("member"));
                req.method = "POST".into();
                req.path = "/login/logout".into();
                assert_eq!(router.dispatch(&req).status, 302);
                req.method = "GET".into();
                req.path = "/private".into();
                assert_eq!(router.dispatch(&req).status, 403);
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}

nirdosha_rt::roles! { Reader = "reader"; Poster = "poster"; }

#[derive(Clone, Default, serde::Serialize)]
struct Message {
    id: i64,
    body: String,
}

fn messages() -> &'static nirdosha_rt::prelude::SharedCell<Vec<Message>> {
    static STORE: std::sync::OnceLock<nirdosha_rt::prelude::SharedCell<Vec<Message>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| nirdosha_rt::prelude::SharedCell::new(Vec::new()))
}

nirdosha_rt::communication_feed! {
    mount: mount_live,
    entity: Message,
    store: messages,
    path: "/live",
    fields: [body: String],
    post_access: requires role "poster",
    read_access: requires role "reader",
    long_poll_seconds: 3,
}

fn exchange(addr: SocketAddr, method: &str, path: &str, role: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nX-Roles: {role}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn live_feed_waits_for_authorized_post_without_blocking_other_requests() {
    let server = Server::start(
        mount_live(Router::new(|req| {
            Auth::login(
                "test",
                &req.header("x-roles")
                    .unwrap_or("")
                    .split(',')
                    .collect::<Vec<_>>(),
            )
        })),
        8,
    );
    let initial = exchange(server.addr, "GET", "/api/live", "reader", "");
    assert!(initial.contains("X-Nirdosha-Revision: 0"));
    let html = exchange(server.addr, "GET", "/live", "reader", "");
    assert!(html.contains("fetch(path+\"?since=\""));
    assert!(!html.contains("http-equiv=\"refresh\""));
    let (tx, rx) = mpsc::channel();
    let addr = server.addr;
    let pending = thread::spawn(move || {
        tx.send(exchange(addr, "GET", "/api/live?since=0", "reader", ""))
            .unwrap();
    });
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(exchange(addr, "GET", "/api/live?since=0", "", "").starts_with("HTTP/1.1 403"));
    assert!(exchange(addr, "POST", "/live", "reader", "body=denied").starts_with("HTTP/1.1 403"));
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(
        exchange(addr, "POST", "/live", "poster", "body=delivered").starts_with("HTTP/1.1 302")
    );
    let response = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("post did not wake long poll");
    assert!(response.contains("X-Nirdosha-Revision: 1"));
    assert!(response.contains("delivered"));
    assert!(!response.contains("denied"));
    pending.join().unwrap();
    // A notification before the next wait must not be lost.
    let stale = exchange(addr, "GET", "/api/live?since=0", "reader", "");
    assert!(stale.contains("X-Nirdosha-Revision: 1"));
    assert!(exchange(addr, "GET", "/api/live?since=bad", "reader", "").starts_with("HTTP/1.1 400"));
}

#[test]
fn feed_timeout_does_not_invent_a_change() {
    let updates = nirdosha_rt::feed::FeedUpdates::new();
    assert_eq!(updates.wait(0, Duration::from_millis(20)), 0);
    updates.publish();
    assert_eq!(updates.wait(0, Duration::from_secs(1)), 1);
}
