//! Proves that the two real, shipped example crates —
//! `crates/plugin-example-native-shout` and `crates/plugin-example-
//! native-kv` — actually work as their own READMEs claim: build them
//! exactly the way a plugin *consumer* would (`cargo build --release
//! -p <crate>`), link the resulting real `.a` files into a real
//! `nirdosha build` output via `NativePluginBuiltin`/
//! `build_with_native_plugins`, and run the resulting native binary.
//! No interpreter anywhere in this path, and no inline/duplicated
//! plugin source the way `native_plugin_codegen.rs`'s own tests use for
//! speed — this test exercises the identical bytes a real user would
//! get from these crates.

use nirdosha::ast::Ty;
use nirdosha::codegen;
use nirdosha::ownership::check_ownership;
use nirdosha::parser::Parser;
use nirdosha::plugin::NativePluginBuiltin;
use nirdosha::smt::analyze;
use nirdosha::token::Lexer;
use nirdosha::typeck::typecheck_with_native_plugins;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    // `crates/compiler` -> up two levels -> the repo root, where the
    // root `Cargo.toml` (and its shared `target/`) lives.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `cargo build --release -p <crate>` against the *root* workspace —
/// the same command this test's own module doc, and each example
/// crate's own README, tells a consumer to run. Building here (rather
/// than assuming a prior `cargo build --release --workspace` already
/// ran) makes this test self-contained; it costs a few seconds the
/// first time and is a cache hit on every run after.
fn build_release(crate_name: &str) {
    let root = workspace_root();
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("-p")
        .arg(crate_name)
        .status()
        .unwrap_or_else(|e| panic!("failed to invoke cargo to build {crate_name}: {e}"));
    assert!(status.success(), "cargo build --release -p {crate_name} failed");
}

/// `lib<crate_name-with-underscores>.a` in the shared workspace
/// `target/release/` — Cargo's own, platform-default staticlib naming
/// convention (this test only runs on Unix CI/dev machines today, same
/// as the rest of this native-plugin test suite; a `windows-msvc` build
/// would need the `.lib`/no-`lib`-prefix naming
/// `crates/compiler/build.rs` already handles for `runtime-kernels`).
fn read_staticlib(crate_name: &str) -> &'static [u8] {
    let lib_name = format!("lib{}.a", crate_name.replace('-', "_"));
    let path = workspace_root().join("target/release").join(&lib_name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    Box::leak(bytes.into_boxed_slice())
}

fn typecheck_and_own_with_plugins(src: &str, plugins: &[NativePluginBuiltin]) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck_with_native_plugins(&program, plugins).expect("typecheck_with_native_plugins should accept this program");
    check_ownership(&program).expect("ownership check should accept this program");
    program
}

