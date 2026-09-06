//! Phase 0 harness for `rfcs/0007-apm-runtime-kernel.md`: real timing
//! numbers for the compiled path's actual current effect kernels
//! (`nir_tcp_*`/`nir_file_*` in `crates/runtime-kernels`), with no
//! admission/lease check in front of them -- the true zero baseline
//! every SLO in that RFC needs re-derived against, not assumed.
//!
//! Each `nir_*` kernel is benchmarked alongside the raw `std` call it
//! wraps, so the numbers separate two different costs that matter for
//! different reasons:
//!   - the **syscall itself** (connect/accept/open/send/recv/read/write)
//!     -- a floor nothing about this RFC can reduce, and
//!   - the **`extern "C"` kernel wrapper's own overhead** on top of that
//!     floor -- the actual proxy for "how much would inserting one more
//!     `extern "C"` lease-check call of similar shape cost," since that
//!     is exactly what the RFC's local admission plane would add at
//!     each call site.
//!
//! Two benchmarks (accept, recv) need a concurrent peer to keep work
//! available so the timed call doesn't block waiting for the network --
//! each is labeled with the caveat that follows from that (RFC 0006's
//! own bench.rs sets the precedent for disclosing this rather than
//! hiding it: see its 64MB-clone caveat).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Matches crates/runtime-kernels/src/lib.rs's actual exported symbols
// exactly (see that file's `nir_tcp_*`/`nir_file_*` doc comments for
// each one's precise contract) -- this harness calls the real compiled
// kernel, not a reimplementation of it.
extern "C" {
    fn nir_tcp_connect(host_ptr: *const u8, host_len: i64, port: i64) -> i64;
    fn nir_tcp_accept(listener_handle: i64) -> i64;
    fn nir_tcp_send(handle: i64, buf_ptr: *const u8, buf_len: i64) -> i64;
    fn nir_tcp_recv(handle: i64, buf_ptr: *mut u8, buf_cap: i64) -> i64;
    fn nir_tcp_stop(handle: i64) -> i32;
    fn nir_file_open(path_ptr: *const u8, path_len: i64, mode_ptr: *const u8, mode_len: i64) -> i64;
    fn nir_file_write(handle: i64, buf_ptr: *const u8, buf_len: i64) -> i64;
    fn nir_file_read(handle: i64, buf_ptr: *mut u8, buf_cap: i64) -> i64;
    fn nir_file_stop(handle: i64) -> i32;
}

fn bench(label: &str, iters: u64, mut f: impl FnMut()) {
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        for _ in 0..iters {
            f();
        }
        let elapsed = start.elapsed();
        if elapsed < best {
            best = elapsed;
        }
    }
    let ns_per_iter = best.as_nanos() as f64 / iters as f64;
    println!(
        "{label:<62} best of 3: {:>10.3} ms total   {:>10.2} ns/iter",
        best.as_secs_f64() * 1000.0,
        ns_per_iter
    );
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown CPU".to_string())
}

fn os_version() -> String {
    std::fs::read_to_string("/proc/version")
        .ok()
        .map(|s| s.lines().next().unwrap_or("").to_string())
        .unwrap_or_else(|| "unknown OS".to_string())
}

