use super::*;
use super::layout::*;
use super::checks::*;
use super::type_infer::*;

// Internal (non-`tests/`) unit tests: these construct `Codegen`
// directly and call private methods with no `.nir`-reachable hook
// yet, so they cannot be ordinary `tests/codegen.rs` integration
// tests (those only see this crate's public API).
/// Real-execution round-trip tests for `emit_encode_value_json`/
/// `emit_decode_value_json` (Stage 1 of reviving compiled `serve` — see
/// the plan this session recovered `05a747c~1:crates/compiler/src/
/// serve.rs`'s `decode_value`/`encode_value` from for the exact wire
/// shape). Necessarily an *internal* `#[cfg(test)]` module, not a
/// `tests/codegen.rs` integration test: those two functions are private
/// `Codegen` methods with no `Expr`/builtin-name hook reachable from real
/// `.nir` source yet (that's a later stage's job — RFC 0010's route
/// dispatch), and `tests/codegen.rs` links against this crate as an
/// ordinary external dependency, so it can't reach a private method
/// regardless. `codegen_for`/`new_codegen` below are a direct copy of
/// `emit_llvm_ir_impl`'s own `Codegen` construction (same file, same
/// private fields — kept intentionally in lockstep, not a parallel
/// "test-only" simplification that could drift from what a real
/// `Codegen` actually looks like); `run_module` duplicates only the
/// essential subset of `build_impl`'s own clang-linking (skipping the
/// platform-specific TLS-framework/Windows extras real programs
/// sometimes need, since these tests never call anything that needs
/// them) rather than refactoring that function to accept raw IR text,
/// which nothing else needs and would be a wider, riskier change for a
/// test-only benefit.
#[cfg(test)]
mod json_roundtrip_tests {
    use super::*;
    use crate::ownership;
    use crate::parser::Parser;
    use crate::smt;
    use crate::token::Lexer;
    use crate::typeck;

    fn build_program(src: &str) -> (Program, SmtReport) {
        let toks = Lexer::new(src).tokenize().expect("lex should succeed");
        let program = Parser::new(toks).parse_program().expect("parse should succeed");
        typeck::typecheck(&program).expect("should typecheck cleanly");
        ownership::check_ownership(&program).expect("should ownership-check cleanly");
        let report = smt::analyze(&program);
        (program, report)
    }

    fn new_codegen<'a>(program: &'a Program, report: &'a SmtReport) -> Codegen<'a> {
        let registry = TypeRegistry::build(program);
        let sigs: HashMap<String, FnSig> =
            program.fns.iter().map(|f| (f.name.clone(), FnSig { params: f.params.iter().map(|p| p.ty.clone()).collect(), ret: f.ret.clone(), requires: f.requires.clone() })).collect();
        let free_map = ownership::compute_free_map(program);
        Codegen {
            out: String::new(),
            entry_allocas: String::new(),
            string_globals: String::new(),
            trampolines: String::new(),
            transact_sites: Vec::new(),
            tmp: 0,
            label: 0,
            smt_report: report,
            free_map,
            sigs,
            current_fn_ret: Ty::Unit,
            current_fn_name: "json_roundtrip_test_main".to_string(),
            current_fn_sret: None,
            current_fn_nfr: None,
            current_fn_nfr_regs: None,
            current_fn_role_view_param: None,
            current_fn_claim_view_param: None,
            terminated: false,
            audited: false,
            registry,
            declared_named_types: HashSet::new(),
            named_type_decls: String::new(),
            workflows: &program.workflows,
        }
    }

    /// Every `nir_json_*` declare `emit_encode_value_json`/
    /// `emit_decode_value_json` can possibly call, plus `write` (this
    /// module's own print-a-`{ptr,i64}`-value-to-stdout observability
    /// primitive — the raw POSIX syscall, not `printf`, since a Nirdosha
    /// `str` value is a `{ptr, i64}` slice with no null terminator to
    /// format around). A fixed, always-declared list, not "only what
    /// this specific test calls" — harmless (an unused `declare` is
    /// inert) and simpler than threading a per-test subset through.
    fn write_preamble(cg: &mut Codegen) {
        // Real production programs get this from `emit_llvm_ir_impl`'s
        // own unconditional preamble (codegen.rs:1692) — needed here too
        // the moment any test calls `cg.function()` on a `.nir` fn whose
        // own codegen emits an aggregate `sret`/field copy (`Stmt::Return`
        // constructing a `Result`, e.g.), which `llvm.memcpy.p0.p0.i64`
        // backs. Missing this happened to keep working locally (this
        // toolchain's `clang` apparently tolerates an undeclared
        // well-known intrinsic) but failed for real on CI's macOS
        // runner with a genuine `use of undefined value` parse error —
        // found by that CI failure, not anticipated in advance.
        writeln!(cg.out, "declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly, ptr noalias readonly, i64, i1 immarg)").unwrap();
        writeln!(cg.out, "declare i64 @write(i32, ptr, i64)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_i64(i64, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_f64(double, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_bool(i32, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_str(ptr, i64, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_i64(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_f64(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_bool(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_str(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_get(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_set_raw(ptr, i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_validate(ptr, i64, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_str_eq(ptr, i64, ptr, i64)").unwrap();
    }

    /// Emits `call i64 @write(1, <ptr>, <len>)` on the `{ptr, i64}` SSA
    /// value `str_val`, followed by a literal newline byte — so a test
    /// that prints two values (the freshly-encoded JSON, then the same
    /// value decoded and re-encoded) gets them back as two distinct
    /// stdout lines to compare against.
    fn emit_print_str(cg: &mut Codegen, str_val: &str) {
        let ptr = cg.fresh_reg("print_ptr");
        writeln!(cg.out, "  {ptr} = extractvalue {{ptr, i64}} {str_val}, 0").unwrap();
        let len = cg.fresh_reg("print_len");
        writeln!(cg.out, "  {len} = extractvalue {{ptr, i64}} {str_val}, 1").unwrap();
        writeln!(cg.out, "  call i64 @write(i32 1, ptr {ptr}, i64 {len})").unwrap();
        let nl = cg.fresh_global("print_nl");
        writeln!(cg.string_globals, "{nl} = private unnamed_addr constant [1 x i8] c\"\\0A\"").unwrap();
        writeln!(cg.out, "  call i64 @write(i32 1, ptr {nl}, i64 1)").unwrap();
    }