/// The real claim: install both example crates the way their READMEs
/// describe, call every one of their six builtins from one `.nir`
/// program (`plugin_shout`'s `str` round trip, `kv_open`/`kv_set`/
/// `kv_get`/`kv_close`'s `handle`+`str` round trip), compile it to a
/// real native binary, and run it.
#[test]
fn the_shipped_example_plugin_crates_compile_and_run_correctly() {
    build_release("nirdosha-plugin-native-shout");
    build_release("nirdosha-plugin-native-kv");
    let shout_lib = read_staticlib("nirdosha-plugin-native-shout");
    let kv_lib = read_staticlib("nirdosha-plugin-native-kv");

    let kv_store_kind = || Ty::Handle("KvStore".to_string());
    let kv_store_ref = || Ty::Ref(Box::new(kv_store_kind()));
    let plugins = vec![
        NativePluginBuiltin { name: "plugin_shout".to_string(), params: vec![Ty::Str], ret: Ty::Str, static_lib: shout_lib, env_var: None, default_max: None },
        NativePluginBuiltin { name: "kv_open".to_string(), params: vec![], ret: kv_store_kind(), static_lib: kv_lib, env_var: None, default_max: None },
        NativePluginBuiltin {
            name: "kv_set".to_string(),
            params: vec![kv_store_ref(), Ty::Str, Ty::Str],
            ret: Ty::I64,
            static_lib: kv_lib,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "kv_get".to_string(),
            params: vec![kv_store_ref(), Ty::Str],
            ret: Ty::Str,
            static_lib: kv_lib,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "kv_close".to_string(),
            params: vec![kv_store_kind()],
            ret: Ty::I64,
            static_lib: kv_lib,
            env_var: None,
            default_max: None,
        },
    ];
    for p in &plugins {
        p.validate().unwrap_or_else(|e| panic!("{e}"));
    }

    let src = r#"
        fn main() -> i64 {
            let shouted: str = plugin_shout("hello")
            let h: handle(KvStore) = kv_open()
            let ok1: i64 = kv_set(&h, "greeting", shouted)
            let got: str = kv_get(&h, "greeting")
            let ok2: i64 = kv_close(h)
            if got == "HELLO!" {
                return ok1 * 10 + ok2
            }
            return -1
        }
    "#;

    let program = typecheck_and_own_with_plugins(src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_native_plugin_examples_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("native_plugin_examples_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    let output = Command::new(&out_path).output().expect("running the compiled binary should succeed");
    // "hello" -> plugin_shout -> "HELLO!" -> stored under "greeting" via
    // kv_set (real, separately-compiled Rust code, both times) -> kv_get
    // reads the identical string back -> the `==` comparison is real
    // proof the round trip preserved the bytes exactly, not just that
    // something non-empty came back -> kv_set/kv_close both report
    // success (1) -> 1*10 + 1 == 11.
    assert_eq!(output.status.code(), Some(11), "compiled binary's exit code: {output:?}");

    let _ = std::fs::remove_dir_all(out_dir);
}

/// rfcs/0011-uniform-service-provider-model.md Phase 8's own closing
/// checklist item, proven the same way the test above proves Phases 1/4
/// of rfcs/0008: not a synthetic inline plugin (`native_plugin_codegen.rs`'s
/// own tests use those, for speed), but the real, separately-compiled
/// `crates/plugin-example-native-authed-http` crate, linked into a real
/// `nirdosha build` output and run as a real process — against a real
/// local HTTP server, on an ephemeral port, observing a real
/// `Authorization: Bearer <token>` header that only the plugin (reading
/// `AUTHEDHTTP_BEARER_TOKEN` from the process environment at connect
/// time) could have set.
#[test]
fn the_shipped_authed_http_plugin_compiles_and_makes_a_real_authenticated_request() {
    build_release("nirdosha-plugin-native-authed-http");
    let authedhttp_lib = read_staticlib("nirdosha-plugin-native-authed-http");

    let plugins = vec![
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_connect".to_string(),
            params: vec![Ty::Str],
            ret: Ty::I64,
            static_lib: authedhttp_lib,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_request".to_string(),
            params: vec![Ty::I64, Ty::Str, Ty::Str],
            ret: Ty::Str,
            static_lib: authedhttp_lib,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_is_valid".to_string(),
            params: vec![Ty::I64],
            ret: Ty::I64,
            static_lib: authedhttp_lib,
            env_var: None,
            default_max: None,
        },
        NativePluginBuiltin {
            name: "authedhttp_provider_authedhttp_close".to_string(),
            params: vec![Ty::I64],
            ret: Ty::I64,
            static_lib: authedhttp_lib,
            env_var: None,
            default_max: None,
        },
    ];
    for p in &plugins {
        p.validate().unwrap_or_else(|e| panic!("{e}"));
    }

    // A real local HTTP server, bound to an ephemeral port (`:0`) --
    // never a fixed port, to avoid CI collisions -- whose only job is to
    // observe the incoming request's `Authorization` header and echo a
    // fixed body back.
    let server = tiny_http::Server::http("127.0.0.1:0").expect("binding an ephemeral local test server should succeed");
    let port = server.server_addr().to_ip().expect("this test only binds an IP address, not a unix socket").port();

    const EXPECTED_TOKEN: &str = "rfc0011-phase8-secret-token";
    let observed_auth_header: std::sync::Arc<std::sync::Mutex<Option<String>>> = std::sync::Arc::new(std::sync::Mutex::new(None));
    let observed_auth_header_for_server = observed_auth_header.clone();
    let server_thread = std::thread::spawn(move || {
        // Exactly one request is expected -- `call_via` is called once
        // in the `.nir` program below.
        let request = server.recv().expect("the test server should receive exactly one request");
        let auth = request.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().to_string());
        *observed_auth_header_for_server.lock().unwrap() = auth;
        let response = tiny_http::Response::from_string("authed-ok".to_string());
        request.respond(response).expect("responding to the test request should succeed");
    });

    let src = r#"
        fn main() -> i64 {
            let result: str = match call_via("authedhttp://127.0.0.1:PORT_PLACEHOLDER", "/secure", "payload") {
                Err(e) => "ERR",
                Ok(resp) => resp.body,
            }
            if result == "authed-ok" {
                return 1
            }
            return 0
        }
    "#
    .replace("PORT_PLACEHOLDER", &port.to_string());

    let program = typecheck_and_own_with_plugins(&src, &plugins);
    let report = analyze(&program);

    let out_dir = std::env::temp_dir().join(format!("nirdosha_authed_http_plugin_example_bin_{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("authed_http_plugin_example_bin");

    codegen::build_with_native_plugins(&program, &report, &out_path, codegen::OptLevel::O2, &plugins, &Default::default())
        .expect("build_with_native_plugins should compile and link cleanly");

    // The bearer token is set on the *compiled binary's own* process
    // environment (not this test process's) -- `_connect` reads it via
    // `std::env::var` at connect time, inside that child process.
    let output = Command::new(&out_path)
        .env("AUTHEDHTTP_BEARER_TOKEN", EXPECTED_TOKEN)
        .output()
        .expect("running the compiled binary should succeed");

    server_thread.join().expect("the test server thread should not panic");

    assert_eq!(output.status.code(), Some(1), "compiled binary's exit code (1 == the plugin's response body round-tripped correctly): {output:?}");
    assert_eq!(
        observed_auth_header.lock().unwrap().as_deref(),
        Some(format!("Bearer {EXPECTED_TOKEN}").as_str()),
        "the real local server must have observed exactly the Authorization header the plugin was told to send via env(...)"
    );

    let _ = std::fs::remove_dir_all(out_dir);
}
