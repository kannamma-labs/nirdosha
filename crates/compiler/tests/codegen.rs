//! Tests for `codegen.rs` — docs/goal.md row 5's first real content: this
//! backend actually produces native binaries, and these tests actually
//! run them, not just check that IR text was emitted. Every bug fixed
//! while building this module (see codegen.rs's doc comments) was found
//! by exactly this kind of check — inspecting real output, not reasoning
//! about the code in the abstract — so the tests here follow the same
//! discipline: run the compiled binary, compare its real stdout/exit
//! code against the interpreter's, don't just assert the pipeline
//! "succeeded."

use std::process::Command;

use nirdosha::ast::Program;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck;

fn parse_checked(src: &str) -> Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    check_ownership(&program).expect("should ownership-check cleanly");
    program
}

/// Parse + typecheck only, *without* ownership checking — for codegen-
/// rejection tests where the rejection is a `check_supported`/`emit_c_main`
/// concern that fires before any ownership question matters (e.g. an
/// affine-field struct, which ownership itself doesn't fully track yet —
/// that's the Phase 4b gap). `emit_llvm_ir` runs `check_supported` first,
/// so it rejects such a program before its own `compute_free_map` ever
/// runs.
#[allow(dead_code)]
fn parse_typed(src: &str) -> Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly");
    program
}

/// Compiles `src` to a real native binary in a fresh temp path at the
/// given optimization level, runs it, and returns its stdout and exit
/// code. Panics (loudly, with clang's own error) if compilation itself
/// fails — a test that expects compilation to fail should call
/// `codegen::build` directly instead.
fn compile_and_run_opt(src: &str, opt: codegen::OptLevel) -> (String, i32) {
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, opt).expect("codegen::build should succeed for this program");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status.code().unwrap_or(-1))
}

/// The default most tests use — `-O2`, matching `nirdosha build`'s own
/// default (module doc: docs/goal.md row 5 is about hardware speed) and, not
/// incidentally, the stronger correctness check: an aggressive optimizer
/// is exactly what would expose a subtly wrong `unreachable` marker that
/// `-O0` happens not to disturb.
fn compile_and_run(src: &str) -> (String, i32) {
    compile_and_run_opt(src, codegen::OptLevel::O2)
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn hello_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/hello.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "8\n");
    assert_eq!(code, 0);
}

#[test]
fn factorial_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/factorial.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "3628800\n");
    assert_eq!(code, 0);
}

#[test]
fn loop_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/loop.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "20\n19\n18\n17\n16\n11\n");
    assert_eq!(code, 0);
}

// ---- spawn/join/chan compile to real OS threads and mailboxes ----------
//
// Backed by `runtime-kernels`' `nir_thread_spawn`/`nir_thread_join`
// (`kernel::thread_pool::Scope`) and `nir_chan_new`/`nir_chan_send`/
// `nir_chan_recv` (`kernel::mailbox`) — real concurrency, not simulated:
// `threads.nir`'s two `spawn`s actually run on separate OS threads, and
// `channels.nir`'s producer/consumer actually hand off values through a
// real cross-thread queue. See `codegen.rs`'s `spawn_thread`/`is_word_sized`
// doc comments for the still-real, disclosed narrower scope (word-sized
// payloads/arguments/results only, for now).

#[test]
fn threads_example_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/threads.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "42\n42\n");
    assert_eq!(code, 0);
}

#[test]
fn channels_example_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/channels.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

// ---- box/&/* are honestly rejected, not silently mis-compiled ----------

#[test]
fn sandbox_example_is_rejected_by_codegen() {
    let program = parse_checked(include_str!("fixtures/sandbox.nir"));
    let report = analyze(&program);
    let result = codegen::emit_llvm_ir(&program, &report);
    assert!(
        result.is_err(),
        "codegen doesn't support `sandbox`/`stop` yet -- must reject, not mis-compile"
    );
}

#[test]
fn sandbox_channels_example_is_rejected_by_codegen() {
    let program = parse_checked(include_str!("fixtures/sandbox_channels.nir"));
    let report = analyze(&program);
    let result = codegen::emit_llvm_ir(&program, &report);
    assert!(
        result.is_err(),
        "codegen doesn't support `sandbox`/`chan` yet -- must reject, not mis-compile"
    );
}

// ---- `str` -- literals, escapes, param/return pass-through, `==`/`!=` --

#[test]
fn strings_example_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/strings.nir");
    let (stdout, code) = compile_and_run(src);
    // `1`/`0`, not `true`/`false` -- a pre-existing, documented cosmetic
    // difference from the interpreter's `render()` (a comparison result
    // printed as a bare `bool` takes the same `i1`-as-`i64` path any
    // other `print(x > y)` already does; see `call()`'s print-arg
    // dispatch), not something this phase introduces.
    assert_eq!(stdout, "hello, nirdosha\nline one\nline two\ttabbed\nworld\n1\n0\n");
    assert_eq!(code, 0);
}

// ---- `main() -> str` (and any `fn`'s `str` param/return) is rejected by
// the "enum favoring" rule (`typeck.rs::check_fn`'s `StrInFnSignature`) --
// the sanctioned replacement is `print`-then-return-`unit`, and for a
// value that must cross a function boundary, the `Text { value: str }`
// carrier struct (unaffected by the rule -- constructing it is a call to
// a name in `callable_names`, never a `program.fns` entry). ------------

#[test]
fn main_printing_a_str_directly_compiles_and_prints_it() {
    let src = r#"
        fn main() -> unit {
            print("hello from main")
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "hello from main\n");
    assert_eq!(code, 0);
}

#[test]
fn a_computed_str_carried_through_a_function_boundary_via_text_compiles_and_matches_interpreter() {
    let src = r#"
        struct Text {
            value: str,
        }

        fn greet(name: Text) -> Text {
            if name.value == "world" {
                return Text("hello world")
            } else {
                return name
            }
        }

        fn main() -> unit {
            let g: Text = greet(Text("world"))
            print(g.value)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "hello world\n");
    assert_eq!(code, 0);
}

#[test]
fn empty_string_literal_compiles_and_prints_a_blank_line() {
    let src = r#"
        fn main() {
            let s: str = ""
            print(s)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "\n");
    assert_eq!(code, 0);
}

