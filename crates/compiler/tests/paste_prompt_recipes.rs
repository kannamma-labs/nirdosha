//! Compile-tests for the recipes taught in
//! `agent-skills/nirdosha/paste-anywhere-prompt.md` — the file hi's
//! Generate mode bakes in (`hi_llm.rs`'s `include_str!`) as the *only*
//! grammar a generating LLM ever sees. A model copies these blocks
//! character-for-character, so a stale or broken recipe there is a
//! guaranteed 3-attempt generate failure for anyone using hi — this
//! suite makes that failure a compile error in CI instead, caught at
//! the moment the recipe is broken, not three user runs later.
//!
//! Each test extracts the actual ```nirdosha blocks from the md (by
//! stable anchor lines, so legitimate recipe edits flow through
//! automatically) and runs them through the same pipeline `nirdosha
//! build` uses: parse -> analyze -> `codegen::build` -> run the binary.
//! The workflow/transact blocks are self-contained as taught; the
//! screen/serve blocks are assembled with the minimal stubs the prompt
//! itself says they reference (the struct, the action/expose target
//! fns), so what gets compiled is exactly what the prompt teaches plus
//! declarations it explicitly names.
//!
//! History this guards against repeating: the transact recipe shipped
//! with `print("committing", amount); return amount` — a semicolon, a
//! rule-4 violation in the language's own teaching prompt, caught only
//! by compile-testing the block (2026-09-11).

use std::process::Command;

use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

/// The prompt file, read live from the repo — never a stale copy.
fn prompt_md() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../agent-skills/nirdosha/paste-anywhere-prompt.md");
    std::fs::read_to_string(&path).expect("agent-skills/nirdosha/paste-anywhere-prompt.md should exist next to the crate")
}

/// Every ```nirdosha fenced block in the md, in order.  Common leading
/// whitespace is stripped from each block so recipes nested inside
/// numbered-list indentation still compile verbatim.
fn nir_blocks() -> Vec<String> {
    let md = prompt_md();
    let mut blocks = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in md.lines() {
        let trimmed = line.trim_start();
        if trimmed == "```nirdosha" && current.is_none() {
            current = Some(Vec::new());
        } else if trimmed == "```" && current.is_some() {
            let raw = current.take().unwrap();
            blocks.push(dedent(&raw).join("\n"));
        } else if let Some(lines) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    blocks
}

/// Remove the largest common leading whitespace prefix shared by all
/// non-empty lines, but never remove more than the minimum indentation.
fn dedent(lines: &[String]) -> Vec<String> {
    let non_empty: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return lines.to_vec();
    }
    let min_indent = non_empty
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l.chars().skip(min_indent).collect()
            }
        })
        .collect()
}

/// The first block containing `anchor` — the prompt teaches one recipe
/// per construct, so "first" is unambiguous for these anchors.
fn block_containing(anchor: &str) -> String {
    nir_blocks()
        .into_iter()
        .find(|b| b.contains(anchor))
        .unwrap_or_else(|| panic!("no ```nirdosha block containing {anchor:?} found in the paste-anywhere prompt -- recipe was moved or removed; update this test's anchor"))
}

fn parse_checked(src: &str) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let mut parser = Parser::new(toks);
    let program = parser.parse_program().expect("parse should succeed");
    typecheck(&program).expect("typecheck should succeed");
    program
}

/// `nirdosha build`'s own pipeline, minus the CLI: parse -> analyze ->
/// `codegen::build` -> run the binary. (Ownership is part of `analyze`
/// in the real pipeline; `check_ownership` is called the same way the
/// `codegen.rs` test harness does.)
fn compile_and_run(src: &str, label: &str) -> (String, i32) {
    let program = parse_checked(src);
    check_ownership(&program).expect("ownership check should succeed");
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_prompt_recipe_{}_{}", label, std::process::id()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2)
        .unwrap_or_else(|e| panic!("codegen::build failed for the paste-anywhere prompt's {label} recipe: {e}"));
    let output = Command::new(&out_path).output().expect("compiled recipe binary should run");
    let _ = std::fs::remove_file(&out_path);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status.code().unwrap_or(-1))
}

#[test]
fn workflow_recipe_as_taught_compiles_and_runs() {
    // The taught block is self-contained: a logging fn + the workflow
    // itself. main() drives the synthesized start_/advance_ fns the way
    // the prompt documents them (empty `ApprovalData()`, the
    // `ApprovalEvent` variants with their `()`).
    let block = block_containing("workflow Approval {");
    let src = format!(
        "{block}\n\nfn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn main() {{\n    let identity: VerifiedIdentity = VerifiedIdentity(\"ravi\", \"https://idp.example.com\", \"corp-pay\", 0, 0, \"requester\")\n    let started: Result(i64, WorkflowActionError) = start_approval(None(), ApprovalData())\n    let ok: bool = match started {{\n        Ok(id) => match advance_approval(identity, id, Approve(), empty_doc()) {{\n            Ok(_) => true,\n            Err(e) => false,\n        }},\n        Err(e) => false,\n    }}\n    print(\"advanced\", ok)\n}}"
    );
    let (out, code) = compile_and_run(&src, "workflow");
    assert_eq!(code, 0);
    assert!(out.contains("advanced"), "workflow recipe output should contain the advanced line, got: {out}");
}

