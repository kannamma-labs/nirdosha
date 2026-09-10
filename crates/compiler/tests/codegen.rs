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

/// A fresh, per-call temp file path, guaranteed distinct across
/// concurrently-running `#[test]`s in this same binary (same
/// pid+atomic-counter scheme `compile_and_run_opt`'s own `out_path`
/// already uses for the compiled binary itself) — every test that opens
/// its own SQLite file (a `transact` durability log, a `db_connect`
/// counter table) needs one of these, not a fixed name, or parallel
/// `cargo test` runs collide on the same file.
fn unique_temp_path(label: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_test_{label}_{}_{}", std::process::id(), unique_suffix()));
    p
}

/// Same as `compile_and_run_opt`, but runs the compiled binary with
/// `envs` set — needed for anything that reads an env var at runtime
/// (`transact`'s `NIRDOSHA_TRANSACT_LOG_PATH`, `kernel::instance_lock`'s
/// own file lock keyed off that same path) rather than a `.nir`-level
/// literal. Every `transact`-using test needs its own unique log path
/// via this, not the bare default (`nirdosha_transact_log.sqlite` in
/// cwd) — two such tests running concurrently under `cargo test`'s
/// default parallelism would otherwise both try to open/lock the exact
/// same file, and the second one now hard-aborts
/// (`emit_c_main`'s `nir_transact_log_init` failure path) rather than
/// silently misbehaving — a real collision, not a hypothetical one,
/// caught by running this suite locally before relying on it.
fn compile_and_run_with_env(src: &str, opt: codegen::OptLevel, envs: &[(&str, &str)]) -> (String, i32) {
    let program = parse_checked(src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, opt).expect("codegen::build should succeed for this program");
    let output = Command::new(&out_path).envs(envs.iter().copied()).output().expect("compiled binary should run");
    let _ = std::fs::remove_file(&out_path);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status.code().unwrap_or(-1))
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

// ---- `froze` — RFC 0006 Pillar 1's `Froze<T>` --------------------------

