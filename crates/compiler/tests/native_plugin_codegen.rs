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
        NativePluginBuiltin {
            name: "plugin_scale".to_string(),
            params: vec![Ty::I64],
            ret: Ty::I64,
            static_lib: Box::leak(lib_bytes.into_boxed_slice()),
            env_var: None,
            default_max: None,
        };
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

/// A native plugin declaring a type this narrow ABI genuinely doesn't
/// reach yet (`Ty::Json`, still a real aggregate/interpreter-only
/// question -- unlike `str`/`handle` below, which rfcs/0008 widens this
/// to accept) is rejected at `validate()` time with a named, actionable
/// reason -- not a confusing LLVM/clang failure surfacing from deep
/// inside a malformed `declare`.
#[test]
fn a_json_typed_native_plugin_is_rejected_by_validate_not_left_to_fail_in_clang() {
    let native_plugin =
        NativePluginBuiltin { name: "bad_plugin".to_string(), params: vec![Ty::Json], ret: Ty::I64, static_lib: &[], env_var: None, default_max: None };
    let err = native_plugin.validate().expect_err("a json parameter must be rejected");
    assert!(err.contains("bad_plugin") && err.contains("value"), "expected a named, actionable reason, got: {err}");
}

/// rfcs/0008-native-plugin-abi-widening.md Phase 1's actual claim: a
/// `str` value crosses the native boundary in both directions, using
/// the exact `#[repr(C)]` two-word-struct-by-value convention
/// `runtime-kernels/src/lib.rs`'s own `Dec128Bits` already establishes
/// for `str`'s `{ptr, i64}` LLVM shape -- not a new ABI invented for
/// this test, the same one `crates/plugin-example-native-shout`'s real,
/// shipped source uses.
#[test]
fn a_str_typed_native_plugin_crosses_the_boundary_in_both_directions() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_str_roundtrip",
        r#"
            #[repr(C)]
            pub struct NirStr { pub ptr: *const u8, pub len: i64 }

            #[no_mangle]
            pub extern "C" fn plugin_shout(s: NirStr) -> NirStr {
                let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) };
                let mut v: Vec<u8> = bytes.iter().map(|b| b.to_ascii_uppercase()).collect();
                v.push(b'!');
                let leaked: &'static mut [u8] = Box::leak(v.into_boxed_slice());
                NirStr { ptr: leaked.as_ptr(), len: leaked.len() as i64 }
            }

            #[no_mangle]
            pub extern "C" fn plugin_str_first_byte(s: NirStr) -> i64 {
                if s.len == 0 { return -1; }
                let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) };
                bytes[0] as i64
            }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    let src = r#"
        fn main() -> i64 {
            let shouted: str = plugin_shout("hi")
            return plugin_str_first_byte(shouted)
        }
    "#;

    let shout = NativePluginBuiltin {
        name: "plugin_shout".to_string(),
        params: vec![Ty::Str],
        ret: Ty::Str,
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    let first_byte = NativePluginBuiltin {
        name: "plugin_str_first_byte".to_string(),
        params: vec![Ty::Str],
        ret: Ty::I64,
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    shout.validate().expect("a str->str signature must validate now");
    first_byte.validate().expect("a str->i64 signature must validate");

    let plugins = [shout, first_byte];
    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_native_str_plugin_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("native_str_plugin_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    // "hi" -> plugin_shout -> "HI!" -> plugin_str_first_byte -> b'H' == 72,
    // computed entirely by the real, separately-compiled Rust functions
    // above, both string value and byte crossing the same LLVM-generated
    // `call` boundary twice in one program.
    assert_eq!(output.status.code(), Some(72), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// rfcs/0008 Phase 1's other half: `handle(Kind)` crosses the native
/// boundary as a plain `i64`, exactly like `Ty::Thread`/`Ty::Channel`/
/// `Ty::File` already do -- proven here by a value minted on one side of
/// the boundary and read back correctly on the other, through generated
/// LLVM IR, no interpreter involved.
#[test]
fn a_handle_typed_native_plugin_crosses_the_boundary_as_a_plain_i64() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_handle_roundtrip",
        r#"
            #[no_mangle]
            pub extern "C" fn plugin_counter_open() -> i64 { 42 }

            #[no_mangle]
            pub extern "C" fn plugin_counter_close(h: i64) -> i64 { h }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    let src = r#"
        fn main() -> i64 {
            let h: handle(Counter) = plugin_counter_open()
            return plugin_counter_close(h)
        }
    "#;

    let open = NativePluginBuiltin {
        name: "plugin_counter_open".to_string(),
        params: vec![],
        ret: Ty::Handle("Counter".to_string()),
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    let close = NativePluginBuiltin {
        name: "plugin_counter_close".to_string(),
        params: vec![Ty::Handle("Counter".to_string())],
        ret: Ty::I64,
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    open.validate().expect("a ()->handle signature must validate now");
    close.validate().expect("a handle->i64 signature must validate");

    let plugins = [open, close];
    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_native_handle_plugin_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("native_handle_plugin_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    assert_eq!(output.status.code(), Some(42), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// The other real-world requirement rfcs/0008 §Phase 1 adds: a *read*
/// of a handle must borrow (`&handle(Kind)`, crossing as a plain `ptr`
/// to the caller's `i64` slot -- `codegen.rs::llvm_ty`'s existing
/// `Ty::Ref(_) => "ptr"` case, no new codegen needed), not consume it,
/// so the same handle can be used any number of times before the one
/// real, final, consuming `close`. Without this, `Ty::Handle`'s affine
/// guarantee (rfcs/0005 §1) would make a real "connect, query N times,
/// close" plugin -- e.g. `crates/plugin-example-native-kv`'s `kv_set`/
/// `kv_get` -- impossible to call more than once.
#[test]
fn a_borrowed_native_plugin_handle_can_be_read_any_number_of_times_before_one_final_close() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_handle_borrow",
        r#"
            #[no_mangle]
            pub extern "C" fn plugin_counter_open() -> i64 { 10 }

            #[no_mangle]
            pub extern "C" fn plugin_counter_peek(h: *const i64) -> i64 {
                unsafe { *h }
            }

            #[no_mangle]
            pub extern "C" fn plugin_counter_close(h: i64) -> i64 { h }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    let src = r#"
        fn main() -> i64 {
            let h: handle(Counter) = plugin_counter_open()
            let a: i64 = plugin_counter_peek(&h)
            let b: i64 = plugin_counter_peek(&h)
            let c: i64 = plugin_counter_close(h)
            return a + b + c
        }
    "#;

    let counter_kind = || Ty::Handle("Counter".to_string());
    let open = NativePluginBuiltin {
        name: "plugin_counter_open".to_string(),
        params: vec![],
        ret: counter_kind(),
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    let peek = NativePluginBuiltin {
        name: "plugin_counter_peek".to_string(),
        params: vec![Ty::Ref(Box::new(counter_kind()))],
        ret: Ty::I64,
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    let close = NativePluginBuiltin {
        name: "plugin_counter_close".to_string(),
        params: vec![counter_kind()],
        ret: Ty::I64,
        static_lib: lib_bytes,
        env_var: None,
        default_max: None,
    };
    open.validate().expect("a ()->handle signature must validate");
    peek.validate().expect("a &handle->i64 signature must validate now");
    close.validate().expect("a handle->i64 signature must validate");

    let plugins = [open, peek, close];
    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_native_handle_borrow_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("native_handle_borrow_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    // 10 (peek) + 10 (peek again -- the same handle, still valid because
    // both reads borrowed it) + 10 (close, the one consuming use) == 30.
    assert_eq!(output.status.code(), Some(30), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// The affine half of the handle claim: `ownership.rs` already lists
/// `Ty::Handle(_)` in `is_affine()` (rfcs/0005 §1) -- this proves that
/// still holds for a *native*-plugin-sourced handle now that codegen
/// accepts the type, not just for the interpreter path. Using a handle
/// twice must be a compile-time ownership error, before codegen (let
/// alone `clang`) ever runs.
#[test]
fn a_native_plugin_handle_used_twice_is_a_compile_time_ownership_error() {
    let src = r#"
        fn main() -> i64 {
            let h: handle(Counter) = plugin_counter_open()
            let a: i64 = plugin_counter_close(h)
            let b: i64 = plugin_counter_close(h)
            return a + b
        }
    "#;
    let open = NativePluginBuiltin {
        name: "plugin_counter_open".to_string(),
        params: vec![],
        ret: Ty::Handle("Counter".to_string()),
        static_lib: &[],
        env_var: None,
        default_max: None,
    };
    let close = NativePluginBuiltin {
        name: "plugin_counter_close".to_string(),
        params: vec![Ty::Handle("Counter".to_string())],
        ret: Ty::I64,
        static_lib: &[],
        env_var: None,
        default_max: None,
    };
    let plugins = [open, close];

    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck_with_native_plugins(&program, &plugins).expect("typecheck_with_native_plugins should accept this program");
    let err = check_ownership(&program).expect_err("using a handle twice must be rejected");
    let msg = format!("{err:?}");
    assert!(msg.contains("h") || msg.to_lowercase().contains("moved"), "expected a use-after-move ownership error, got: {msg}");
}

/// rfcs/0011 §4's actual claim: registration is eager, at process
/// startup, over the full set the compiler linked — not lazy,
/// deferred to first use. Two synthetic `conn`-shape providers are
/// linked (each with all four required functions, so
/// `validate_plugin_roster` accepts them) but **never called** from
/// the `.nir` program at all; if registration happened before `nir_
/// main` runs the way Phase 4 requires, both providers' domains show
/// up in the flight-recorder dump printed automatically at process
/// exit, alongside the 7 built-ins -- proving the registration calls
/// codegen emits in `main`'s preamble actually ran, not that the
/// providers merely linked successfully.
#[test]
fn eager_plugin_registration_shows_up_in_the_flight_recorder_dump_before_any_user_code_runs() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_two_conn_providers",
        r#"
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_connect(host: i64) -> i64 { host }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_op(conn: i64) -> i64 { conn }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_is_valid(conn: i64) -> i64 { let _ = conn; 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_close(conn: i64) -> i64 { conn }

            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_connect(host: i64) -> i64 { host }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_op(conn: i64) -> i64 { conn }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_is_valid(conn: i64) -> i64 { let _ = conn; 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_close(conn: i64) -> i64 { conn }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    fn provider_fns(scheme: &str, lib_bytes: &'static [u8]) -> [NativePluginBuiltin; 4] {
        [
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_connect"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_op"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_is_valid"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_close"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
        ]
    }

    let mut plugins = Vec::new();
    plugins.extend(provider_fns("fakeschemea", lib_bytes));
    plugins.extend(provider_fns("fakeschemeb", lib_bytes));
    for p in &plugins {
        p.validate().expect("every provider function's ABI shape must validate");
    }

    // Deliberately calls neither provider -- registration must not
    // depend on any `.nir` call site reaching it (rfcs/0011 §4).
    let src = r#"
        fn main() -> i64 {
            return 0
        }
    "#;

    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_eager_plugin_registration_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("eager_plugin_registration_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    assert_eq!(output.status.code(), Some(0), "compiled binary's exit code: {output:?}");

    // `nir_kernel_flight_recorder_dump` prints to stderr (this file's
    // sibling tests' own established convention, see
    // `nirdosha_ops_console.rs`'s identical assumption).
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "db_provider_fakeschemea: held=0 grants=0 denials=0 stale_rehydrated=0",
        "db_provider_fakeschemeb: held=0 grants=0 denials=0 stale_rehydrated=0",
        "db: held=0 grants=0 denials=0 stale_rehydrated=0",
        "http: held=0 grants=0 denials=0 stale_rehydrated=0",
    ] {
        assert!(stderr.contains(expected), "expected flight-recorder dump to contain {expected:?}, got:\n{stderr}");
    }

    let _ = std::fs::remove_dir_all(out_dir);
}

/// Phase 6's actual claim: `db_connect`'s runtime scheme dispatch
/// (rfcs/0011-uniform-service-provider-model.md §2) really calls
/// through to a registered plugin provider, and — the RFC's own
/// structural-safety property — a handle connected through one
/// provider only ever dispatches through *that* provider's own
/// function pointers, never a different provider's, even though both
/// share one process-wide dispatch table. Two synthetic `conn`-shape
/// providers, each returning a JSON array of a distinguishable length
/// from `_op`; a `.nir` program connects to both and multiplies the two
/// lengths together into one exit code that could only come out right
/// if each handle reached its own provider's `_op`, not the other's.
#[test]
fn db_connect_dispatches_to_a_registered_plugin_provider_and_never_crosses_providers() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_two_conn_providers_real",
        r#"
            #[repr(C)]
            pub struct NirStr { pub ptr: *const u8, pub len: i64 }

            fn leak_str(s: &'static str) -> NirStr {
                NirStr { ptr: s.as_ptr(), len: s.len() as i64 }
            }

            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_connect(_host: NirStr) -> i64 { 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_op(_conn: i64, _arg: NirStr) -> NirStr { leak_str("[0,0,0]") }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_is_valid(_conn: i64) -> i64 { 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemea_close(_conn: i64) -> i64 { 0 }

            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_connect(_host: NirStr) -> i64 { 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_op(_conn: i64, _arg: NirStr) -> NirStr { leak_str("[0,0]") }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_is_valid(_conn: i64) -> i64 { 1 }
            #[no_mangle]
            pub extern "C" fn db_provider_fakeschemeb_close(_conn: i64) -> i64 { 0 }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    fn provider_fns(scheme: &str, lib_bytes: &'static [u8]) -> [NativePluginBuiltin; 4] {
        [
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_connect"),
                params: vec![Ty::Str],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_op"),
                params: vec![Ty::I64, Ty::Str],
                ret: Ty::Str,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_is_valid"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
            NativePluginBuiltin {
                name: format!("db_provider_{scheme}_close"),
                params: vec![Ty::I64],
                ret: Ty::I64,
                static_lib: lib_bytes,
                env_var: None,
                default_max: None,
            },
        ]
    }

    let mut plugins = Vec::new();
    plugins.extend(provider_fns("fakeschemea", lib_bytes));
    plugins.extend(provider_fns("fakeschemeb", lib_bytes));
    for p in &plugins {
        p.validate().expect("every provider function's ABI shape must validate");
    }

    let src = r#"
        fn query_len_inner(conn: db) -> i64 {
            let len: i64 = match db_query(conn, "irrelevant") {
                Err(e) => -2,
                Ok(rows) => match json_array_len(rows) {
                    Err(e) => -3,
                    Ok(n) => n,
                },
            }
            stop conn
            return len
        }

        fn connect_a() -> i64 {
            return match db_connect("fakeschemea://user@host/dba") {
                Err(e) => -1,
                Ok(conn) => query_len_inner(conn),
            }
        }

        fn connect_b() -> i64 {
            return match db_connect("fakeschemeb://user@host/dbb") {
                Err(e) => -1,
                Ok(conn) => query_len_inner(conn),
            }
        }

        fn main() -> i64 {
            let la: i64 = connect_a()
            let lb: i64 = connect_b()
            return la * 10 + lb
        }
    "#;

    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_db_connect_plugin_dispatch_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("db_connect_plugin_dispatch_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    // `fakeschemea`'s `_op` returns a 3-element array, `fakeschemeb`'s a
    // 2-element one -- `3 * 10 + 2 = 32`. A cross-provider mixup (either
    // handle reaching the *other* provider's `_op`) would produce `33`,
    // `22`, or some other value, never `32`.
    assert_eq!(output.status.code(), Some(32), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// The disclosed "no provider registered for scheme X" error path
/// (rfcs/0011 §2 step 1) — a scheme with no matching built-in and no
/// registered plugin must fail cleanly at runtime, not panic or hang.
#[test]
fn db_connect_against_an_unregistered_scheme_is_a_clean_runtime_error_not_a_panic() {
    let src = r#"
        fn close_and_zero(conn: db) -> i64 {
            stop conn
            return 0
        }

        fn main() -> i64 {
            return match db_connect("totallyunregisteredscheme://user@host/db") {
                Err(e) => 7,
                Ok(conn) => close_and_zero(conn),
            }
        }
    "#;

    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck_with_native_plugins(&program, &[]).expect("typecheck_with_native_plugins should accept this program");
    check_ownership(&program).expect("ownership check should accept this program");
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_db_connect_unregistered_scheme_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("db_connect_unregistered_scheme_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &[], &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    assert_eq!(output.status.code(), Some(7), "an unregistered scheme must be a clean Err, not a hang/panic: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// `call_via`'s plugin fallback, and specifically rfcs/0011 §2's
/// corrected `call`-shape admission scope: "`kernel::acquire(domain)`
/// brackets *each `_request` call's* own internal checkout... not held
/// across multiple calls the way `conn`/`stream`'s session-scoped
/// bracketing is." A synthetic `call`-shape provider is registered with
/// `default_max: Some(1)` (its domain's admission ceiling is exactly
/// one concurrently-held checkout), and `call_via` is called **twice,
/// sequentially** against it. `call_via` never hands back a `.nir`-level
/// handle to `stop` — there is no other release point in this program at
/// all — so the second call can only succeed if the first call's
/// admission was released the moment its `_request` returned, proving
/// per-request (not session-scoped) bracketing. If release were
/// accidentally held across calls (the session-scoped bug this test
/// exists to catch), the second call would be denied immediately
/// (`kernel::acquire` is non-blocking CAS, never a hang) and its
/// response body would be the `Err` fallback, not the plugin's real
/// echo.
#[test]
fn call_via_brackets_admission_per_request_not_per_session() {
    let lib_bytes = compile_native_plugin_staticlib(
        "plugin_authedhttp_provider",
        r#"
            #[repr(C)]
            pub struct NirStr { pub ptr: *const u8, pub len: i64 }

            fn leak_str(s: String) -> NirStr {
                let leaked: &'static str = Box::leak(s.into_boxed_str());
                NirStr { ptr: leaked.as_ptr(), len: leaked.len() as i64 }
            }

            #[no_mangle]
            pub extern "C" fn authedhttp_provider_authedhttp_connect(_host: NirStr) -> i64 { 1 }

            #[no_mangle]
            pub extern "C" fn authedhttp_provider_authedhttp_request(_conn: i64, path: NirStr, _body: NirStr) -> NirStr {
                let path_str = unsafe {
                    std::str::from_utf8(std::slice::from_raw_parts(path.ptr, path.len as usize)).unwrap_or("")
                };
                leak_str(format!("resp:{path_str}"))
            }

            #[no_mangle]
            pub extern "C" fn authedhttp_provider_authedhttp_is_valid(_conn: i64) -> i64 { 1 }

            #[no_mangle]
            pub extern "C" fn authedhttp_provider_authedhttp_close(_conn: i64) -> i64 { 0 }
        "#,
    );
    let lib_bytes: &'static [u8] = Box::leak(lib_bytes.into_boxed_slice());

    let plugins = [
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_connect".to_string(),
            params: vec![Ty::Str],
            ret: Ty::I64,
            static_lib: lib_bytes,
            env_var: None,
            // Forces this provider's domain ceiling to exactly 1
            // concurrently-held checkout -- the whole point of this
            // test.
            default_max: Some(1),
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_request".to_string(),
            params: vec![Ty::I64, Ty::Str, Ty::Str],
            ret: Ty::Str,
            static_lib: lib_bytes,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_is_valid".to_string(),
            params: vec![Ty::I64],
            ret: Ty::I64,
            static_lib: lib_bytes,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_close".to_string(),
            params: vec![Ty::I64],
            ret: Ty::I64,
            static_lib: lib_bytes,
            env_var: None,
            default_max: None,
        },
    ];
    for p in &plugins {
        p.validate().expect("every provider function's ABI shape must validate");
    }

    let src = r#"
        fn main() -> i64 {
            let r1: str = match call_via("authedhttp://host", "/p1", "b1") {
                Err(e) => "ERR1",
                Ok(resp) => resp.body,
            }
            let r2: str = match call_via("authedhttp://host", "/p2", "b2") {
                Err(e) => "ERR2",
                Ok(resp) => resp.body,
            }
            let ok1: bool = r1 == "resp:/p1"
            let ok2: bool = r2 == "resp:/p2"
            if ok1 {
                if ok2 {
                    return 1
                }
            }
            return 0
        }
    "#;

    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_call_via_per_request_admission_test_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("call_via_per_request_admission_test_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    assert_eq!(
        output.status.code(),
        Some(1),
        "both call_via calls must succeed with the correct per-call response, even at a domain ceiling of 1, \
         proving admission is released after each _request rather than held across calls: {output:?}"
    );

    let _ = std::fs::remove_dir_all(out_dir);
}