    /// Assembles `cg.out`/`cg.entry_allocas`/`cg.string_globals`/
    /// `cg.named_type_decls` into one real module (the exact splice
    /// order `emit_llvm_ir_impl`/`function` themselves use — see their
    /// own comments), links it against the real `nir_json_*`
    /// implementations (this crate's own embedded `RUNTIME_KERNELS_LIB`,
    /// the same staticlib every real compiled binary links), runs it,
    /// and returns its stdout split into lines. Panics on any failure
    /// (lex/parse/typecheck are `expect()`ed elsewhere; a link/run
    /// failure here means the test's own generated IR was wrong, which
    /// should fail loudly, not be swallowed).
    fn run_module(mut cg: Codegen, alloca_splice_pos: usize) -> Vec<String> {
        writeln!(cg.out, "  ret i32 0").unwrap();
        writeln!(cg.out, "}}").unwrap();
        cg.out.insert_str(alloca_splice_pos, &cg.entry_allocas.clone());
        cg.out.push_str(&cg.string_globals);
        cg.out.push_str(&cg.trampolines);
        cg.out.insert_str(0, &cg.named_type_decls);

        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut ll_path = std::env::temp_dir();
        ll_path.push(format!("nirdosha_json_roundtrip_test_{}_{n}.ll", std::process::id()));
        std::fs::write(&ll_path, &cg.out).expect("writing test .ll");

        let mut runtime_lib_path = std::env::temp_dir();
        runtime_lib_path.push(format!("nirdosha_json_roundtrip_test_runtime_{}_{n}.a", std::process::id()));
        std::fs::write(&runtime_lib_path, RUNTIME_KERNELS_LIB).expect("writing test runtime lib");

        let mut bin_path = std::env::temp_dir();
        bin_path.push(format!("nirdosha_json_roundtrip_test_bin_{}_{n}", std::process::id()));

        let mut clang_cmd = std::process::Command::new("clang");
        clang_cmd.arg(&ll_path).arg(&runtime_lib_path).arg(OptLevel::O0.clang_flag());
        #[cfg(unix)]
        clang_cmd.arg("-lm");
        // Real macOS CI failure, not anticipated in advance (this
        // harness's own doc comment used to claim these tests "never
        // call anything that needs them" — wrong: `rustc`'s release
        // build merges `runtime-kernels` and its whole dependency graph
        // into very few codegen units, so `native-tls`/`tokio-postgres`
        // symbols needing these frameworks ride along in
        // `RUNTIME_KERNELS_LIB` regardless of what the specific test
        // calls — the exact same mechanics `build_impl`'s own identical
        // `#[cfg(target_os = "macos")]` block already documents, just
        // never copied into this simplified test-only linker
        // invocation). Every test in this module links this same
        // `RUNTIME_KERNELS_LIB`, so this needs no per-test conditioning.
        #[cfg(target_os = "macos")]
        clang_cmd
            .arg("-framework")
            .arg("Security")
            .arg("-framework")
            .arg("CoreFoundation")
            .arg("-framework")
            .arg("SystemConfiguration");
        let link_result = clang_cmd.arg("-o").arg(&bin_path).output().expect("running clang");
        let _ = std::fs::remove_file(&ll_path);
        let _ = std::fs::remove_file(&runtime_lib_path);
        assert!(link_result.status.success(), "clang failed:\nstdout:\n{}\nstderr:\n{}", String::from_utf8_lossy(&link_result.stdout), String::from_utf8_lossy(&link_result.stderr));

        let run_result = std::process::Command::new(&bin_path).output().expect("running compiled test binary");
        let _ = std::fs::remove_file(&bin_path);
        assert!(run_result.status.success(), "test binary exited non-zero: {:?}\nstdout:\n{}\nstderr:\n{}", run_result.status, String::from_utf8_lossy(&run_result.stdout), String::from_utf8_lossy(&run_result.stderr));
        String::from_utf8(run_result.stdout).expect("test binary stdout should be UTF-8").lines().map(|l| l.to_string()).collect()
    }

    /// Starts `main`'s own IR (declares + `define i32 @main() {\nentry:\n`)
    /// and returns the position `run_module` should later splice
    /// `entry_allocas` into — the same `alloca_splice_pos` bookkeeping
    /// `function()` itself does, exposed here since this module drives
    /// `cg.out` directly instead of going through `function()`.
    fn start_main(cg: &mut Codegen) -> usize {
        write_preamble(cg);
        writeln!(cg.out, "define i32 @main() {{").unwrap();
        writeln!(cg.out, "entry:").unwrap();
        cg.out.len()
    }