#[test]
fn transact_recipe_as_taught_compiles_and_runs() {
    // The taught block is self-contained (all six step fns + settle);
    // main() just settles a positive amount, which must commit and
    // print -- and the recipe must contain no `;` (the 2026-09-11 bug
    // this suite was born from was exactly that).
    let block = block_containing("fn db_up()");
    assert!(!block.contains(';'), "the transact recipe in the paste-anywhere prompt contains a `;` -- rule 4 violation in the very block that teaches the construct");
    let src = format!("{block}\n\nfn main() {{\n    let ok: bool = settle(10)\n    print(\"settled\", ok)\n}}");
    let (out, code) = compile_and_run(&src, "transact");
    assert_eq!(code, 0);
    assert!(out.contains("committing"), "transact recipe should commit the positive amount, got: {out}");
    assert!(out.contains("settled"), "transact recipe should print its log line, got: {out}");
}

#[test]
fn screen_and_serve_recipes_as_taught_compile() {
    // Both taught blocks reference declarations by name (the prompt
    // says a screen's `->` target and an exposed fn must be real) —
    // assemble them with exactly those minimal declarations and a
    // main(), the same way a generated program would.
    let screen = block_containing("screen PaymentRequest {");
    let serve = block_containing("serve {");
    let src = format!(
        "struct PaymentRequest {{\n    request_id: i64,\n    account_id: i64,\n    amount_cents: i64,\n    status: str,\n}}\n\nfn approve_payment(instance_id: i64) -> bool {{ return true }}\n\nfn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn list_payment_request() -> json {{ return empty_doc() }}\n\nfn stat_open_approval_count() -> i64 {{ return 0 }}\n\n{screen}\n\n{serve}\n\nfn main() {{\n    print(\"ready\")\n}}"
    );
    let (out, code) = compile_and_run(&src, "screen_serve");
    assert_eq!(code, 0);
    assert!(out.contains("ready"), "screen+serve recipe output should contain the ready line, got: {out}");
}

#[test]
fn json_display_loops_as_taught_compile_and_run() {
    // The Print entry's display loops are fragments (`doc`/`queue_json`
    // arrive from somewhere in a real program) — bind them the way the
    // prompt's own json examples do and the taught block must compile
    // and print the extracted scalars. One fenced block in the md holds
    // both the object loop and the array loop; splice it once, with
    // both bindings in front.
    let block = block_containing("landing for");
    let src = format!(
        "fn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn main() {{\n    let doc: json = match json_parse(\"{{\\\"user\\\":\\\"ravi\\\"}}\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n    let queue_json: json = match json_parse(\"[{{\\\"status\\\":\\\"pending\\\"}},{{\\\"status\\\":\\\"paid\\\"}}]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n{block}\n}}"
    );
    let (out, code) = compile_and_run(&src, "json_display");
    assert_eq!(code, 0);
    assert!(out.contains("ravi"), "json display loop should extract the user field, got: {out}");
    assert!(out.contains("pending"), "json display loop should walk the array items, got: {out}");
}

#[test]
fn identity_acquire_pattern_as_taught_compiles_and_runs() {
    // Rule 19's exact acquire/check_role recipe must compile and run
    // when assembled with only a main() that exercises it.
    let block = block_containing("fn try_approve");
    let src = format!(
        "{block}\n\nfn main() {{
    let identity: VerifiedIdentity = VerifiedIdentity(\"meera\", \"https://idp.example.com\", \"corp-pay\", 0, 0, \"finance_director\")
    let ok: bool = try_approve(identity, 1)
    print(\"approved\", ok)
}}"
    );
    let (out, code) = compile_and_run(&src, "identity_acquire");
    assert_eq!(code, 0);
    assert!(out.contains("approved"), "identity/acquire recipe output should contain approved line, got: {out}");
}

#[test]
fn transact_txn_id_recipe_as_taught_compiles_and_runs() {
    // Rule 20's exact transact recipe with txn_id must compile and
    // commit.
    let block = block_containing("fn call_processor(txn_id: str");
    assert!(!block.contains(';'), "the txn_id transact recipe contains a `;`");
    let src = format!(
        "{block}\n\nfn main() {{
    let ok: bool = settle(10)
    print(\"settled\", ok)
}}"
    );
    let (out, code) = compile_and_run(&src, "transact_txn_id");
    assert_eq!(code, 0);
    assert!(out.contains("settled"), "txn_id transact recipe should settle, got: {out}");
}

#[test]
fn prompt_forbids_user_and_userrole() {
    // Rule 19's negative instruction: the prompt must not tell the
    // model to invent a User/UserRole type for role handling.
    let md = prompt_md();
    assert!(
        !md.contains("struct User"),
        "the prompt must not contain a `struct User` recipe"
    );
    assert!(
        !md.contains("enum UserRole"),
        "the prompt must not contain an `enum UserRole` recipe"
    );
}

#[test]
fn prompt_has_no_stale_unsupported_claims() {
    // The 2026-09 stale-text failure mode: the prompt claimed db/json/
    // transact don't compile months after they did, and a generating
    // model reading that avoided or mangled the very features a prompt
    // asked for. These spellings are what the compiled set actually
    // supports today (examples 36/46/55 prove it) -- if the prompt
    // ever reverts to claiming otherwise, fail here.
    let md = prompt_md();
    assert!(!md.contains("aren't in that\nset yet"), "the prompt claims db/json/transact aren't in the compiled set -- they are; this exact stale sentence burned a real 3-attempt generate");
    assert!(md.contains("data {}"), "the prompt must teach the compiled workflow shape (empty data block), not the interpreter-only non-empty one");
    assert!(md.contains("return transact {"), "the prompt must show the exact transact block spelling");
    assert!(md.contains("serve {"), "the prompt must show the exact serve/expose spelling");
}