use std::path::Path;
use std::process::ExitCode;

use nirdosha_hi::mcp_tools::{tools_call, tools_list, McpCallLog};

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
        "ingest" | "sync" | "link" | "impact" | "serve" => cmd_graph(&cwd, &sub, args),
        other => {
            eprintln!("unknown `nirdosha-hi` subcommand `{other}` -- usage: nirdosha-hi [ingest|sync|link|impact|serve|plugin|mcp] ...");
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
                        "synced {} file(s): {} unit(s) seen, {} added, {} changed, {} edge(s) flagged possibly_stale",
                        r.files_scanned, r.units_seen, r.units_added, r.units_changed, r.edges_flagged
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
                    println!("hi API listening on http://127.0.0.1:{} (Ctrl+C to stop)", handle.port);
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
        return format!("no reachable nodes from `{target}` -- try `nirdosha-hi link` or `nirdosha-hi sync` first.\n");
    }
    let mut out = format!("impact of `{target}` ({} node(s){}):\n", report.hits.len(), if report.partial { ", partial -- bound reached" } else { "" });
    for h in &report.hits {
        let flag = h.flag.as_deref().map(|f| format!("  [{f}]")).unwrap_or_default();
        let location = match (&h.source_ref, h.line, h.col) {
            (Some(path), Some(line), Some(col)) => format!("  ({path}:{line}:{col})"),
            _ => String::new(),
        };
        out.push_str(&format!("  depth {} {} {} `{}`{location}{flag}\n", h.depth, h.kind, h.edge_kind, h.title.as_deref().unwrap_or(&h.node_id)));
    }
    out
}

/// Bare `nirdosha-hi`: auto-scaffolds/syncs `.nir/hi.db` (best-effort --
/// a sync problem degrades to a logged warning, never blocks the window
/// from opening), then opens the native build-mode window and blocks
/// until it's closed. `NIRDOSHA_HI_DISABLE=1` skips the scaffold/sync
/// step entirely.
fn cmd_window(cwd: &Path) -> ExitCode {
    if !nirdosha_hi::hi_graph::is_disabled(&|k| std::env::var(k).ok()) {
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
            Err(e) => eprintln!("hi: couldn't open .nir/hi.db, continuing without it ({}=1 to silence this): {e}", nirdosha_hi::hi_graph::HI_DISABLE_VAR),
        }
    }
    match nirdosha_hi::hi_window::open(cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn write_mcp_message(stdout: &mut impl std::io::Write, value: &serde_json::Value) {
    let _ = writeln!(stdout, "{}", serde_json::to_string(value).expect("an MCP response always serializes"));
    let _ = stdout.flush();
}

/// Routes one already-parsed JSON-RPC message. Returns `None` for a
/// notification (`id` absent from the original request) -- per the MCP
/// stdio transport spec the server must never write a response for one.
fn mcp_dispatch(method: &str, params: &serde_json::Value, id: Option<&serde_json::Value>, log: &mut McpCallLog) -> Option<serde_json::Value> {
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
        Err((code, message)) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

/// `nirdosha-hi mcp` -- the v2 MCP tool surface's stdio transport
/// (JSON-RPC 2.0, newline-delimited). Every tool call (`tools_call`)
/// reuses the identical dispatcher `nirdosha-hi`'s own embedded
/// self-repair loop calls in-process -- one implementation, one call
/// log (`McpCallLog`, surface `"mcp-stdio"`), regardless of transport.
fn cmd_mcp(_args: impl Iterator<Item = String>) -> ExitCode {
    let mut log = McpCallLog::new("mcp-stdio");
    eprintln!("[nirdosha-hi mcp] tool-call log: {}", log.path().display());
    log.log_session_start(serde_json::json!({
        "protocol": "mcp-stdio-2025-06-18",
        "serverInfo": { "name": "nirdosha-hi", "version": env!("CARGO_PKG_VERSION") },
    }));
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in std::io::BufRead::lines(stdin.lock()) {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                write_mcp_message(
                    &mut stdout,
                    &serde_json::json!({ "jsonrpc": "2.0", "id": serde_json::Value::Null, "error": { "code": -32700, "message": format!("parse error: {e}") } }),
                );
                continue;
            }
        };
        let id = request.get("id");
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let empty_params = serde_json::Value::Null;
        let params = request.get("params").unwrap_or(&empty_params);
        if let Some(response) = mcp_dispatch(method, params, id, &mut log) {
            write_mcp_message(&mut stdout, &response);
        }
    }
    ExitCode::SUCCESS
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
        eprintln!("usage: nirdosha-hi plugin install [--dry-run] <pack.json> | sign <pack.json> --key <key.pk8> --identity <name> | list | revoke <pack-id>");
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
                match nirdosha_hi::hi_plugin::dry_run_install(&conn, &cwd, &bytes, &format!("dry-run {path}"),
                ) {
                    Ok(id) => {
                        println!("dry-run ok: pack {id} from {path} (load + own contracts proved against a stub program)");
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
                match nirdosha_hi::hi_plugin::verify_and_install_signed_pack(&conn, &cwd, &envelope, &path) {
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
                match nirdosha_hi::hi_plugin::install_pack_from_bytes(&conn, &cwd, &bytes, &path,
                ) {
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
                        eprintln!("unknown argument `{other}` -- usage: nirdosha-hi plugin sign <pack.json> --key <key.pk8> --identity <name> [-o <signed-pack.json>]");
                        return ExitCode::FAILURE;
                    }
                }
            }
            let (Some(manifest_path), Some(key_path), Some(identity)) = (manifest_path, key_path, identity) else {
                eprintln!("usage: nirdosha-hi plugin sign <pack.json> --key <key.pk8> --identity <name> [-o <signed-pack.json>]");
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
                    let json = serde_json::to_string_pretty(&envelope).expect("SignedPackEnvelope always serializes");
                    let out_path = out.unwrap_or_else(|| format!("{manifest_path}.signed.json"));
                    if let Err(e) = std::fs::write(&out_path, &json) {
                        eprintln!("writing {out_path}: {e}");
                        return ExitCode::FAILURE;
                    }
                    println!("wrote {out_path} -- install it with `nirdosha-hi plugin install {out_path}`");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("signing failed: {msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "list" => {
            match nirdosha_hi::hi_plugin::list_packs(&conn) {
                Ok(()) => ExitCode::SUCCESS,
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
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
