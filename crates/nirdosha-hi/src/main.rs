use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use nirdosha_hi::mcp_tools::{McpCallLog, tools_call, tools_list};

/// `nirdosha-hi`'s entry point -- extracted out of the native `nirdosha`
/// compiler binary's `hi`/`mcp` subcommands (2026-09-16), so `hi` and
/// the v2 MCP tool surface it shares with `nirdosha-hi mcp` never
/// depend on the native `.nir` compiler crate at all. Bare `nirdosha-hi`
/// opens the native build-mode window (same as bare `nirdosha hi`
/// used to); every other subcommand below matches what `nirdosha hi
/// <subcommand>` used to dispatch to, verbatim.
fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error resolving the current directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(sub) = args.next() else {
        return cmd_window(&cwd);
    };
    match sub.as_str() {
        "mcp" => cmd_mcp(args),
        "plugin" => cmd_plugin(args),
        "graph" => cmd_typed_graph(args),
        "workflow" => cmd_workflow(args),
        "ingest" | "sync" | "link" | "impact" | "serve" => cmd_graph(&cwd, &sub, args),
        other => {
            eprintln!(
                "unknown `nirdosha-hi` subcommand `{other}` -- usage: nirdosha-hi [ingest|sync|link|impact|serve|plugin|mcp|graph|workflow] ..."
            );
            ExitCode::FAILURE
        }
    }
}

