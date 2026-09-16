//! End-to-end tests for `nirdosha mcp` (`cmd_mcp`/`mcp_dispatch` in
//! `main.rs`, `nirdosha-master-plan.md` Part 3 Sprint 1's MCP server,
//! parity target: Acutis, Imandra, Kōdo). Spawns the real binary with
//! piped stdin/stdout and speaks the stdio transport directly
//! (newline-delimited JSON-RPC, per the MCP spec) -- the same
//! "spawn the real binary" pattern `verify_verdict.rs`/`fix_command.rs`/
//! `explain_command.rs` use for one-shot commands, extended here to a
//! long-lived request/response session.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    stderr: BufReader<std::process::ChildStderr>,
}

impl McpSession {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nirdosha"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped, not nulled: `cmd_mcp` discloses the session's
            // tool-call log path on stderr at startup -- the tests
            // below read that one line to assert the log itself.
            .stderr(Stdio::piped())
            .spawn()
            .expect("nirdosha mcp should spawn");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let stderr = BufReader::new(child.stderr.take().expect("piped stderr"));
        Self { child, stdin, stdout, stderr }
    }

    /// The disclosed NDJSON call-log path `cmd_mcp` prints on stderr
    /// at startup (surface `"mcp-stdio"`) -- the first line it ever
    /// writes, before reading any stdin, so one `read_line` is
    /// exactly enough to obtain it.
    fn log_path(&mut self) -> std::path::PathBuf {
        let mut line = String::new();
        self.stderr.read_line(&mut line).expect("read from nirdosha mcp's stderr should succeed");
        let path = line
            .strip_prefix("[nirdosha mcp] tool-call log: ")
            .unwrap_or_else(|| panic!("first stderr line should disclose the log path, got: {line:?}"))
            .trim();
        std::path::PathBuf::from(path)
    }

    fn send(&mut self, value: &serde_json::Value) {
        let line = serde_json::to_string(value).expect("request should serialize");
        writeln!(self.stdin, "{line}").expect("write to nirdosha mcp's stdin should succeed");
        self.stdin.flush().expect("flush should succeed");
    }

    /// Reads exactly one response line -- used after every request that
    /// carries an `id` (a notification gets no response at all, and no
    /// test here waits for one).
    fn recv(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read from nirdosha mcp's stdout should succeed");
        assert!(n > 0, "nirdosha mcp closed stdout before sending a response");
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("response line was not valid JSON ({e}): {line:?}"))
    }

    fn initialize(&mut self) -> serde_json::Value {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0.0.0" } },
        }));
        let response = self.recv();
        self.send(&serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        response
    }

    fn call_tool(&mut self, id: i64, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        }));
        self.recv()
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_negotiates_the_protocol_version_and_advertises_tools() {
    let mut session = McpSession::start();
    let response = session.initialize();
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18", "response: {response}");
    assert_eq!(response["result"]["serverInfo"]["name"], "nirdosha", "response: {response}");
    assert!(response["result"]["capabilities"]["tools"].is_object(), "response: {response}");
}

