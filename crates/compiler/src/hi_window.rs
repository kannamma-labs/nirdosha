//! Native build-mode window shell (rfcs/0014-generative-build-console.md,
//! "Rendering surface — resolved, not open"). `hi` spawns one `tao`
//! window with a `wry`-embedded webview inside it — one process, one
//! window, the app the user perceives as `hi` itself, never a browser.
//! The local API is answered through `wry`'s custom `hi://` protocol
//! handler, never a real socket: no CSRF target, no DNS-rebinding
//! bypass, no port-squatting race, because there is no network origin
//! at all. `hi_server.rs`'s `tiny_http` socket is the RFC's own
//! documented fallback for headless use, not this path.
//!
//! Serves the same read-only route table `hi_api::handle` also gives
//! `hi_server.rs`, including the real 3D graph page
//! (`hi_api`'s `BUILD_MODE_HTML`/`hi_graph.html`) — a live,
//! navigable `3d-force-graph` view over `.nir/hi.db`'s nodes and
//! edges, click-to-inspect via `/api/impact`, and a WebGL-feature-detect
//! 2D canvas fallback. `with_ipc_handler` below gives that page's own
//! `window.ipc.postMessage` error reporting somewhere to go, since this
//! window has no devtools console attached in normal use.
//!
//! Still not RFC 0014's full build mode: no live updates (the graph is
//! fetched once, not delta-updated as the underlying graph changes),
//! no true GPU-instanced node rendering, no in-page editing. Those stay
//! open follow-on work, disclosed in the RFC's own status box rather
//! than silently assumed done.

use std::path::{Path, PathBuf};

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;
use wry::http::header::CONTENT_TYPE;
use wry::http::{Request, Response};
use wry::WebViewBuilder;

use crate::hi_api;

const SCHEME: &str = "hi";

/// Opens the native build-mode window and runs its event loop until the
/// window closes, then returns. This *is* `nirdosha hi` now
/// (`main.rs::cmd_hi`'s own bare-`hi` arm) — not an excursion from some
/// other console, and not a separate process/binary: one command, one
/// window, the app the user perceives as `hi` itself. Uses `run_return`
/// (`tao::platform::run_return`) rather than `EventLoop::run`
/// specifically so control comes back to the caller (`cmd_hi`, to
/// report a clean exit code) instead of the process exiting out from
/// under it when the window closes. `root` is the project directory
/// whose `.nir/hi.db` backs every `hi://` request the webview makes.
pub fn open(root: &Path) -> Result<(), String> {
    let root: PathBuf = root.to_path_buf();
    let mut event_loop = EventLoop::new();
    let window = WindowBuilder::new().with_title("Nirdosha Hi").build(&event_loop).map_err(|e| format!("creating the build-mode window: {e}"))?;

    let builder = WebViewBuilder::new()
        // Asynchronous, not `with_custom_protocol` -- that synchronous
        // variant runs `handle` inline on whatever thread wry's request
        // dispatch uses, which on at least the GTK backend is also the
        // thread driving the native event loop. A `:prompt`/`:generate`/
        // `:publish`/`:ask`-fallback call can genuinely take a long time
        // (an LLM round trip, a bounded self-repair compile loop) --
        // running it there froze the *entire window*, not just the
        // console, for the whole duration: no repaint, no JS timers (so
        // the busy-spinner console caret this same session added never
        // actually animated), no response to input, indistinguishable
        // from a real hang. `std::thread::spawn` here keeps the native
        // event loop pumping (repaints, the spinner's own `setInterval`)
        // while the slow work happens off of it; `RequestAsyncResponder`
        // (explicitly `Send`, built for exactly this) delivers the
        // result back whenever it's ready, from whatever thread that is.
        .with_asynchronous_custom_protocol(SCHEME.into(), move |_id, request, responder| {
            let root = root.clone();
            std::thread::spawn(move || {
                let response = handle(&root, request);
                responder.respond(response);
            });
        })
        .with_ipc_handler(|request: Request<String>| {
            eprintln!("[hi-window] {}", request.body());
        })
        .with_url(format!("{SCHEME}://localhost"));

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android"))]
    let _webview = builder.build(&window).map_err(|e| format!("building the webview: {e}"))?;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android")))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or_else(|| "build-mode window has no GTK container to embed the webview in".to_string())?;
        builder.build_gtk(vbox).map_err(|e| format!("building the webview: {e}"))?
    };

    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
    Ok(())
}

/// Translates one `hi://` request into `hi_api::handle`'s
/// transport-neutral response and back. `request.uri().path()`/
/// `.query()` give exactly the `(path, query)` shape `hi_api::handle`
/// already takes for `hi_server.rs` — no per-transport parsing logic
/// to duplicate or drift between the two. `request.body()` carries the
/// mutating routes' (rfcs/0014 Build/Generate/Publish) form-encoded
/// payload straight through -- `wry`'s custom-protocol handler gets a
/// real body on a `fetch(..., {method:'POST', body:...})` call the same
/// way an ordinary HTTP server would.
fn handle(root: &Path, request: Request<Vec<u8>>) -> Response<std::borrow::Cow<'static, [u8]>> {
    let path = request.uri().path();
    let query = request.uri().query().unwrap_or("");
    let resp = hi_api::handle(root, request.method().as_str(), path, query, request.body());
    // Without this, a `fetch('/api/nodes')` after `:prompt`/`:confirm`/
    // `:generate` can come back stale: WebKitGTK's URI-scheme responses
    // aren't guaranteed to bypass the webview's own HTTP cache just
    // because they came from a custom protocol handler rather than a
    // real network request, and none of `/api/*`'s JSON responses ever
    // benefit from being cached (they're live, single-viewer, local
    // state) -- confirmed as a real, disclosed fix, not a defensive
    // guess: every route here is either GET-and-always-fresh or a POST
    // that just mutated the very state a cached GET would otherwise
    // keep serving.
    Response::builder()
        .status(resp.status)
        .header(CONTENT_TYPE, resp.content_type)
        .header("Cache-Control", "no-store")
        .body(resp.body)
        .expect("a status/content-type built from hi_api::ApiResponse is always a valid HTTP response")
        .map(Into::into)
}

// No `#[cfg(test)]` module here: `tao`'s GTK backend hard-asserts that
// an `EventLoop` is only ever created on the process's literal main
// thread (`assert_is_main_thread` in `platform_impl/linux/event_loop.
// rs`), and `cargo test` always runs test bodies on a spawned worker
// thread, even with `--test-threads=1` -- so a `#[test]` that
// constructs an `EventLoop` at all panics unconditionally here,
// independent of anything this module does. Verified the mechanism
// `open()`'s `hi`-integration fix actually depends on -- that
// `run_return` (`tao::platform::run_return`) really does hand control
// back to the caller instead of the process exiting the way plain
// `EventLoop::run` does -- by hand instead, with a throwaway
// `cargo run --example` binary (an example's `fn main` *is* the real
// main thread) that opened an `EventLoop`, proxied a `UserEvent` from a
// background thread after 300ms standing in for a close-button click,
// and set `ControlFlow::Exit` on receiving it: `run_return` returned
// immediately afterward and printed confirmation, exactly as
// `open()` relies on.