    #[test]
    fn i64_round_trips_through_encode_then_decode() {
        let (program, report) = build_program("fn main() requires(public) { }");
        let mut cg = new_codegen(&program, &report);
        let splice = start_main(&mut cg);

        let slot = cg.fresh_reg("x");
        cg.emit_alloca(&slot, "i64");
        writeln!(cg.out, "  store i64 42, ptr {slot}").unwrap();
        let json1 = cg.emit_encode_value_json(&Ty::I64, &slot).expect("encode i64");
        emit_print_str(&mut cg, &json1);

        let result_ptr = cg.emit_decode_value_json(&Ty::I64, &json1).expect("decode i64");
        let result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str])).unwrap();
        let payload_ptr = cg.fresh_reg("payload_ptr");
        writeln!(cg.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {result_ptr}, i32 0, i32 1").unwrap();
        let json2 = cg.emit_encode_value_json(&Ty::I64, &payload_ptr).expect("re-encode decoded i64");
        emit_print_str(&mut cg, &json2);

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec!["42".to_string(), "42".to_string()]);
    }

    #[test]
    fn bool_and_str_round_trip() {
        let (program, report) = build_program("fn main() requires(public) { }");
        let mut cg = new_codegen(&program, &report);
        let splice = start_main(&mut cg);

        let bslot = cg.fresh_reg("b");
        cg.emit_alloca(&bslot, "i1");
        writeln!(cg.out, "  store i1 true, ptr {bslot}").unwrap();
        let bjson = cg.emit_encode_value_json(&Ty::Bool, &bslot).unwrap();
        emit_print_str(&mut cg, &bjson);

        let sslot = cg.fresh_reg("s");
        cg.emit_alloca(&sslot, "{ptr, i64}");
        let lit = cg.fresh_global("lit");
        writeln!(cg.string_globals, "{lit} = private unnamed_addr constant [5 x i8] c\"ada\\22x\"").unwrap();
        let partial = cg.fresh_reg("s_partial");
        writeln!(cg.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {lit}, 0").unwrap();
        let full = cg.fresh_reg("s_full");
        writeln!(cg.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 5, 1").unwrap();
        writeln!(cg.out, "  store {{ptr, i64}} {full}, ptr {sslot}").unwrap();
        let sjson = cg.emit_encode_value_json(&Ty::Str, &sslot).unwrap();
        emit_print_str(&mut cg, &sjson);

        let bdecoded = cg.emit_decode_value_json(&Ty::Bool, &bjson).unwrap();
        let bool_result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![Ty::Bool, Ty::Str])).unwrap();
        let bpayload = cg.fresh_reg("bpayload");
        writeln!(cg.out, "  {bpayload} = getelementptr inbounds {bool_result_llty}, ptr {bdecoded}, i32 0, i32 1").unwrap();
        let bjson2 = cg.emit_encode_value_json(&Ty::Bool, &bpayload).unwrap();
        emit_print_str(&mut cg, &bjson2);

        let sdecoded = cg.emit_decode_value_json(&Ty::Str, &sjson).unwrap();
        let str_result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str])).unwrap();
        let spayload = cg.fresh_reg("spayload");
        writeln!(cg.out, "  {spayload} = getelementptr inbounds {str_result_llty}, ptr {sdecoded}, i32 0, i32 1").unwrap();
        let sjson2 = cg.emit_encode_value_json(&Ty::Str, &spayload).unwrap();
        emit_print_str(&mut cg, &sjson2);

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec!["true".to_string(), "\"ada\\\"x\"".to_string(), "true".to_string(), "\"ada\\\"x\"".to_string()]);
    }

    #[test]
    fn struct_with_a_nested_struct_field_round_trips() {
        let (program, report) = build_program(
            "struct Inner { n: i64 }\n\
             struct Outer { id: i64, active: bool, inner: Inner }\n\
             fn main() requires(public) { }",
        );
        let mut cg = new_codegen(&program, &report);
        let outer_ty = Ty::Named("Outer".to_string(), vec![]);
        let inner_ty = Ty::Named("Inner".to_string(), vec![]);
        let splice = start_main(&mut cg);

        let outer_llty = cg.llvm_ty(&outer_ty).unwrap();
        let inner_llty = cg.llvm_ty(&inner_ty).unwrap();
        let scratch = cg.fresh_reg("outer");
        cg.emit_alloca(&scratch, &outer_llty);
        let (id_idx, _) = cg.field_index_and_ty(&outer_ty, "id").unwrap();
        let id_ptr = cg.fresh_reg("id_ptr");
        writeln!(cg.out, "  {id_ptr} = getelementptr inbounds {outer_llty}, ptr {scratch}, i32 0, i32 {id_idx}").unwrap();
        writeln!(cg.out, "  store i64 7, ptr {id_ptr}").unwrap();
        let (active_idx, _) = cg.field_index_and_ty(&outer_ty, "active").unwrap();
        let active_ptr = cg.fresh_reg("active_ptr");
        writeln!(cg.out, "  {active_ptr} = getelementptr inbounds {outer_llty}, ptr {scratch}, i32 0, i32 {active_idx}").unwrap();
        writeln!(cg.out, "  store i1 false, ptr {active_ptr}").unwrap();
        let (inner_idx, _) = cg.field_index_and_ty(&outer_ty, "inner").unwrap();
        let inner_field_ptr = cg.fresh_reg("inner_field_ptr");
        writeln!(cg.out, "  {inner_field_ptr} = getelementptr inbounds {outer_llty}, ptr {scratch}, i32 0, i32 {inner_idx}").unwrap();
        let (n_idx, _) = cg.field_index_and_ty(&inner_ty, "n").unwrap();
        let n_ptr = cg.fresh_reg("n_ptr");
        writeln!(cg.out, "  {n_ptr} = getelementptr inbounds {inner_llty}, ptr {inner_field_ptr}, i32 0, i32 {n_idx}").unwrap();
        writeln!(cg.out, "  store i64 99, ptr {n_ptr}").unwrap();

        let json1 = cg.emit_encode_value_json(&outer_ty, &scratch).unwrap();
        emit_print_str(&mut cg, &json1);

        let decoded = cg.emit_decode_value_json(&outer_ty, &json1).unwrap();
        let result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![outer_ty.clone(), Ty::Str])).unwrap();
        let payload_ptr = cg.fresh_reg("outer_payload_ptr");
        writeln!(cg.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {decoded}, i32 0, i32 1").unwrap();
        let json2 = cg.emit_encode_value_json(&outer_ty, &payload_ptr).unwrap();
        emit_print_str(&mut cg, &json2);

        let lines = run_module(cg, splice);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], lines[1], "decode-then-re-encode should reproduce the original JSON byte-for-byte");
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["active"], false);
        assert_eq!(parsed["inner"]["n"], 99);
    }

    #[test]
    fn result_ok_and_err_both_round_trip() {
        let (program, report) = build_program("struct Item { id: i64 }\nfn main() requires(public) { }");
        let mut cg = new_codegen(&program, &report);
        let item_ty = Ty::Named("Item".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![item_ty.clone(), Ty::Str]);
        let splice = start_main(&mut cg);

        let result_llty = cg.llvm_ty(&result_ty).unwrap();
        let item_llty = cg.llvm_ty(&item_ty).unwrap();

        // `Ok(Item(5))`.
        let ok_scratch = cg.fresh_reg("ok_result");
        cg.emit_alloca(&ok_scratch, &result_llty);
        let ok_tag_ptr = cg.fresh_reg("ok_tag_ptr");
        writeln!(cg.out, "  {ok_tag_ptr} = getelementptr inbounds {result_llty}, ptr {ok_scratch}, i32 0, i32 0").unwrap();
        writeln!(cg.out, "  store i64 0, ptr {ok_tag_ptr}").unwrap();
        let ok_payload_ptr = cg.fresh_reg("ok_payload_ptr");
        writeln!(cg.out, "  {ok_payload_ptr} = getelementptr inbounds {result_llty}, ptr {ok_scratch}, i32 0, i32 1").unwrap();
        let (id_idx, _) = cg.field_index_and_ty(&item_ty, "id").unwrap();
        let item_scratch = cg.fresh_reg("item");
        cg.emit_alloca(&item_scratch, &item_llty);
        let item_id_ptr = cg.fresh_reg("item_id_ptr");
        writeln!(cg.out, "  {item_id_ptr} = getelementptr inbounds {item_llty}, ptr {item_scratch}, i32 0, i32 {id_idx}").unwrap();
        writeln!(cg.out, "  store i64 5, ptr {item_id_ptr}").unwrap();
        let item_loaded = cg.fresh_reg("item_loaded");
        writeln!(cg.out, "  {item_loaded} = load {item_llty}, ptr {item_scratch}").unwrap();
        writeln!(cg.out, "  store {item_llty} {item_loaded}, ptr {ok_payload_ptr}").unwrap();
        let ok_json = cg.emit_encode_value_json(&result_ty, &ok_scratch).unwrap();
        emit_print_str(&mut cg, &ok_json);

        // `Err("bad")`.
        let err_scratch = cg.fresh_reg("err_result");
        cg.emit_alloca(&err_scratch, &result_llty);
        let err_tag_ptr = cg.fresh_reg("err_tag_ptr");
        writeln!(cg.out, "  {err_tag_ptr} = getelementptr inbounds {result_llty}, ptr {err_scratch}, i32 0, i32 0").unwrap();
        writeln!(cg.out, "  store i64 1, ptr {err_tag_ptr}").unwrap();
        let err_payload_ptr = cg.fresh_reg("err_payload_ptr");
        writeln!(cg.out, "  {err_payload_ptr} = getelementptr inbounds {result_llty}, ptr {err_scratch}, i32 0, i32 1").unwrap();
        let err_lit = cg.fresh_global("err_lit");
        writeln!(cg.string_globals, "{err_lit} = private unnamed_addr constant [3 x i8] c\"bad\"").unwrap();
        let err_partial = cg.fresh_reg("err_partial");
        writeln!(cg.out, "  {err_partial} = insertvalue {{ptr, i64}} undef, ptr {err_lit}, 0").unwrap();
        let err_full = cg.fresh_reg("err_full");
        writeln!(cg.out, "  {err_full} = insertvalue {{ptr, i64}} {err_partial}, i64 3, 1").unwrap();
        writeln!(cg.out, "  store {{ptr, i64}} {err_full}, ptr {err_payload_ptr}").unwrap();
        let err_json = cg.emit_encode_value_json(&result_ty, &err_scratch).unwrap();
        emit_print_str(&mut cg, &err_json);

        // Decode both back and re-encode, to prove `emit_decode_result_json`
        // too, not just `emit_encode_result_json`.
        let ok_decoded = cg.emit_decode_value_json(&result_ty, &ok_json).unwrap();
        let outer_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![result_ty.clone(), Ty::Str])).unwrap();
        let ok_outer_tag_ptr = cg.fresh_reg("ok_outer_tag_ptr");
        writeln!(cg.out, "  {ok_outer_tag_ptr} = getelementptr inbounds {outer_llty}, ptr {ok_decoded}, i32 0, i32 0").unwrap();
        let ok_outer_tag = cg.fresh_reg("ok_outer_tag");
        writeln!(cg.out, "  {ok_outer_tag} = load i64, ptr {ok_outer_tag_ptr}").unwrap();
        let ok_outer_payload_ptr = cg.fresh_reg("ok_outer_payload_ptr");
        writeln!(cg.out, "  {ok_outer_payload_ptr} = getelementptr inbounds {outer_llty}, ptr {ok_decoded}, i32 0, i32 1").unwrap();
        let ok_rejson = cg.emit_encode_value_json(&result_ty, &ok_outer_payload_ptr).unwrap();
        emit_print_str(&mut cg, &ok_rejson);
        let ok_outer_tag_slot = cg.fresh_reg("ok_outer_tag_slot");
        cg.emit_alloca(&ok_outer_tag_slot, "i64");
        writeln!(cg.out, "  store i64 {ok_outer_tag}, ptr {ok_outer_tag_slot}").unwrap();
        let ok_outer_tag_json = cg.emit_encode_value_json(&Ty::I64, &ok_outer_tag_slot).unwrap();
        emit_print_str(&mut cg, &ok_outer_tag_json);

        let err_decoded = cg.emit_decode_value_json(&result_ty, &err_json).unwrap();
        let err_outer_payload_ptr = cg.fresh_reg("err_outer_payload_ptr");
        writeln!(cg.out, "  {err_outer_payload_ptr} = getelementptr inbounds {outer_llty}, ptr {err_decoded}, i32 0, i32 1").unwrap();
        let err_rejson = cg.emit_encode_value_json(&result_ty, &err_outer_payload_ptr).unwrap();
        emit_print_str(&mut cg, &err_rejson);

        let lines = run_module(cg, splice);
        assert_eq!(lines.len(), 5);
        let ok_val: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(ok_val["ok"]["id"], 5);
        let err_val: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(err_val["err"], "bad");
        assert_eq!(lines[2], lines[0], "decode(Ok)-then-re-encode should reproduce the original JSON");
        assert_eq!(lines[3], "0", "decoding an `{{\"ok\":...}}` payload should land in the outer Result's Ok branch (tag 0)");
        assert_eq!(lines[4], lines[1], "decode(Err)-then-re-encode should reproduce the original JSON");
    }

    #[test]
    fn zero_payload_enum_round_trips_through_encode_then_decode() {
        let (program, report) = build_program(
            "enum Status {\n    Pending,\n    Approved,\n    Rejected,\n}\n\
             fn main() requires(public) { }",
        );
        let mut cg = new_codegen(&program, &report);
        let status_ty = Ty::Named("Status".to_string(), vec![]);
        let splice = start_main(&mut cg);

        // `Status::Approved` (tag 1).
        let status_llty = cg.llvm_ty(&status_ty).unwrap();
        let slot = cg.fresh_reg("status");
        cg.emit_alloca(&slot, &status_llty);
        let tag_ptr = cg.fresh_reg("status_tag_ptr");
        writeln!(cg.out, "  {tag_ptr} = getelementptr inbounds {status_llty}, ptr {slot}, i32 0, i32 0").unwrap();
        writeln!(cg.out, "  store i64 1, ptr {tag_ptr}").unwrap();

        let json1 = cg.emit_encode_value_json(&status_ty, &slot).unwrap();
        emit_print_str(&mut cg, &json1);

        let decoded = cg.emit_decode_value_json(&status_ty, &json1).unwrap();
        let result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![status_ty.clone(), Ty::Str])).unwrap();
        let payload_ptr = cg.fresh_reg("status_payload_ptr");
        writeln!(cg.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {decoded}, i32 0, i32 1").unwrap();
        let json2 = cg.emit_encode_value_json(&status_ty, &payload_ptr).unwrap();
        emit_print_str(&mut cg, &json2);

        // Decoding an unknown variant name must be a clean `Err`, not a
        // crash -- the `no_match` arm's own diagnostic message.
        let bad_lit = cg.fresh_global("bad_variant_lit");
        writeln!(cg.string_globals, "{bad_lit} = private unnamed_addr constant [7 x i8] c\"\\22Bogus\\22\"").unwrap();
        let bad_partial = cg.fresh_reg("bad_partial");
        writeln!(cg.out, "  {bad_partial} = insertvalue {{ptr, i64}} undef, ptr {bad_lit}, 0").unwrap();
        let bad_json = cg.fresh_reg("bad_json");
        writeln!(cg.out, "  {bad_json} = insertvalue {{ptr, i64}} {bad_partial}, i64 7, 1").unwrap();
        let bad_decoded = cg.emit_decode_value_json(&status_ty, &bad_json).unwrap();
        let bad_tag_ptr = cg.fresh_reg("bad_tag_ptr");
        writeln!(cg.out, "  {bad_tag_ptr} = getelementptr inbounds {result_llty}, ptr {bad_decoded}, i32 0, i32 0").unwrap();
        let bad_tag = cg.fresh_reg("bad_tag");
        writeln!(cg.out, "  {bad_tag} = load i64, ptr {bad_tag_ptr}").unwrap();
        let bad_tag_slot = cg.fresh_reg("bad_tag_slot");
        cg.emit_alloca(&bad_tag_slot, "i64");
        writeln!(cg.out, "  store i64 {bad_tag}, ptr {bad_tag_slot}").unwrap();
        let bad_tag_json = cg.emit_encode_value_json(&Ty::I64, &bad_tag_slot).unwrap();
        emit_print_str(&mut cg, &bad_tag_json);

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec!["\"Approved\"".to_string(), "\"Approved\"".to_string(), "1".to_string()]);
    }

    #[test]
    fn json_value_passes_through_encode_and_decode_unchanged() {
        let (program, report) = build_program("fn main() requires(public) { }");
        let mut cg = new_codegen(&program, &report);
        let splice = start_main(&mut cg);

        // An object, not a bare string -- the case `nir_json_decode_str`
        // would wrongly reject (it only accepts a JSON string literal).
        let lit = cg.fresh_global("json_obj_lit");
        let text = "{\"a\":1,\"b\":true}";
        writeln!(cg.string_globals, "{lit} = private unnamed_addr constant [{} x i8] c\"{}\"", text.len(), llvm_escape_bytes(text.as_bytes())).unwrap();
        let slot = cg.fresh_reg("json_val");
        cg.emit_alloca(&slot, "{ptr, i64}");
        let partial = cg.fresh_reg("json_val_partial");
        writeln!(cg.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {lit}, 0").unwrap();
        let full = cg.fresh_reg("json_val_full");
        writeln!(cg.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {}, 1", text.len()).unwrap();
        writeln!(cg.out, "  store {{ptr, i64}} {full}, ptr {slot}").unwrap();

        let json1 = cg.emit_encode_value_json(&Ty::Json, &slot).unwrap();
        emit_print_str(&mut cg, &json1);

        let decoded = cg.emit_decode_value_json(&Ty::Json, &json1).unwrap();
        let result_llty = cg.llvm_ty(&Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str])).unwrap();
        let payload_ptr = cg.fresh_reg("json_payload_ptr");
        writeln!(cg.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {decoded}, i32 0, i32 1").unwrap();
        let json2 = cg.emit_encode_value_json(&Ty::Json, &payload_ptr).unwrap();
        emit_print_str(&mut cg, &json2);

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec![text.to_string(), text.to_string()], "an object `json` value must decode successfully and round-trip byte-for-byte");
    }
}