#[test]
fn tools_list_advertises_exactly_the_five_master_plan_tools_plus_constructs_and_ui_conventions() {
    let mut session = McpSession::start();
    session.initialize();
    session.send(&serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let response = session.recv();
    let tools = response["result"]["tools"].as_array().expect("tools should be an array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().expect("name should be a string")).collect();
    assert_eq!(
        names,
        vec!["verify_code", "get_grammar", "fix", "describe", "certify_code", "get_nirdosha_constructs", "get_ui_conventions"],
        "response: {response}"
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "tool: {tool}");
    }
}

/// A v2 candidate that builds cleanly under plain `cargo` (the comment
/// is inert to rustc) but carries a `nirdosha:contract` doc with an
/// unknown JSON key -- `cc::model::Contract`'s `deny_unknown_fields`
/// makes that a real finding under `cargo-nirdosha`'s scanner, the v2
/// analogue of the retired native tests' `DISPROVED` fixture (a source
/// that's fine by one reader and flagged by the other).
const V2_CANDIDATE_WITH_CONTRACT_VIOLATION: &str = "/// nirdosha:contract {\"effects\":[\"pure\"],\"bogus_field\":true}\nfn bad(a: i64) -> i64 {\n    a - 1\n}\n\nfn main() {\n    println!(\"{}\", bad(1));\n}\n";

const V2_CLEAN_CANDIDATE: &str = "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    println!(\"{}\", add(2, 3));\n}\n";

#[test]
fn certify_code_issues_a_deterministic_certificate_for_every_verdict() {
    let mut session = McpSession::start();
    session.initialize();
    // A source with a contract violation still gets its certificate --
    // the same honest "here is the conclusive evidence this code is
    // wrong" contract `nirdosha certify` documents natively.
    let source = serde_json::json!({ "source": V2_CANDIDATE_WITH_CONTRACT_VIOLATION });
    let first = session.call_tool(2, "certify_code", source.clone());
    let second = session.call_tool(3, "certify_code", source);
    let first_structured = &first["result"]["structuredContent"];
    let second_structured = &second["result"]["structuredContent"];
    assert_eq!(first_structured["certificate_version"], "nirdosha.certificate/v2-source-scan", "response: {first}");
    assert_eq!(first_structured["verdict"], "violations_found", "response: {first}");
    assert_eq!(first_structured["builds"], true, "the comment is inert to rustc, so this should still build: {first}");
    assert_eq!(first_structured["evidence_tier"], "source_scan", "no proof pipeline exists for v2 yet: {first}");
    assert!(first_structured["source_hash"].as_str().expect("hash should be a string").starts_with("sha256:"), "response: {first}");
    assert!(
        first_structured["violations"].as_array().expect("violations should be an array").iter().any(|v| v.as_str().unwrap().contains("bogus_field")),
        "response: {first}"
    );
    // Deterministic: byte-for-byte identical for the same source and
    // compiler version -- the property that makes a certificate
    // reproducible by any third party.
    assert_eq!(&first_structured, &second_structured, "two calls on the same source must produce identical certificates");
}

#[test]
fn certify_code_missing_source_is_a_protocol_error() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "certify_code", serde_json::json!({}));
    assert_eq!(response["error"]["code"], -32602, "response: {response}");
}

#[test]
fn every_call_is_logged_in_the_disclosed_ndjson_log() {
    let mut session = McpSession::start();
    session.initialize();
    let log_path = session.log_path();
    let verify = session.call_tool(2, "verify_code", serde_json::json!({ "source": V2_CANDIDATE_WITH_CONTRACT_VIOLATION }));
    assert_eq!(verify["result"]["structuredContent"]["verdict"], "violations_found", "response: {verify}");
    let grammar = session.call_tool(3, "get_grammar", serde_json::json!({}));
    assert!(grammar["result"]["structuredContent"]["comment_layer_grammar"].is_string(), "response: {grammar}");

    let contents = std::fs::read_to_string(&log_path).expect("the disclosed log file should exist");
    let records: Vec<serde_json::Value> = contents
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("log line was not valid JSON ({e}): {l:?}")))
        .collect();
    // session_start + one record per tools/call (initialize and
    // notifications are wire-level protocol, not tool calls -- the
    // log records the capability surface, not the transport).
    assert_eq!(records.len(), 3, "records: {records:?}");
    assert_eq!(records[0]["event"], "session_start");
    assert_eq!(records[0]["surface"], "mcp-stdio");
    assert_eq!(records[1]["tool"], "verify_code");
    assert_eq!(records[1]["call_id"], 1);
    assert_eq!(records[1]["surface"], "mcp-stdio");
    assert_eq!(records[1]["verdict"], "violations_found");
    assert!(records[1]["source"]["sha256"].as_str().expect("hash should be a string").starts_with("sha256:"));
    assert!(records[1]["ts"].as_str().expect("ts should be a string").ends_with('Z'));
    assert_eq!(records[2]["tool"], "get_grammar");
    assert_eq!(records[2]["call_id"], 2);
    let grammar_record = &records[2];
    assert!(grammar_record.get("source").is_none(), "get_grammar carries no source: {grammar_record:?}");
    assert_eq!(records[2]["outcome"], "ok");
    // The user-facing cleanliness contract: the test leaves no
    // scratch log behind.
    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_code_reports_contract_violations() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "verify_code", serde_json::json!({ "source": V2_CANDIDATE_WITH_CONTRACT_VIOLATION }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["verdict"], "violations_found", "response: {response}");
    assert_eq!(structured["builds"], true, "response: {response}");
    // A malformed contract never becomes a parsed `ContractInfo` -- it's
    // a finding (`violations`, asserted below), not a counted contract.
    assert_eq!(structured["contracts_found"], 0, "response: {response}");
    assert_eq!(response["result"]["isError"], false, "response: {response}");
    let text_reparsed: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().expect("text should be a string")).expect("text should be valid JSON");
    assert_eq!(&text_reparsed, structured, "response: {response}");
}

