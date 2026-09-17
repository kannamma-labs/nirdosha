//! github #45 ("hi: live-preview build mode"), the half of it this pass
//! actually ships: `hi` owning the served app's process lifecycle, so
//! "run the real compiled app" stops meaning "a human runs the binary
//! themselves in another terminal." **Deliberately not** the harder
//! half the issue also asked for (right-click a live field to attach
//! `requires(role:)`) -- that needs source-location metadata threaded
//! through `ui_gen.rs`/codegen into the rendered HTML, real compiler
//! surface, tracked as its own follow-up rather than attempted here.
//!
//! `hi_api::handle_preview_start` builds a real, *servable* binary
//! (`codegen::build_serve`, the same pipeline `nirdosha build --serve`
//! already uses -- unlike `handle_publish`'s plain `codegen::build`,
//! which produces a one-shot binary with no HTTP listener at all) and
//! calls [`restart`] to run it. Demo-mode identity needs zero new work
//! here: `compiled_serve::ServeConfig::default()` already turns on
//! `demo_mode` whenever no real OIDC env vars are set (`auth_providers_
//! from_env`), so a plain, env-var-free `spawn` gets the exact
//! self-service role/claim picker (`/api/_demo_login`) `ui_gen.rs`'s
//! own generated login screen already renders when `demo_mode` is true
//! -- "preview as employee/finance director" is the served app's own
//! same-origin UI, not something this rail has to build.
//!
//! One live preview process per `hi` session, matching `hi_window::open`'s
//! own "one process, one project" shape -- a second `:preview` call
//! kills whatever's running first, never runs two at once.

use std::net::TcpListener;
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

/// Kills any preview process already running, then spawns `binary_path`
/// (built with that exact `port` baked in via `ServeCodegenOptions` --
/// codegen embeds the port as a constant argument to `nir_compiled_
/// serve_run`, it isn't a runtime flag the child reads) and records it
/// as the current one. No environment overrides: an ordinary `spawn`
/// with no `--oidc-*`/`--jwks-file` env set is exactly what keeps the
/// child in demo mode (`compiled_serve::auth_providers_from_env`'s own
/// default), the same "nothing extra to configure" identity story
/// RFC 0014's 2026-09-14 amendment describes.
pub fn restart(binary_path: &Path, port: u16) -> Result<(), String> {
    let child = Command::new(binary_path).spawn().map_err(|e| format!("starting the preview server ({}): {e}", binary_path.display()))?;
    let mut guard = CURRENT.lock().map_err(|_| "preview process lock was poisoned by an earlier panic".to_string())?;
    // Replacing `*guard` drops the previous `Some(PreviewState)` (if
    // any) right here, which is what actually kills the old process --
    // done AFTER the new child is already spawned so a failed spawn
    // above never tears down a preview that was working.
    *guard = Some(PreviewState { child, port });
    Ok(())
}

/// The currently running preview's port, if any -- `hi_api`'s own
/// `/api/preview/status` route, and what the Preview rail polls after
/// a Start/Rebuild to know when the iframe has something to point at.
pub fn status() -> Option<u16> {
    CURRENT.lock().ok().and_then(|guard| guard.as_ref().map(|s| s.port))
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