fn cmd_graph(cwd: &Path, sub: &str, mut args: impl Iterator<Item = String>) -> ExitCode {
    let conn = match nirdosha_hi::hi_graph::open(cwd) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = nirdosha_hi::hi_plugin::ensure_default_packs(&conn, cwd) {
        eprintln!("hi: default domain packs failed to install, continuing: {e}");
    }
    if let Err(e) = nirdosha_hi::hi_plugin::reload_installed_packs(&conn, cwd) {
        eprintln!("hi: installed domain packs failed to reload, continuing: {e}");
    }
    match sub {
        "ingest" => {
            let Some(doc) = args.next() else {
                eprintln!("usage: nirdosha-hi ingest <doc.md>");
                return ExitCode::FAILURE;
            };
            match nirdosha_hi::hi_graph::ingest_document(&conn, Path::new(&doc)) {
                Ok(n) => {
                    println!("ingested {n} new chunk(s) from {doc}");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "sync" => {
            let files: Vec<String> = args.collect();
            match nirdosha_hi::hi_graph::sync(&conn, cwd, &files) {
                Ok(r) => {
                    println!(
                        "synced {} file(s): {} unit(s) seen, {} added, {} changed, {} edge(s) flagged possibly_stale, {} screen-nav edge(s) derived",
                        r.files_scanned,
                        r.units_seen,
                        r.units_added,
                        r.units_changed,
                        r.edges_flagged,
                        r.nav_edges
                    );
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "link" => {
            let (Some(req_id), Some(target)) = (args.next(), args.next()) else {
                eprintln!("usage: nirdosha-hi link <requirement-id> <fn|struct|enum:name>");
                return ExitCode::FAILURE;
            };
            match nirdosha_hi::hi_graph::link(&conn, &req_id, &target) {
                Ok(()) => {
                    println!("linked {req_id} <-> {target}");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "impact" => {
            let Some(target) = args.next() else {
                eprintln!("usage: nirdosha-hi impact <target>");
                return ExitCode::FAILURE;
            };
            match nirdosha_hi::hi_graph::impact(&conn, &target) {
                Ok(report) => {
                    print!("{}", format_impact_report(&target, &report));
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "serve" => {
            drop(conn);
            match nirdosha_hi::hi_server::serve(cwd) {
                Ok(handle) => {
                    println!(
                        "hi API listening on http://127.0.0.1:{} (Ctrl+C to stop)",
                        handle.port
                    );
                    loop {
                        std::thread::park();
                    }
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => unreachable!("dispatched only for the four subcommands matched above"),
    }
}

/// One rendering of a bounded impact walk (rfcs/0013), flagged nodes
/// listed first (`hi_graph::impact` already sorts them that way).
fn format_impact_report(target: &str, report: &nirdosha_hi::hi_graph::ImpactReport) -> String {
    if report.hits.is_empty() {
        return format!(
            "no reachable nodes from `{target}` -- try `nirdosha-hi link` or `nirdosha-hi sync` first.\n"
        );
    }
    let mut out = format!(
        "impact of `{target}` ({} node(s){}):\n",
        report.hits.len(),
        if report.partial {
            ", partial -- bound reached"
        } else {
            ""
        }
    );
    for h in &report.hits {
        let flag = h
            .flag
            .as_deref()
            .map(|f| format!("  [{f}]"))
            .unwrap_or_default();
        let location = match (&h.source_ref, h.line, h.col) {
            (Some(path), Some(line), Some(col)) => format!("  ({path}:{line}:{col})"),
            _ => String::new(),
        };
        out.push_str(&format!(
            "  depth {} {} {} `{}`{location}{flag}\n",
            h.depth,
            h.kind,
            h.edge_kind,
            h.title.as_deref().unwrap_or(&h.node_id)
        ));
    }
    out
}

/// Bare `nirdosha-hi`: auto-scaffolds/syncs `.nir/hi.db` (best-effort --
/// a sync problem degrades to a logged warning, never blocks the window
/// from opening), then starts the headless `hi_server.rs` and opens its
/// URL in a Chromium-family browser's `--app=` mode window (no address
/// bar, tabs, or menu) if one is found on `PATH`, printing the URL to
/// open by hand otherwise. Blocks (via `park()`) until Ctrl+C -- the
/// window is meant to run alongside this process, not in place of it
/// (see `launch_app_window`'s own doc comment). `NIRDOSHA_HI_DISABLE=1`
/// skips the scaffold/sync step entirely.
fn cmd_window(cwd: &Path) -> ExitCode {
    if !nirdosha_hi::graph_transport::is_typed(cwd)
        && !nirdosha_hi::hi_graph::is_disabled(&|k| std::env::var(k).ok())
    {
        match nirdosha_hi::hi_graph::open(cwd) {
            Ok(conn) => {
                if let Err(e) = nirdosha_hi::hi_plugin::ensure_default_packs(&conn, cwd) {
                    eprintln!("hi: default domain packs failed to install, continuing: {e}");
                }
                if let Err(e) = nirdosha_hi::hi_plugin::reload_installed_packs(&conn, cwd) {
                    eprintln!("hi: installed domain packs failed to reload, continuing: {e}");
                }
                if let Err(e) = nirdosha_hi::hi_graph::sync(&conn, cwd, &[]) {
                    eprintln!("hi: sync failed, continuing with a possibly-stale graph: {e}");
                }
            }
            Err(e) => eprintln!(
                "hi: couldn't open .nir/hi.db, continuing without it ({}=1 to silence this): {e}",
                nirdosha_hi::hi_graph::HI_DISABLE_VAR
            ),
        }
    }
    match nirdosha_hi::hi_server::serve(cwd) {
        Ok(handle) => {
            let url = format!("http://127.0.0.1:{}/", handle.port);
            if launch_app_window(&url) {
                println!(
                    "hi running at {url} (opened in a browser app window -- Ctrl+C here to stop)"
                );
            } else {
                println!(
                    "hi API listening on {url} -- no Chromium-family browser found on PATH to open it as an app window; open that URL yourself. (Ctrl+C to stop)"
                );
            }
            loop {
                std::thread::park();
            }
        }
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// Opens `url` in a Chromium-family browser's `--app=` mode -- a plain
/// window with no address bar, tabs, bookmarks bar, or menu: this app's
/// one rendering surface (RFC 0014's original embedded-`wry`/`tao`
/// webview was retired in favor of this -- see that RFC's 2026-09-20
/// amendment). Deliberately not an embedded webview: this adds zero
/// new build-time dependencies (no GTK/WebKitGTK) by execing whatever
/// browser the user already has installed, rather than embedding one.
/// Firefox's own site-specific-browser support has been inconsistent
/// across versions and is deliberately not in this list -- a Chromium-
/// family browser (which all support `--app=`) is a safe, common
/// assumption on a Linux dev machine; a caller with none of these
/// installed just gets the plain URL to open by hand (this function's
/// own `false` return, handled by `cmd_window` above). Returns whether
/// a candidate browser was actually found and spawned -- never blocks
/// waiting for it to exit, since the window is meant to run alongside
/// this process, not in place of it.
fn launch_app_window(url: &str) -> bool {
    const CANDIDATES: &[&str] = &[
        "google-chrome-stable",
        "google-chrome",
        "chromium",
        "chromium-browser",
        "microsoft-edge-stable",
        "microsoft-edge",
        "brave-browser",
    ];
    CANDIDATES.iter().any(|browser| {
        Command::new(browser)
            .arg(format!("--app={url}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    })
}

fn write_mcp_message(stdout: &mut impl std::io::Write, value: &serde_json::Value) {
    let _ = writeln!(
        stdout,
        "{}",
        serde_json::to_string(value).expect("an MCP response always serializes")
    );
    let _ = stdout.flush();
}

/// Routes one already-parsed JSON-RPC message. Returns `None` for a
/// notification (`id` absent from the original request) -- per the MCP
/// stdio transport spec the server must never write a response for one.
fn mcp_dispatch(
    method: &str,
    params: &serde_json::Value,
    id: Option<&serde_json::Value>,
    log: &mut McpCallLog,
) -> Option<serde_json::Value> {
    let id = id?.clone();
    let result = match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "nirdosha-hi", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => tools_call(params, log),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

/// `nirdosha-hi mcp` -- the v2 MCP tool surface's stdio transport
/// (JSON-RPC 2.0, newline-delimited). Every tool call (`tools_call`)
/// reuses the identical dispatcher `nirdosha-hi`'s own embedded
/// self-repair loop calls in-process -- one implementation, one call
/// log (`McpCallLog`, surface `"mcp-stdio"`), regardless of transport.
fn cmd_mcp(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match nirdosha_hi::graph_transport::Options::parse(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e.envelope());
            return ExitCode::FAILURE;
        }
    };
    let mut project = match options.open().and_then(|g| {
        g.map(nirdosha_hi::graph_transport::Session::new)
            .transpose()
    }) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e.envelope());
            return ExitCode::FAILURE;
        }
    };
    let mut log = McpCallLog::new("mcp-stdio");
    eprintln!("[nirdosha-hi mcp] tool-call log: {}", log.path().display());
    log.log_session_start(serde_json::json!({
        "protocol": "mcp-stdio-2025-06-18",
        "serverInfo": { "name": "nirdosha-hi", "version": env!("CARGO_PKG_VERSION") },
    }));
    let (sender, receiver) = std::sync::mpsc::sync_channel(32);
    std::thread::spawn(move || {
        for line in std::io::BufRead::lines(std::io::stdin().lock()) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut stdout = std::io::stdout();
    loop {
        if let Some(project) = &mut project {
            for notification in project.notifications() {
                write_mcp_message(&mut stdout, &notification);
            }
        }
        let line = match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(Ok(line)) => line,
            Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: serde_json::Value = match nirdosha_graph::hash::parse(line) {
            Ok(v) => v,
            Err(e) => {
                write_mcp_message(
                    &mut stdout,
                    &serde_json::json!({ "jsonrpc": "2.0", "id": serde_json::Value::Null, "error": { "code": -32700, "message": format!("parse error: {}", e.message) } }),
                );
                continue;
            }
        };
        let id = request.get("id");
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let empty_params = serde_json::Value::Null;
        let params = request.get("params").unwrap_or(&empty_params);
        if let (Some(project), Some(id)) = (&mut project, id) {
            if let Some(mut response) = project.dispatch(method, params) {
                response["jsonrpc"] = serde_json::json!("2.0");
                response["id"] = id.clone();
                write_mcp_message(&mut stdout, &response);
                continue;
            }
        }
        if let Some(mut response) = mcp_dispatch(method, params, id, &mut log) {
            if method == "initialize" && project.is_some() {
                response["result"]["capabilities"]["resources"] =
                    serde_json::json!({"subscribe":true,"listChanged":false});
            }
            write_mcp_message(&mut stdout, &response);
        }
    }
    ExitCode::SUCCESS
}
/// Initialization/migration are explicit host operations, never implicit reads.
fn cmd_typed_graph(mut args: impl Iterator<Item = String>) -> ExitCode {
    let action = args.next().unwrap_or_default();
    let result = (|| -> nirdosha_graph::Result<serde_json::Value> {
        let options = nirdosha_hi::graph_transport::Options::parse(args)?;
        let root = options
            .project
            .ok_or_else(|| nirdosha_graph::Error::new("SCHEMA_INVALID", "Specify --project"))?;
        let graph = match action.as_str() {
            "init" => nirdosha_graph::Graph::initialize(
                &root,
                &options.state,
                nirdosha_graph::store::Access::reviewer("local"),
            )?,
            "migrate" => nirdosha_graph::Graph::migrate(
                &root,
                &options.state,
                nirdosha_graph::store::Access::reviewer("local"),
            )?,
            _ => {
                return Err(nirdosha_graph::Error::new(
                    "SCHEMA_INVALID",
                    "Usage: nirdosha-hi graph init|migrate --project PATH [--state-dir PATH]",
                ));
            }
        };
        Ok(serde_json::json!({"graph":graph.version()?}))
    })();
    match result {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.envelope());
            ExitCode::FAILURE
        }
    }
}

/// `nirdosha-hi workflow ...` -- RFC 0021.b's approval/workflow
/// runtime (`nirdosha-workflow`), wired in as its own CLI subcommand
/// family rather than folded into `graph`'s MCP transport: the RFC's
/// own status header calls it "independent delivery work, not a phase
/// of the core graph service," and its trust boundary is genuinely
/// different (`install_mapping`/`deploy`'s own doc comments: "trusted
/// operator boundary," "do not expose this function to application
/// users" -- these are host/operator actions, not LLM-agent-driven
/// graph authoring the way `graph`'s MCP tools are). Every action
/// shares `--db PATH --authority FILE` (the runtime's own SQLite file
/// and its `Authority` JSON config); action-specific arguments follow,
/// structured JSON ones (state/mapping/event/data/evidence) always as
/// `--flag FILE`, matching `graph`'s own file-based-argument
/// convention rather than inline JSON shell-escaping. No hook adapter
/// is registered (`NoHooks`) -- a workflow whose `on_entry`/`on_exit`
/// spec actually names a hook function fails with
/// `UNSUPPORTED_TARGET`, honestly, rather than silently no-op'ing; a
/// real host embedding this runtime supplies its own `Hooks` impl
/// directly against the library, not through this CLI.
fn cmd_workflow(mut args: impl Iterator<Item = String>) -> ExitCode {
    let action = args.next().unwrap_or_default();
    match workflow_dispatch(&action, args) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.envelope());
            ExitCode::FAILURE
        }
    }
}

/// `cmd_workflow`'s own logic, factored out so tests can call it
/// directly and assert on the real `Value`/`Error` it returns, instead
/// of parsing captured stdout/stderr from a subprocess.
fn workflow_dispatch(
    action: &str,
    mut args: impl Iterator<Item = String>,
) -> nirdosha_graph::Result<serde_json::Value> {
    (|| -> nirdosha_graph::Result<serde_json::Value> {
        let mut db: Option<std::path::PathBuf> = None;
        let mut authority_path: Option<std::path::PathBuf> = None;
        let mut rest: Vec<String> = Vec::new();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--db" => {
                    db = Some(
                        args.next()
                            .ok_or_else(|| {
                                nirdosha_graph::Error::new(
                                    "SCHEMA_INVALID",
                                    "Missing value for --db",
                                )
                            })?
                            .into(),
                    )
                }
                "--authority" => {
                    authority_path = Some(
                        args.next()
                            .ok_or_else(|| {
                                nirdosha_graph::Error::new(
                                    "SCHEMA_INVALID",
                                    "Missing value for --authority",
                                )
                            })?
                            .into(),
                    )
                }
                _ => {
                    rest.push(flag);
                    if let Some(v) = args.next() {
                        rest.push(v);
                    }
                }
            }
        }
        let db =
            db.ok_or_else(|| nirdosha_graph::Error::new("SCHEMA_INVALID", "Specify --db PATH"))?;
        let authority_path = authority_path.ok_or_else(|| {
            nirdosha_graph::Error::new("SCHEMA_INVALID", "Specify --authority FILE")
        })?;
        let authority: nirdosha_workflow::Authority = read_json(&authority_path)?;
        let runtime = nirdosha_workflow::Runtime::open(&db, authority)?;
        let flag = |name: &str| {
            rest.iter()
                .position(|f| f == name)
                .and_then(|i| rest.get(i + 1))
                .cloned()
        };
        let required_flag = |name: &str| -> nirdosha_graph::Result<String> {
            flag(name).ok_or_else(|| {
                nirdosha_graph::Error::new("SCHEMA_INVALID", format!("Specify {name}"))
            })
        };
        let number = |name: &str| -> nirdosha_graph::Result<u64> {
            required_flag(name)?.parse().map_err(|_| {
                nirdosha_graph::Error::new("SCHEMA_INVALID", format!("{name} must be a number"))
            })
        };
        match action {
            "capabilities" => Ok(runtime.capabilities()),
            "deploy" => {
                let state: nirdosha_graph::schema::State =
                    read_json(std::path::Path::new(&required_flag("--state")?))?;
                Ok(serde_json::json!({"definition_id": runtime.deploy(&state)?}))
            }
            "install-mapping" => {
                let mapping: nirdosha_workflow::Mapping =
                    read_json(std::path::Path::new(&required_flag("--mapping")?))?;
                runtime.install_mapping(&mapping)?;
                Ok(serde_json::json!({"installed": true}))
            }
            "issue-credential" => Ok(
                serde_json::json!({"credential": runtime.issue_credential(&required_flag("--issuer")?, &required_flag("--tenant")?, &required_flag("--subject")?, number("--expires")?)?}),
            ),
            "start" => {
                let data: serde_json::Value =
                    read_json(std::path::Path::new(&required_flag("--data")?))?;
                runtime.start(
                    &required_flag("--definition")?,
                    &required_flag("--workflow")?,
                    &required_flag("--credential")?,
                    data,
                    &required_flag("--key")?,
                    &nirdosha_workflow::NoHooks,
                )
            }
            "event" => {
                let event: nirdosha_workflow::Event =
                    read_json(std::path::Path::new(&required_flag("--event")?))?;
                runtime.event(
                    &required_flag("--credential")?,
                    &event,
                    &nirdosha_workflow::NoHooks,
                )
            }
            "instance" => Ok(serde_json::to_value(
                runtime.instance(&required_flag("--id")?)?,
            )?),
            "outbox" => {
                Ok(serde_json::json!({"outbox": runtime.outbox(&required_flag("--instance")?)?}))
            }
            "claim-outbox" => Ok(runtime
                .claim_outbox(number("--lease-seconds")?)?
                .unwrap_or(serde_json::Value::Null)),
            "complete-outbox" => {
                let evidence: serde_json::Value =
                    read_json(std::path::Path::new(&required_flag("--evidence")?))?;
                runtime.complete_outbox(
                    &required_flag("--id")?,
                    &required_flag("--token")?,
                    &required_flag("--outcome")?,
                    &evidence,
                )?;
                Ok(serde_json::json!({"completed": true}))
            }
            "reconcile-outbox" => {
                let evidence: serde_json::Value =
                    read_json(std::path::Path::new(&required_flag("--evidence")?))?;
                runtime.reconcile_outbox(
                    &required_flag("--id")?,
                    &required_flag("--outcome")?,
                    &evidence,
                )?;
                Ok(serde_json::json!({"reconciled": true}))
            }
            "migrate-instance" => {
                let expected_revision = number("--expected-revision")?;
                let data: serde_json::Value =
                    read_json(std::path::Path::new(&required_flag("--data")?))?;
                runtime.migrate_instance(
                    &required_flag("--key")?,
                    &required_flag("--instance")?,
                    expected_revision,
                    &required_flag("--definition")?,
                    &required_flag("--workflow")?,
                    &required_flag("--target")?,
                    data,
                    &required_flag("--operator-credential")?,
                    &nirdosha_workflow::NoHooks,
                )
            }
            _ => Err(nirdosha_graph::Error::new(
                "SCHEMA_INVALID",
                "Usage: nirdosha-hi workflow <capabilities|deploy|install-mapping|issue-credential|start|event|instance|outbox|claim-outbox|complete-outbox|reconcile-outbox|migrate-instance> --db PATH --authority FILE ...",
            )),
        }
    })()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> nirdosha_graph::Result<T> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        nirdosha_graph::Error::new("SCHEMA_INVALID", format!("reading {}: {e}", path.display()))
    })?;
    serde_json::from_str(&text).map_err(|e| {
        nirdosha_graph::Error::new("SCHEMA_INVALID", format!("parsing {}: {e}", path.display()))
    })
}

/// `nirdosha-hi plugin install`'s shape-based dispatch: a signed envelope
/// (`nirdosha-hi plugin sign`'s own output) is a JSON object carrying
/// `manifest_json`/`signature`/`public_key` at the top level, which no
/// plain pack manifest does (a `PackManifest` has `id`/`name`/
/// `invariants`/... instead) -- `None` for anything that doesn't parse
/// as that exact shape, so a plain manifest falls straight through to
/// the existing unsigned install path unchanged.
fn parse_signed_pack_envelope(bytes: &[u8]) -> Option<nirdosha_hi::hi_plugin::SignedPackEnvelope> {
    serde_json::from_slice(bytes).ok()
}

fn cmd_plugin(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(sub) = args.next() else {
        eprintln!(
            "usage: nirdosha-hi plugin install [--dry-run] <pack.json> | sign <pack.json> --key <key.pk8> --identity <name> | list | revoke <pack-id>"
        );
        return ExitCode::FAILURE;
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error resolving the current directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    let conn = match nirdosha_hi::hi_graph::open(&cwd) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    match sub.as_str() {
        "install" => {
            let mut dry_run = false;
            let mut path: Option<String> = None;
            for arg in args {
                if arg == "--dry-run" {
                    dry_run = true;
                } else if path.is_none() {
                    path = Some(arg);
                } else {
                    eprintln!("usage: nirdosha-hi plugin install [--dry-run] <pack.json>");
                    return ExitCode::FAILURE;
                }
            }
            let Some(path) = path else {
                eprintln!("usage: nirdosha-hi plugin install [--dry-run] <pack.json>");
                return ExitCode::FAILURE;
            };
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("reading {path}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if dry_run {
                match nirdosha_hi::hi_plugin::dry_run_install(
                    &conn,
                    &cwd,
                    &bytes,
                    &format!("dry-run {path}"),
                ) {
                    Ok(id) => {
                        println!(
                            "dry-run ok: pack {id} from {path} (load + own contracts proved against a stub program)"
                        );
                        ExitCode::SUCCESS
                    }
                    Err(msg) => {
                        eprintln!("dry-run failed: {msg}");
                        ExitCode::FAILURE
                    }
                }
            } else if let Some(envelope) = parse_signed_pack_envelope(&bytes) {
                // RFC 0016 Phase 4: a signed envelope (`nirdosha-hi plugin
                // sign`'s own output) installs through the verify-then-
                // install path instead of the plain one -- detected by
                // shape (this file carries `manifest_json`/`signature`/
                // `public_key` at the top level, which no plain pack
                // manifest does), not by a separate flag the caller has
                // to remember to pass.
                match nirdosha_hi::hi_plugin::verify_and_install_signed_pack(
                    &conn, &cwd, &envelope, &path,
                ) {
                    Ok(id) => {
                        println!("installed signed pack {id} from {path}");
                        ExitCode::SUCCESS
                    }
                    Err(msg) => {
                        eprintln!("install failed: {msg}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                match nirdosha_hi::hi_plugin::install_pack_from_bytes(&conn, &cwd, &bytes, &path) {
                    Ok(id) => {
                        println!("installed pack {id} from {path}");
                        ExitCode::SUCCESS
                    }
                    Err(msg) => {
                        eprintln!("install failed: {msg}");
                        ExitCode::FAILURE
                    }
                }
            }
        }
        "sign" => {
            let mut key_path: Option<String> = None;
            let mut identity: Option<String> = None;
            let mut out: Option<String> = None;
            let mut manifest_path: Option<String> = None;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--key" => key_path = args.next(),
                    "--identity" => identity = args.next(),
                    "-o" => out = args.next(),
                    other if manifest_path.is_none() => manifest_path = Some(other.to_string()),
                    other => {
                        eprintln!(
                            "unknown argument `{other}` -- usage: nirdosha-hi plugin sign <pack.json> --key <key.pk8> --identity <name> [-o <signed-pack.json>]"
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            let (Some(manifest_path), Some(key_path), Some(identity)) =
                (manifest_path, key_path, identity)
            else {
                eprintln!(
                    "usage: nirdosha-hi plugin sign <pack.json> --key <key.pk8> --identity <name> [-o <signed-pack.json>]"
                );
                return ExitCode::FAILURE;
            };
            let bytes = match std::fs::read(&manifest_path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("reading {manifest_path}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match nirdosha_hi::hi_plugin::sign_pack(&bytes, &key_path, identity) {
                Ok(envelope) => {
                    let json = serde_json::to_string_pretty(&envelope)
                        .expect("SignedPackEnvelope always serializes");
                    let out_path = out.unwrap_or_else(|| format!("{manifest_path}.signed.json"));
                    if let Err(e) = std::fs::write(&out_path, &json) {
                        eprintln!("writing {out_path}: {e}");
                        return ExitCode::FAILURE;
                    }
                    println!(
                        "wrote {out_path} -- install it with `nirdosha-hi plugin install {out_path}`"
                    );
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("signing failed: {msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "list" => match nirdosha_hi::hi_plugin::list_packs(&conn) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        "revoke" => {
            let Some(id) = args.next() else {
                eprintln!("usage: nirdosha-hi plugin revoke <pack-id>");
                return ExitCode::FAILURE;
            };
            match nirdosha_hi::hi_plugin::revoke_pack(&conn, &cwd, &id) {
                Ok(()) => {
                    println!("revoked pack {id}");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("unknown plugin subcommand `{other}` -- use install, sign, list, or revoke");
            ExitCode::FAILURE
        }
    }
}

/// Real end-to-end proof that `nirdosha-hi workflow ...` actually
/// reaches `nirdosha-workflow`'s runtime -- not just that it compiles.
/// Drives the exact six-eyes/maker-exclusion/outbox scenario
/// `nirdosha-workflow/tests/runtime.rs`'s own fixture already proves
/// against the library directly, this time entirely through
/// `workflow_dispatch` (the same function `cmd_workflow`'s real CLI
/// entry point calls), so a regression in the CLI wiring itself --
/// flag parsing, file reads, the `--db`/`--authority` plumbing -- would
/// fail here even if the library's own tests still pass.
#[cfg(test)]
mod workflow_cli_tests {
    use super::*;
    use nirdosha_graph::schema::{Node, State, spec_version};
    use serde_json::{Value, json};

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nirdosha_hi_workflow_cli_test_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_json(dir: &std::path::Path, name: &str, value: &Value) -> String {
        let path = dir.join(name);
        std::fs::write(&path, value.to_string()).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn dispatch(action: &str, flags: &[(&str, &str)]) -> nirdosha_graph::Result<Value> {
        let args = flags
            .iter()
            .flat_map(|(k, v)| [k.to_string(), v.to_string()]);
        workflow_dispatch(action, args)
    }

    fn node(id: &str, kind: &str, spec: Value) -> Node {
        Node {
            id: id.into(),
            kind: kind.into(),
            title: id.into(),
            symbol: None,
            entity_revision: 1,
            deleted: false,
            origin: "test".into(),
            spec_schema_version: spec_version(kind),
            spec,
            source_refs: vec![],
            provenance_ids: vec![],
            protected: false,
            observation: None,
        }
    }

    /// The same fixture `nirdosha-workflow`'s own crate tests use: a
    /// two-slot (`operations`/`compliance`) six-eyes approval policy
    /// gating a single `review -> approved` transition.
    fn definition_state() -> State {
        let mut state = State::default();
        for n in [
            node(
                "w",
                "Workflow",
                json!({"state_ids":["review","approved"],"transition_ids":["finalize"],"initial_state_id":"review"}),
            ),
            node(
                "review",
                "WorkflowState",
                json!({"workflow_id":"w","terminal":"none","approval_policy_id":"approval"}),
            ),
            node(
                "approved",
                "WorkflowState",
                json!({"workflow_id":"w","terminal":"success"}),
            ),
            node(
                "finalize",
                "WorkflowTransition",
                json!({"workflow_id":"w","from_state_id":"review","to_state_id":"approved","event":"finalize","approval_policy_id":"approval"}),
            ),
            node("ops", "Role", json!({"name":"operations"})),
            node("compliance", "Role", json!({"name":"compliance"})),
            node(
                "approval",
                "ApprovalPolicy",
                json!({"identity_basis":"distinct_person","maker_counts":true,"exclude_maker":true,"cross_stage_distinct":true,"decision_lifetime_ms":60000,"rejection":"reject","invalidate_on_change":true,"stages":[{"id":"reviewers","mode":"parallel","quorum":2,"slots":{"operations":{"role_ids":["ops"]},"compliance":{"role_ids":["compliance"]}}}]}),
            ),
        ] {
            state.nodes.insert(n.id.clone(), n);
        }
        state
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn mapping_json(subject: &str, person: &str) -> Value {
        json!({"mapping_schema_version":nirdosha_workflow::identity::MAPPING_SCHEMA_VERSION,"authority_key_id":"company-directory-key-1","audience":"payments","issuer":"company-login","tenant":"company","subject":subject,"person":person,"roles":["ops","compliance"],"claims":[],"revision":1,"revocation_revision":0,"issued_at":now(),"expires_at":now()+110,"revoked":false,"integrity_proof":null})
    }

    #[test]
    fn workflow_cli_drives_a_real_six_eyes_approval_to_completion() {
        let dir = scratch_dir("approval");
        let db = dir.join("wf.db").to_str().unwrap().to_string();
        let authority = write_json(
            &dir,
            "authority.json",
            &json!({"authority_id":"company-directory","key_id":"company-directory-key-1","audience":"payments","tenant":"company","issuers":["company-login"],"max_freshness_seconds":120}),
        );
        let state_file = write_json(
            &dir,
            "state.json",
            &serde_json::to_value(definition_state()).unwrap(),
        );

        let deploy = dispatch(
            "deploy",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--state", &state_file),
            ],
        )
        .unwrap();
        let definition_id = deploy["definition_id"].as_str().unwrap().to_string();

        for (subject, person) in [
            ("maker", "person-m"),
            ("alice", "person-a"),
            ("bob", "person-b"),
        ] {
            let mapping_file = write_json(
                &dir,
                &format!("mapping_{subject}.json"),
                &mapping_json(subject, person),
            );
            dispatch(
                "install-mapping",
                &[
                    ("--db", &db),
                    ("--authority", &authority),
                    ("--mapping", &mapping_file),
                ],
            )
            .unwrap();
        }
        let credential = |subject: &str| -> String {
            dispatch(
                "issue-credential",
                &[
                    ("--db", &db),
                    ("--authority", &authority),
                    ("--issuer", "company-login"),
                    ("--tenant", "company"),
                    ("--subject", subject),
                    ("--expires", &(now() + 100).to_string()),
                ],
            )
            .unwrap()["credential"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let maker = credential("maker");
        let alice = credential("alice");
        let bob = credential("bob");

        let data = write_json(&dir, "data.json", &json!({"amount": 100}));
        let start = dispatch(
            "start",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--definition", &definition_id),
                ("--workflow", "w"),
                ("--credential", &maker),
                ("--data", &data),
                ("--key", "start"),
            ],
        )
        .unwrap();
        let instance_id = start["instance"]["id"].as_str().unwrap().to_string();

        let instance = |db: &str, authority: &str, id: &str| {
            dispatch(
                "instance",
                &[("--db", db), ("--authority", authority), ("--id", id)],
            )
            .unwrap()
        };

        let i = instance(&db, &authority, &instance_id);
        let decide_alice = write_json(
            &dir,
            "decide_alice.json",
            &json!({"key":"a","instance_id":instance_id,"expected_revision":i["revision"],"kind":"decision","payload":{"policy_id":"approval","stage":"reviewers","slot":"operations","decision":"approve","payload_hash":i["payload_hash"],"round":i["round"]}}),
        );
        dispatch(
            "event",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--credential", &alice),
                ("--event", &decide_alice),
            ],
        )
        .unwrap();

        let i = instance(&db, &authority, &instance_id);
        let decide_bob = write_json(
            &dir,
            "decide_bob.json",
            &json!({"key":"b","instance_id":instance_id,"expected_revision":i["revision"],"kind":"decision","payload":{"policy_id":"approval","stage":"reviewers","slot":"compliance","decision":"approve","payload_hash":i["payload_hash"],"round":i["round"]}}),
        );
        dispatch(
            "event",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--credential", &bob),
                ("--event", &decide_bob),
            ],
        )
        .unwrap();

        // The maker cannot approve their own request -- proven the same
        // way the library's own test does, through the CLI this time.
        let i = instance(&db, &authority, &instance_id);
        let decide_maker = write_json(
            &dir,
            "decide_maker.json",
            &json!({"key":"m","instance_id":instance_id,"expected_revision":i["revision"],"kind":"decision","payload":{"policy_id":"approval","stage":"reviewers","slot":"operations","decision":"approve","payload_hash":i["payload_hash"],"round":i["round"]}}),
        );
        assert_eq!(
            dispatch(
                "event",
                &[
                    ("--db", &db),
                    ("--authority", &authority),
                    ("--credential", &maker),
                    ("--event", &decide_maker)
                ]
            )
            .unwrap_err()
            .code,
            "MAKER_EXCLUDED"
        );

        let i = instance(&db, &authority, &instance_id);
        let finalize = write_json(
            &dir,
            "finalize.json",
            &json!({"key":"final","instance_id":instance_id,"expected_revision":i["revision"],"kind":"transition","payload":{"transition_id":"finalize"}}),
        );
        let result = dispatch(
            "event",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--credential", &maker),
                ("--event", &finalize),
            ],
        )
        .unwrap();
        assert_eq!(result["instance"]["state_id"], "approved");

        let outbox = dispatch(
            "outbox",
            &[
                ("--db", &db),
                ("--authority", &authority),
                ("--instance", &instance_id),
            ],
        )
        .unwrap();
        assert_eq!(outbox["outbox"], json!([]));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn workflow_cli_capabilities_reports_the_real_runtime_manifest() {
        let dir = scratch_dir("capabilities");
        let db = dir.join("wf.db").to_str().unwrap().to_string();
        let authority = write_json(
            &dir,
            "authority.json",
            &json!({"authority_id":"a","key_id":"key-1","audience":"aud","tenant":"t","issuers":["iss"],"max_freshness_seconds":60}),
        );
        let caps = dispatch(
            "capabilities",
            &[("--db", &db), ("--authority", &authority)],
        )
        .unwrap();
        assert_eq!(caps["adapter"], "nirdosha-workflow/sqlite-v1");
        // RFC 0021.b: "distinct_person(authority_id, contract_version,
        // freshness_policy)" -- real values, not a bare literal.
        let distinct_person = caps["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|c| c.as_str().filter(|s| s.starts_with("distinct_person(")))
            .unwrap();
        assert!(
            distinct_person.contains("authority_id=a"),
            "{distinct_person}"
        );
        assert!(
            distinct_person.contains(nirdosha_workflow::identity::MAPPING_SCHEMA_VERSION),
            "{distinct_person}"
        );
        assert!(
            distinct_person.contains("freshness_policy=60s"),
            "{distinct_person}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