#[test]
fn verify_code_reports_a_clean_v2_candidate() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "verify_code", serde_json::json!({ "source": V2_CLEAN_CANDIDATE }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["verdict"], "clean", "response: {response}");
    assert_eq!(structured["builds"], true, "response: {response}");
    assert_eq!(structured["violations"].as_array().expect("violations should be an array").len(), 0, "response: {response}");
}

#[test]
fn verify_code_missing_source_is_a_protocol_error_not_a_tool_error() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "verify_code", serde_json::json!({}));
    assert_eq!(response["error"]["code"], -32602, "response: {response}");
    assert!(response.get("result").is_none(), "response: {response}");
}

#[test]
fn unknown_tool_name_is_a_protocol_error() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "not_a_real_tool", serde_json::json!({}));
    assert_eq!(response["error"]["code"], -32602, "response: {response}");
}

#[test]
fn get_grammar_returns_the_real_gbnf_file() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "get_grammar", serde_json::json!({}));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["format"], "v2-comment-layer", "response: {response}");
    let grammar = structured["comment_layer_grammar"].as_str().expect("comment_layer_grammar should be a string");
    assert!(grammar.contains("nirdosha:contract"), "response should embed the real comment-kind registry: {response}");
    assert!(grammar.contains("nirdosha:workflow"), "response should embed the real comment-kind registry: {response}");
    assert!(structured["role_gating"].as_str().expect("role_gating should be a string").contains("RoleProof"), "response: {response}");
}

#[test]
fn get_nirdosha_constructs_reports_every_construct_as_compiling() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "get_nirdosha_constructs", serde_json::json!({}));
    let constructs = response["result"]["structuredContent"]["constructs"].as_array().expect("constructs should be an array");
    assert!(!constructs.is_empty(), "response: {response}");
    for c in constructs {
        assert!(c["name"].as_str().is_some(), "entry missing name: {c}");
        assert!(c["example"].as_str().is_some(), "entry missing example: {c}");
        // Every construct this module claims is real should actually
        // compile against the binary under test -- a `false` here means
        // either the inventory or the compiler has regressed, and
        // `cargo test --test capabilities` is where that gets diagnosed.
        assert_eq!(c["supported"], true, "construct should compile against this build: {c}");
        assert!(c["diagnostic"].is_null(), "a supported construct should carry no diagnostic: {c}");
    }
    let names: Vec<&str> = constructs.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"fn + arithmetic"), "response: {response}");
    assert!(names.contains(&"workflow (state machine)"), "response: {response}");
}

#[test]
fn get_ui_conventions_covers_the_real_archetype_macros() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "get_ui_conventions", serde_json::json!({}));
    let structured = &response["result"]["structuredContent"];

    let macros = structured["archetype_macros"].as_array().expect("archetype_macros should be an array");
    let macro_names: Vec<&str> = macros.iter().map(|m| m["macro"].as_str().unwrap()).collect();
    assert!(macro_names.contains(&"nirdosha_rt::crud_screens!"), "response: {response}");
    assert!(macro_names.contains(&"nirdosha_rt::dashboard!"), "response: {response}");
    for m in macros {
        assert!(m["invocation"].as_str().is_some(), "entry missing invocation: {m}");
        assert_eq!(m["checked_by"].as_str().unwrap().contains("rustc"), true, "every archetype should be rustc-checked: {m}");
    }

    assert!(structured["role_gating"].as_str().expect("role_gating should be a string").contains("RoleProof"), "response: {response}");
    assert!(
        structured["declarative_comment_layer"].as_str().expect("declarative_comment_layer should be a string").contains("inert"),
        "response: {response}"
    );
}

#[test]
fn fix_reports_the_verdict_without_applying_by_default() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "fix", serde_json::json!({ "source": V2_CANDIDATE_WITH_CONTRACT_VIOLATION }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["before"]["verdict"], "violations_found", "response: {response}");
    assert_eq!(structured["applied"].as_array().expect("applied should be an array").len(), 0, "response: {response}");
    assert_eq!(structured["apply_requested"], false, "response: {response}");
    assert!(structured.get("patched_source").is_none(), "response: {response}");
}