fn free_port() -> u16 {
    // Bind :0, read back the assigned port, drop the listener -- the
    // same "ask the OS for a free one" idiom `tests/serve.rs` and
    // `tests/observability_layer2a.rs` already use elsewhere in this
    // repo, avoiding a hardcoded port this harness might collide on.
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn main() {
    println!("kernel_bench -- Phase 0 harness for rfcs/0007-apm-runtime-kernel.md");
    println!("CPU: {}", cpu_model());
    println!("{}", os_version());
    println!();

    // ---- 1. TCP connect + close ------------------------------------
    // The RFC's "boundary lease reservation" SLO (p99 <= 50us) targets
    // exactly this kind of call -- connect is the natural admission
    // point for the compiled path today (RFC 0007 SS4.2), since no
    // classified-request boundary exists yet.
    {
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let acceptor = std::thread::spawn(move || {
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => drop(s),
                    Err(_) => std::thread::yield_now(),
                }
            }
        });
        std::thread::sleep(Duration::from_millis(20));

        bench("1a. raw std::net::TcpStream::connect + drop", 5_000, || {
            let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            drop(s);
        });

        let host = "127.0.0.1";
        bench("1b. nir_tcp_connect + nir_tcp_stop", 5_000, || unsafe {
            let h = nir_tcp_connect(host.as_ptr(), host.len() as i64, port as i64);
            assert!(h >= 0, "nir_tcp_connect failed");
            assert_eq!(nir_tcp_stop(h), 0);
        });

        stop.store(true, Ordering::Relaxed);
        let _ = acceptor.join();
    }

    // ---- 2. TCP accept ----------------------------------------------
    // CAVEAT: accept() blocks until a connection is pending, so this
    // isolates "the accept call once work is available," not accept in
    // total isolation -- a background thread keeps connecting for the
    // duration so the listener's backlog rarely runs dry, the same
    // trade RFC 0006's bench.rs makes explicit for its own #6/#7.
    {
        let port = free_port();
        let raw_listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let connector = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                    drop(s);
                }
            }
        });
        std::thread::sleep(Duration::from_millis(20));

        bench(
            "2. nir_tcp_accept + nir_tcp_stop (CAVEAT: backlog kept warm by a concurrent connector, not isolated)",
            2_000,
            || unsafe {
                // Only measurable via the raw fd -- ManuallyDrop-style
                // reuse of the same listener handle nir_tcp_accept
                // expects (`i64` fd), matching how codegen.rs's own
                // generated calls pass a `tcp_listener` value through.
                use std::os::unix::io::AsRawFd;
                let h = nir_tcp_accept(raw_listener.as_raw_fd() as i64);
                assert!(h >= 0, "nir_tcp_accept failed");
                assert_eq!(nir_tcp_stop(h), 0);
            },
        );

        stop.store(true, Ordering::Relaxed);
        let _ = connector.join();
    }

    // ---- 3. TCP send (writer side, hot path) ------------------------
    // The RFC's "added per-effect admission latency" SLO (p99 <= 100ns)
    // targets exactly this class of call. A background thread drains
    // the peer continuously so nir_tcp_send never blocks on a full
    // kernel socket buffer.
    {
        let (client, server) = {
            let port = free_port();
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let (server, _) = listener.accept().unwrap();
            (client, server)
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let mut drain_side = server.try_clone().unwrap();
        let drainer = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while !stop2.load(Ordering::Relaxed) {
                match drain_side.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });

        let payload = [0u8; 8];
        {
            let mut client_raw = client.try_clone().unwrap();
            bench("3a. raw std::net::TcpStream::write_all (8 bytes)", 200_000, || {
                client_raw.write_all(&payload).unwrap();
            });
        }
        {
            use std::os::unix::io::AsRawFd;
            let h = client.as_raw_fd() as i64;
            bench("3b. nir_tcp_send (8 bytes)", 200_000, || unsafe {
                let n = nir_tcp_send(h, payload.as_ptr(), payload.len() as i64);
                assert_eq!(n, payload.len() as i64);
            });
        }

        drop(client);
        stop.store(true, Ordering::Relaxed);
        let _ = drainer.join();
        drop(server);
    }

    // ---- 4. TCP recv (reader side, hot path) ------------------------
    // CAVEAT: like #2, this needs a concurrent writer keeping the
    // kernel socket buffer non-empty so recv rarely blocks -- the
    // number reported is "recv cost when data is usually already
    // available," not recv cost in total isolation from network wait.
    {
        let (client, server) = {
            let port = free_port();
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let (server, _) = listener.accept().unwrap();
            (client, server)
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let mut writer_side = client.try_clone().unwrap();
        let writer = std::thread::spawn(move || {
            let payload = [0u8; 4096];
            while !stop2.load(Ordering::Relaxed) {
                if writer_side.write_all(&payload).is_err() {
                    break;
                }
            }
        });
        std::thread::sleep(Duration::from_millis(20));

        {
            let mut server_raw = server.try_clone().unwrap();
            let mut buf = [0u8; 64];
            bench(
                "4a. raw std::net::TcpStream::read (64-byte buf, CAVEAT: warmed by a concurrent writer)",
                200_000,
                || {
                    let _ = server_raw.read(&mut buf).unwrap();
                },
            );
        }
        {
            use std::os::unix::io::AsRawFd;
            let h = server.as_raw_fd() as i64;
            let mut buf = [0u8; 64];
            bench(
                "4b. nir_tcp_recv (64-byte buf, CAVEAT: warmed by a concurrent writer)",
                200_000,
                || unsafe {
                    let n = nir_tcp_recv(h, buf.as_mut_ptr(), buf.len() as i64);
                    assert!(n > 0);
                },
            );
        }

        // `drop(client)` alone would not close the socket here -- the
        // writer thread holds its own duplicated fd (`writer_side`) to
        // the same connection, and is very likely blocked inside
        // `write_all` with nothing left to drain it now the timed loop
        // above has stopped reading. `shutdown` acts on the shared
        // socket, not the individual fd, so it unblocks that pending
        // write (with an error) regardless of which duplicate called
        // it -- unlike `drop`, which only closes *this* fd and leaves
        // the socket open as long as any duplicate remains.
        stop.store(true, Ordering::Relaxed);
        client.shutdown(std::net::Shutdown::Both).ok();
        drop(client);
        let _ = writer.join();
        drop(server);
    }

    // ---- 5. File open + close ----------------------------------------
    {
        let dir = std::env::temp_dir().join(format!("kernel_bench_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path_a = dir.join("a.txt");
        let path_b = dir.join("b.txt");
        let path_b_str = path_b.to_str().unwrap();

        bench("5a. raw std::fs::File::create + drop", 20_000, || {
            let f = std::fs::File::create(&path_a).unwrap();
            drop(f);
        });

        let mode = "w";
        bench("5b. nir_file_open(\"w\") + nir_file_stop", 20_000, || unsafe {
            let h = nir_file_open(
                path_b_str.as_ptr(),
                path_b_str.len() as i64,
                mode.as_ptr(),
                mode.len() as i64,
            );
            assert!(h >= 0, "nir_file_open failed");
            assert_eq!(nir_file_stop(h), 0);
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 6. File write / read round trip ------------------------------
    {
        let dir = std::env::temp_dir().join(format!("kernel_bench2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path_raw = dir.join("raw.bin");
        let path_nir = dir.join("nir.bin");
        let payload = [0u8; 4096];

        {
            let mut f = std::fs::File::create(&path_raw).unwrap();
            bench("6a. raw std::fs::File::write_all (4096 bytes)", 20_000, || {
                f.write_all(&payload).unwrap();
            });
        }
        {
            let mut f = std::fs::File::open(&path_raw).unwrap();
            let mut buf = [0u8; 4096];
            bench("6b. raw std::fs::File::read (4096 bytes)", 20_000, || {
                use std::io::{Seek, SeekFrom};
                f.seek(SeekFrom::Start(0)).unwrap();
                let _ = f.read(&mut buf).unwrap();
            });
        }
        {
            let path_str = path_nir.to_str().unwrap();
            let mode = "w";
            let h = unsafe {
                nir_file_open(path_str.as_ptr(), path_str.len() as i64, mode.as_ptr(), mode.len() as i64)
            };
            assert!(h >= 0);
            bench("6c. nir_file_write (4096 bytes)", 20_000, || unsafe {
                let n = nir_file_write(h, payload.as_ptr(), payload.len() as i64);
                assert_eq!(n, payload.len() as i64);
            });
            unsafe { assert_eq!(nir_file_stop(h), 0) };

            // No `nir_file_seek` kernel exists, so unlike 6b (which
            // seeks back to 0 each iteration on one open handle), a
            // read past the first one hits real EOF -- reopening the
            // file fresh each trial is the only way to get 3 real
            // reads instead of 2 EOF returns. Timed manually (not via
            // `bench()`, whose 3-trial loop assumes one long-lived
            // handle) and reported as a single-call-per-trial number,
            // not an amortized one.
            let mode = "r";
            let mut best = Duration::MAX;
            for _ in 0..3 {
                let h = unsafe {
                    nir_file_open(path_str.as_ptr(), path_str.len() as i64, mode.as_ptr(), mode.len() as i64)
                };
                assert!(h >= 0);
                let mut buf = [0u8; 4096];
                let start = Instant::now();
                let n = unsafe { nir_file_read(h, buf.as_mut_ptr(), buf.len() as i64) };
                let elapsed = start.elapsed();
                assert_eq!(n, payload.len() as i64);
                if elapsed < best {
                    best = elapsed;
                }
                unsafe { assert_eq!(nir_file_stop(h), 0) };
            }
            println!(
                "{:<62} best of 3: {:>10.3} ms total   {:>10.2} ns/iter (single call/trial, CAVEAT: excludes the open/close each trial needs -- see comment above)",
                "6d. nir_file_read (4096 bytes)",
                best.as_secs_f64() * 1000.0,
                best.as_nanos() as f64
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    println!();
    println!("Done. Numbers above are the current, zero-admission baseline --");
    println!("compare against rfcs/0007-apm-runtime-kernel.md SS5's targets");
    println!("(~100ns/effect hot path, ~50us boundary reservation) before");
    println!("treating those SLOs as validated for this substrate.");
}
