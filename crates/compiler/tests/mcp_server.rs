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
}

impl McpSession {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nirdosha"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("nirdosha mcp should spawn");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Self { child, stdin, stdout }
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
fn tools_list_advertises_exactly_the_four_master_plan_tools() {
    let mut session = McpSession::start();
    session.initialize();
    session.send(&serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let response = session.recv();
    let tools = response["result"]["tools"].as_array().expect("tools should be an array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().expect("name should be a string")).collect();
    assert_eq!(names, vec!["verify_code", "get_grammar", "fix", "describe"], "response: {response}");
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "tool: {tool}");
    }
}

#[test]
fn verify_code_reports_a_disproved_counterexample() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(
        2,
        "verify_code",
        serde_json::json!({ "source": "fn bad(a: i64) -> i64 {\n    return a - 1\n}\n\nvalidate bad {\n    post: result >= a\n}\n" }),
    );
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["verdict"], "DISPROVED", "response: {response}");
    assert_eq!(structured["contracts"]["failed"], 1, "response: {response}");
    assert_eq!(structured["source"], "<inline>", "response: {response}");
    assert_eq!(response["result"]["isError"], false, "response: {response}");
    let text_reparsed: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().expect("text should be a string")).expect("text should be valid JSON");
    assert_eq!(&text_reparsed, structured, "response: {response}");
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
    let grammar = response["result"]["structuredContent"]["grammar"].as_str().expect("grammar should be a string");
    assert_eq!(response["result"]["structuredContent"]["format"], "gbnf", "response: {response}");
    assert!(grammar.contains("::="), "response should embed real GBNF, not a placeholder: {response}");
}

#[test]
fn fix_reports_an_auto_patch_without_applying_by_default() {
    let mut session = McpSession::start();
    session.initialize();
    let response =
        session.call_tool(2, "fix", serde_json::json!({ "source": "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n" }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["before"]["typecheck"]["errors"][0]["fix"]["applicability"], "auto", "response: {response}");
    assert_eq!(structured["applied"].as_array().expect("applied should be an array").len(), 0, "response: {response}");
    assert!(structured.get("patched_source").is_none(), "response: {response}");
}

#[test]
fn fix_with_apply_returns_the_patched_source() {
    let mut session = McpSession::start();
    session.initialize();
    let response = session.call_tool(
        2,
        "fix",
        serde_json::json!({ "source": "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n", "apply": true }),
    );
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["applied"].as_array().expect("applied should be an array").len(), 1, "response: {response}");
    assert_eq!(structured["patched_source"], "fn main() {\n    let amount: i64 = 5\n    print(amount)\n}\n", "response: {response}");
    assert_eq!(structured["after"]["typecheck"]["status"], "passed", "response: {response}");
}

#[test]
fn describe_summarizes_functions_structs_enums_and_validates() {
    let mut session = McpSession::start();
    session.initialize();
    let source = "enum Shape {\n    Circle(f64),\n}\n\nstruct Player {\n    game_state: str,\n}\n\nfn area(s: Shape) -> f64 {\n    return match s {\n        Circle(r) => r * r,\n    }\n}\n\nvalidate area {\n    post: result >= 0.0\n}\n";
    let response = session.call_tool(2, "describe", serde_json::json!({ "source": source }));
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["functions"][0]["name"], "area", "response: {response}");
    // `loader::load_program` merges in the prelude's own structs/enums
    // (Option/Result/HttpResponse/...) ahead of this file's own
    // declarations, so find by name rather than assume index 0 -- the
    // prelude's presence is expected, real behavior, not a defect.
    let structs = structured["structs"].as_array().expect("structs should be an array");
    let player = structs.iter().find(|s| s["name"] == "Player").unwrap_or_else(|| panic!("Player struct missing: {response}"));
    assert_eq!(player["fields"][0]["name"], "game_state", "response: {response}");
    let enums = structured["enums"].as_array().expect("enums should be an array");
    let shape = enums.iter().find(|e| e["name"] == "Shape").unwrap_or_else(|| panic!("Shape enum missing: {response}"));
    assert_eq!(shape["variants"][0]["name"], "Circle", "response: {response}");
    assert_eq!(structured["validates"][0]["fn_name"], "area", "response: {response}");
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