/// Real, execution-based tests of `emit_serve_route_wrapper` itself —
/// the highest-risk new code in this section (hand-emitted IR with
/// real control flow, not just a straight-line encode/decode). Same
/// harness shape as `json_roundtrip_tests` (a real `Codegen`, real
/// `clang` link against the real embedded runtime, real process run),
/// duplicated rather than shared across the two private test modules —
/// not worth the `pub(super)` noise for a handful of small helpers.
#[cfg(test)]
mod serve_wrapper_tests {
    use super::*;
    use crate::ownership;
    use crate::parser::Parser;
    use crate::smt;
    use crate::token::Lexer;
    use crate::typeck;

    fn build_program(src: &str) -> (Program, SmtReport) {
        let toks = Lexer::new(src).tokenize().expect("lex should succeed");
        let program = Parser::new(toks).parse_program().expect("parse should succeed");
        typeck::typecheck(&program).expect("should typecheck cleanly");
        ownership::check_ownership(&program).expect("should ownership-check cleanly");
        let report = smt::analyze(&program);
        (program, report)
    }

    fn new_codegen<'a>(program: &'a Program, report: &'a SmtReport) -> Codegen<'a> {
        let registry = TypeRegistry::build(program);
        let sigs: HashMap<String, FnSig> =
            program.fns.iter().map(|f| (f.name.clone(), FnSig { params: f.params.iter().map(|p| p.ty.clone()).collect(), ret: f.ret.clone(), requires: f.requires.clone() })).collect();
        let free_map = ownership::compute_free_map(program);
        Codegen {
            out: String::new(),
            entry_allocas: String::new(),
            string_globals: String::new(),
            trampolines: String::new(),
            transact_sites: Vec::new(),
            tmp: 0,
            label: 0,
            smt_report: report,
            free_map,
            sigs,
            current_fn_ret: Ty::Unit,
            current_fn_name: "serve_wrapper_test_main".to_string(),
            current_fn_sret: None,
            current_fn_nfr: None,
            current_fn_nfr_regs: None,
            current_fn_role_view_param: None,
            current_fn_claim_view_param: None,
            terminated: false,
            audited: false,
            registry,
            declared_named_types: HashSet::new(),
            named_type_decls: String::new(),
            workflows: &program.workflows,
        }
    }

