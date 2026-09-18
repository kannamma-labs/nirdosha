//! Hi owns the v2 app preview process. The app is a real compiled Rust
//! binary; its own `Router::serve` binds the literal port in `main`.
//! Login and role behavior are whatever the generated app registered.
//! Preview waits for the port before reporting success and remembers
//! the first mounted UI route for the preview iframe.
//!
//! One live preview process per `hi` session, matching `hi_window::open`'s
//! own "one process, one project" shape -- a second `:preview` call
//! kills whatever's running first, never runs two at once.

use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Mutex;

/// One managed preview child process. `Drop` kills it, so losing every
/// handle to a `PreviewState` (an explicit [`stop`], or a `restart`
/// replacing it) always cleans the OS process up -- never a leaked
/// server nobody remembers is still bound to a port.
struct PreviewState {
    child: Child,
    port: u16,
    path: String,
}

impl Drop for PreviewState {
    fn drop(&mut self) {
        // Best-effort: a process that already exited on its own (a
        // crash, a bind failure) makes `kill` fail harmlessly here --
        // never a reason to panic during cleanup.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static CURRENT: Mutex<Option<PreviewState>> = Mutex::new(None);

/// Asks the OS for a free local port by binding to port 0 and reading
/// back what it assigned, then dropping the listener -- the standard
/// "let the kernel pick, then reuse the number" trick for handing a
/// free port to a process this one is about to spawn. Inherently a
/// short race (nothing stops another process claiming it in between),
/// but the same race every tool that does this accepts, and the
/// spawned binary's own bind failure (surfaced through [`restart`]'s
/// `Err`) is the honest fallback if it's ever actually lost.
pub fn pick_free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("finding a free local port for the preview server: {e}"))?;
    listener.local_addr().map(|addr| addr.port()).map_err(|e| format!("reading the bound preview port back: {e}"))
}

/// Replaces the current v2 preview process, then waits until the new
/// binary listens on the literal port in its `main().serve(port)` call.
/// A process that exits or never binds returns an error to the UI.
pub fn restart(binary_path: &Path, port: u16, path: &str) -> Result<(), String> {
    let mut guard = CURRENT.lock().map_err(|_| "preview process lock was poisoned by an earlier panic".to_string())?;
    // The new server may use the same literal port as the old one.
    *guard = None;
    let mut child = Command::new(binary_path).spawn().map_err(|e| format!("starting the preview server ({}): {e}", binary_path.display()))?;
    let address = (std::net::Ipv4Addr::LOCALHOST, port);
    for _ in 0..40 {
        if let Some(status) = child.try_wait().map_err(|e| format!("checking preview process: {e}"))? {
            return Err(format!("preview server exited before listening on port {port} ({status})"));
        }
        if TcpStream::connect(address).is_ok() {
            *guard = Some(PreviewState { child, port, path: path.to_string() });
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(format!("preview server did not listen on port {port} within 2 seconds"))
}

/// The currently running preview's port, if any -- `hi_api`'s own
/// `/api/preview/status` route, and what the Preview rail polls after
/// a Start/Rebuild to know when the iframe has something to point at.
pub fn status() -> Option<u16> {
    let mut guard = CURRENT.lock().ok()?;
    let exited = guard.as_mut().and_then(|state| state.child.try_wait().ok()).flatten().is_some();
    if exited { *guard = None; }
    guard.as_ref().map(|state| state.port)
}

pub fn path() -> Option<String> {
    status()?;
    CURRENT.lock().ok()?.as_ref().map(|state| state.path.clone())
}

/// Stops the current preview process, if any -- `hi_api`'s own
/// `/api/preview/stop` route, and called from `hi_window::open` right
/// after its event loop returns, so closing the `hi` window doesn't
/// leave an orphaned server bound to a port behind it.
pub fn stop() {
    if let Ok(mut guard) = CURRENT.lock() {
        *guard = None;
    }
}

/// `CURRENT` above is one `static`, process-wide -- shared not just by
/// every test in this module but also by `hi_api::tests`' own
/// `/api/preview/*` tests (`cargo test`'s default parallelism runs
/// every `#[test]` in the binary concurrently by default). Held for a
/// whole test body, same pattern `hi_llm.rs`'s own `COVERAGE_TESTS_LOCK`
/// already uses for its shared state, so two preview-touching tests
/// never race each other's `restart`/`stop`/`status` calls.
#[cfg(test)]
pub(crate) static PREVIEW_TESTS_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // No test spawns a real preview child here: doing so would need a
    // real compiled `compiled-serve`-linked binary on disk, which is
    // exactly what `hi_api::tests` already builds and exercises for
    // `handle_publish`'s own plain (non-serve) codegen path -- this
    // module's own tests stick to what's real and fast without one.

    #[test]
    fn pick_free_port_returns_a_nonzero_port() {
        let port = pick_free_port().expect("the OS should always hand back a free ephemeral port in a test sandbox");
        assert_ne!(port, 0);
    }

    #[test]
    fn status_and_stop_are_both_fine_with_nothing_running() {
        let _g = PREVIEW_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Every other test in this process may have already left a
        // preview process registered in the same `static CURRENT` --
        // `stop()` first makes this test's own assertions meaningful
        // regardless of run order, rather than assuming a pristine
        // static.
        stop();
        assert_eq!(status(), None);
        stop(); // idempotent
        assert_eq!(status(), None);
    }
}