#[test]
fn fix_with_apply_still_applies_nothing_since_no_v2_fixer_exists_yet() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "fix", serde_json::json!({ "source": V2_CANDIDATE_WITH_CONTRACT_VIOLATION, "apply": true }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["applied"].as_array().expect("applied should be an array").len(), 0, "response: {response}");
    assert_eq!(structured["apply_requested"], true, "response: {response}");
    assert!(structured.get("patched_source").is_none(), "response: {response}");
}

#[test]
fn describe_summarizes_functions_structs_enums_and_nirdosha_declarations() {
    let mut session = McpSession::start();
    session.initialize();
    let source = "enum Shape {\n    Circle(f64),\n}\n\nstruct Player {\n    game_state: String,\n}\n\n/// nirdosha:validate {\"fn\":\"area\",\"post\":[\"result >= 0.0\"]}\nfn area(s: Shape) -> f64 {\n    match s {\n        Shape::Circle(r) => r * r,\n    }\n}\n";
    let response = session.call_tool(2, "describe", serde_json::json!({ "source": source }));
    let structured = &response["result"]["structuredContent"];
    let functions = structured["functions"].as_array().expect("functions should be an array");
    assert!(functions.iter().any(|f| f["name"] == "area"), "response: {response}");
    let structs = structured["structs"].as_array().expect("structs should be an array");
    let player = structs.iter().find(|s| s["name"] == "Player").unwrap_or_else(|| panic!("Player struct missing: {response}"));
    assert_eq!(player["fields"][0], "game_state", "response: {response}");
    let enums = structured["enums"].as_array().expect("enums should be an array");
    let shape = enums.iter().find(|e| e["name"] == "Shape").unwrap_or_else(|| panic!("Shape enum missing: {response}"));
    assert_eq!(shape["variants"][0], "Circle", "response: {response}");
    let declarations = structured["nirdosha_declarations"].as_array().expect("nirdosha_declarations should be an array");
    let validate = declarations.iter().find(|d| d["kind"] == "validate").unwrap_or_else(|| panic!("nirdosha:validate declaration missing: {response}"));
    assert_eq!(validate["owner"], "area", "response: {response}");
    assert!(validate["payload"].as_str().unwrap().contains("result >= 0.0"), "response: {response}");
}

#[test]
fn describe_does_not_require_the_source_to_typecheck() {
    // Mirrors cmd_emit_ast's own documented contract: a parse-only
    // summary should still work for code that fails typecheck, e.g. a
    // deliberate type mismatch below.
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(2, "describe", serde_json::json!({ "source": "fn broken() -> i64 {\n    return \"not an i64\"\n}\n" }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["functions"][0]["name"], "broken", "response: {response}");
}

#[test]
fn a_notification_gets_no_response_at_all() {
    let mut session = McpSession::start();
    session.initialize();
    // `initialized` (sent by `initialize()` above) must not have
    // produced a response -- prove it by sending a real request right
    // after and confirming the *first* line read back answers *that*
    // request, not a leftover response to the notification.
    session.send(&serde_json::json!({ "jsonrpc": "2.0", "id": 99, "method": "ping" }));
    let response = session.recv();
    assert_eq!(response["id"], 99, "a stray response to the `initialized` notification would have arrived first: {response}");
}

#[test]
fn unknown_method_is_method_not_found() {
    let mut session = McpSession::start();
    session.initialize();
    session.send(&serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "not/a/real/method" }));
    let response = session.recv();
    assert_eq!(response["error"]["code"], -32601, "response: {response}");
}

#[test]
fn malformed_json_gets_a_parse_error_and_the_session_keeps_working() {
    let mut session = McpSession::start();
    session.initialize();
    writeln!(session.stdin, "{{not valid json").expect("write should succeed");
    session.stdin.flush().expect("flush should succeed");
    let parse_error_response = session.recv();
    assert_eq!(parse_error_response["error"]["code"], -32700, "response: {parse_error_response}");
    // The session must still be usable afterward.
    session.send(&serde_json::json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }));
    let ping_response = session.recv();
    assert_eq!(ping_response["id"], 3, "response: {ping_response}");
}