    /// Every `nir_*` builtin a route wrapper can possibly call, plus
    /// `write` for observability — same "fixed, always-declared list"
    /// convention `json_roundtrip_tests::write_preamble` already uses.
    fn write_preamble(cg: &mut Codegen) {
        // Real production programs get this from `emit_llvm_ir_impl`'s
        // own unconditional preamble (codegen.rs:1692) — needed here too
        // the moment any test calls `cg.function()` on a `.nir` fn whose
        // own codegen emits an aggregate `sret`/field copy (`Stmt::Return`
        // constructing a `Result`, e.g.), which `llvm.memcpy.p0.p0.i64`
        // backs. Missing this happened to keep working locally (this
        // toolchain's `clang` apparently tolerates an undeclared
        // well-known intrinsic) but failed for real on CI's macOS
        // runner with a genuine `use of undefined value` parse error —
        // found by that CI failure, not anticipated in advance.
        writeln!(cg.out, "declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly, ptr noalias readonly, i64, i1 immarg)").unwrap();
        writeln!(cg.out, "declare i64 @write(i32, ptr, i64)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_i64(i64, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_f64(double, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_bool(i32, ptr)").unwrap();
        writeln!(cg.out, "declare void @nir_json_encode_str(ptr, i64, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_i64(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_f64(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_bool(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_decode_str(ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_get(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_get_str(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_set_raw(ptr, i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_json_array_get(ptr, i64, i64, ptr, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_check_role(ptr, i64, ptr, i64)").unwrap();
        writeln!(cg.out, "declare i32 @nir_extract_claim(ptr, i64, ptr, i64, ptr)").unwrap();
        writeln!(cg.out, "declare i32 @nir_str_eq(ptr, i64, ptr, i64)").unwrap();
    }