#[test]
fn froze_example_compiles_and_shares_a_value_across_two_real_threads() {
    let src = include_str!("fixtures/froze.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// A genuine "nested reply-obligation" deadlock (RFC 0006's own Pillar 5
/// evidence class — see `fixtures/deadlock.nir`'s own doc comment) must
/// be caught by `runtime-kernels`' dynamic stall detector
/// (`kernel::concurrency_wait_begin`) and turned into a fast, clean
/// process abort — never a silent, unbounded hang. Polls with its own
/// hard deadline rather than calling `Command::output()` directly
/// (which would block forever, hanging this whole test binary, if
/// detection ever regressed) — a timeout here is a real test failure
/// ("detection didn't fire"), not a flake to retry past.
#[test]
fn a_nested_reply_obligation_deadlock_is_detected_and_aborted_not_hung() {
    let program = parse_checked(include_str!("fixtures/deadlock.nir"));
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed for this program");

    let mut recorder_path = std::env::temp_dir();
    recorder_path.push(format!("nirdosha_test_deadlock_recorder_{}_{}.log.gz", std::process::id(), unique_suffix()));

    let mut child = Command::new(&out_path)
        .env("NIRDOSHA_KERNEL_RECORDER_PATH", &recorder_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("compiled binary should start");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait should succeed") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&recorder_path);

    let Some(status) = status else {
        panic!("compiled deadlock.nir hung past a 10s deadline -- the runtime deadlock detector did not fire");
    };
    assert!(
        !status.success(),
        "compiled deadlock.nir must abort, not exit cleanly, once every thread is blocked with nothing left to unblock it"
    );

    // Read the piped stderr directly, not via `wait_with_output()` --
    // `try_wait()` above already reaped the child (a second `wait()`
    // call on an already-reaped process is an error on Unix), and the
    // exit status is already in hand.
    use std::io::Read;
    let mut stderr = String::new();
    child.stderr.take().expect("stderr was piped").read_to_string(&mut stderr).expect("reading stderr should succeed");
    assert!(
        stderr.contains("deadlock detected"),
        "expected the kernel's own deadlock diagnostic on stderr, got:\n{stderr}"
    );
    // The richer diagnostic (`kernel::WaitTarget`/`waiting_registry`) —
    // names the actual stuck call kind, not just a bare thread count,
    // so a `.nir` author can find the two `recv` call sites that formed
    // the cycle.
    assert!(
        stderr.matches("is blocked in `recv` on chan handle").count() == 2,
        "expected both blocked threads' `recv` targets named individually, got:\n{stderr}"
    );
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

// ---- RNG is per-thread, not process-wide (fixed 2026-09) --------------

#[test]
fn a_spawned_threads_rand_seed_does_not_perturb_the_spawning_threads_stream() {
    // The real race this backend used to have: a process-wide RNG
    // stream meant a spawned thread's own `rand_seed`/`rand_f64` calls
    // could silently corrupt the spawning thread's own draws. Proven
    // fixed by comparing two independent draws with nothing interleaved
    // against the same two draws with a real spawned thread seeding and
    // drawing from its *own* stream in between -- if the fix holds, the
    // spawning thread's own sequence must be byte-for-byte identical
    // either way.
    let control = r#"
        fn main() {
            rand_seed(42)
            let a: f64 = rand_f64()
            let b: f64 = rand_f64()
            print(a)
            print(b)
        }
    "#;
    let (control_out, control_code) = compile_and_run(control);
    assert_eq!(control_code, 0);

    let with_spawn = r#"
        fn worker() -> unit {
            rand_seed(99)
            let _: f64 = rand_f64()
            return
        }
        fn main() {
            rand_seed(42)
            let a: f64 = rand_f64()
            let h: thread unit = spawn worker()
            join h
            let b: f64 = rand_f64()
            print(a)
            print(b)
        }
    "#;
    let (spawn_out, spawn_code) = compile_and_run(with_spawn);
    assert_eq!(spawn_code, 0);

    assert_eq!(
        control_out, spawn_out,
        "a spawned thread seeding/drawing its own RNG must not perturb the spawning thread's own stream -- got control={control_out:?} vs with_spawn={spawn_out:?}"
    );
}

#[test]
fn a_freshly_spawned_thread_gets_its_own_unseeded_rng_by_default() {
    // Matches the interpreter's own documented behavior
    // (`Interpreter::rng`'s doc comment): a spawned function's RNG
    // starts unseeded even though the spawning thread already seeded
    // its own -- calling `rand_f64` inside the spawned thread without
    // seeding it there too must abort, the same as
    // `rand_f64_before_seed_aborts_at_runtime` does for `main`, not
    // silently inherit the spawning thread's seed/position.
    let src = r#"
        fn worker() -> f64 {
            return rand_f64()
        }
        fn main() {
            rand_seed(42)
            let h: thread f64 = spawn worker()
            let r: f64 = join h
            print(r)
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "a spawned thread's own unseeded rand_f64 call must abort, not silently inherit the spawning thread's seed");
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

/// `file`/`open` codegen (`nir_file_open`/`_write`/`_read`/`_stop`) —
/// real, shipped 2026-09-05, but until now with no automated regression
/// test at all: `examples/features/24_file_io.nir` (the worked example
/// `docs/LANGUAGE.md`/`docs/PROTOLANG_PORT.md` both point readers at)
/// was only ever verified by hand. This mirrors that example directly:
/// write, append (without truncating), read-to-content, and read-past-
/// EOF returning `""` rather than an error, against a real file this
/// process actually creates.
#[test]
fn compiled_file_open_write_append_read_round_trips_real_bytes_on_disk() {
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_test_file_io_{}_{}.txt", std::process::id(), unique_suffix()));
    let path_str = path.to_str().unwrap();
    let src = format!(
        r#"
        struct Text {{
            value: str,
        }}

        fn main() {{
            let path: str = "{path_str}"

            let out: file = open(path, "w")
            send(out, "first line\n")
            stop out

            let appended: file = open(path, "a")
            send(appended, "second line\n")
            stop appended

            let inp: file = open(path, "r")
            let content: str = recv(inp)
            stop inp
            let read_back: Text = Text(content)
            print(read_back.value)

            let inp2: file = open(path, "r")
            let all: str = recv(inp2)
            let eof: str = recv(inp2)
            stop inp2
            print(eof == "")
        }}
    "#
    );
    let (stdout, code) = compile_and_run(&src);
    let on_disk = std::fs::read_to_string(&path).expect("the compiled binary should have actually written this file");
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0);
    // `print(bool)` prints `1`/`0`, not `true`/`false` (docs/LANGUAGE.md §10's own cosmetic-only note).
    assert_eq!(stdout, "first line\nsecond line\n\n1\n");
    assert_eq!(on_disk, "first line\nsecond line\n");
}

/// Phase 0 (compiled `serve`): a real curl-shaped request over a real
/// socket, twice, against the *same* running compiled process, routed by
/// path to two different compiled `fn`s -- `str_index_of`/`str_slice`/
/// `len(str)` doing the request-line parsing, plain `if`/`else if` + `==`
/// doing the routing. Same "spawn the compiled binary since `accept`
/// blocks, connect a real client from the test" shape as
/// `compiled_listen_accept_serves_a_real_client`, except this server's
/// own `while true` loop never exits on its own, so the test kills the
/// child at the end instead of waiting on it.
#[test]
fn compiled_serve_routes_by_path_to_two_compiled_functions() {
    let port = free_port();
    let src = format!(
        r#"
        struct Text {{
            value: str,
        }}

        fn handle_hello() -> Text {{
            return Text("hello from /api/hello")
        }}

        fn handle_echo() -> Text {{
            return Text("hello from /api/echo")
        }}

        fn main() {{
            let l: tcp_listener = listen({port})
            while true {{
                let conn: tcp = accept(l)
                let req: str = recv(conn)

                let line_end: i64 = str_index_of(req, "\r\n")
                let line: str = str_slice(req, 0, line_end)
                let sp1: i64 = str_index_of(line, " ")
                let after_method: str = str_slice(line, sp1 + 1, len(line))
                let sp2: i64 = str_index_of(after_method, " ")
                let path: str = str_slice(after_method, 0, sp2)

                if path == "/api/hello" {{
                    let body: Text = handle_hello()
                    send(conn, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                    send(conn, body.value)
                }} else if path == "/api/echo" {{
                    let body: Text = handle_echo()
                    send(conn, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                    send(conn, body.value)
                }} else {{
                    send(conn, "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
                }}
                stop conn
            }}
        }}
    "#
    );

    let program = parse_checked(&src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed");
    let mut child = Command::new(&out_path).spawn().expect("compiled binary should start");

    use std::io::{Read, Write};
    let request = |path: &str| -> Vec<u8> {
        let mut attempt = 0;
        let mut conn = loop {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(_) if attempt < 50 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("could not connect to the compiled listener: {e}"),
            }
        };
        conn.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        buf
    };

    let hello_response = String::from_utf8_lossy(&request("/api/hello")).into_owned();
    let echo_response = String::from_utf8_lossy(&request("/api/echo")).into_owned();
    let missing_response = String::from_utf8_lossy(&request("/api/unknown")).into_owned();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&out_path);

    assert!(hello_response.starts_with("HTTP/1.1 200 OK"), "unexpected response: {hello_response:?}");
    assert!(hello_response.ends_with("hello from /api/hello"), "unexpected response: {hello_response:?}");
    assert!(echo_response.starts_with("HTTP/1.1 200 OK"), "unexpected response: {echo_response:?}");
    assert!(echo_response.ends_with("hello from /api/echo"), "unexpected response: {echo_response:?}");
    assert!(missing_response.starts_with("HTTP/1.1 404 Not Found"), "unexpected response: {missing_response:?}");
    assert_ne!(hello_response, echo_response, "two different routes must produce two different bodies");
}

/// The *production* compiled-`serve` path (`nirdosha build --serve`,
/// RFC 0010) end to end — real, but until now not covered by any
/// automated test: `crates/compiled-serve/README.md` and this file's
/// own `compiled_serve_routes_by_path_to_two_compiled_functions` above
/// both cover only the older, primitives-first `tcp_listener`/`accept`
/// style (`51_compiled_serve.nir`'s own shape, GET-only, hand-parsed).
/// This test instead goes through `codegen::build_serve` — the same
/// entry point `main.rs`'s `--serve` CLI flag calls — to prove
/// `serve { expose ... }`'s exposure model (`typeck::exposed_fn_names`)
/// really is wired to `crates/compiled-serve`'s real HTTP engine: a
/// `GET` and a `POST` carrying a real body both reach the same exposed
/// `fn` (proving `Content-Length`/body parsing works, the exact gap
/// the older minimal engine still has), and an unrouted path 404s.
#[test]
fn compiled_serve_production_path_exposes_a_route_via_a_real_http_post_with_a_body() {
    let port = free_port();
    let src = r#"
        struct Greeting {
            message: str,
        }

        fn say_hello() -> Greeting requires(public) {
            return Greeting("hello from real compiled serve")
        }

        serve {
            expose say_hello
        }

        fn main() {
        }
    "#;

    let program = parse_checked(src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_serve_{}_{}", std::process::id(), unique_suffix()));
    let opts = codegen::ServeCodegenOptions { port, ui_html: Vec::new() };
    codegen::build_serve(&program, &report, &out_path, codegen::OptLevel::O2, &opts).expect("codegen::build_serve should succeed");
    let mut child = Command::new(&out_path).spawn().expect("compiled serve binary should start");

    use std::io::{Read, Write};
    let raw_request = |request: &str| -> Vec<u8> {
        let mut attempt = 0;
        let mut conn = loop {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(_) if attempt < 50 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("could not connect to the compiled serve listener: {e}"),
            }
        };
        conn.write_all(request.as_bytes()).unwrap();
        conn.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        buf
    };

    let get_response = String::from_utf8_lossy(&raw_request("GET /api/say_hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")).into_owned();
    let body = "{}";
    let post_response = String::from_utf8_lossy(&raw_request(&format!(
        "POST /api/say_hello HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )))
    .into_owned();
    let missing_response = String::from_utf8_lossy(&raw_request("GET /api/no_such_route HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")).into_owned();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&out_path);

    assert!(get_response.starts_with("HTTP/1.1 200"), "unexpected GET response: {get_response:?}");
    assert!(get_response.contains("hello from real compiled serve"), "unexpected GET response: {get_response:?}");
    assert!(post_response.starts_with("HTTP/1.1 200"), "a POST with a real Content-Length body should reach the same exposed fn: {post_response:?}");
    assert!(post_response.contains("hello from real compiled serve"), "unexpected POST response: {post_response:?}");
    assert!(missing_response.starts_with("HTTP/1.1 404"), "an unexposed path should 404, not silently match: {missing_response:?}");
}

#[test]
fn str_slice_and_str_index_of_parse_a_request_line() {
    let src = r#"
        fn main() {
            let s: str = "GET /api/hello HTTP/1.1"
            print(len(s))
            let sp1: i64 = str_index_of(s, " ")
            print(sp1)
            let method: str = str_slice(s, 0, sp1)
            print(method)
            let rest: str = str_slice(s, sp1 + 1, len(s))
            let sp2: i64 = str_index_of(rest, " ")
            let path: str = str_slice(rest, 0, sp2)
            print(path)
            print(str_index_of(s, "xyz"))
            print(path == "/api/hello")
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "23\n3\nGET\n/api/hello\n-1\n1\n");
}

#[test]
fn str_slice_traps_on_out_of_bounds_end() {
    let src = r#"
        fn main() {
            let s: str = "hello"
            let bad: str = str_slice(s, 2, 10)
            print(bad)
        }
    "#;
    let (_, code) = compile_and_run(src);
    assert_ne!(code, 0, "an out-of-bounds str_slice should trap, not succeed");
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

/// End-to-end: `check_role` compiled for real (not just a signature that
/// typechecks) hands back a genuine, unforgeable `RoleView` when a
/// `VerifiedIdentity`'s `claims_json` (read as a plain comma-separated
/// role list — `check_role`'s disclosed simplification, not real JSON
/// parsing) contains the requested role, and a real `Err` when it
/// doesn't. That `RoleView` is then the *only* way a caller can prove a
/// role to a `requires(role: ...)`-masked struct field: matching role ->
/// the real value passes through on return; non-matching role -> the
/// field is zeroed; `check_role` failing outright means no `RoleView` is
/// ever produced at all (fail-closed at the proof layer, before masking
/// even runs).
#[test]
fn check_role_produces_real_role_view_that_drives_field_masking() {
    let src = r#"
        struct Text {
            value: str,
        }
        struct Employee {
            name: str,
            department: str,
            salary: f64 requires(role: "admin"),
        }
        fn get_employee(caller: RoleView, name: Text, department: Text, salary: f64) -> Employee {
            return Employee(name.value, department.value, salary)
        }
        fn main() {
            let admin_identity: VerifiedIdentity = VerifiedIdentity("alice", "https://example.com", "my-app", 0, 0, "admin,editor")
            let guest_identity: VerifiedIdentity = VerifiedIdentity("bob", "https://example.com", "my-app", 0, 0, "guest")

            let admin_employee: Employee = match check_role(admin_identity, "admin") {
                Ok(admin_role) => get_employee(admin_role, Text("Ada"), Text("Engineering"), 150000.0),
                Err(e) => Employee("", "", -1.0),
            }
            print(admin_employee.salary)

            let guest_employee: Employee = match check_role(guest_identity, "guest") {
                Ok(guest_role) => get_employee(guest_role, Text("Ada"), Text("Engineering"), 150000.0),
                Err(e) => Employee("", "", -1.0),
            }
            print(guest_employee.salary)
            print(guest_employee.name)

            let denied: bool = match check_role(guest_identity, "admin") {
                Ok(r) => true,
                Err(e) => false,
            }
            print(denied)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "150000.000000\n0.000000\nAda\n0\n");
}

/// End-to-end, real compiled-and-run coverage for the rest of Row 12
/// (`docs/nirdosha_row12_functions_identity.md`): dotted-path claim
/// lookup, sessions, refresh-token exchange (including real single-use
/// enforcement, not just the affine type-level guarantee), revocation,
/// and API-key validation — all against `crates/runtime-kernels/src/
/// kernel/identity.rs`'s real kernels, not stubs.
#[test]
fn row12_remaining_identity_builtins_compile_and_run_for_real() {
    let src = r#"
        struct Text {
            value: str,
        }
        fn report_reissued(id: VerifiedIdentity) -> i64 {
            print(id.subject)
            print(id.issued_at)
            return 0
        }
        fn report_reissue_error(msg: Text) -> i64 {
            print(msg.value)
            print(-1)
            return 0
        }
        // Routes a match-arm-bound `ClaimView`'s `.value` through a real
        // function call rather than field-accessing it directly in the
        // arm body -- `local_ty_of` resolves a match's own result type
        // from `arms[0].body` *before* `match_enum` binds the arm's
        // pattern variable into scope, a real, pre-existing, general
        // codegen ordering gap unrelated to this phase's new builtins
        // (confirmed: no existing test in this file field-accesses a
        // match-arm binding directly either). Working around it here,
        // not fixing the shared ordering issue as part of this phase.
        fn report_claim(c: ClaimView) -> i64 {
            print(c.value)
            return 0
        }
        fn main() {
            let identity: VerifiedIdentity = VerifiedIdentity("alice", "https://example.com", "my-app", 9999999999, 0, "{\"org\":{\"roles\":[\"admin\",\"auditor\"]},\"profile\":{\"department\":\"cardiology\"},\"revoked\":false}")

            // check_role_path / extract_claim_path
            let has_admin: bool = match check_role_path(identity, "org.roles", "admin") {
                Ok(r) => true,
                Err(e) => false,
            }
            print(has_admin)
            let has_owner: bool = match check_role_path(identity, "org.roles", "owner") {
                Ok(r) => true,
                Err(e) => false,
            }
            print(has_owner)
            let claim_reported: i64 = match extract_claim_path(identity, "profile.department") {
                Ok(c) => report_claim(c),
                Err(e) => report_reissue_error(Text(e)),
            }

            // check_revocation
            print(check_revocation(identity))

            // create_application_session / session_cookie
            let session: ApplicationSession = create_application_session(identity)
            print(session.identity_subject)
            print(session.expires_at - session.created_at)
            let cookie: str = session_cookie(session)
            print(cookie)

            // verify_session: a real server-side lookup (red-team report
            // A2) -- the session minted above round-trips the same
            // subject/issuer it was created with, and an unknown session
            // id is a clean Err, not a panic or a false positive.
            let looked_up_subject: str = match verify_session(session.session_id) {
                Ok(v) => v.subject,
                Err(e) => e,
            }
            print(looked_up_subject)
            let looked_up_issuer: str = match verify_session(session.session_id) {
                Ok(v) => v.issuer,
                Err(e) => e,
            }
            print(looked_up_issuer)
            let unknown_session_ok: bool = match verify_session("not-a-real-session-id") {
                Ok(v) => true,
                Err(e) => false,
            }
            print(unknown_session_ok)

            // new_refresh_token / exchange_refresh_token, including real
            // single-use enforcement server-side (not just the affine
            // box field's own compile-time single-use guarantee).
            let refresh_handle: RefreshTokenHandle = new_refresh_token(session.expires_at + 3600)
            let reported: i64 = match exchange_refresh_token(identity, refresh_handle, 42) {
                Ok(new_identity) => report_reissued(new_identity),
                Err(e) => report_reissue_error(Text(e)),
            }

            // validate_api_key: a real constant-time sha256 compare, not
            // a lookup table -- the caller supplies the expected hash.
            let real_hash: str = sha256_hex("my-secret-api-key")
            let key_ok: bool = match validate_api_key("my-secret-api-key", real_hash) {
                Ok(id) => true,
                Err(e) => false,
            }
            print(key_ok)
            let key_bad: bool = match validate_api_key("wrong-key", real_hash) {
                Ok(id) => true,
                Err(e) => false,
            }
            print(key_bad)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0, "stdout so far: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1", "check_role_path should find admin at org.roles");
    assert_eq!(lines[1], "0", "check_role_path should not find owner at org.roles");
    assert_eq!(lines[2], "cardiology");
    assert_eq!(lines[3], "0", "revoked:false must read as not revoked");
    assert_eq!(lines[4], "alice", "session.identity_subject must copy the identity's own subject");
    assert_eq!(lines[5], "28800", "a fresh session's real 8-hour lifetime");
    assert!(lines[6].contains("HttpOnly") && lines[6].contains("Max-Age=28800"), "session_cookie: {}", lines[6]);
    assert_eq!(lines[7], "alice", "verify_session must look up the same subject the session was created with");
    assert_eq!(lines[8], "https://example.com", "verify_session must look up the same issuer the session was created with");
    assert_eq!(lines[9], "0", "verify_session against an unknown session id must be a clean Err, not a false positive");
    assert_eq!(lines[10], "alice", "exchange_refresh_token should reissue the same subject");
    assert_eq!(lines[11], "42", "exchange_refresh_token should carry the new issued_at through");
    assert_eq!(lines[12], "1", "the correct api key must validate");
    assert_eq!(lines[13], "0", "the wrong api key must not validate");
}

/// `nfr(...)`'s per-call instrumentation (registration global, the
/// `nir_nfr_call_begin`/`nir_nfr_call_end` pair wrapped around every
/// return path) must be fully transparent to a program that never
/// crosses any of its thresholds — same return values, same control
/// flow, as an unannotated function. Threshold-violation/escalation
/// behavior itself is exercised manually (it's timing/HTTP-dependent,
/// not a good fit for a deterministic unit test); this test is the
/// permanent regression guard for the wiring itself.
#[test]
fn nfr_annotation_is_transparent_to_normal_execution() {
    let src = r#"
        fn add(a: i64, b: i64) -> i64 nfr(latency_ms: 1000, concurrency_max: 10) {
            return a + b
        }
        fn main() {
            print(add(2, 3))
            print(add(10, 20))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "5\n30\n");
}

/// `nfr(error_rate_max: ...)` requires the tag+payload `Result` return
/// shape so `call_end` can inspect the real tag (not just accept a
/// literal "never errors") — this exercises that inspection on both the
/// `Ok` and `Err` path through a real compiled-and-run binary.
#[test]
fn nfr_error_rate_tracks_real_result_tag_on_both_arms() {
    let src = r#"
        enum MyError { Bad }
        fn risky(fail: bool) -> Result(i64, MyError) nfr(error_rate_max: 0.5) {
            if fail {
                return Err(Bad())
            }
            return Ok(42)
        }
        fn main() {
            match risky(false) {
                Ok(v) => print(v),
                Err(e) => print(-1),
            }
            match risky(true) {
                Ok(v) => print(v),
                Err(e) => print(-1),
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "42\n-1\n");
}

/// Ordinary (ungated) first-class functions, compiled for real
/// (2026-09): a bare top-level fn name used as a value (`apply(double,
/// 21)`) evaluates to its own address (`Expr::Ident`'s fallback when
/// `name` isn't a local variable), and calling a `fn(..)->..`-typed
/// local (`apply`'s own `f` parameter) emits a real indirect call
/// (`Codegen::call_indirect`) — no proof, no `acquire`, just a plain
/// value like any other.
#[test]
fn ordinary_first_class_function_value_compiles_and_calls_indirectly() {
    let src = r#"
        fn double(x: i64) -> i64 {
            return x * 2
        }
        fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
            return f(x)
        }
        fn main() {
            print(apply(double, 21))
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "42\n");
}

/// `acquire name(proof)` compiled for real (2026-09, LANGUAGE.md §6a):
/// a `requires`-gated fn's value is obtained only through a real
/// `check_role`-produced `RoleView` matching its declared role —
/// `Ok(f)` then calls the acquired function like any other `Ty::Fn`
/// value (`call_indirect`), `Err(reason)` when the given identity's own
/// real `RoleView` doesn't prove the required role. This is the
/// function-level counterpart to
/// `check_role_produces_real_role_view_that_drives_field_masking`'s
/// field-level masking — both driven by the same compiled `check_role`,
/// neither involving the interpreter in any way.
///
/// The inner `match`'s arms are deliberately `Err` before `Ok`:
/// `Codegen::match_expr` infers the match's own result type from its
/// *first* arm's body, evaluated before that arm's own bindings exist
/// — `Ok(f) => f(...)` can't be first here, since `f` isn't in scope
/// yet at that point. A real, disclosed ordering constraint, not a bug
/// in this test.
#[test]
fn acquire_produces_real_callable_fn_value_gated_by_check_role() {
    let src = r#"
        struct Text { value: str }
        struct Employee {
            name: str,
            department: str,
            salary: f64,
        }
        fn get_employee(caller: RoleView, name: Text, department: Text, salary: f64) -> Employee
            effect(pure)
            requires(role: "admin")
        {
            return Employee(name.value, department.value, salary)
        }
        fn main() {
            let admin_identity: VerifiedIdentity = VerifiedIdentity("alice", "https://idp", "app", 0, 0, "admin")
            let admin_view: Employee = match check_role(admin_identity, "admin") {
                Err(e) => Employee("", "", -1.0),
                Ok(role) => match acquire get_employee(role) {
                    Err(msg) => Employee("denied", "", -2.0),
                    Ok(f) => f(role, Text("Ada"), Text("Eng"), 150000.0),
                },
            }
            print(admin_view.salary)

            let guest_identity: VerifiedIdentity = VerifiedIdentity("bob", "https://idp", "app", 0, 0, "guest")
            let guest_view: Employee = match check_role(guest_identity, "guest") {
                Err(e) => Employee("", "", -1.0),
                Ok(role) => match acquire get_employee(role) {
                    Err(msg) => Employee("denied", "", -2.0),
                    Ok(f) => f(role, Text("Ada"), Text("Eng"), 150000.0),
                },
            }
            print(guest_view.salary)
            print(guest_view.name)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "150000.000000\n-2.000000\ndenied\n");
}

/// Phase 1 (identity crypto): a genuine HMAC-SHA256 JWT, verified end to
/// end in a compiled binary via real `jsonwebtoken` signature verification
/// against a static JWKS -- same fixture as `examples/features/
/// 30_identity_oidc.nir`/`31_mock_identity_provider.nir` (`kid:"key1"`,
/// `kty:"oct"`, `k:"bXktc2VjcmV0LWtleQ"` = `"my-secret-key"`). Arms are
/// `Err(...)`-first throughout -- the documented, pre-existing
/// `match_expr` workaround (`docs/PHASE0.md`'s "Twenty-first update":
/// `local_ty_of` infers a match's result type from its *first* arm's
/// body, evaluated before that arm's own bindings exist in scope, so an
/// `Ok(id) => id`-shaped first arm doesn't resolve; reordering is the
/// complete, correct fix, not something this phase needed to touch).
#[test]
fn oidc_validate_token_verifies_a_real_jwt_and_drives_check_role_and_extract_claim() {
    let src = r#"
        struct Text {
            value: str,
        }

        fn main() {
            let token: str = "eyJhbGciOiAiSFMyNTYiLCAia2lkIjogImtleTEifQ.eyJzdWIiOiAiYWxpY2UiLCAiaXNzIjogImh0dHBzOi8vZXhhbXBsZS5jb20iLCAiYXVkIjogIm15LWFwcCIsICJleHAiOiAyMDAwMDAwMDAwLCAiaWF0IjogMTcwMDAwMDAwMCwgInJvbGVzIjogWyJwaHlzaWNpYW4iXSwgImRlcGFydG1lbnQiOiAiY2FyZGlvbG9neSJ9.nrFdeqNDwXWLeGzud6X9Q4ITzCXULzZBBK8y51LGYXs"
            let jwks: str = "{\"keys\":[{\"kid\":\"key1\",\"kty\":\"oct\",\"k\":\"bXktc2VjcmV0LWtleQ\"}]}"

            let identity: VerifiedIdentity = match oidc_validate_token(token, "https://example.com", "my-app", jwks) {
                Err(e) => VerifiedIdentity("", "", "", 0, 0, "{}"),
                Ok(id) => id,
            }
            print(identity.subject)
            print(identity.issuer)
            print(identity.audience)

            let has_physician: bool = match check_role(identity, "physician") {
                Err(e) => false,
                Ok(proof) => true,
            }
            print(has_physician)

            let has_admin: bool = match check_role(identity, "admin") {
                Err(e) => false,
                Ok(proof) => true,
            }
            print(has_admin)

            let department: Text = match extract_claim(identity, "department") {
                Err(e) => Text(e),
                Ok(claim) => Text(claim.value),
            }
            print(department.value)

            print(identity_expired(identity, 999999999999))
            print(identity_expired(identity, 1700000001))

            let wrong_issuer_rejected: bool = match oidc_validate_token(token, "https://wrong-issuer.example", "my-app", jwks) {
                Err(e) => true,
                Ok(id) => false,
            }
            print(wrong_issuer_rejected)

            // A tampered signature (last character of the token flipped)
            // is a real `Err`, never a trap.
            let tampered: str = "eyJhbGciOiAiSFMyNTYiLCAia2lkIjogImtleTEifQ.eyJzdWIiOiAiYWxpY2UiLCAiaXNzIjogImh0dHBzOi8vZXhhbXBsZS5jb20iLCAiYXVkIjogIm15LWFwcCIsICJleHAiOiAyMDAwMDAwMDAwLCAiaWF0IjogMTcwMDAwMDAwMCwgInJvbGVzIjogWyJwaHlzaWNpYW4iXSwgImRlcGFydG1lbnQiOiAiY2FyZGlvbG9neSJ9.nrFdeqNDwXWLeGzud6X9Q4ITzCXULzZBBK8y51LGYXt"
            let tampered_rejected: bool = match oidc_validate_token(tampered, "https://example.com", "my-app", jwks) {
                Err(e) => true,
                Ok(id) => false,
            }
            print(tampered_rejected)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "alice\nhttps://example.com\nmy-app\n1\n0\ncardiology\n1\n0\n1\n1\n"
    );
}

/// `mock_issue_token`'s real implementation, round-tripped through the
/// exact same `oidc_validate_token`/`check_role`/`extract_claim` pipeline
/// the test just above already proves against a hand-written fixture
/// token -- this one signs its own token instead, against the same
/// `kid:"key1"`/`kty:"oct"` JWKS, and feeds it straight back in. Real
/// HMAC-SHA256 signing, not an echo: a wrong audience is still rejected
/// by real signature/claim verification, and a JWKS with no usable
/// signing key is a real `Err`, never a trap.
#[test]
fn mock_issue_token_signs_a_real_jwt_that_oidc_validate_token_accepts() {
    let src = r#"
        struct Text {
            value: str,
        }

        fn main() {
            let jwks: str = "{\"keys\":[{\"kid\":\"key1\",\"kty\":\"oct\",\"k\":\"bXktc2VjcmV0LWtleQ\"}]}"

            let token: Text = match mock_issue_token("alice", "https://example.com", "my-app", 1700000000, 3600, "{\"roles\":[\"physician\"],\"department\":\"cardiology\"}", jwks) {
                Err(e) => Text(e),
                Ok(t) => Text(t),
            }
            print(len(token.value) > 0)

            let identity: VerifiedIdentity = match oidc_validate_token(token.value, "https://example.com", "my-app", jwks) {
                Err(e) => VerifiedIdentity("", "", "", 0, 0, "{}"),
                Ok(id) => id,
            }
            print(identity.subject)
            print(identity.issuer)
            print(identity.audience)
            print(identity.expires_at - identity.issued_at == 3600)

            let has_physician: bool = match check_role(identity, "physician") {
                Err(e) => false,
                Ok(proof) => true,
            }
            print(has_physician)

            let department: Text = match extract_claim(identity, "department") {
                Err(e) => Text(e),
                Ok(claim) => Text(claim.value),
            }
            print(department.value)

            // Wrong audience is rejected by real claim verification, not
            // just echoed back -- proves this is a real signed check, not
            // a no-op stand-in.
            let wrong_audience_rejected: bool = match oidc_validate_token(token.value, "https://example.com", "wrong-app", jwks) {
                Err(e) => true,
                Ok(id) => false,
            }
            print(wrong_audience_rejected)

            // A JWKS with no usable (`oct`) signing key can't issue a
            // token at all -- a real `Err`, never a trap.
            let no_key_jwks: str = "{\"keys\":[]}"
            let no_key_rejected: bool = match mock_issue_token("alice", "https://example.com", "my-app", 1700000000, 3600, "{}", no_key_jwks) {
                Err(e) => true,
                Ok(t) => false,
            }
            print(no_key_rejected)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "1\nalice\nhttps://example.com\nmy-app\n1\n1\ncardiology\n1\n1\n"
    );
}

/// Phase 2 (`db`, SQLite via `rusqlite`): a real, unmodified copy of
/// `examples/features/27_database.nir` — `db_connect`/`db_execute`/
/// `db_query`/`json_array_get`/`json_get_str`/`stop`, compiled and run
/// against a real in-memory SQLite database. A schema is created, two
/// rows inserted, a filtered `SELECT` round-tripped through the new
/// `Ty::Json`-as-text representation, a row updated, and a connection
/// failure (a real `Err`, never a trap) all exercised end to end.
#[test]
fn db_connect_execute_query_round_trips_real_sqlite_rows() {
    let src = include_str!("../../../examples/features/27_database.nir");
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "ada\n1\n");
}

/// `env(name) -> Result(str, str)` (RFC 0011 §1) — `Ok(value)` when the
/// process environment variable is set at the compiled binary's own
/// runtime (not at compile time), `Err(_)` when unset. Uniquely-named
/// vars, same reasoning `unique_temp_path`'s own doc comment gives for
/// SQLite files: a generic name here could collide with something real
/// in whatever environment `cargo test` itself happens to run under.
#[test]
fn env_round_trips_ok_when_set_and_err_when_unset() {
    // `describe`-as-a-fn would need `str` in its own signature -- banned
    // (`typeck::TypeErrorKind::StrInFnSignature`, `result_of`'s own doc
    // comment: builtins are exempt from this ban, plain user fns aren't)
    // -- so both matches are inlined directly in `main` instead, same as
    // `json_accessors_...`'s own `let name: str = match json_get_str(...)`
    // shape just above.
    let src = r#"
        fn main() {
            let set_var: str = match env("NIRDOSHA_RFC0011_ENV_TEST_SET_VAR") {
                Err(e) => "MISSING",
                Ok(v) => v,
            }
            print(set_var)
            let unset_var: str = match env("NIRDOSHA_RFC0011_ENV_TEST_UNSET_VAR") {
                Err(e) => "MISSING",
                Ok(v) => v,
            }
            print(unset_var)
        }
    "#;
    let (stdout, code) =
        compile_and_run_with_env(src, codegen::OptLevel::O2, &[("NIRDOSHA_RFC0011_ENV_TEST_SET_VAR", "hello-rfc-0011")]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "hello-rfc-0011\nMISSING\n");
}

/// The rest of the `json_*` accessor surface `27_database.nir` doesn't
/// happen to exercise — `json_get_i64`/`json_get_f64`/`json_get_bool`/
/// `json_array_len` read out of a real `db_query` row (SQLite has no
/// native boolean column, so `active` round-trips as `0`/`1`, matching
/// `NirBindValue`'s own documented bool-as-integer convention), plus
/// `json_parse`/`json_get`/`json_set_str` against plain literals. `Json`
/// and `Str` are distinct types even though they share one compiled
/// representation (`Ty::Json`'s `llvm_ty` arm) — `typeck.rs` enforces
/// the distinction like any other nominal type, so every `json_get_*`
/// call here reads directly from the live `Ty::Json` value in scope
/// (`row`, `parsed`), never through a `Text`-wrapped detour; only the
/// final *scalar* extractions (`str`/`i64`/`f64`/`bool`) cross
/// `build_row`'s own function boundary, carried in one `Row` struct
/// (struct *fields* aren't inspected by the `str`-at-boundary ban —
/// `contains_str` only walks a generic type's own type arguments, not a
/// plain struct's field list — the same reason the ban's own suggested
/// `struct Text { value: str }` fix works at all). Arms are
/// `Err(...)`-first throughout — the same documented, pre-existing
/// `match_expr` ordering workaround the identity test above already
/// uses (a bare bound identifier as an arm's body, e.g. `Ok(f) => f`,
/// needs its real type already known when `match_expr` infers the whole
/// match's result type from its *first* arm; putting `Err` first
/// sidesteps that inference order entirely).
#[test]
fn json_accessors_read_every_scalar_type_out_of_a_real_db_query_result() {
    let src = r#"
        struct Row {
            name: str,
            id: i64,
            score: f64,
            active: bool,
            len_is_err: bool,
        }

        // `json` is a plain parameter type here (like `db` already is for
        // `conn` below) -- not banned, since `contains_str` only walks a
        // generic type's own type arguments, and bare `Ty::Json` isn't
        // one. Kept as its own function so `build_row` below never needs
        // a multi-statement match-arm body (this grammar's `=>` takes one
        // expression, not a `{ }` block) to combine several `json_get_*`
        // reads into one `Row`.
        fn row_from_json(row: json) -> Row {
            let name: str = match json_get_str(row, "name") {
                Err(e) => "",
                Ok(s) => s,
            }
            let id: i64 = match json_get_i64(row, "id") {
                Err(e) => -1,
                Ok(n) => n,
            }
            let score: f64 = match json_get_f64(row, "score") {
                Err(e) => -1.0,
                Ok(f) => f,
            }
            let active: bool = match json_get_bool(row, "active") {
                Err(e) => false,
                Ok(b) => b,
            }
            let len_is_err: bool = match json_array_len(row) {
                Err(e) => true, // `row` is one object, not an array
                Ok(n) => false,
            }
            return Row(name, id, score, active, len_is_err)
        }

        fn build_row(conn: db) -> Row {
            let created: i64 = match db_execute(conn, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER)") {
                Err(e) => -1,
                Ok(n) => n,
            }
            let inserted: i64 = match db_execute(conn, "INSERT INTO t (name, score, active) VALUES (?, ?, ?)", "ada", 4.5, true) {
                Err(e) => -1,
                Ok(n) => n,
            }
            let result: Row = match db_query(conn, "SELECT * FROM t") {
                Err(e) => Row("", -1, -1.0, false, false),
                Ok(rows) => match json_array_get(rows, 0) {
                    Err(e) => Row("", -1, -1.0, false, false),
                    Ok(row) => row_from_json(row),
                },
            }
            stop conn
            return result
        }

        fn main() {
            let row: Row = match db_connect(":memory:") {
                Err(e) => Row("", -1, -1.0, false, false),
                Ok(conn) => build_row(conn),
            }
            print(row.name)        // ada
            print(row.id)          // 1
            print(row.score)       // 4.5
            print(row.active)      // true -- bound as SQLite integer 1; `nir_json_get_bool` accepts a real JSON boolean or a 0/1 integer
            print(row.len_is_err)  // true

            let parsed: bool = match json_parse("{\"a\": 1}") {
                Err(e) => false,
                Ok(j) => match json_get_i64(j, "a") {
                    Err(e) => false,
                    Ok(n) => n == 1,
                },
            }
            print(parsed) // 1 (bool prints as 1/0, not "true"/"false")

            // Every match's first-listed arm here is a literal, not a
            // bare bound identifier -- `Ok(s) => s`-shaped passthroughs
            // (like the innermost arm below) are fine in *second*
            // position (`match_expr` only infers the whole match's
            // result type from `arms[0]`'s body, `docs/PHASE0.md`'s
            // "Twenty-first update"), but a bare identifier *first*
            // (`Err(e) => e` would have been, here) hits that same
            // inference-order gap regardless of whether the payload is
            // an aggregate or, as here, a plain `str`/`json` value.
            let nested: str = match json_parse("{\"outer\": {\"inner\": \"value\"}}") {
                Err(e) => "parse-error",
                Ok(doc) => match json_get(doc, "outer") {
                    Err(e) => "get-error",
                    Ok(inner) => match json_get_str(inner, "inner") {
                        Err(e) => "get-str-error",
                        Ok(s) => s,
                    },
                },
            }
            print(nested) // value

            let set_ok: bool = match json_parse("null") {
                Err(e) => false,
                Ok(null_doc) => match json_set_str(null_doc, "extra", "value") {
                    Err(e) => false,
                    Ok(j) => match json_get_str(j, "extra") {
                        Err(e) => false,
                        Ok(s) => s == "value",
                    },
                },
            }
            print(set_ok) // 1
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "ada\n1\n4.500000\n1\n1\n1\nvalue\n1\n");
}

/// Phase 3 (`transact`, Layer 1 only — `docs/TRANSACT.md`,
/// `codegen::emit_transact`'s own doc comment has the full scope): a
/// real, unmodified copy of `examples/features/36_transact.nir` —
/// `precheck`/`network`/`verify`/`commit`/`compensate`/`log`, the
/// implicit `network`/`verify`/`txn_id` bindings, and a `transact { ... }`
/// expression's own real `bool` value, all compiled and run for real.
/// `checkout(10)` commits, `checkout(-5)` compensates, `minimal(1)`
/// (no `precheck`/`compensate`/`log`) commits.
#[test]
fn transact_commits_and_compensates_for_real_matching_the_checked_in_example() {
    let src = include_str!("../../../examples/features/36_transact.nir");
    let log_path = unique_temp_path("transact_log_a");
    let (stdout, code) = compile_and_run_with_env(src, codegen::OptLevel::O2, &[("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "committing\n10\nlogged\n10\n1\n1\ncompensating\n-5\nlogged\n-5\n0\n0\ncommitting\n1\n1\n");
    let _ = std::fs::remove_file(&log_path);
}

/// `precheck == false` aborts the whole block immediately — nothing
/// durable would ever be written (moot today, no durability log exists
/// yet), but concretely: `network`/`verify`/`commit`/`compensate`/`log`
/// must never run at all, and the block's own value is `false`. Not
/// exercised by `36_transact.nir` itself (both its `checkout` calls have
/// `db_reachable()` return `true`), so a dedicated test.
#[test]
fn transact_precheck_false_skips_every_other_slot() {
    let src = r#"
        fn never_reachable() -> bool {
            print("db down")
            return false
        }
        fn call_api(txn_id: str, amount: i64) -> i64 {
            print("network ran") // must never print
            return amount
        }
        fn check(resp: i64) -> bool {
            print("verify ran") // must never print
            return resp > 0
        }
        fn update_db(amount: i64) -> i64 {
            print("commit ran") // must never print
            return amount
        }

        fn main() {
            let result: bool = transact {
                precheck: never_reachable()
                network:  call_api(txn_id, 5)
                verify:   check(network)
                commit:   update_db(5)
            }
            print(result)
        }
    "#;
    let log_path = unique_temp_path("transact_log_b");
    let (stdout, code) = compile_and_run_with_env(src, codegen::OptLevel::O2, &[("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "db down\n0\n");
    let _ = std::fs::remove_file(&log_path);
}

/// `verify == false` with no `compensate` slot at all is still
/// immediately terminal (`docs/TRANSACT.md`'s own explicit note) — the
/// block yields `false`, nothing else runs.
#[test]
fn transact_verify_false_with_no_compensate_slot_yields_false() {
    let src = r#"
        fn call_api(txn_id: str, amount: i64) -> i64 {
            return amount
        }
        fn check(resp: i64) -> bool {
            return resp > 0
        }
        fn update_db(amount: i64) -> i64 {
            print("commit ran") // must never print
            return amount
        }

        fn main() {
            let result: bool = transact {
                network: call_api(txn_id, -5)
                verify:  check(network)
                commit:  update_db(-5)
            }
            print(result)
        }
    "#;
    let log_path = unique_temp_path("transact_log_c");
    let (stdout, code) = compile_and_run_with_env(src, codegen::OptLevel::O2, &[("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "0\n");
    let _ = std::fs::remove_file(&log_path);
}

/// `network`'s `retry`/`timeout` modifiers are a real, architectural
/// rejection (`check_expr`'s pre-pass), not a generic "unsupported"
/// fallthrough — confirms the specific error path, not just that *some*
/// error occurs.
#[test]
fn transact_network_retry_is_explicitly_rejected_not_silently_ignored() {
    let src = r#"
        fn call_api(txn_id: str, amount: i64) -> i64 { return amount }
        fn check(resp: i64) -> bool { return resp > 0 }
        fn update_db(amount: i64) -> i64 { return amount }

        fn main() {
            let result: bool = transact {
                network: call_api(txn_id, 5) retry 3
                verify:  check(network)
                commit:  update_db(5)
            }
            print(result)
        }
    "#;
    let program = parse_checked(src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    let err = codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect_err("retry should be rejected");
    assert!(err.contains("retry"), "unexpected error message: {err}");
}

/// Phase 4's real durability contribution over the old Layer-1-only
/// behavior above: `commit`'s own bounded retry-with-backoff
/// (`Codegen::emit_call_with_retry`) when its return type is
/// `Result(_, _)`. `commit_flaky`'s own attempt counter is a real
/// `db_execute`d row in a small SQLite side-table (not an in-process
/// counter, since a fresh process — see the replay test below — must
/// see the same count) — it returns `Err` for its first 4 real calls
/// and `Ok` from the 5th on. `TRANSACT_RETRY_MAX_ATTEMPTS == 3` means
/// the live path alone (1 initial + 2 retries) can only ever reach
/// attempt 3 — provably not enough to succeed here — so the durability
/// log must show the row still `commit_pending` (never `committed`)
/// once the process exits, and the live `transact` expression's own
/// value is still `true` (this function's pre-existing, unchanged
/// semantics: it reports which branch `verify` took, not whether
/// `commit` is confirmed durable).
#[test]
fn transact_commit_retries_with_backoff_and_leaves_the_row_pending_on_exhaustion() {
    let counter_db = unique_temp_path("transact_retry_counter");
    let counter_db_display = counter_db.display();
    let log_path = unique_temp_path("transact_retry_log");
    let src = format!(
        r#"
        fn call_api(txn_id: str, amount: i64) -> i64 {{ return amount }}
        fn check(resp: i64) -> bool {{ return resp > 0 }}

        // One `let` statement per `db_execute`/`db_query` call, an error
        // mid-sequence folded to a sentinel `-1` count rather than
        // propagated -- this is the same shape `examples/features/27_database.nir`'s
        // own `run_all` already establishes, specifically so `stop conn`
        // (below) is one unconditional statement that always runs, on
        // every path, rather than needing a `stop` on each of several
        // early-return error arms (a `db` handle is affine, exactly like
        // `box`/`tcp`/`file` -- it must be `stop`ped on every path that
        // created it).
        struct ErrMsg {{
            value: str,
        }}

        fn run_flaky(conn: db, amount: i64) -> Result(i64, ErrMsg) {{
            let created: i64 = match db_execute(conn, "CREATE TABLE IF NOT EXISTS attempts (n INTEGER)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let seeded: i64 = match db_execute(conn, "INSERT INTO attempts (n) SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM attempts)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let bumped: i64 = match db_execute(conn, "UPDATE attempts SET n = n + 1") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let count: i64 = match db_query(conn, "SELECT n FROM attempts") {{
                Ok(rows) => match json_array_get(rows, 0) {{
                    Ok(row) => match json_get_i64(row, "n") {{
                        Ok(n) => n,
                        Err(e) => -1,
                    }},
                    Err(e) => -1,
                }},
                Err(e) => -1,
            }}
            stop conn
            if count < 5 {{
                return Err(ErrMsg("still failing"))
            }}
            return Ok(amount)
        }}

        fn commit_flaky(amount: i64) -> Result(i64, ErrMsg) {{
            return match db_connect("{counter_db_display}") {{
                Ok(conn) => run_flaky(conn, amount),
                Err(e) => Err(ErrMsg(e)),
            }}
        }}

        fn main() {{
            let result: bool = transact {{
                network: call_api(txn_id, 42)
                verify:  check(network)
                commit:  commit_flaky(42)
            }}
            print(result)
        }}
    "#
    );
    let (stdout, code) = compile_and_run_with_env(&src, codegen::OptLevel::O2, &[("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n"); // `transact`'s own value: `true` -- verify took the commit branch
    let attempts: i64 = rusqlite::Connection::open(&counter_db).unwrap().query_row("SELECT n FROM attempts", [], |r| r.get(0)).unwrap();
    assert_eq!(attempts, 3, "live retry should make exactly TRANSACT_RETRY_MAX_ATTEMPTS real calls, no more/fewer");
    let log_conn = rusqlite::Connection::open(&log_path).unwrap();
    let state: String = log_conn.query_row("SELECT state FROM nirdosha_transact_log", [], |r| r.get(0)).unwrap();
    assert_eq!(state, "commit_pending", "every live attempt failed -- the row must be left pending for replay, never marked committed");
    let _ = std::fs::remove_file(&counter_db);
    let _ = std::fs::remove_file(&log_path);
}

/// The other half of the same story: crash replay actually finishing a
/// `commit_pending` row a prior process run left behind. Same
/// `commit_flaky`/`finish` program as above, run **twice** against the
/// same durability log *and* the same attempt-counter database (a real,
/// separate process each time — `Command::new` — not two calls inside
/// one process, since replay's whole point is recovering across a
/// restart): the first run's 3 live attempts (counts 1/2/3) all fail
/// exactly as above, leaving a `commit_pending` row. The second run's
/// `main` prologue replays that row *before* its own `main` body ever
/// executes (`emit_c_main`'s ordering) — the replay trampoline's own
/// independent 3-attempt budget reaches counts 4 (still `Err`, 4<5) then
/// 5 (`>=5`, `Ok`) on its second try, well inside its own budget, and
/// `nir_transact_replay_all` marks that first row `committed`. The
/// second run's own fresh `transact` (a new `txn_id`) then also commits
/// immediately (count is already `>=5`) — asserted here only as "some
/// second row exists," since this test's real subject is the *first*
/// row's fate, not the second transact's own mechanics.
#[test]
fn transact_replay_finishes_a_commit_pending_row_left_by_a_prior_process() {
    let counter_db = unique_temp_path("transact_replay_counter");
    let counter_db_display = counter_db.display();
    let log_path = unique_temp_path("transact_replay_log");
    let src = format!(
        r#"
        fn call_api(txn_id: str, amount: i64) -> i64 {{ return amount }}
        fn check(resp: i64) -> bool {{ return resp > 0 }}

        // One `let` statement per `db_execute`/`db_query` call, an error
        // mid-sequence folded to a sentinel `-1` count rather than
        // propagated -- this is the same shape `examples/features/27_database.nir`'s
        // own `run_all` already establishes, specifically so `stop conn`
        // (below) is one unconditional statement that always runs, on
        // every path, rather than needing a `stop` on each of several
        // early-return error arms (a `db` handle is affine, exactly like
        // `box`/`tcp`/`file` -- it must be `stop`ped on every path that
        // created it).
        struct ErrMsg {{
            value: str,
        }}

        fn run_flaky(conn: db, amount: i64) -> Result(i64, ErrMsg) {{
            let created: i64 = match db_execute(conn, "CREATE TABLE IF NOT EXISTS attempts (n INTEGER)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let seeded: i64 = match db_execute(conn, "INSERT INTO attempts (n) SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM attempts)") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let bumped: i64 = match db_execute(conn, "UPDATE attempts SET n = n + 1") {{
                Ok(n) => n,
                Err(e) => -1,
            }}
            let count: i64 = match db_query(conn, "SELECT n FROM attempts") {{
                Ok(rows) => match json_array_get(rows, 0) {{
                    Ok(row) => match json_get_i64(row, "n") {{
                        Ok(n) => n,
                        Err(e) => -1,
                    }},
                    Err(e) => -1,
                }},
                Err(e) => -1,
            }}
            stop conn
            if count < 5 {{
                return Err(ErrMsg("still failing"))
            }}
            return Ok(amount)
        }}

        fn commit_flaky(amount: i64) -> Result(i64, ErrMsg) {{
            return match db_connect("{counter_db_display}") {{
                Ok(conn) => run_flaky(conn, amount),
                Err(e) => Err(ErrMsg(e)),
            }}
        }}

        fn main() {{
            let result: bool = transact {{
                network: call_api(txn_id, 42)
                verify:  check(network)
                commit:  commit_flaky(42)
            }}
            print(result)
        }}
    "#
    );
    let program = parse_checked(&src);
    let report = analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect("codegen::build should succeed for this program");

    let envs = [("NIRDOSHA_TRANSACT_LOG_PATH", log_path.to_str().unwrap())];
    let run1 = Command::new(&out_path).envs(envs).output().expect("first run should execute");
    assert_eq!(run1.status.code(), Some(0));
    {
        let log_conn = rusqlite::Connection::open(&log_path).unwrap();
        let pending: i64 = log_conn.query_row("SELECT COUNT(*) FROM nirdosha_transact_log WHERE state = 'commit_pending'", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 1, "first run's commit must exhaust its retry budget and leave exactly one row pending");
    }

    let run2 = Command::new(&out_path).envs(envs).output().expect("second run should execute");
    assert_eq!(run2.status.code(), Some(0));
    let _ = std::fs::remove_file(&out_path);

    let log_conn = rusqlite::Connection::open(&log_path).unwrap();
    let pending: i64 = log_conn.query_row("SELECT COUNT(*) FROM nirdosha_transact_log WHERE state = 'commit_pending'", [], |r| r.get(0)).unwrap();
    assert_eq!(pending, 0, "replay must finish the row the first process left behind -- none may still be commit_pending");
    let committed: i64 = log_conn.query_row("SELECT COUNT(*) FROM nirdosha_transact_log WHERE state = 'committed'", [], |r| r.get(0)).unwrap();
    assert_eq!(committed, 2, "the replayed row, plus the second run's own fresh (already-past-the-threshold) transact");

    let _ = std::fs::remove_file(&counter_db);
    let _ = std::fs::remove_file(&log_path);
}

/// Phase 4 (`mq`, Redis): a real, compiled `mq_connect`/`mq_publish`/
/// `mq_consume` round trip against the real local Redis instance this
/// environment already has running at `127.0.0.1:6379` (same host/port
/// `examples/features/28_message_queue.nir` itself uses) — not a mock.
/// `Ok(m) => m` (a bare bound identifier as `mq_consume`'s Ok arm) is
/// deliberately *not* written first here — the same documented
/// `match_expr` ordering workaround the `transact`/`db` tests above
/// already use.
#[test]
fn mq_publish_and_consume_round_trip_a_real_message() {
    let src = r#"
        struct Text { value: str }

        fn round_trip(conn: mq) -> Text {
            let published: bool = match mq_publish(conn, "nirdosha_codegen_test_queue", "hello from a compiled binary") {
                Err(e) => false,
                Ok(u) => true,
            }
            print(published)

            let consumed: Text = match mq_consume(conn, "nirdosha_codegen_test_queue", 2) {
                Err(e) => Text(e),
                Ok(m) => Text(m),
            }
            stop conn
            return consumed
        }

        fn main() {
            let result: Text = match mq_connect("127.0.0.1", 6379) {
                Err(e) => Text(e),
                Ok(conn) => round_trip(conn),
            }
            print(result.value)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\nhello from a compiled binary\n");
}

/// Phase 4 (`http`/`https`): a real, compiled `http_get`/`http_post`
/// round trip against a real local TCP server the same test spawns
/// (self-contained, no external network dependency, same shape
/// `compiled_listen_accept_serves_a_real_client` already uses) — proves
/// the full compiled pipeline (`codegen::emit_http_call` -> `nir_http_get`/
/// `nir_http_post` -> a real socket), not just the kernel in isolation
/// (already covered by `runtime-kernels`' own `http_kernel_tests`).
/// `https_get`/`https_post` (vendored-OpenSSL-linked, chunked-transfer-
/// encoding-decoding) are exercised directly against a real production
/// server (`example.com`) as a manual verification step instead of an
/// automated test here, to keep this suite free of an external-network
/// dependency — the shared `parse_http_response`/`decode_chunked_body`
/// logic underneath both is already covered by `runtime-kernels`' own
/// unit tests either way.
#[test]
fn http_get_and_post_round_trip_against_a_real_local_server() {
    let get_port = free_port();
    let src = format!(
        r#"
        struct HttpResult {{
            status: i64,
            body: str,
        }}

        fn server(l: tcp_listener) -> unit {{
            let conn: tcp = accept(l)
            let req: str = recv(conn)
            send(conn, "HTTP/1.1 201 Created\r\nConnection: close\r\n\r\ncreated ok")
            stop conn
            stop l
            return
        }}

        fn main() {{
            let l: tcp_listener = listen({get_port})
            let h: thread unit = spawn server(l)

            let result: HttpResult = match http_get("127.0.0.1", {get_port}, "/api/thing") {{
                Err(e) => HttpResult(-1, e),
                Ok(resp) => HttpResult(resp.status, resp.body),
            }}
            print(result.status)
            print(result.body)
            join h
        }}
    "#
    );
    let (stdout, code) = compile_and_run(&src);
    assert_eq!(code, 0);
    assert_eq!(stdout, "201\ncreated ok\n");

    let post_port = free_port();
    let src2 = format!(
        r#"
        struct HttpResult {{
            status: i64,
            body: str,
        }}

        // `thread`/`spawn`/`join` compile word-sized payloads only
        // (`docs/LANGUAGE.md` §10) -- `str`/struct results don't cross
        // that boundary yet, so the server prints the request it
        // received itself, from inside the spawned thread, rather than
        // returning it through `join`.
        fn server(l: tcp_listener) -> unit {{
            let conn: tcp = accept(l)
            let req: str = recv(conn)
            print(req) // the real request the server received, proving Content-Length + body were sent
            send(conn, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nack")
            stop conn
            stop l
            return
        }}

        fn main() {{
            let l: tcp_listener = listen({post_port})
            let h: thread unit = spawn server(l)

            let result: HttpResult = match http_post("127.0.0.1", {post_port}, "/submit", "payload=42") {{
                Err(e) => HttpResult(-1, e),
                Ok(resp) => HttpResult(resp.status, resp.body),
            }}
            print(result.status)
            print(result.body)
            join h
        }}
    "#
    );
    let (stdout2, code2) = compile_and_run(&src2);
    assert_eq!(code2, 0);
    // The server's own `print(req)` runs (in real wall-clock order)
    // before `main`'s `http_post` can possibly finish reading a
    // response — the server must `recv`, print, and `send` before
    // `main`'s blocking read returns at all — so `req`'s print always
    // lands first in `stdout`, found by running this, not assumed.
    let request_line = stdout2.lines().next().unwrap();
    assert!(request_line.starts_with("POST /submit HTTP/1.1"), "unexpected request: {request_line}");
    assert!(stdout2.contains("Content-Length: 10"), "unexpected request: {stdout2}");
    assert!(stdout2.trim_end().ends_with("200\nack"), "unexpected tail: {stdout2}");
}

/// Phase 5 (`workflow` Layer 1, `docs/WORKFLOW.md`): a real, compiled
/// `start_<workflow>`/`advance_<workflow>` round trip — an empty-`data`
/// workflow (`Codegen::resolve_workflow_layer1`'s own disclosed
/// restriction; `38_workflow.nir`'s own non-empty `data` block is why
/// that example still isn't compiled-runnable this round, see
/// `examples/features/README.md`), `on_entry`/`on_exit` actions that
/// both read the implicit `instance_id` binding, an ordinary transition,
/// and a `terminal` state reached for real. Also proves
/// `Err(NoSuchTransition)`/`Err(InstanceNotFound)` are real `Err`s, never
/// traps, for a bad event and a nonexistent instance id respectively.
#[test]
fn workflow_start_advance_runs_on_entry_on_exit_and_reaches_a_terminal_state() {
    let src = r#"
        fn log_open(instance_id: i64) -> unit {
            print("opened", instance_id)
        }
        fn log_close(instance_id: i64) -> unit {
            print("closed", instance_id)
        }
        fn log_exit_open(instance_id: i64) -> unit {
            print("exiting open", instance_id)
        }

        workflow Ticket {
            state Open {
                on_entry {
                    log_open(instance_id)
                }
                on_exit {
                    log_exit_open(instance_id)
                }
                on Close -> Closed
            }
            state Closed terminal {
                on_entry {
                    log_close(instance_id)
                }
            }
        }

        // `advance_ticket`'s trailing `payload: json` argument needs a
        // real `json` value, not a `Result(json, str)` — this mirrors
        // `38_workflow.nir`'s own `decide_approval` wrapper exactly,
        // *including* its arm order: `Ok(payload) => advance_ticket(...)`
        // (a plain call with a fully known return type) must come first.
        // Swapping the order — `Err(...) => Err(NoSuchTransition())`
        // first — hits a different, also pre-existing `local_ty_of`/
        // `ctor_ty` gap: with no expected-type context, a generic
        // `Err(...)`-wrapping arm can't resolve which `Result`
        // instantiation it belongs to when it's `arms[0]`.
        fn advance_now(identity: VerifiedIdentity, instance_id: i64, event: TicketEvent) -> Result(bool, WorkflowActionError) {
            return match json_parse("{}") {
                Ok(payload) => advance_ticket(identity, instance_id, event, payload),
                Err(e) => Err(NoSuchTransition()),
            }
        }

        fn main() {
            let identity: VerifiedIdentity = VerifiedIdentity("u1", "iss", "aud", 9999999999, 0, "{}")

            let instance_id: i64 = match start_ticket(None(), TicketData()) {
                Err(e) => -1,
                Ok(id) => id,
            }
            print(instance_id)

            let closed: bool = match advance_now(identity, instance_id, Close()) {
                Err(e) => false,
                Ok(v) => v,
            }
            print(closed)

            // `Closed` has no outgoing transitions -- firing `Close()`
            // again is a real `Err` (`NoSuchTransition`), never a trap.
            let bad_event: bool = match advance_now(identity, instance_id, Close()) {
                Err(e) => true,
                Ok(v) => false,
            }
            print(bad_event)

            // A nonexistent instance id is a real `Err` (`InstanceNotFound`),
            // never a trap either.
            let missing: bool = match advance_now(identity, 999999, Close()) {
                Err(e) => true,
                Ok(v) => false,
            }
            print(missing)
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0, "stdout: {stdout}");
    // `print(a, b)` prints each argument on its own line (`Codegen::call`'s
    // `"print"` arm), and `Open`'s `on_entry` runs *inside* `start_ticket`
    // itself, before it returns the new instance id — so "opened"/"1"
    // land before `main`'s own `print(instance_id)` line. `true`/`false`
    // print as `1`/`0` (`Codegen::call`'s own disclosed cosmetic
    // difference from the deleted interpreter's `"true"`/`"false"`).
    assert_eq!(
        stdout,
        "opened\n1\n1\nexiting open\n1\nclosed\n1\n1\n1\n1\n",
        "unexpected stdout: {stdout}"
    );
}

/// Phase 5 (`send_email`, `docs/WORKFLOW.md`): a workflow's `on_entry`
/// action really does post an authenticated HTTPS-shaped request to a
/// real local server, driven by a real, admin-editable provider row in a
/// real SQLite table — not a mock. Also proves the not-configured path
/// (no provider row at all) is a real `Err`, never a trap.
#[test]
fn workflow_on_entry_send_email_posts_to_a_real_configured_provider() {
    let port = free_port();
    let src = format!(
        r#"
        fn server(l: tcp_listener) -> unit {{
            let conn: tcp = accept(l)
            let req: str = recv(conn)
            print(req)
            send(conn, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nsent")
            stop conn
            stop l
        }}

        // Neither `db` nor `json` has a literal an `Err` arm could fall
        // back to (there's no syntax for an opaque handle, and
        // `27_database.nir`'s own idiom never binds a bare `json` out of
        // a match either) -- so, like that file, every `Result`-typed
        // value here is consumed inside a nested match, never unwrapped
        // into a bare `let x: db/json = match {{ ... }}` with a made-up
        // fallback.
        fn notify_with_conn(conn: db, instance_id: i64) -> unit {{
            let created: i64 = match db_execute(conn, "CREATE TABLE email_provider_config (id INTEGER PRIMARY KEY, active INTEGER, host TEXT, port INTEGER, path TEXT, api_key TEXT, from_address TEXT)") {{
                Err(e) => -1,
                Ok(n) => n,
            }}
            let inserted: i64 = match db_execute(conn, "INSERT INTO email_provider_config (active, host, port, path, api_key, from_address) VALUES (1, ?, ?, ?, ?, ?)", "127.0.0.1", {port}, "/send", "secret-key-123", "noreply@example.com") {{
                Err(e) => -1,
                Ok(n) => n,
            }}
            match json_parse("{{}}") {{
                Ok(vars) => match send_email(conn, ByRole("reviewer"), "ticket_opened", vars) {{
                    Ok(v) => print(v),
                    Err(e) => print(false),
                }},
                Err(e) => print(e),
            }}
            stop conn
        }}

        fn notify_open(instance_id: i64) -> unit {{
            match db_connect(":memory:") {{
                Ok(conn) => notify_with_conn(conn, instance_id),
                Err(e) => print(e),
            }}
        }}

        workflow Ticket {{
            state Open {{
                on_entry {{
                    notify_open(instance_id)
                }}
                on Close -> Closed
            }}
            state Closed terminal {{
            }}
        }}

        // No provider row at all this time -- a real `Err`, never a trap.
        fn check_not_configured(conn: db) -> bool {{
            let not_configured: bool = match json_parse("{{}}") {{
                Ok(vars) => match send_email(conn, ByRole("reviewer"), "ticket_opened", vars) {{
                    Ok(v) => false,
                    Err(e) => true,
                }},
                Err(e) => false,
            }}
            stop conn
            return not_configured
        }}

        fn main() {{
            let l: tcp_listener = listen({port})
            let h: thread unit = spawn server(l)

            let instance_id: i64 = match start_ticket(None(), TicketData()) {{
                Err(e) => -1,
                Ok(id) => id,
            }}
            print(instance_id)
            join h

            match db_connect(":memory:") {{
                Ok(conn) => print(check_not_configured(conn)),
                Err(e) => print(false),
            }}
        }}
    "#
    );
    let (stdout, code) = compile_and_run(&src);
    assert_eq!(code, 0, "stdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    // The server's own `print(req)` (real happens-before: `send_email`'s
    // blocking POST must complete before `start_ticket`/`main` can move
    // on) lands before `main`'s own prints -- same real, verified-by-
    // running ordering `http_get_and_post_round_trip_against_a_real_local_server`
    // already established. Real, authenticated, JSON-bodied POST: the
    // provider's own `api_key` shows up as a real `Authorization: Bearer`
    // header, and the body is `{"to","from","template","vars"}` built
    // straight from the real, admin-editable provider row and the
    // `send_email` call's own arguments (`send_via_provider`'s own doc
    // comment) -- not a hand-typed test fixture.
    assert!(lines[0].starts_with("POST /send HTTP/1.1"), "unexpected request line: {}", lines[0]);
    assert!(stdout.contains("Authorization: Bearer secret-key-123"), "missing auth header: {stdout}");
    assert!(
        stdout.contains(r#""from":"noreply@example.com""#) && stdout.contains(r#""template":"ticket_opened""#) && stdout.contains(r#""to":"reviewer""#),
        "unexpected body: {stdout}"
    );
    // Tail, in real observed order: the on_entry action's own
    // `print(v)` (the successful `send_email` -> `true`), `main`'s
    // `print(instance_id)`, then `print(check_not_configured(conn))`
    // (no provider row the second time -- a real `Err`, never a trap,
    // reported as `true`).
    assert_eq!(&lines[lines.len() - 3..], ["1", "1", "1"], "unexpected tail: {stdout}");
}

/// Phase 5 (`__workflow_overdue`, `docs/ROADMAP.md` A15): `state {
/// sla_seconds: N }` plus the synthesized `list_<workflow>_overdue()`
/// really do detect a stale instance — no scheduling/cron primitive
/// exists in this language (`docs/WORKFLOW.md`'s own "Deliberate non-
/// goals" section), so this is the disclosed, queryable fallback an
/// external scheduler would poll. Also proves a state with *no*
/// `sla_seconds` entry is never reported overdue, and that a
/// non-terminal `state` with no matching `sla_seconds` key still moves
/// on normally.
#[test]
fn workflow_list_overdue_reports_a_real_stale_instance_and_ignores_states_with_no_sla() {
    let src = r#"
        workflow Ticket {
            state Open {
                sla_seconds: 1
                on Close -> Closed
            }
            state Closed terminal {
            }
        }

        fn main() {
            let instance_id: i64 = match start_ticket(None(), TicketData()) {
                Err(e) => -1,
                Ok(id) => id,
            }
            print(instance_id)

            // Not overdue yet -- `entered_at` is "now". `print` doesn't
            // support an aggregate (`WorkflowActionError`) argument yet,
            // so the (real, but here-unreachable) `Err` arm prints a
            // fixed literal instead of the error value itself.
            match list_ticket_overdue() {
                Ok(j) => print(j), // []
                Err(e) => print("overdue query failed"),
            }

            sleep_ms(1100)

            match list_ticket_overdue() {
                Ok(j) => print(j), // [{"instance_id":1,"state":"Open","age_seconds":...}]
                Err(e) => print("overdue query failed"),
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0, "stdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "[]", "a brand-new instance shouldn't be overdue yet: {stdout}");
    assert!(
        lines[2].contains(r#""instance_id":1"#) && lines[2].contains(r#""state":"Open""#),
        "expected the stale instance to be reported overdue: {stdout}"
    );
}

/// A `workflow` with a non-empty `data {{ ... }}` block is explicitly
/// rejected, not silently miscompiled — `Codegen::resolve_workflow_layer1`'s
/// own doc comment: this Layer 1 runtime has nowhere durable to persist
/// a `data` value past the initial `start_*` call.
#[test]
fn workflow_with_a_data_block_is_explicitly_rejected_not_silently_dropped() {
    let src = r#"
        workflow Approval {
            data {
                amount: i64,
            }
            state Pending {
                on Approve -> Approved
            }
            state Approved terminal {
            }
        }

        fn main() {
            let instance_id: i64 = match start_approval(None(), ApprovalData(100)) {
                Err(e) => -1,
                Ok(id) => id,
            }
            print(instance_id)
        }
    "#;
    let program = parse_checked(src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    let err = codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect_err("a non-empty `data` block should be rejected");
    assert!(err.contains("data"), "unexpected error message: {err}");
}

/// `__workflow_link_advance` (a magic-link `*_via_link` fn, only
/// synthesized for a workflow that declares a `link`-marked transition)
/// is explicitly rejected too — same real durable-storage gap, but
/// scoped only to programs that actually declare one, unlike the
/// `data`-block restriction above which applies to the whole workflow.
#[test]
fn workflow_link_marked_transitions_are_explicitly_rejected_not_silently_dropped() {
    let src = r#"
        workflow Approval {
            state Pending {
                on link Approve -> Approved
            }
            state Approved terminal {
            }
        }

        fn main() {
            let instance_id: i64 = match start_approval(None(), ApprovalData()) {
                Err(e) => -1,
                Ok(id) => id,
            }
            print(instance_id)
        }
    "#;
    let program = parse_checked(src);
    let report = nirdosha::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_test_{}_{}", std::process::id(), unique_suffix()));
    let err = codegen::build(&program, &report, &out_path, codegen::OptLevel::O2).expect_err("a `link`-marked transition's `*_via_link` fn should be rejected");
    assert!(err.contains("magic-link") || err.contains("__workflow_link_advance"), "unexpected error message: {err}");
}

/// Regression: a real `nirdosha hi`-generated program (captured live —
/// see a `nirdosha_hi_*.log`/`.nir` pair from a real session) panicked
/// `codegen.rs`'s own `declare_named_type` with `internal error: entered
/// unreachable code: ... decl_name="Ok" args=[]`, crashing the whole
/// process rather than reporting an ordinary diagnostic. Root cause:
/// `local_ty_of`'s `Expr::Match` arm assumed the first arm's body always
/// represents the whole match's type, which breaks specifically when
/// that body is a bare `Ok(..)`/`Err(..)` reconstruction of `Result`
/// (`Ok(r)`'s own payload only ever pins down `T`, never `E` -- see
/// `match_result_ty`'s own doc comment) -- an entirely ordinary "pass a
/// Result value straight through" pattern, not a contrived one:
/// `emit_ui.rs`'s own tests already used this exact shape (`match
/// json_parse(..) { Ok(v) => Ok(v), Err(e) => Err(0) }`) without ever
/// tripping this, only because those tests never call `codegen::build`
/// at all -- typeck alone never had this bug. This test does call it,
/// at `-O2`, and actually runs the result, so it would have caught the
/// crash directly.
#[test]
fn match_arm_reconstructing_a_bare_ok_err_result_does_not_panic_codegen() {
    let src = r#"
        fn parse_it() -> Result(json, i64) requires(public) {
            let parsed: Result(json, i64) = match json_parse("[1, 2, 3]") {
                Ok(v) => Ok(v),
                Err(e) => Err(0),
            }
            return parsed
        }

        fn main() requires(public) {
            let r: Result(json, i64) = parse_it()
            match r {
                Ok(_) => print("ok"),
                Err(e) => print(e),
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "ok");
}

/// Regression: a second, real captured crash after the fix above --
/// same `unreachable!` (`decl_name="Err"` this time), same
/// `match_result_ty`, but for the *other* shape a bare reconstruction
/// arm takes: `Err(e) => Err(SomeVariant(e))`, wrapping the bound
/// payload in another constructor rather than passing it through
/// unchanged. The overwhelmingly common real pattern this shape comes
/// from: converting a builtin's own raw error type into a user error
/// enum's own variant, exactly like every `db_execute`/`db_query` call
/// in a real generated CRUD app does (`Err(e) => Err(DbError(e))`).
#[test]
fn match_arm_wrapping_a_bound_payload_in_another_constructor_does_not_panic_codegen() {
    let src = r#"
        enum ErrorCode {
            DbError(str),
        }

        fn parse_it() -> Result(json, ErrorCode) requires(public) {
            let parsed: Result(json, ErrorCode) = match json_parse("[1, 2, 3]") {
                Ok(v) => Ok(v),
                Err(e) => Err(DbError(e)),
            }
            return parsed
        }

        fn main() requires(public) {
            let r: Result(json, ErrorCode) = parse_it()
            match r {
                Ok(_) => print("ok"),
                Err(e) => print("err"),
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "ok");
}

/// Regression: the real root cause behind all three crashes above --
/// not fixed by any of those individual patches, only by the actual
/// architectural gap they were symptoms of. Captured directly from a
/// real `nirdosha hi` session's login/auth function (unmodified in
/// shape, just given a `:memory:` db so this test needs no fixture
/// file): an `if`/`else` whose `else` branch is a `match`, whose own
/// `Ok` arm's body is *another* `match`, whose own `Ok` arm's body is
/// an `if`/`else` reconstructing a bare `Ok`/`Err` `Result` three
/// levels deep. Every prior fix (`match_result_ty`'s sibling-combining,
/// `if_result_ty`'s sibling-preferring) is a heuristic that re-derives
/// a branch's type with *no* context; this one has a real context two
/// frames up (the outer `let`'s declared type) that was simply being
/// dropped by `expr_ptr_expected` the moment a nested expression wasn't
/// a *direct* constructor call. Threading `expected` all the way down
/// (this function's own real fix) resolves every level correctly on
/// its own, without needing any of those heuristics at this depth.
#[test]
fn expected_type_threads_through_deeply_nested_if_match_reconstructions() {
    let src = r#"
        enum ErrorCode {
            DbError(str),
            NotFound(),
        }

        struct Creds {
            username: str,
            password: str,
        }

        fn authenticate_inner(conn: db, creds: Creds) -> Result(json, ErrorCode) requires(public) {
            let fk: i64 = match db_execute(conn, "PRAGMA foreign_keys = ON") {
                Ok(n) => n,
                Err(e) => -1,
            }
            let setup: i64 = match db_execute(conn, "CREATE TABLE IF NOT EXISTS user (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT, password TEXT)") {
                Ok(n) => n,
                Err(e) => -1,
            }
            let result: Result(json, ErrorCode) = if fk < 0 || setup < 0 {
                Err(DbError("failed to setup user table"))
            } else {
                match db_query(conn, "SELECT id FROM user WHERE username = ? AND password = ?", creds.username, creds.password) {
                    Ok(rows) => match json_array_len(rows) {
                        Ok(n) => if n > 0 {
                            match json_array_get(rows, 0) {
                                Ok(row) => Ok(row),
                                Err(e) => Err(DbError(e)),
                            }
                        } else {
                            Err(NotFound())
                        },
                        Err(e) => Err(DbError(e)),
                    },
                    Err(e) => Err(DbError(e)),
                }
            }
            stop(conn)
            return result
        }

        fn authenticate(creds: Creds) -> Result(json, ErrorCode) requires(public) {
            return match db_connect(":memory:") {
                Ok(conn) => authenticate_inner(conn, creds),
                Err(e) => Err(DbError(e)),
            }
        }

        fn main() requires(public) {
            let creds: Creds = Creds("nobody", "wrong-password")
            match authenticate(creds) {
                Ok(_) => print("found"),
                Err(e) => match e {
                    NotFound() => print("not found"),
                    DbError(msg) => print(msg),
                },
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "not found");
}

/// Regression: a *third* real captured crash (same `unreachable!`,
/// `decl_name="Err"`) after the two fixes above -- this time the bare
/// `Err(..)` reconstruction wasn't in a `match` arm at all, it was one
/// branch of an `if`/`else` *expression* used as a `let`'s RHS, with
/// the other branch a nested `match` that itself resolves cleanly
/// (`if cond { Err(DbError(msg)) } else { match ... { ... } }`) -- a
/// real generated shape (a schema-setup guard clause before a query).
/// `if_expr` had its own, never-fixed copy of the exact same "only the
/// representative branch's type is checked" assumption `match_expr`
/// already had fixed for it -- this is the whack-a-mole outcome that
/// motivated `if_result_ty`/`is_unresolved_ok_err_placeholder` being
/// shared, principled functions instead of another one-off patch.
#[test]
fn if_else_branch_reconstructing_a_bare_err_result_does_not_panic_codegen() {
    let src = r#"
        enum ErrorCode {
            DbError(str),
        }

        fn compute(bad: bool) -> Result(json, ErrorCode) requires(public) {
            let result: Result(json, ErrorCode) = if bad {
                Err(DbError("setup failed"))
            } else {
                match json_parse("[1, 2, 3]") {
                    Ok(v) => Ok(v),
                    Err(e) => Err(DbError(e)),
                }
            }
            return result
        }

        fn main() requires(public) {
            let r: Result(json, ErrorCode) = compute(false)
            match r {
                Ok(_) => print("ok"),
                Err(e) => print("err"),
            }
        }
    "#;
    let (stdout, code) = compile_and_run(src);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "ok");
}