#[test]
fn same_length_different_content_strings_compare_unequal() {
    let src = r#"
        fn main() {
            let a: str = "abc"
            let b: str = "abd"
            print(a == b)
            print(a != b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "0\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn tcp_client_example_compiles_now_that_tcp_codegen_landed() {
    // Stale since Phase B1: `tcp`/`connect`/`send`/`recv`/`stop` all
    // compile now — this only checks that `emit_llvm_ir` itself succeeds
    // (valid IR), not that the program *runs* correctly against a real
    // service, since `examples/tcp_client.nir`'s own doc comment says it
    // needs `python3 -m http.server 8000` running externally — a real
    // end-to-end round trip against a self-contained loopback server is
    // covered instead by the dedicated tests just above.
    let program = parse_checked(include_str!("fixtures/tcp_client.nir"));
    let report = analyze(&program);
    let result = codegen::emit_llvm_ir(&program, &report);
    assert!(result.is_ok(), "tcp/connect/send/recv/stop should all compile: {:?}", result.err());
}

// ---- the bug this module actually shipped with, pinned as a regression -

#[test]
fn narrow_type_overflow_actually_traps_at_runtime() {
    // The real bug found by testing (see codegen.rs's `guard_in_range`
    // doc comment): computing arithmetic directly at a narrow LLVM
    // width (`add i8`) wraps silently on overflow, the same as any
    // two's-complement machine addition, which meant the range check
    // was comparing an already-wrapped value against the very bounds
    // it's supposed to catch escaping -- it could never fire. 100 + 100
    // overflows i8 (max 127); a correct backend has to trap, not
    // silently produce -56 and exit 0.
    let src = r#"
        fn main() -> i64 {
            let a: i8 = 100
            let b: i8 = 100
            let c: i8 = a + b
            return 0
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "100 + 100 overflows i8 -- the compiled binary must not exit 0");
}

// ---- Phase 1: unsigned integers (u8/u16/u32/u64/usize) ----------------
//
// The one real signed-vs-unsigned instruction choice this backend needs
// is at `widen_to_i64` (a narrower-than-i64 value loaded off the stack
// must `zext`, not `sext`) -- every downstream op (`+`/`-`/`*`, `<`/`>`,
// `/`) is computed at i64 width already and stays byte-identical between
// signed/unsigned once widened correctly, since `Ty::bounds()` caps
// every unsigned type's legal range at `[0, i64::MAX]` (never touching
// the sign bit). These tests pick boundary values a wrong `sext` would
// visibly get wrong (e.g. `200_u8` sign-extends to a *negative* i64 as
// `i8`, so `200 > 5` and `200 / 5` would both come out wrong), not just
// values that happen to work either way.

#[test]
fn u8_comparison_and_division_are_correct_at_a_value_that_would_sign_extend_negative() {
    let src = r#"
        fn main() {
            let a: u8 = 200
            let b: u8 = 5
            print(a > b)
            print(a / b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n40\n");
    assert_eq!(code, 0);
}

#[test]
fn u16_u32_u64_usize_comparison_and_division_match_interpreter() {
    let src = r#"
        fn main() {
            let c: u16 = 60000
            let d: u16 = 7
            print(c > d)
            print(c / d)

            let e: u32 = 4000000000
            let f: u32 = 3
            print(e > f)
            print(e / f)

            let g: u64 = 9000000000000000000
            let h: u64 = 2
            print(g > h)
            print(g / h)

            let i: usize = 42
            let j: usize = 5
            print(i / j)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n8571\n1\n1333333333\n1\n4500000000000000000\n8\n");
    assert_eq!(code, 0);
}

#[test]
fn u32_addition_above_i32_max_is_correct_not_sign_extended() {
    // 4000000000 + 100000000 = 4100000000 -- within u32's range (max
    // 4294967295) but well past i32::MAX (2147483647); a wrong `sext`
    // widening would corrupt this into a negative i64 before the add.
    let src = r#"
        fn add(a: u32, b: u32) -> u32 {
            return a + b
        }

        fn main() {
            print(add(4000000000, 100000000))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "4100000000\n");
    assert_eq!(code, 0);
}

#[test]
fn u8_underflow_traps_at_runtime_same_as_signed_overflow() {
    let src = r#"
        fn compute(a: u8, b: u8) -> u8 {
            return a - b
        }

        fn main() {
            print(compute(3, 10))
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "3 - 10 underflows u8 -- the compiled binary must not exit 0");
}

#[test]
fn out_of_range_unsigned_literal_is_rejected_statically() {
    // Unaffected by this phase's codegen change (typeck.rs already
    // range-checks every integer type generically via `Ty::in_range`) --
    // included as a regression guard that accepting `u8`/etc. in
    // codegen didn't loosen this static check.
    let src = r#"
        fn main() {
            let a: u8 = 256
            print(a)
        }
    "#;
    let toks = nirdosha::token::Lexer::new(src).tokenize().expect("lex should succeed");
    let program = nirdosha::parser::Parser::new(toks).parse_program().expect("parse should succeed");
    assert!(nirdosha::typeck::typecheck(&program).is_err(), "256 does not fit in u8, should be a static type error");
}

#[test]
fn main_returning_an_unsigned_type_directly_compiles_and_exits_with_that_code() {
    let src = r#"
        fn main() -> u8 {
            return 200
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 200);
}

#[test]
fn division_by_zero_traps_at_runtime() {
    let src = r#"
        fn main() -> i64 {
            let z: i64 = 0
            let x: i64 = 10 / z
            return 0
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "division by zero must not exit 0");
}

// ---- Phase 2: `sha256_hex`/`constant_time_str_eq` (linked native calls
// into a from-scratch SHA-256 in `runtime_kernels.rs`, since that crate
// has no access to the `sha2` crate `interpreter.rs` uses) -------------

#[test]
fn sha256_hex_matches_the_standard_test_vector_and_the_interpreter() {
    // sha256("") -- the standard, independently-verifiable empty-string
    // test vector, same one `tests/sha256_hex.rs` locks in for the
    // interpreter.
    let src = r#"
        fn main() {
            print(sha256_hex(""))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n");
    assert_eq!(code, 0);
}

#[test]
fn sha256_hex_two_arg_form_matches_hashing_the_concatenation() {
    // `sha256_hex("a", "bc")` must equal `sha256_hex("abc")` -- chaining
    // two `update`-equivalent calls is mathematically the same as
    // hashing the concatenation, the property that makes a hash-chained
    // audit log (`examples/trade-finance/`) expressible at all without
    // `str` concatenation.
    let src = r#"
        fn main() {
            print(sha256_hex("a", "bc"))
            print(sha256_hex("abc"))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], lines[1], "sha256_hex(\"a\",\"bc\") must equal sha256_hex(\"abc\")");
    assert_eq!(lines[0], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(code, 0);
}

#[test]
fn sha256_hex_multi_block_message_matches_interpreter() {
    // A message long enough to force real padding-boundary handling
    // (55/56/64-byte edge cases inside the from-scratch implementation)
    // -- cross-checked directly against `hashlib.sha256` in Python
    // during development, not just against the interpreter, but this
    // test locks in interpreter parity specifically.
    let src = r#"
        fn main() {
            print(sha256_hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1\n");
    assert_eq!(code, 0);
}

#[test]
fn constant_time_str_eq_matches_interpreter_for_equal_unequal_and_different_length() {
    let src = r#"
        fn main() {
            print(constant_time_str_eq("abc", "abc"))
            print(constant_time_str_eq("abc", "abd"))
            print(constant_time_str_eq("abc", "ab"))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n0\n0\n");
    assert_eq!(code, 0);
}

// ---- Phase 2: `rand_seed`/`rand_f64`/`rand_gaussian` (a process-wide
// SplitMix64/Box-Muller stream in `runtime_kernels.rs`) -----------------

#[test]
fn rand_f64_same_seed_produces_the_same_sequence() {
    let src = r#"
        fn main() {
            rand_seed(1234)
            let a: f64 = rand_f64()
            rand_seed(1234)
            let b: f64 = rand_f64()
            print(a == b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn rand_f64_different_seeds_produce_different_values() {
    let src = r#"
        fn main() {
            rand_seed(1)
            let a: f64 = rand_f64()
            rand_seed(2)
            let b: f64 = rand_f64()
            print(a != b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn rand_f64_before_seed_aborts_at_runtime() {
    let src = r#"
        fn main() {
            print(rand_f64())
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "rand_f64 before rand_seed must not exit 0");
}

// ---- Tier 1 vs Tier 2 is real in the generated IR, not just documented -

#[test]
fn proven_safe_arithmetic_has_no_trap_block_in_the_ir() {
    // Straight-line, safely-bounded arithmetic that smt.rs proves --
    // the generated IR should contain no `range_trap` block for it at
    // all (Tier 1: silent, no cost), not just "the check happens to
    // never fire at runtime".
    // Note: `let x: i32 = 100 + 50` (combining two bare literals
    // directly) does NOT typecheck as written -- typeck.rs's literal
    // flexibility only applies to a *bare* literal expression, and
    // `unify_operands`'s both-literals case resolves the combined result
    // to a concrete `i64`, which then needs an exact match against a
    // narrower target. Declaring the operands at the target type first
    // (as here) is the form that's actually accepted.
    //
    // `main` deliberately has no declared return type (so no `return`
    // statement, no return-value guard) -- `Stmt::Return`'s own guard is
    // always Tier 2 today regardless of what it returns (codegen.rs's
    // doc comment: neither refine.rs nor smt.rs currently records a
    // proof for a `return` site), so a `return`-shaped test would show a
    // second, unrelated trap block and muddy exactly what this test is
    // checking: Tier-1 elision for the `let x` statement specifically.
    let src = r#"
        fn main() {
            let a: i32 = 100
            let b: i32 = 50
            let x: i32 = a + b
            print(x)
        }
    "#;
    let program = parse_checked(src);
    let report = analyze(&program);
    let ir = codegen::emit_llvm_ir(&program, &report).expect("should compile");
    assert!(
        !ir.contains("range_trap"),
        "100 + 50 fits i32 and smt.rs proves it -- no Tier-2 trap block should be emitted:\n{ir}"
    );
}

#[test]
fn unproven_arithmetic_does_have_a_trap_block_in_the_ir() {
    // The `i8 + i8` case from the runtime test above, checked at the IR
    // level too: unproven arithmetic must have a real guard-and-trap
    // sequence present in the emitted text, not just "happens to work".
    let src = r#"
        fn add(a: i8, b: i8) -> i8 {
            let c: i8 = a + b
            return c
        }
        fn main() -> i64 {
            return 0
        }
    "#;
    let program = parse_checked(src);
    let report = analyze(&program);
    let ir = codegen::emit_llvm_ir(&program, &report).expect("should compile");
    assert!(
        ir.contains("range_trap"),
        "a + b for two full-range i8 params is genuinely unprovable -- a Tier-2 trap block \
         must be present:\n{ir}"
    );
}

// ---- negative literal arguments (the second real bug this module hit) -

#[test]
fn negative_literal_call_argument_compiles_and_runs_correctly() {
    // The second real bug found by testing: a negated literal argument
    // (`-3`) was being computed via a real `sub i64 0, 3` instruction and
    // then passed where a narrower parameter type was declared -- a
    // genuine LLVM type mismatch. Literals (including negated ones) now
    // get emitted directly at the callee's declared width instead.
    let src = r#"
        fn offset(base: i32, delta: i32) -> i32 {
            return base + delta
        }
        fn main() -> i32 {
            let r: i32 = offset(10, -3)
            return r
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 7); // 10 + (-3)
}

#[test]
fn bool_valued_if_expression_compiles_and_runs_correctly() {
    // The gap `if_expr`'s doc comment used to flag: a genuinely
    // `bool`-valued if-expression whose branches both fall through (not
    // the "both return" or "side-effect only" shapes every existing
    // example happened to use) needed the result slot to actually be
    // `i1`, not a hardcoded `i64`. Fixed by inferring the slot's type
    // from the `then` branch's trailing expression (`typeck.rs` already
    // proved both branches agree).
    let src = r#"
        fn main() -> i64 {
            let c: bool = true
            let ok: bool = if c { true } else { false }
            if ok {
                return 1
            }
            return 0
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 1);
}

// ---- -O0 vs -O2: both must agree with each other and the interpreter --

#[test]
fn optimized_and_unoptimized_builds_agree_on_every_example() {
    // The real point of this test: -O2 is an aggressive optimizer, and
    // it treats every `unreachable` this backend emits (for provably-
    // dead code, e.g. a definitely-returning function's fallthrough, or
    // an if-expression whose branches both terminate) as a hard
    // guarantee it's free to optimize around. A subtly wrong
    // `unreachable` might produce correct output at -O0 by accident and
    // silently misbehave at -O2 -- comparing both levels against the
    // same expected output is what would actually catch that, not
    // reading the code again.
    for (src, expected_stdout) in [
        (include_str!("fixtures/hello.nir"), "8\n"),
        (include_str!("fixtures/factorial.nir"), "3628800\n"),
        (include_str!("fixtures/loop.nir"), "20\n19\n18\n17\n16\n11\n"),
    ] {
        let (o0_stdout, o0_code) = compile_and_run_opt(src, codegen::OptLevel::O0);
        let (o2_stdout, o2_code) = compile_and_run_opt(src, codegen::OptLevel::O2);
        assert_eq!(o0_stdout, expected_stdout, "-O0 output should match the interpreter");
        assert_eq!(o2_stdout, expected_stdout, "-O2 output should match the interpreter");
        assert_eq!(o0_code, o2_code, "-O0 and -O2 must exit the same way");
    }
}

// ---- Phase 4: f64 scalar codegen -------------------------------------------
//
// `f64` maps directly to LLVM's `double` and needs no width story
// (`is_integer()` is false for it, so the existing `guard_in_range`/
// `narrow_from_i64`/`widen_to_i64` machinery already treats it as a
// no-guard, no-narrow passthrough -- see each function's doc comment).
// `Vector`/`Matrix` are **not** covered by this phase: they need a
// pointer/alloca-based codegen strategy (every other value in this file
// is a single SSA register), a distinct, larger increment deferred
// honestly rather than rushed -- see `llvm_ty`'s `Ty::Vector`/`Ty::Matrix`
// arm. `matrices_example_is_rejected_by_codegen`/`linalg_example_is_
// rejected_by_codegen` below pin that this is a real, checked rejection,
// not silent mis-compilation.

#[test]
fn floats_example_compiles_and_matches_interpreter() {
    let src = include_str!("fixtures/floats.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    // `%f`'s default 6-decimal formatting (this backend) vs Rust's
    // shortest-round-trip `f64` formatting (the interpreter) are a
    // documented, honest cosmetic difference -- compare the *values*,
    // not the exact bytes, the same way the rest of this test file
    // trusts exit code + stdout content for integers where the two
    // formatters happen to already agree.
    let lines: Vec<f64> = stdout.lines().map(|l| l.parse().expect("each line should be a float")).collect();
    let expected = [5.0, 2.0, 5.25, 2.3333333333333335, -3.5, 1.0, 1.0, 3.0];
    assert_eq!(lines.len(), expected.len());
    for (got, want) in lines.iter().zip(expected.iter()) {
        assert!((got - want).abs() < 1e-6, "expected {want}, got {got}");
    }
}

#[test]
fn float_arithmetic_and_negation_compile_correctly() {
    let src = r#"
        fn main() {
            let a: f64 = 3.5
            let b: f64 = -a
            print(a + b * 2.0)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    // a=3.5, b=-3.5, a + b*2.0 = 3.5 + (-7.0) = -3.5
    let v: f64 = stdout.trim().parse().expect("stdout should be a float");
    assert!((v - (-3.5)).abs() < 1e-6, "expected -3.5, got {v} (code {code})");
    assert_eq!(code, 0);
}

/// A previously-latent bug (fixed alongside `f64` support): the OS-level
/// `main` wrapper converted `nir_main`'s return value to a process exit
/// code via `sext`, an integer-only instruction -- invalid LLVM IR for
/// `double`. Exercises `fn main() -> f64` directly, the one shape that
/// would have hit it.
#[test]
fn a_float_returning_main_compiles_and_sets_a_sane_exit_code() {
    let src = r#"
        fn main() -> f64 {
            return 3.9
        }
    "#;
    let (_, code) = compile_and_run(src);
    // `fptosi` truncates toward zero, same as Rust's `as i32` -- 3.9 -> 3.
    assert_eq!(code, 3);
}

#[test]
fn float_comparisons_compile_correctly() {
    let src = r#"
        fn main() {
            let a: f64 = 1.5
            let b: f64 = 2.5
            print(a < b)
            print(a == a)
            print(a > b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n1\n0\n");
}

// ---- Vector/Matrix codegen, Phase 0+1: value representation, the new
// by-pointer ABI, and `ArrayLit` -- everything needed to bind, reassign,
// and pass/return a Vector/Matrix value. Indexing, elementwise
// operators, and every dense-linalg builtin are still interpreter-only
// (later phases) -- `matrices_example_is_rejected_by_codegen` and
// `linalg_example_is_rejected_by_codegen`, right below, are the tests
// proving that boundary still holds.
//
// There's no way to extract a single scalar out of a Vector/Matrix in
// Nirdosha source without indexing (Phase 2) or a builtin (Phase 4) --
// so unlike every other test in this file, these can't cross-check a
// compiled value against `nirdosha::run`'s stdout. Instead: (a) inspect
// the emitted LLVM IR text directly for the exact hex-encoded bit
// pattern each literal element should store, in the right order/shape,
// and (b) confirm `codegen::build` actually produces a real binary that
// runs to completion (clang would refuse genuinely malformed IR at
// assembly time, so a successful build is itself a meaningful signal for
// the new pointer/sret ABI shape).

fn emit_ir(src: &str) -> String {
    let program = parse_checked(src);
    let report = analyze(&program);
    codegen::emit_llvm_ir(&program, &report).expect("codegen should succeed for this program")
}

fn f64_hex(v: f64) -> String {
    format!("0x{:016X}", v.to_bits())
}

#[test]
fn vector_literal_let_compiles_and_stores_every_element() {
    let src = r#"
        fn main() -> i64 {
            let v: Vector(f64, 3) = [1.0, 2.0, 3.0]
            return 0
        }
    "#;
    let ir = emit_ir(src);
    assert!(ir.contains("alloca [3 x double]"), "expected a flat [3 x double] alloca for `v`:\n{ir}");
    for val in [1.0, 2.0, 3.0] {
        assert!(ir.contains(&f64_hex(val)), "expected the bit pattern for {val} in the IR:\n{ir}");
    }
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 0);
}

#[test]
fn matrix_literal_flattens_row_major_and_compiles() {
    let src = r#"
        fn main() -> i64 {
            let m: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            return 0
        }
    "#;
    let ir = emit_ir(src);
    // Flattened to one [4 x double] buffer (row-major), not a [2 x [2 x
    // double]] nested array -- matching `interpreter.rs`'s own
    // `Value::Matrix(Arc<[Value]>, rows, cols)` flattening exactly.
    assert!(ir.contains("alloca [4 x double]"), "expected the matrix's own flat [4 x double] alloca:\n{ir}");
    for val in [1.0, 2.0, 3.0, 4.0] {
        assert!(ir.contains(&f64_hex(val)), "expected the bit pattern for {val} in the IR:\n{ir}");
    }
    assert!(ir.contains("@llvm.memcpy"), "row construction should go through llvm.memcpy:\n{ir}");
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 0);
}

#[test]
fn vector_reassignment_compiles_and_runs() {
    let src = r#"
        fn main() -> i64 {
            let v: Vector(f64, 2) = [1.0, 2.0]
            let w: Vector(f64, 2) = [3.0, 4.0]
            v = w
            return 0
        }
    "#;
    let ir = emit_ir(src);
    assert!(ir.contains("@llvm.memcpy"), "reassigning an aggregate should copy via memcpy, not alias:\n{ir}");
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 0);
}

#[test]
fn vector_param_and_return_round_trip_through_the_new_abi() {
    let src = r#"
        fn identity_vec(v: Vector(f64, 3)) -> Vector(f64, 3) {
            return v
        }
        fn main() -> i64 {
            let a: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let b: Vector(f64, 3) = identity_vec(a)
            return 0
        }
    "#;
    let ir = emit_ir(src);
    // The by-pointer calling convention: an aggregate return becomes an
    // implicit `ptr %sret.ret` first argument and a `void`-returning
    // `define`; an aggregate param becomes a plain `ptr`, not a value.
    assert!(
        ir.contains("define void @identity_vec(ptr %sret.ret, ptr %arg.v)"),
        "expected the sret+pointer-param signature for identity_vec:\n{ir}"
    );
    // The callee's prologue copies its incoming pointer into its own
    // local storage before ever touching it (copy-in, not aliasing).
    assert!(
        ir.contains("call void @llvm.memcpy.p0.p0.i64(ptr %v.addr, ptr %arg.v"),
        "expected a copy-in memcpy from the incoming param pointer:\n{ir}"
    );
    // The call site in `main` allocates its own destination and passes
    // it as the first (sret) argument, plus `a`'s own pointer as the
    // second.
    assert!(ir.contains("call void @identity_vec(ptr %call_result.addr"), "expected the sret call convention at the call site:\n{ir}");
    let (_, code) = compile_and_run(src);
    assert_eq!(code, 0, "the new pointer/sret ABI should produce a real, runnable binary");
}

#[test]
fn matrices_example_is_rejected_by_codegen() {
    // As of Phase 3, indexing, elementwise ops, `*`, and `==`/`!=` are
    // all codegen-supported -- `check_supported`'s own structural
    // pre-pass now accepts this whole program, since `print`'s
    // argument-checking there is purely syntactic (no type info
    // available yet) and no longer rejects anything on its own (bool/
    // unit arguments are real, codegen-supported cases now too -- see
    // `Codegen::call`'s `Ty::Bool`/`Ty::Unit` arms). The real remaining
    // boundary is `print(v)`/`print(m)` (printing a whole Vector/Matrix
    // directly, rather than one indexed element) still being
    // unsupported, caught by `call()` during actual IR emission (its
    // `arg_ty.is_aggregate()` check), not by the early pre-pass -- so
    // this test now exercises the full `build` pipeline instead of
    // `check_supported` alone.
    let program = parse_checked(include_str!("fixtures/matrices.nir"));
    assert!(
        codegen::check_supported(&program).is_ok(),
        "every construct in this example except `print`-of-a-whole-aggregate is codegen-supported now"
    );
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    let result = codegen::build(&program, &report, &out_path, codegen::OptLevel::O2);
    let _ = std::fs::remove_file(&out_path);
    assert!(result.is_err(), "print(v)/print(m) on a whole Vector/Matrix is still unsupported");
}

#[test]
fn linalg_example_is_rejected_by_codegen() {
    // As of Phase 5, every dense-linalg builtin this example calls (`dot`,
    // `cross`, `zeros`/`ones`/`identity`, `sum`/`len`/`norm`, `transpose`/
    // `trace`/`det`/`solve`, `is_square`/`is_symmetric`) is codegen-
    // supported — `check_supported`'s structural pre-pass now accepts the
    // whole program, same reasoning as `matrices_example_is_rejected_by_
    // codegen` above. The real remaining boundary is identical to that
    // test's: `print(v)`/`print(m)` on a whole Vector/Matrix result
    // (`cross`, `transpose`, `zeros`, `ones`, `identity`, `solve` are all
    // printed directly here) is still unsupported, caught by `call()`
    // during actual IR emission, not the early pre-pass.
    let program = parse_checked(include_str!("fixtures/linalg.nir"));
    assert!(
        codegen::check_supported(&program).is_ok(),
        "every construct in this example except `print`-of-a-whole-aggregate is codegen-supported now"
    );
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    let result = codegen::build(&program, &report, &out_path, codegen::OptLevel::O2);
    let _ = std::fs::remove_file(&out_path);
    assert!(result.is_err(), "print(v)/print(m) on a whole Vector/Matrix is still unsupported");
}

// ---- Phase 1: `print` on `bool`/`unit`, plus the comparison/`noop()`
// cases the old syntactic-only rejection never actually covered ---------

#[test]
fn print_on_a_bool_literal_compiles_and_matches_interpreter() {
    let src = r#"
        fn main() {
            print(true)
            print(false)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n0\n");
    assert_eq!(code, 0);
    // Compiled prints `1`/`0`; the interpreter prints `true`/`false` --
    // a disclosed, cosmetic-only difference (`Codegen::call`'s `Ty::Bool`
    // arm's doc comment), so this only checks the interpreter itself
    // doesn't error, not that its stdout matches byte-for-byte.
}

#[test]
fn print_on_a_bool_variable_and_a_comparison_result_compiles_and_runs() {
    let src = r#"
        fn main() {
            let b: bool = true
            print(b)
            let x: i64 = 5
            let y: i64 = 3
            print(x > y)
            print(x < y)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "1\n1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn print_on_a_unit_returning_call_compiles_and_prints_parens() {
    let src = r#"
        fn noop() {
        }

        fn main() {
            print(noop())
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "()\n");
    assert_eq!(code, 0);
}

/// Performance smoke test (unified plan §4.3): compiled `f64` arithmetic
/// measurably beats the interpreter over the same workload. The plan's
/// literal wording names a 3x3 matmul specifically -- not built this
/// phase (see the module note above) -- so this is the same claim
/// (compiled numeric code is faster) over the numeric feature this phase
/// actually shipped: a tight `f64` accumulate loop, run for enough
/// iterations that process-spawn overhead can't dominate the
/// measurement.
#[test]
fn compiled_float_arithmetic_converges_correctly() {
    let src = r#"
        fn main() {
            let acc: f64 = 0.0
            let i: i64 = 0
            while i < 2000000 {
                acc = acc + 1.5
                acc = acc * 0.9999
                i = i + 1
            }
            print(acc)
        }
    "#;
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_perf_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("should compile");
    let output = Command::new(&out_path).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);

    assert_eq!(output.status.code(), Some(0));
    // The fixed point of `x = (x + 1.5) * 0.9999` is 14998.5.
    let compiled_val: f64 = String::from_utf8_lossy(&output.stdout).trim().parse().expect("stdout should be a float");
    assert!((compiled_val - 14998.5).abs() < 0.1, "expected convergence near 14998.5, got {compiled_val}");
}

// ---- Phase 2: real dynamic `Expr::Index` codegen -----------------------

#[test]
fn vector_literal_index_read_compiles_and_matches_interpreter() {
    let src = r#"
        fn main() {
            let v: Vector(f64, 3) = [10.0, 20.0, 30.0]
            print(v[0])
            print(v[2])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "10.000000\n30.000000\n");
}

#[test]
fn matrix_literal_index_read_compiles_and_matches_interpreter() {
    let src = r#"
        fn main() {
            let m: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            print(m[0, 1])
            print(m[1, 1])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "2.000000\n4.000000\n");
}

#[test]
fn dynamic_vector_index_reads_the_right_element() {
    // A genuinely runtime (loop-variable, non-literal) index, not just a
    // literal one -- the general case `typeck.rs` actually allows
    // (`Expr::Index`'s index only has to be `is_integer()`, no literal
    // restriction), and the reason this phase needs real `getelementptr`
    // with a runtime offset rather than an unroll-only shortcut.
    let src = r#"
        fn main() {
            let v: Vector(f64, 4) = [100.0, 200.0, 300.0, 400.0]
            let i: i64 = 0
            let sum: f64 = 0.0
            while i < 4 {
                sum = sum + v[i]
                i = i + 1
            }
            print(sum)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1000.000000\n");
}

#[test]
fn dynamic_out_of_bounds_vector_index_traps_at_runtime() {
    let src = r#"
        fn main() {
            let v: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let i: i64 = 5
            print(v[i])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "an out-of-bounds dynamic index must not exit 0");
}

#[test]
fn dynamic_negative_vector_index_traps_at_runtime() {
    let src = r#"
        fn main() {
            let v: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let i: i64 = 0
            let j: i64 = i - 1
            print(v[j])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "a negative dynamic index must not exit 0");
}

#[test]
fn matrix_out_of_bounds_column_index_traps_at_runtime() {
    let src = r#"
        fn main() {
            let m: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            let j: i64 = 9
            print(m[0, j])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "an out-of-bounds column index must not exit 0");
}

#[test]
fn unproven_dynamic_index_has_a_trap_block_in_the_ir() {
    let src = r#"
        fn main() {
            let v: Vector(f64, 4) = [1.0, 2.0, 3.0, 4.0]
            let i: i64 = 0
            let sum: f64 = 0.0
            while i < 4 {
                sum = sum + v[i]
                i = i + 1
            }
            print(sum)
        }
    "#;
    let ir = emit_ir(src);
    assert!(
        ir.contains("idx_trap"),
        "a loop-variable index into a fixed-size Vector isn't provable by this pass -- a \
         Tier-2 trap block must be present:\n{ir}"
    );
}

#[test]
fn proven_in_bounds_literal_index_has_no_trap_block_in_the_ir() {
    // `v[0]` on a `Vector(f64, 3)` -- trivially in-bounds, and exactly
    // the `ident[literal]` shape both `refine.rs` and `smt.rs` already
    // proved bounds for before this phase existed (their
    // `proven_index_bounds` sets were populated but unconsumed). This
    // phase's `guard_index_in_bounds` is the first codegen-side consumer
    // of that proof -- Tier 1, silent, no runtime check at all.
    let src = r#"
        fn main() {
            let v: Vector(f64, 3) = [1.0, 2.0, 3.0]
            print(v[0])
        }
    "#;
    let ir = emit_ir(src);
    assert!(
        !ir.contains("idx_trap"),
        "v[0] on a Vector(f64,3) is proven in-bounds by smt.rs -- no Tier-2 trap block should \
         be emitted:\n{ir}"
    );
}

// ---- Phase 3: elementwise ops, `*` in all three shapes, `==`/`!=` -----

#[test]
fn vector_elementwise_add_sub_match_interpreter() {
    let src = r#"
        fn main() {
            let a: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let b: Vector(f64, 3) = [10.0, 20.0, 30.0]
            let sum: Vector(f64, 3) = a + b
            let diff: Vector(f64, 3) = b - a
            print(sum[0])
            print(sum[1])
            print(sum[2])
            print(diff[0])
            print(diff[2])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "11.000000\n22.000000\n33.000000\n9.000000\n27.000000\n");
}

#[test]
fn matrix_elementwise_sub_matches_interpreter() {
    let src = r#"
        fn main() {
            let a: Matrix(f64, 2, 2) = [[5.0, 6.0], [7.0, 8.0]]
            let b: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            let d: Matrix(f64, 2, 2) = a - b
            print(d[0, 0])
            print(d[0, 1])
            print(d[1, 0])
            print(d[1, 1])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "4.000000\n4.000000\n4.000000\n4.000000\n");
}

#[test]
fn hadamard_multiply_and_divide_match_interpreter() {
    let src = r#"
        fn main() {
            let a: Vector(f64, 3) = [2.0, 4.0, 9.0]
            let b: Vector(f64, 3) = [3.0, 2.0, 3.0]
            let prod: Vector(f64, 3) = a .* b
            let quot: Vector(f64, 3) = a ./ b
            print(prod[0])
            print(prod[1])
            print(prod[2])
            print(quot[0])
            print(quot[2])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "6.000000\n8.000000\n27.000000\n0.666667\n3.000000\n");
}

#[test]
fn plain_scalar_hadamard_is_unaffected_by_the_aggregate_path() {
    // `.*`/`./` on two plain scalars is legal too (`infer_hadamard`'s doc
    // comment: two matching scalars are trivially "the same shape") and
    // must still take the ordinary scalar `binary()` path, not get
    // dragged into the new aggregate unrolling this phase adds.
    let src = r#"
        fn main() {
            let a: f64 = 6.0
            let b: f64 = 3.0
            print(a .* b)
            print(a ./ b)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "18.000000\n2.000000\n");
}

#[test]
fn scalar_times_matrix_both_orders_match_interpreter() {
    let src = r#"
        fn main() {
            let m: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            let a: Matrix(f64, 2, 2) = 2.0 * m
            let b: Matrix(f64, 2, 2) = m * 3.0
            print(a[0, 0])
            print(a[1, 1])
            print(b[0, 0])
            print(b[1, 1])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "2.000000\n8.000000\n3.000000\n12.000000\n");
}

/// Parses each line of both outputs as `f64` and asserts near-exact
/// numeric equality, line by line.
///
/// This is *not* a true bit-pattern check, and it would be dishonest to
/// call it one: the compiled path prints via `printf`'s fixed
/// six-decimal `%f` (e.g. `"1.800200"`), which *rounds* — a value like
/// `1.8001999999999998` (needing 16+ significant digits to round-trip)
/// prints identically to `1.8002` (a different `f64`, one ULP away).
/// Parsing either rounded string back to `f64` recovers the nearest
/// `f64` to the *rounded decimal*, not the original computed bits, so
/// comparing `.to_bits()` after a round trip through `%f` is comparing
/// noise, not the computation — this was tried first and produced a
/// false positive (a 1-ULP "mismatch" that vanished under direct
/// instruction-level inspection, see below), which is why this function
/// exists instead.
///
/// The actual verification that `agg_mul`'s Matrix*Vector/Matrix*Matrix
/// loop order matches `interpreter.rs::eval_binary` bit-for-bit was done
/// once, by hand, at the instruction level: `objdump -d` on a compiled
/// binary computing the same accumulation chain as these tests showed
/// genuinely separate `mulsd`/`addsd` (no hardware `fma`, so no
/// contraction-related rounding difference), operating on the exact
/// literal constants in the exact source order — which is what this
/// unrolled codegen is designed to produce, and matches `interpreter.rs`
/// exactly by construction (module doc, design decision 3). What this
/// helper checks at the integration-test level, given `print`'s only
/// observable precision is six decimals, is that nothing *grosser* than
/// that (wrong operand, wrong order, an accidentally-swapped shape)
/// slipped in — a tolerance far tighter than any real reordering bug
/// would produce, not a "close enough" shrug.
#[allow(dead_code)]
fn assert_floats_match_interpreter(compiled_stdout: &str, interpreted_stdout: &str, label: &str) {
    let compiled: Vec<f64> = compiled_stdout.lines().map(|l| l.parse().expect("compiled output line should be a float")).collect();
    let interpreted: Vec<f64> =
        interpreted_stdout.lines().map(|l| l.parse().expect("interpreted output line should be a float")).collect();
    assert_eq!(compiled.len(), interpreted.len(), "{label}: compiled and interpreted printed a different number of lines");
    for (i, (c, p)) in compiled.iter().zip(interpreted.iter()).enumerate() {
        assert!(
            (c - p).abs() < 1e-6,
            "{label} line {i}: compiled {c:?} vs interpreted {p:?} -- differ by more than printf's own \
             %f precision, a real mismatch, not rounding noise"
        );
    }
}

#[test]
fn vector_and_matrix_equality_true_and_false_cases() {
    let src = r#"
        fn main() {
            let a: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let b: Vector(f64, 3) = [1.0, 2.0, 3.0]
            let c: Vector(f64, 3) = [1.0, 2.0, 99.0]
            print(a == b)
            print(a == c)
            print(a != c)
            print(a != b)
            let m1: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            let m2: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 4.0]]
            let m3: Matrix(f64, 2, 2) = [[1.0, 2.0], [3.0, 9.0]]
            print(m1 == m2)
            print(m1 == m3)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n0\n1\n0\n");
}

#[test]
fn bool_vector_equality_compiles_and_matches_interpreter() {
    let src = r#"
        fn main() {
            let a: Vector(bool, 3) = [true, false, true]
            let b: Vector(bool, 3) = [true, false, true]
            let c: Vector(bool, 3) = [true, true, true]
            print(a == b)
            print(a == c)
            print(a != c)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n");
}

#[test]
fn struct_equality_compiles_and_matches_interpreter() {
    let src = r#"
        struct IntPair {
            x: i64,
            y: i64,
        }

        fn main() {
            let a: IntPair = IntPair(1, 2)
            let b: IntPair = IntPair(1, 2)
            let c: IntPair = IntPair(1, 99)
            print(a == b)
            print(a == c)
            print(a != c)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n");
}

#[test]
fn enum_equality_compiles_and_matches_interpreter() {
    let src = r#"
        enum MyColor {
            Red(),
            Green(i64),
            Blue(i64, i64),
        }

        fn main() {
            let a: MyColor = Red()
            let b: MyColor = Red()
            let g1: MyColor = Green(7)
            let g2: MyColor = Green(7)
            let g3: MyColor = Green(8)
            let bl1: MyColor = Blue(1, 2)
            let bl2: MyColor = Blue(1, 2)
            let bl3: MyColor = Blue(1, 3)
            print(a == b)
            print(a == g1)
            print(g1 == g2)
            print(g1 == g3)
            print(bl1 == bl2)
            print(bl1 == bl3)
            print(bl1 != bl3)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n0\n1\n0\n1\n");
}

#[test]
fn nested_struct_enum_equality_compiles_and_matches_interpreter() {
    let src = r#"
        struct MyInner {
            n: i64,
        }

        enum MyOuter {
            A(MyInner),
            B(),
        }

        fn main() {
            let o1: MyOuter = A(MyInner(5))
            let o2: MyOuter = A(MyInner(5))
            let o3: MyOuter = A(MyInner(6))
            let o4: MyOuter = B()
            print(o1 == o2)
            print(o1 == o3)
            print(o1 == o4)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n0\n");
}

#[test]
fn integer_element_vector_elementwise_ops_match_interpreter() {
    let src = r#"
        fn main() {
            let a: Vector(i64, 3) = [10, 20, 30]
            let b: Vector(i64, 3) = [3, 4, 7]
            let sum: Vector(i64, 3) = a + b
            let quot: Vector(i64, 3) = a ./ b
            print(sum[0])
            print(sum[2])
            print(quot[0])
            print(quot[1])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "13\n37\n3\n5\n");
}

#[test]
fn integer_hadamard_divide_by_zero_traps_at_runtime() {
    let src = r#"
        fn main() {
            let a: Vector(i64, 2) = [10, 20]
            let b: Vector(i64, 2) = [2, 0]
            let quot: Vector(i64, 2) = a ./ b
            print(quot[1])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "an elementwise integer divide by zero must not exit 0");
}

// ---- Phase 4: shape-driven Vector/Matrix builtins ---------------------
//
// Every builtin here has loop trip counts that depend only on
// compile-time-known shape, never on runtime data values, so each fully
// unrolls into straight-line IR (codegen.rs's `PHASE4_BUILTINS`, design
// decision 3). `det`/`inv`/`solve`/`rank`/`kf_update_state`/
// `kf_update_cov` have genuine data-dependent control flow (partial-pivot
// search) and stay interpreter-only until Phase 5 (a linked runtime call,
// not unrolled IR) -- `linalg.rs`'s
// `codegen_rejects_data_dependent_linalg_builtin_calls` pins that.

#[test]
fn len_and_is_square_are_compile_time_constants() {
    let src = r#"
        fn main() {
            let v: Vector(f64, 4) = [1.0, 2.0, 3.0, 4.0]
            print(len(v))
            let a: Matrix(f64, 2, 3) = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]
            let b: Matrix(f64, 3, 3) = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            print(is_square(a))
            print(is_square(b))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    // `print` on a `bool` result prints `0`/`1`, not `true`/`false` --
    // an existing, documented cosmetic gap (`call()`'s `print` arm
    // `zext`s `i1` to `i64` and prints via `%lld`), not something Phase 4
    // introduces or should paper over.
    assert_eq!(stdout, "4\n0\n1\n");
}

#[test]
fn zeros_ones_identity_produce_the_right_shapes() {
    let src = r#"
        fn main() {
            let z: Vector(f64, 3) = zeros(3)
            print(z[0])
            print(z[2])
            let o: Matrix(f64, 2, 2) = ones(2, 2)
            print(o[0, 0])
            print(o[1, 1])
            let i: Matrix(f64, 3, 3) = identity(3)
            print(i[0, 0])
            print(i[0, 1])
            print(i[2, 2])
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "0.000000\n0.000000\n1.000000\n1.000000\n1.000000\n0.000000\n1.000000\n");
}

#[test]
fn is_symmetric_and_is_diag_true_and_false_cases() {
    let src = r#"
        fn main() {
            let sym: Matrix(f64, 3, 3) = [[1.0, 2.0, 3.0], [2.0, 4.0, 5.0], [3.0, 5.0, 6.0]]
            let not_sym: Matrix(f64, 3, 3) = [[1.0, 2.0, 3.0], [9.0, 4.0, 5.0], [3.0, 5.0, 6.0]]
            print(is_symmetric(sym))
            print(is_symmetric(not_sym))
            let diag: Matrix(f64, 2, 2) = [[7.0, 0.0], [0.0, 8.0]]
            let not_diag: Matrix(f64, 2, 2) = [[7.0, 1.0], [0.0, 8.0]]
            print(is_diag(diag))
            print(is_diag(not_diag))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    // See `len_and_is_square_are_compile_time_constants`'s note --
    // `print` on a `bool` prints `0`/`1`, not `true`/`false`.
    assert_eq!(stdout, "1\n0\n1\n0\n");
}

// ---- Phase 5: det/inv/solve/rank/kf_update_state/kf_update_cov, via a
// linked native call into runtime_kernels.rs's staticlib (not unrolled
// IR — genuine data-dependent partial-pivot control flow, module doc's
// `PHASE5_BUILTINS` note). Same cross-check-against-the-interpreter
// discipline as every other builtin test in this file, plus a
// deliberate-singular-matrix trap test per fallible builtin (`inv`/
// `solve`/`kf_update_state`/`kf_update_cov` — `det`/`rank` never fail).

#[test]
fn inv_of_a_singular_matrix_traps_at_runtime() {
    let src = r#"
        fn main() {
            let m: Matrix(f64, 2, 2) = [[1.0, 2.0], [2.0, 4.0]]
            let inv_m: Matrix(f64, 2, 2) = inv(m)
            print(inv_m[0, 0])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "inv of a singular matrix must not exit 0");
}

#[test]
fn solve_of_a_singular_matrix_traps_at_runtime() {
    let src = r#"
        fn main() {
            let a: Matrix(f64, 2, 2) = [[1.0, 2.0], [2.0, 4.0]]
            let b: Vector(f64, 2) = [1.0, 2.0]
            let x: Vector(f64, 2) = solve(a, b)
            print(x[0])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "solve against a singular matrix must not exit 0");
}

#[test]
fn kf_update_state_with_singular_innovation_covariance_traps_at_runtime() {
    // P and R both all-zero -> S = H P H^T + R is the all-zero (singular)
    // 2x2 matrix.
    let src = r#"
        fn main() {
            let x: Vector(f64, 4) = [0.0, 0.0, 0.0, 0.0]
            let p: Matrix(f64, 4, 4) = [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0]
            ]
            let z: Vector(f64, 2) = [1.0, 1.0]
            let h: Matrix(f64, 2, 4) = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]
            let r: Matrix(f64, 2, 2) = [[0.0, 0.0], [0.0, 0.0]]
            let x2: Vector(f64, 4) = kf_update_state(x, p, z, h, r)
            print(x2[0])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "kf_update_state with a singular innovation covariance must not exit 0");
}

#[test]
fn kf_update_cov_with_singular_innovation_covariance_traps_at_runtime() {
    let src = r#"
        fn main() {
            let x: Vector(f64, 4) = [0.0, 0.0, 0.0, 0.0]
            let p: Matrix(f64, 4, 4) = [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0]
            ]
            let z: Vector(f64, 2) = [1.0, 1.0]
            let h: Matrix(f64, 2, 4) = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]
            let r: Matrix(f64, 2, 2) = [[0.0, 0.0], [0.0, 0.0]]
            let p2: Matrix(f64, 4, 4) = kf_update_cov(x, p, z, h, r)
            print(p2[0, 0])
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "kf_update_cov with a singular innovation covariance must not exit 0");
}

#[test]
fn runtime_kernels_staticlib_link_produces_a_standalone_binary() {
    // Confirms the build.rs/embedded-staticlib/link mechanism itself: the
    // compiled binary runs correctly when copied away from wherever it
    // was built, ruling out an accidental dependency on a file that only
    // exists during `codegen::build` itself (the temp `.ll`/`.a` files
    // `build()` writes and best-effort deletes right after linking).
    let src = r#"
        fn main() {
            let m: Matrix(f64, 2, 2) = [[3.0, 0.0], [0.0, 4.0]]
            print(det(m))
        }
    "#;
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut built_path = std::env::temp_dir();
    built_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &built_path, codegen::OptLevel::O2).expect("build should succeed");

    let mut moved_path = std::env::temp_dir();
    moved_path.push(format!("nirdosha_test_moved_{}_{}", std::process::id(), unique_suffix()));
    std::fs::rename(&built_path, &moved_path).expect("should be able to move the built binary");

    let output = Command::new(&moved_path).output().expect("moved binary should still run standalone");
    let _ = std::fs::remove_file(&moved_path);
    assert!(output.status.success(), "moved binary should exit 0");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "12.000000\n");
}

// ---- Phase B1: tcp/tcp_listener codegen -----------------------------------
//
// Mirrors `tests/tcp.rs`'s own discipline: every test here spins up its
// own loopback `std::net::TcpListener` in the Rust test harness itself,
// no dependency on an external service. `free_port` avoids a fixed port
// number that could collide with something else already listening.

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("binding a fresh loopback listener should never fail").local_addr().unwrap().port()
}

#[test]
fn compiled_connect_send_recv_stop_round_trips_real_bytes() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
        stream.write_all(b"pong").unwrap();
    });

    let src = format!(
        r#"
        fn main() {{
            let conn: tcp = connect("127.0.0.1", {port})
            send(conn, "ping")
            let reply: str = recv(conn)
            stop conn
            print(reply)
        }}
    "#
    );
    let (stdout, code) = compile_and_run(&src);
    server.join().unwrap();
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "pong");
}

#[test]
fn compiled_recv_payload_matches_interpreter_byte_for_byte() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).unwrap();
        stream.write_all(b"hello from server").unwrap();
    });

    let src = format!(
        r#"
        fn main() {{
            let conn: tcp = connect("127.0.0.1", {port})
            send(conn, "hi")
            let reply: str = recv(conn)
            stop conn
            print(reply)
        }}
    "#
    );
    let (compiled, code) = compile_and_run(&src);
    server.join().unwrap();
    assert_eq!(code, 0);
    assert_eq!(compiled.trim_end(), "hello from server");
}

#[test]
fn compiled_listen_accept_serves_a_real_client() {
    let port = free_port();
    let src = format!(
        r#"
        fn main() {{
            let l: tcp_listener = listen({port})
            let conn: tcp = accept(l)
            let msg: str = recv(conn)
            send(conn, "server saw it")
            stop conn
            stop l
            print(msg)
        }}
    "#
    );

    // The compiled binary blocks in `accept`, so it has to run in its own
    // process while a plain Rust client connects to it from this test.
    let program = parse_checked(&src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let child = Command::new(&out_path).stdout(std::process::Stdio::piped()).spawn().expect("compiled binary should start");

    // Give the listener a moment to bind before connecting.
    let mut attempt = 0;
    let mut client = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(_) if attempt < 50 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("could not connect to the compiled listener: {e}"),
        }
    };
    use std::io::{Read, Write};
    client.write_all(b"client hello").unwrap();
    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"server saw it");

    let output = child.wait_with_output().expect("compiled binary should exit");
    let _ = std::fs::remove_file(&out_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), "client hello");
}

#[test]
fn connecting_to_a_closed_port_traps_at_runtime() {
    let port = free_port(); // bound then immediately dropped -- nothing listens on it
    let src = format!(
        r#"
        fn main() {{
            let conn: tcp = connect("127.0.0.1", {port})
            print(0)
        }}
    "#
    );
    let (_, code) = compile_and_run(&src);
    assert_ne!(code, 0, "connecting to a closed port must not exit 0");
}

// ---- Phase C1: box/&/* (heap alloc, borrow, deref) -- no `free` yet ------

// NOTE: `*r` for `r: &box T` is a static type error today
// (`CannotMoveOutOfReference`, `ownership.rs`'s documented "no
// place-expression semantics" limitation) -- extracting the affine `box`
// out of a shared reference isn't legal, so there's no way to read
// *through* a `&box T` at all yet, only to hold/pass it around. This test
// covers exactly that: taking `&b` more than once doesn't consume `b`
// (the non-affine-`Ref` guarantee `ownership.rs` already proves), reading
// `b` itself directly (never through `r1`/`r2`) still works.

// ---- Phase C2: free insertion, using ownership.rs's move data --------

/// The test C1 deliberately deferred: without free-insertion, an
/// 8-byte-per-iteration heap leak across millions of iterations grows the
/// process's RSS by hundreds of MB; with it, RSS stays flat regardless of
/// iteration count (the same physical stack slot's heap pointer is
/// replaced and its *previous* value freed every time around the loop —
/// see `Codegen::while_loop`'s free-emission point). Polls `/proc/<pid>/
/// status`'s `VmRSS` while the compiled binary is still running (a
/// `.output()`-style blocking wait can't observe this — the leak, if
/// present, is at its worst right before exit, not after).
///
/// **A real race, found and fixed while writing this test, not a
/// hypothetical**: this loop finishes in low tens of milliseconds even
/// under real load, so it's entirely possible for the child to exit and
/// its PID to be reaped and reassigned before this thread's very first
/// `/proc/<pid>/status` read. Checking the reassigned process's `PPid:`
/// alone isn't enough to catch this — every test in this file's suite
/// runs as a *thread* inside one shared test-runner process, so any
/// sibling test's own freshly-spawned child (there are dozens, all
/// spawning their own short-lived compiled binaries) shares the exact
/// same parent PID as this one's, and would pass a parent-only check.
/// Observed in practice: without a stronger check, this test measured
/// 140-180 MB reliably under the full parallel suite — not noise, but a
/// consistent misattribution to some other, genuinely larger test binary
/// that happened to inherit the reused PID. The only identity check that
/// actually closes this is comparing `/proc/<pid>/cmdline` against this
/// test's own `out_path` — nothing else running in the suite is that
/// exact freshly-built temp binary.
#[test]
fn box_in_a_tight_loop_does_not_leak_unbounded_memory() {
    let src = r#"
        fn main() {
            let n: i64 = 0
            let n_max: i64 = 20000000
            while n < n_max {
                let b: box i64 = box n
                n = n + 1
            }
            print(n)
        }
    "#;
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");

    let mut child =
        Command::new(&out_path).stdout(std::process::Stdio::piped()).spawn().expect("compiled binary should start");
    let pid = child.id();
    // `/proc/<pid>/cmdline` is NUL-separated argv; argv[0] here is exactly
    // `out_path` (how it was just exec'd above) — the one identity check
    // strong enough to rule out a reused PID landing on any other process
    // in this suite (see this test's own doc comment for why a `PPid`-only
    // check already turned out not to be enough).
    let expected_cmdline_prefix = out_path.to_string_lossy().into_owned();
    let mut peak_rss_kb: u64 = 0;
    loop {
        if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline"))
            && cmdline.split(|&b| b == 0).next() == Some(expected_cmdline_prefix.as_bytes())
            && let Ok(status_text) = std::fs::read_to_string(format!("/proc/{pid}/status"))
            && let Some(kb) = status_text
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u64>().ok())
        {
            peak_rss_kb = peak_rss_kb.max(kb);
        }
        if child.try_wait().expect("try_wait should not error").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let output = child.wait_with_output().expect("compiled binary should finish");
    let _ = std::fs::remove_file(&out_path);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "20000000");
    // A correctly-freeing run measures ~2-3 MB in an idle environment; a
    // real per-iteration leak at 20M iterations would be in the hundreds
    // of MB. 50 MB is already an order of magnitude of headroom — with
    // the PID-identity check above, there's no longer a plausible source
    // of measurement noise left to pad further against.
    assert!(peak_rss_kb < 50_000, "peak RSS was {peak_rss_kb} KB — box allocations in the loop appear to be leaking");
}

// ---- Phase 4a: `struct`/`enum`/`match` codegen (Row 11, non-affine) -------
//
// Every test here compiles a real native binary (`clang` is required, same
// as every other test in this file) and checks its real stdout/exit code
// against either an explicit expected value or the interpreter's own
// output for the identical source — the same diff-based parity discipline
// the `box`/`str`/`Vector` tests above already use. The IR-assertion
// style mirrors `vector_param_and_return_round_trip_*` (emit IR, grep it
// for the structural marker the lowering is supposed to produce).

#[test]
fn non_generic_struct_round_trips_through_a_function_param_and_return() {
    let src = r#"
        struct Point {
            x: f64,
            y: f64,
        }
        fn shift(p: Point) -> Point {
            return Point(p.x + 1.0, p.y + 2.0)
        }
        fn main() {
            let p: Point = Point(3.0, 4.0)
            let q: Point = shift(p)
            print(q.x)
            print(q.y)
        }
    "#;
    // The named struct type is declared once, and `shift`'s sret return +
    // by-pointer param are the same calling convention Vector/Matrix use.
    let ir = emit_ir(src);
    assert!(ir.contains("%Point = type { double, double }"), "expected a named struct decl:\n{ir}");
    assert!(ir.contains("define void @shift(ptr %sret.ret, ptr %arg.p)"), "expected sret + by-pointer param:\n{ir}");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    let lines: Vec<f64> = stdout.lines().map(|l| l.parse().unwrap()).collect();
    assert_eq!(lines, [4.0, 6.0]);
}

#[test]
fn nested_struct_field_access_compiles() {
    let src = r#"
        struct Inner {
            v: i64,
        }
        struct Outer {
            a: Inner,
            b: i64,
        }
        fn main() {
            let o: Outer = Outer(Inner(7), 3)
            print(o.a.v)
            print(o.b)
        }
    "#;
    let ir = emit_ir(src);
    // A nested named-typed field is declared *before* the outer struct
    // (dependency order), and the outer struct's body references it.
    assert!(ir.contains("%Inner = type { i64 }"), "expected the nested struct declared:\n{ir}");
    assert!(ir.contains("%Outer = type { %Inner, i64 }"), "expected the outer struct to reference the inner:\n{ir}");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n3\n");
}