    fn emit_print_str(cg: &mut Codegen, str_val: &str) {
        let ptr = cg.fresh_reg("print_ptr");
        writeln!(cg.out, "  {ptr} = extractvalue {{ptr, i64}} {str_val}, 0").unwrap();
        let len = cg.fresh_reg("print_len");
        writeln!(cg.out, "  {len} = extractvalue {{ptr, i64}} {str_val}, 1").unwrap();
        writeln!(cg.out, "  call i64 @write(i32 1, ptr {ptr}, i64 {len})").unwrap();
        let nl = cg.fresh_global("print_nl");
        writeln!(cg.string_globals, "{nl} = private unnamed_addr constant [1 x i8] c\"\\0A\"").unwrap();
        writeln!(cg.out, "  call i64 @write(i32 1, ptr {nl}, i64 1)").unwrap();
    }

    fn emit_print_i32(cg: &mut Codegen, val: &str) {
        let wide = cg.fresh_reg("print_i32_wide");
        writeln!(cg.out, "  {wide} = sext i32 {val} to i64").unwrap();
        let scratch = cg.fresh_reg("print_i32_json_scratch");
        cg.emit_alloca(&scratch, "{ptr, i64}");
        writeln!(cg.out, "  call void @nir_json_encode_i64(i64 {wide}, ptr {scratch})").unwrap();
        let loaded = cg.fresh_reg("print_i32_json");
        writeln!(cg.out, "  {loaded} = load {{ptr, i64}}, ptr {scratch}").unwrap();
        emit_print_str(cg, &loaded);
    }

    /// Builds a `{ptr, i64}` SSA value for the literal string `s`, as a
    /// fresh `private unnamed_addr constant` global — the same "hand a
    /// route wrapper its input as a real global, not a value computed
    /// at runtime" shape a real `nir_json_array_get`/bearer-token-
    /// derived call site would produce.
    fn str_literal(cg: &mut Codegen, prefix: &str, s: &str) -> String {
        let g = cg.fresh_global(prefix);
        writeln!(cg.string_globals, "{g} = private unnamed_addr constant [{} x i8] c\"{}\"", s.len(), llvm_escape_bytes(s.as_bytes())).unwrap();
        let v0 = cg.fresh_reg(&format!("{prefix}_v0"));
        writeln!(cg.out, "  {v0} = insertvalue {{ptr, i64}} undef, ptr {g}, 0").unwrap();
        let v1 = cg.fresh_reg(&format!("{prefix}_v1"));
        writeln!(cg.out, "  {v1} = insertvalue {{ptr, i64}} {v0}, i64 {}, 1", s.len()).unwrap();
        v1
    }

