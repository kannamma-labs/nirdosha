//! The real, end-to-end proof for
//! `rfcs/0005-plugin-boundary-safety-and-performance.md` §3's
//! `NativePluginBuiltin`: a genuine third-party Rust function, compiled
//! to a real `staticlib`, linked into a real `nirdosha build` output via
//! `codegen::build_with_native_plugins`, and the resulting **native
//! binary actually run** — not a codegen-emits-plausible-IR check, the
//! same "run the real pipeline end to end" standard
//! `crates/plugin-example-rot13/tests/end_to_end.rs` already holds
//! itself to for the interpreted path.
//!
//! Answers the question `docs/ECOSYSTEM.md` names as a deliberate,
//! disclosed limit ("no stable calling convention from generated LLVM
//! IR into an opaque `Arc<dyn Fn>` exists... plugins stay permanently
//! interpreter-only for the compiled path") for exactly the scalar-only
//! subset `NativePluginBuiltin::validate` accepts: it doesn't, anymore,
//! for that subset.

use nirdosha::ast::Ty;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::plugin::NativePluginBuiltin;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck_with_native_plugins;
use std::process::Command;

/// Compiles a tiny real Rust source file to a `staticlib` via a direct
/// `rustc` invocation (the same "shell out to a real toolchain" posture
/// `codegen::build` itself already takes for `clang`) and returns the
/// resulting `.a` file's bytes — standing in for what a real plugin
/// author's own `build.rs`/`include_bytes!(concat!(env!("OUT_DIR"), ...))`
/// pattern (`NativePluginBuiltin::static_lib`'s own doc comment) would
/// produce ahead of time.
fn compile_native_plugin_staticlib(fn_name: &str, rust_src: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("nirdosha_native_plugin_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src_path = dir.join(format!("{fn_name}.rs"));
    std::fs::write(&src_path, rust_src).expect("write plugin source");
    let lib_path = dir.join(format!("lib{fn_name}.a"));
    let status = Command::new("rustc")
        .arg("--crate-type")
        .arg("staticlib")
        .arg("-O")
        .arg(&src_path)
        .arg("-o")
        .arg(&lib_path)
        .status()
        .expect("rustc should be on PATH -- same assumption codegen::build already makes about clang");
    assert!(status.success(), "rustc failed to compile the test plugin");
    std::fs::read(&lib_path).expect("read compiled staticlib")
}

/// The full real pipeline, `.nir` source to a typechecked+ownership-
/// checked `Program` — mirrors `main.rs::typecheck_and_own_impl`, minus
/// the file-loader indirection (inline source here) and `validate`'s
/// contract check (irrelevant to this test).
fn typecheck_and_own_with_plugins(src: &str, plugins: &[NativePluginBuiltin]) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck_with_native_plugins(&program, plugins).expect("typecheck_with_native_plugins should accept this program");
    check_ownership(&program).expect("ownership check should accept this program");
    program
}

/// The actual claim: a `.nir` program calling a native plugin builtin
/// compiles to a real native binary (through `build_with_native_plugins`,
/// no interpreter involved) and running that binary produces the
/// correct, real answer.
#[test]
fn a_native_plugin_call_compiles_and_the_native_binary_runs_correctly() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_scale",
        r#"
            #[no_mangle]
            pub extern "C" fn plugin_scale(x: i64) -> i64 {
                x.wrapping_mul(2).wrapping_add(1)
            }
        "#,
    );

    let src = r#"
        fn main() -> i64 {
            return plugin_scale(20)
        }
    "#;

    let native_plugin =
        NativePluginBuiltin { name: "plugin_scale".to_string(), params: vec![Ty::I64], ret: Ty::I64, static_lib: Box::leak(lib_bytes.into_boxed_slice()) };
    native_plugin.validate().expect("a scalar i64->i64 signature must validate");

    let program = typecheck_and_own_with_plugins(src, std::slice::from_ref(&native_plugin));
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_native_plugin_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("native_plugin_test_bin");

    codegen::build_with_native_plugins(
        &program,
        &report,
        &out_path,
        codegen::OptLevel::O2,
        std::slice::from_ref(&native_plugin),
        &Default::default(),
    )
    .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    // `fn main() -> i64` becomes the process's own exit code (this
    // backend's real, existing convention -- not this test's own
    // invention), not stdout. `20 * 2 + 1 = 41` -- computed by the
    // *real, separately-compiled* Rust function above, called from
    // *generated LLVM IR*, not by the interpreter (which never runs in
    // this test at all).
    assert_eq!(output.status.code(), Some(41), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// A native plugin declaring a non-scalar type (`str`) is rejected at
/// `validate()` time with a named, actionable reason -- not a confusing
/// LLVM/clang failure surfacing from deep inside a malformed `declare`.
#[test]
fn a_str_typed_native_plugin_is_rejected_by_validate_not_left_to_fail_in_clang() {
    let native_plugin = NativePluginBuiltin { name: "bad_plugin".to_string(), params: vec![Ty::Str], ret: Ty::I64, static_lib: &[] };
    let err = native_plugin.validate().expect_err("a str parameter must be rejected");
    assert!(err.contains("bad_plugin") && err.contains("scalar"), "expected a named, actionable reason, got: {err}");
}

