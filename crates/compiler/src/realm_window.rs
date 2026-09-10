//! Native build-mode window shell (rfcs/0014-generative-build-console.md,
//! "Rendering surface — resolved, not open"). `hi` spawns one `tao`
//! window with a `wry`-embedded webview inside it — one process, one
//! window, the app the user perceives as `hi` itself, never a browser.
//! The local API is answered through `wry`'s custom `hi://` protocol
//! handler, never a real socket: no CSRF target, no DNS-rebinding
//! bypass, no port-squatting race, because there is no network origin
//! at all. `realm_server.rs`'s `tiny_http` socket is the RFC's own
//! documented fallback for headless use, not this path.
//!
//! Serves the same read-only route table `realm_api::handle` also gives
//! `realm_server.rs`, including the real 3D graph page
//! (`realm_api`'s `BUILD_MODE_HTML`/`realm_graph.html`) — a live,
//! navigable `3d-force-graph` view over `.nir/realm.db`'s nodes and
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
use tao::window::WindowBuilder;
use wry::http::header::CONTENT_TYPE;
use wry::http::{Request, Response};
use wry::WebViewBuilder;

use crate::realm_api;

const SCHEME: &str = "hi";

/// Opens the native build-mode window and runs its event loop. Blocks
/// for the life of the window (`tao::EventLoop::run` never returns);
/// `root` is the project directory whose `.nir/realm.db` backs every
/// `hi://` request the webview makes.
pub fn open(root: &Path) -> Result<(), String> {
    let root: PathBuf = root.to_path_buf();
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().with_title("Nirdosha Realm").build(&event_loop).map_err(|e| format!("creating the build-mode window: {e}"))?;

    let builder = WebViewBuilder::new()
        .with_custom_protocol(SCHEME.into(), move |_id, request| handle(&root, request))
        .with_ipc_handler(|request: Request<String>| {
            eprintln!("[realm-window] {}", request.body());
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

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}

/// Translates one `hi://` request into `realm_api::handle`'s
/// transport-neutral response and back. `request.uri().path()`/
/// `.query()` give exactly the `(path, query)` shape `realm_api::handle`
/// already takes for `realm_server.rs` — no per-transport parsing logic
/// to duplicate or drift between the two.
fn handle(root: &Path, request: Request<Vec<u8>>) -> Response<std::borrow::Cow<'static, [u8]>> {
    let path = request.uri().path();
    let query = request.uri().query().unwrap_or("");
    let resp = realm_api::handle(root, request.method().as_str(), path, query);
    Response::builder().status(resp.status).header(CONTENT_TYPE, resp.content_type).body(resp.body).expect("a status/content-type built from realm_api::ApiResponse is always a valid HTTP response").map(Into::into)
}