    fn run_module(mut cg: Codegen, alloca_splice_pos: usize) -> Vec<String> {
        writeln!(cg.out, "  ret i32 0").unwrap();
        writeln!(cg.out, "}}").unwrap();
        cg.out.insert_str(alloca_splice_pos, &cg.entry_allocas.clone());
        cg.out.push_str(&cg.string_globals);
        cg.out.push_str(&cg.trampolines);
        cg.out.insert_str(0, &cg.named_type_decls);

        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut ll_path = std::env::temp_dir();
        ll_path.push(format!("nirdosha_serve_wrapper_test_{}_{n}.ll", std::process::id()));
        std::fs::write(&ll_path, &cg.out).expect("writing test .ll");

        let mut runtime_lib_path = std::env::temp_dir();
        runtime_lib_path.push(format!("nirdosha_serve_wrapper_test_runtime_{}_{n}.a", std::process::id()));
        std::fs::write(&runtime_lib_path, RUNTIME_KERNELS_LIB).expect("writing test runtime lib");

        let mut bin_path = std::env::temp_dir();
        bin_path.push(format!("nirdosha_serve_wrapper_test_bin_{}_{n}", std::process::id()));

        let mut clang_cmd = std::process::Command::new("clang");
        clang_cmd.arg(&ll_path).arg(&runtime_lib_path).arg(OptLevel::O0.clang_flag());
        #[cfg(unix)]
        clang_cmd.arg("-lm");
        // Real macOS CI failure, not anticipated in advance (this
        // harness's own doc comment used to claim these tests "never
        // call anything that needs them" — wrong: `rustc`'s release
        // build merges `runtime-kernels` and its whole dependency graph
        // into very few codegen units, so `native-tls`/`tokio-postgres`
        // symbols needing these frameworks ride along in
        // `RUNTIME_KERNELS_LIB` regardless of what the specific test
        // calls — the exact same mechanics `build_impl`'s own identical
        // `#[cfg(target_os = "macos")]` block already documents, just
        // never copied into this simplified test-only linker
        // invocation). Every test in this module links this same
        // `RUNTIME_KERNELS_LIB`, so this needs no per-test conditioning.
        #[cfg(target_os = "macos")]
        clang_cmd
            .arg("-framework")
            .arg("Security")
            .arg("-framework")
            .arg("CoreFoundation")
            .arg("-framework")
            .arg("SystemConfiguration");
        let link_result = clang_cmd.arg("-o").arg(&bin_path).output().expect("running clang");
        let _ = std::fs::remove_file(&ll_path);
        let _ = std::fs::remove_file(&runtime_lib_path);
        assert!(link_result.status.success(), "clang failed:\nstdout:\n{}\nstderr:\n{}", String::from_utf8_lossy(&link_result.stdout), String::from_utf8_lossy(&link_result.stderr));

        let run_result = std::process::Command::new(&bin_path).output().expect("running compiled test binary");
        let _ = std::fs::remove_file(&bin_path);
        assert!(run_result.status.success(), "test binary exited non-zero: {:?}\nstdout:\n{}\nstderr:\n{}", run_result.status, String::from_utf8_lossy(&run_result.stdout), String::from_utf8_lossy(&run_result.stderr));
        String::from_utf8(run_result.stdout).expect("test binary stdout should be UTF-8").lines().map(|l| l.to_string()).collect()
    }

    /// Starts `main`'s own IR *after* every route wrapper under test has
    /// already been fully emitted (each is a self-contained `define...{
    /// ... }`, its own `entry_allocas` cleared-then-spliced internally
    /// by `emit_serve_route_wrapper`) — the explicit `clear()` here
    /// matters precisely because this module, unlike
    /// `json_roundtrip_tests`, never has `main` be the *first* function
    /// in the module: without it, `main`'s own allocas would be spliced
    /// in on top of whatever the last route wrapper already consumed
    /// and left behind in the same shared `entry_allocas` buffer.
    fn start_main(cg: &mut Codegen) -> usize {
        write_preamble(cg);
        cg.entry_allocas.clear();
        writeln!(cg.out, "define i32 @main() {{").unwrap();
        writeln!(cg.out, "entry:").unwrap();
        cg.out.len()
    }

    /// Declares an `out_*` scratch pair (`ptr`/`i64` for body or
    /// cookie) a wrapper call needs, returning the two `alloca`d
    /// addresses to pass as its `out_*_ptr` arguments.
    fn out_scratch(cg: &mut Codegen, prefix: &str) -> (String, String) {
        let ptr_slot = cg.fresh_reg(&format!("{prefix}_ptr_slot"));
        cg.emit_alloca(&ptr_slot, "ptr");
        let len_slot = cg.fresh_reg(&format!("{prefix}_len_slot"));
        cg.emit_alloca(&len_slot, "i64");
        (ptr_slot, len_slot)
    }

    #[test]
    fn a_public_route_decodes_args_calls_the_fn_and_encodes_the_result() {
        let src = r#"
            struct Product {
                name: str,
                price: i64,
            }
            fn create_product(p: Product) -> Result(Product, i64) requires(public) {
                return Ok(p)
            }
            fn main() requires(public) { }
        "#;
        let (program, report) = build_program(src);
        let mut cg = new_codegen(&program, &report);
        for fdecl in &program.fns {
            cg.function(fdecl).expect("fn codegen should succeed");
        }
        let f = program.fns.iter().find(|f| f.name == "create_product").expect("fn exists");
        cg.emit_serve_route_wrapper(f).expect("wrapper codegen should succeed");

        let splice = start_main(&mut cg);
        let args_json = str_literal(&mut cg, "args", r#"[{"name":"widget","price":100}]"#);
        let args_ptr = cg.fresh_reg("args_ptr");
        writeln!(cg.out, "  {args_ptr} = extractvalue {{ptr, i64}} {args_json}, 0").unwrap();
        let args_len = cg.fresh_reg("args_len");
        writeln!(cg.out, "  {args_len} = extractvalue {{ptr, i64}} {args_json}, 1").unwrap();
        let (body_ptr_slot, body_len_slot) = out_scratch(&mut cg, "body");
        let (cookie_ptr_slot, cookie_len_slot) = out_scratch(&mut cg, "cookie");
        let code = cg.fresh_reg("code");
        writeln!(
            cg.out,
            "  {code} = call i32 @__serve_route_create_product(ptr {args_ptr}, i64 {args_len}, ptr null, i64 0, ptr {body_ptr_slot}, ptr {body_len_slot}, ptr {cookie_ptr_slot}, ptr {cookie_len_slot})"
        )
        .unwrap();
        emit_print_i32(&mut cg, &code);
        let body_ptr = cg.fresh_reg("final_body_ptr");
        writeln!(cg.out, "  {body_ptr} = load ptr, ptr {body_ptr_slot}").unwrap();
        let body_len = cg.fresh_reg("final_body_len");
        writeln!(cg.out, "  {body_len} = load i64, ptr {body_len_slot}").unwrap();
        let body_val = cg.fresh_reg("final_body_val");
        writeln!(cg.out, "  {body_val} = insertvalue {{ptr, i64}} undef, ptr {body_ptr}, 0").unwrap();
        let body_val2 = cg.fresh_reg("final_body_val2");
        writeln!(cg.out, "  {body_val2} = insertvalue {{ptr, i64}} {body_val}, i64 {body_len}, 1").unwrap();
        emit_print_str(&mut cg, &body_val2);

        let lines = run_module(cg, splice);
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
        assert_eq!(lines[0], "0", "a successful Ok(...) return should map to code 0");
        let body: serde_json::Value = serde_json::from_str(&lines[1]).expect("body should be real JSON");
        assert_eq!(body["ok"]["name"], "widget");
        assert_eq!(body["ok"]["price"], 100);
    }

    #[test]
    fn a_missing_required_argument_is_a_business_error_not_a_crash() {
        let src = r#"
            fn double_it(x: i64) -> Result(i64, i64) requires(public) {
                return Ok(x * 2)
            }
            fn main() requires(public) { }
        "#;
        let (program, report) = build_program(src);
        let mut cg = new_codegen(&program, &report);
        for fdecl in &program.fns {
            cg.function(fdecl).expect("fn codegen should succeed");
        }
        let f = program.fns.iter().find(|f| f.name == "double_it").expect("fn exists");
        cg.emit_serve_route_wrapper(f).expect("wrapper codegen should succeed");

        let splice = start_main(&mut cg);
        // An empty args array -- `double_it` needs one argument, so
        // `nir_json_array_get` at index 0 must fail, and the wrapper
        // must turn that into a real `1` (business error) response,
        // never a crash/trap.
        let args_json = str_literal(&mut cg, "args", "[]");
        let args_ptr = cg.fresh_reg("args_ptr");
        writeln!(cg.out, "  {args_ptr} = extractvalue {{ptr, i64}} {args_json}, 0").unwrap();
        let args_len = cg.fresh_reg("args_len");
        writeln!(cg.out, "  {args_len} = extractvalue {{ptr, i64}} {args_json}, 1").unwrap();
        let (body_ptr_slot, body_len_slot) = out_scratch(&mut cg, "body");
        let (cookie_ptr_slot, cookie_len_slot) = out_scratch(&mut cg, "cookie");
        let code = cg.fresh_reg("code");
        writeln!(
            cg.out,
            "  {code} = call i32 @__serve_route_double_it(ptr {args_ptr}, i64 {args_len}, ptr null, i64 0, ptr {body_ptr_slot}, ptr {body_len_slot}, ptr {cookie_ptr_slot}, ptr {cookie_len_slot})"
        )
        .unwrap();
        emit_print_i32(&mut cg, &code);

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec!["1".to_string()], "a missing argument should be business-error code 1, not a crash");
    }

    #[test]
    fn a_role_gated_route_rejects_no_token_and_the_wrong_role_then_accepts_the_right_one() {
        let src = r#"
            fn admin_only() -> Result(i64, i64) requires(role: "admin") {
                return Ok(1)
            }
            fn main() requires(public) { }
        "#;
        let (program, report) = build_program(src);
        let mut cg = new_codegen(&program, &report);
        for fdecl in &program.fns {
            cg.function(fdecl).expect("fn codegen should succeed");
        }
        let f = program.fns.iter().find(|f| f.name == "admin_only").expect("fn exists");
        cg.emit_serve_route_wrapper(f).expect("wrapper codegen should succeed");

        let splice = start_main(&mut cg);
        let args_json = str_literal(&mut cg, "args", "[]");
        let args_ptr = cg.fresh_reg("args_ptr");
        writeln!(cg.out, "  {args_ptr} = extractvalue {{ptr, i64}} {args_json}, 0").unwrap();
        let args_len = cg.fresh_reg("args_len");
        writeln!(cg.out, "  {args_len} = extractvalue {{ptr, i64}} {args_json}, 1").unwrap();

        // Call 1: no token at all.
        {
            let (body_ptr_slot, body_len_slot) = out_scratch(&mut cg, "body1");
            let (cookie_ptr_slot, cookie_len_slot) = out_scratch(&mut cg, "cookie1");
            let code = cg.fresh_reg("code1");
            writeln!(
                cg.out,
                "  {code} = call i32 @__serve_route_admin_only(ptr {args_ptr}, i64 {args_len}, ptr null, i64 0, ptr {body_ptr_slot}, ptr {body_len_slot}, ptr {cookie_ptr_slot}, ptr {cookie_len_slot})"
            )
            .unwrap();
            emit_print_i32(&mut cg, &code);
        }

        // Call 2: a token, but the wrong role.
        {
            let identity_json = str_literal(&mut cg, "identity_wrong_role", r#"{"claims_json":"{\"roles\":[\"user\"]}"}"#);
            let id_ptr = cg.fresh_reg("id_ptr2");
            writeln!(cg.out, "  {id_ptr} = extractvalue {{ptr, i64}} {identity_json}, 0").unwrap();
            let id_len = cg.fresh_reg("id_len2");
            writeln!(cg.out, "  {id_len} = extractvalue {{ptr, i64}} {identity_json}, 1").unwrap();
            let (body_ptr_slot, body_len_slot) = out_scratch(&mut cg, "body2");
            let (cookie_ptr_slot, cookie_len_slot) = out_scratch(&mut cg, "cookie2");
            let code = cg.fresh_reg("code2");
            writeln!(
                cg.out,
                "  {code} = call i32 @__serve_route_admin_only(ptr {args_ptr}, i64 {args_len}, ptr {id_ptr}, i64 {id_len}, ptr {body_ptr_slot}, ptr {body_len_slot}, ptr {cookie_ptr_slot}, ptr {cookie_len_slot})"
            )
            .unwrap();
            emit_print_i32(&mut cg, &code);
        }

        // Call 3: a token with the right role.
        {
            let identity_json = str_literal(&mut cg, "identity_right_role", r#"{"claims_json":"{\"roles\":[\"admin\"]}"}"#);
            let id_ptr = cg.fresh_reg("id_ptr3");
            writeln!(cg.out, "  {id_ptr} = extractvalue {{ptr, i64}} {identity_json}, 0").unwrap();
            let id_len = cg.fresh_reg("id_len3");
            writeln!(cg.out, "  {id_len} = extractvalue {{ptr, i64}} {identity_json}, 1").unwrap();
            let (body_ptr_slot, body_len_slot) = out_scratch(&mut cg, "body3");
            let (cookie_ptr_slot, cookie_len_slot) = out_scratch(&mut cg, "cookie3");
            let code = cg.fresh_reg("code3");
            writeln!(
                cg.out,
                "  {code} = call i32 @__serve_route_admin_only(ptr {args_ptr}, i64 {args_len}, ptr {id_ptr}, i64 {id_len}, ptr {body_ptr_slot}, ptr {body_len_slot}, ptr {cookie_ptr_slot}, ptr {cookie_len_slot})"
            )
            .unwrap();
            emit_print_i32(&mut cg, &code);
        }

        let lines = run_module(cg, splice);
        assert_eq!(lines, vec!["2".to_string(), "2".to_string(), "0".to_string()], "no-token and wrong-role should both be 2 (unauthorized); the right role should succeed");
    }
}
