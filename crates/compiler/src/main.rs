use std::process::ExitCode;

// The native verify/fix/certify pipeline -- previously defined in this
// file, now living in the library (`nirdosha::verify_pipeline`) so this
// binary's CLI subcommands stay thin wrappers over one implementation.
// The v2 MCP tool surface this file used to also share
// (`tools_call`/`tools_list`/`McpCallLog`, `nirdosha mcp`) moved to the
// standalone `nirdosha-hi` crate (`nirdosha-hi mcp`) 2026-09-16 -- this
// crate has no dependency on it at all anymore.
use nirdosha::verify_pipeline::{
    build_certificate, isolation_violations_from_anomalies, nfr_commitments_from_program, nfr_drift, run_verify_pipeline, sha256_hex, sign_certificate, write_auto_patches, Certificate, FixReport,
    NfrEscalation, ProofVerdict,
};

fn main() -> ExitCode {
    // No interpreter, no `run`/`serve`/`--sandbox-worker` — every
    // remaining subcommand is compiled-path or frontend-only, so no
    // up-front flag scanning is needed before dispatch.
    let mut args = std::env::args().skip(1);
    let first = match args.next() {
        Some(a) => a,
        None => {
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    match first.as_str() {
        "init" => cmd_init(args),
        "gen-crud" => cmd_gen_crud(args),
        "build" => cmd_build(args),
        "verify" => cmd_verify(args),
        "fix" => cmd_fix(args),
        "explain" => cmd_explain(args),
        "certify" => cmd_certify(args),
        "check-isolation" => cmd_check_isolation(args),
        "check-drift" => cmd_check_drift(args),
        "check-guarantees" => cmd_check_guarantees(args),
        "verify-binary" => cmd_verify_binary(args),
        "keygen" => cmd_keygen(args),
        "verify-certificate" => cmd_verify_certificate(args),
        "equivalence" => cmd_equivalence(args),
        "attest" => cmd_attest(args),
        "audit" => cmd_audit(args),
        "suggest-contracts" => cmd_suggest_contracts(args),
        "emit-llvm" => cmd_emit_llvm(args),
        "emit-ast" => cmd_emit_ast(args),
        "emit-ui" => cmd_emit_ui(args),
        "emit-catalog" => cmd_emit_catalog(args),
        "grammar-export" => cmd_grammar_export(args),
        "roles" => cmd_roles(args),
        "hi" | "mcp" | "plugin" => {
            let suffix = if first == "hi" { String::new() } else { format!(" {first}") };
            eprintln!("`nirdosha {first}` moved to the standalone `nirdosha-hi` binary 2026-09-16 (`nirdosha-hi{suffix}`) -- this compiler binary no longer depends on it.");
            ExitCode::FAILURE
        }
        other => {
            eprintln!("unknown subcommand `{other}` -- nirdosha has no interpreter/`run`/`serve` mode anymore; use `build` or `emit-llvm`.");
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  nirdosha init <project-name> [--dest <path>] [--no-email] [--no-roles] [--sms] [--push] [--force]");
    eprintln!("                                      scaffold a self-contained project folder: a starter");
    eprintln!("                                      <project-name>.nir (with the standing Email/RoleMapping");
    eprintln!("                                      admin-panel fixtures, unless disabled), a bundled copy of");
    eprintln!("                                      this executable, and a run.sh/run.bat launcher");
    eprintln!("  nirdosha gen-crud <plan.json> --db <db_connect literal> [-o out.nir]");
    eprintln!("                                      deterministic struct+CRUD .nir source from a JSON");
    eprintln!("                                      entity plan (struct_name/fields/crud_slots/screen_title/");
    eprintln!("                                      field_labels per entity, plus a flat kpis list) --");
    eprintln!("                                      real db_connect/db_execute/db_query bodies, no LLM");
    eprintln!("  nirdosha build <file.nir> -o <out> [--opt0]");
    eprintln!("                                      compile to a native binary (LLVM, -O2 by default); also");
    eprintln!("                                      emits <out>.guarantees.json (RFC 0017's guarantee bundle --");
    eprintln!("                                      inferred effects/gated exports, for `verify-binary` to check");
    eprintln!("                                      later with no recompilation); if <file.nir>.guarantees.json");
    eprintln!("                                      exists, the build also fails on any guarantee violation");
    eprintln!("  nirdosha verify <file.nir> [--in-toto]");
    eprintln!("                                      typecheck/ownership/contract-check only, no LLVM/clang");
    eprintln!("                                      needed -- 3-valued JSON verdict on stdout, exit 0/1/2;");
    eprintln!("                                      --in-toto wraps it as an in-toto v1 Statement, see below");
    eprintln!("  nirdosha fix <file.nir> [--apply] [--in-toto]");
    eprintln!("                                      same checks as verify, plus a byte-offset FixPatch per");
    eprintln!("                                      obligation where one exists (auto/assisted/manual);");
    eprintln!("                                      --apply writes every `auto` patch to the file in place");
    eprintln!("  nirdosha explain [<code>]           print the machine-learnable error index (JSON on stdout);");
    eprintln!("                                      with no <code>, lists every NIR-code and its title");
    eprintln!("  nirdosha certify <file.nir> [--sign <key.pk8>] [--isolation-log <ops.json>] [--in-toto]");
    eprintln!("                                      same checks as verify, wrapped in a deterministic, hash-pinned");
    eprintln!("                                      Certificate v0/v1 (source_hash/grammar_hash/evidence_tier/...;");
    eprintln!("                                      --sign adds a real Ed25519 signature, see `nirdosha keygen`;");
    eprintln!("                                      --isolation-log attaches real observed transaction-isolation");
    eprintln!("                                      anomalies from a saved op-history log, see `check-isolation`)");
    eprintln!("  nirdosha keygen [-o <key.pk8>]      generate an Ed25519 keypair for `nirdosha certify --sign`");
    eprintln!("  nirdosha check-isolation <ops.json> [--teach <hint>] [--in-toto]");
    eprintln!("                                      run the transaction-isolation anomaly detector");
    eprintln!("                                      (`crates/isolation-core`, the same algorithm `transact`'s");
    eprintln!("                                      live db-op tracking uses) over a saved operation-history");
    eprintln!("                                      log on demand -- JSON array of {{txn,resource,kind,seq}};");
    eprintln!("                                      exit 0 (clean) / 1 (anomaly found) / 2 (bad input);");
    eprintln!("                                      --teach records a runtime lesson hi_llm's generate loop");
    eprintln!("                                      proactively consults for a future transact-shaped task");
    eprintln!("  nirdosha check-drift <file.nir> <escalations.json> [--teach <hint>] [--in-toto]");
    eprintln!("                                      compare <file.nir>'s declared nfr(...) commitments");
    eprintln!("                                      against a real saved NIRDOSHA_OBSERVABILITY_URL escalation");
    eprintln!("                                      log (JSON array of {{function,nfr,threshold,actual}} --");
    eprintln!("                                      nfr.rs's own escalation wire shape) and, if any commitment");
    eprintln!("                                      drifted, re-run real verification on <file.nir> and report");
    eprintln!("                                      both; exit 0 (no drift) / 1 (drift found); --teach as above");
    eprintln!("  nirdosha check-guarantees <file.nir> [--manifest <file.nir.guarantees.json>] [--in-toto]");
    eprintln!("                                      RFC 0017: check <file.nir> against a security guarantee");
    eprintln!("                                      manifest's capability ceiling/exported-contract/role-");
    eprintln!("                                      vocabulary rules (--manifest defaults to <file.nir>.guarantees.json");
    eprintln!("                                      next to it); exit 0 (clean, or no manifest present) / 1 (violation)");
    eprintln!("  nirdosha verify-binary <bundle.guarantees.json> --against <policy.json> [--in-toto]");
    eprintln!("                                      RFC 0017: check an already-emitted guarantee bundle (written");
    eprintln!("                                      by `nirdosha build`) against an operator policy -- same manifest");
    eprintln!("                                      JSON shape as --manifest above -- with no recompilation; the");
    eprintln!("                                      binary-side dual of `verify`; exit 0 (satisfies) / 1 (violation)");
    eprintln!("  nirdosha verify-certificate <certificate.json>");
    eprintln!("                                      check a signed certificate's signature against its own");
    eprintln!("                                      embedded public key");
    eprintln!("  nirdosha equivalence <file.nir> <fn_a> <fn_b>");
    eprintln!("                                      prove fn_a and fn_b compute the same result for every");
    eprintln!("                                      input, or find a real counterexample where they diverge");
    eprintln!("  nirdosha attest <file.nir> --reviewer <name> --role agent|human --key <key.pk8>");
    eprintln!("                  --trust-config <config.json> [--note <text>] [-o <attestation.json>]");
    eprintln!("                                      sign a real, unforgeable review attestation for a file");
    eprintln!("  nirdosha audit <file.nir> --trust-config <config.json> [--attestation <a.json>]...");
    eprintln!("                                      consolidated trust report: formal verdict + every");
    eprintln!("                                      attestation's real signature/trust/staleness status");
    eprintln!("  nirdosha suggest-contracts <file.nir> <fn_name>");
    eprintln!("                                      ask an LLM for a validate block, then really check it");
    eprintln!("                                      with Z3 before recommending it (needs an LLM provider,");
    eprintln!("                                      see NIRDOSHA_LLM_PROVIDER_KEY/OPENAI_API_KEY)");
    eprintln!("  nirdosha mcp                        run an MCP server on stdio (JSON-RPC, newline-delimited) --");
    eprintln!("                                      exposes verify_code/get_grammar/fix/describe/certify_code/");
    eprintln!("                                      get_nirdosha_constructs/get_ui_conventions as MCP tools;");
    eprintln!("                                      launch via an MCP client's config, not interactively");
    eprintln!("  nirdosha plugin install [--dry-run] <pack.json>  (also accepts a signed envelope from `plugin sign`)");
    eprintln!("  nirdosha plugin sign <pack.json> --key <key.pk8> --identity <name> [-o <signed-pack.json>]");
    eprintln!("                                      install or refresh a 5a domain plugin; --dry-run checks");
    eprintln!("                                      the manifest and proves its own contracts against a stub");
    eprintln!("  nirdosha plugin list                list installed/available domain plugins");
    eprintln!("  nirdosha plugin revoke <pack-id>    remove a plugin and its non-waivable invariants");
    eprintln!("  nirdosha emit-llvm <file.nir>       print the generated LLVM IR");
    eprintln!("  nirdosha emit-ast <file.nir>        print the parsed AST as JSON (docs/goal.md row 9)");
    eprintln!("  nirdosha emit-ui <file.nir> [-o out.html] [--theme theme.json] [--manifest-path Cargo.toml]");
    eprintln!("                                      derive a Material-styled web UI from struct/fn conventions;");
    eprintln!("                                      --manifest-path (or an auto-detected Cargo.toml next to");
    eprintln!("                                      <file.nir>) links any nir-ui-component crates it depends on");
    eprintln!("                                      (rfcs/0009 Phase B)");
    eprintln!("  nirdosha emit-catalog [-o out.json]");
    eprintln!("                                      print the std UI catalog (rfcs/0009 Phase 0) -- the closed");
    eprintln!("                                      layout/control/chart/theme vocabulary emit-ui renders, as data");
    eprintln!("  nirdosha grammar-export [--root <repo checkout>] [-o out_dir]");
    eprintln!("                                      mechanically-derived EBNF+GBNF: runs the real parser over");
    eprintln!("                                      examples/**/*.nir + the capabilities corpus and renders what");
    eprintln!("                                      it actually walked -- not hand-transcribed from parser.rs");
    eprintln!("  nirdosha roles <file.nir> [-o out.json]");
    eprintln!("                                      every role/claim gate in the program, grouped by role/claim:");
    eprintln!("                                      which fns it gates (requires(role/claim: ...)) and which");
    eprintln!("                                      screen fields it gates (view/edit) -- pure static analysis");
    eprintln!("                                      over data typeck/ui_gen already compute, no new runtime");
    eprintln!("                                      concept (ROADMAP.md A6, \"Roles -> functions/fields report\")");
    eprintln!("  nirdosha hi                          open the native build-mode window: a live 3D graph over");
    eprintln!("                                      .nir/hi.db (rfcs/0013/0014) -- auto-scaffolds/syncs .nir/");
    eprintln!("                                      first (NIRDOSHA_HI_DISABLE=1 to skip). This *is* `hi` --");
    eprintln!("                                      not a separate tool alongside it.");
    eprintln!("  nirdosha hi ingest <doc.md>          content-address, chunk, and FTS-index a requirement/");
    eprintln!("                                      decision/design doc into .nir/hi.db (rfcs/0013)");
    eprintln!("  nirdosha hi sync [<file.nir> ...]    re-run the code-hash walk bare `hi` already runs on");
    eprintln!("                                      startup; with no files, walks every .nir file under the cwd");
    eprintln!("  nirdosha hi link <req-id> <fn|struct|enum|screen:name>");
    eprintln!("                                      record a manual IMPLEMENTS/IMPLEMENTED_BY edge");
    eprintln!("  nirdosha hi impact <target>          bounded impact report (a requirement/decision id, or a");
    eprintln!("                                      code-unit name/kind:name) -- CI-friendly, non-interactive");
    eprintln!("  nirdosha hi serve                    headless HTTP fallback for the same .nir/hi.db graph");
    eprintln!("                                      (rfcs/0014) -- scripting/CI use, never the default");
}

/// Load (resolving any `use "..."` — `docs/ROADMAP.md` Track F, F2 piece 3)
/// -> typecheck -> ownership-check, shared by `build` and `emit-llvm`,
/// applied here before ever generating code. Codegen's own
/// `check_supported` (a third, narrower gate — signed-integer/bool/unit
/// only, no `box`/`&`/`*`) runs separately, inside `codegen::build`/
/// `emit_llvm_ir` themselves, since it's specific to this backend, not a
/// property of the language generally. Returns the entry file's own
/// source alongside the (possibly multi-file-merged) `Program` — a
/// holdover from when `cmd_sandbox_worker` (`Interpreter::new`'s own
/// `source` argument) was a real caller; that command was removed along
/// with the interpreter (see `typecheck_and_own_optional_main_with_ui_components`'s
/// doc comment below), and both current callers (`cmd_build`/
/// `cmd_emit_llvm`) discard it (`let (program, _src) = ...`).
fn typecheck_and_own(path: &str) -> Result<(nirdosha::ast::Program, String), String> {
    typecheck_and_own_impl(path, true, &[])
}

/// Same as `typecheck_and_own`, but does not require a `fn main()` — for
/// commands that never execute an entrypoint (`emit-ui` is the only one
/// left; `serve`/`--sandbox-worker` were removed along with the
/// interpreter — see `typeck::typecheck_optional_main`'s doc comment
/// for the full "why" this still applies to `emit-ui`), plus every
/// linked `ui_plugin::NativeUiComponent`'s `name` as a legal `layout`
/// widget kind (rfcs/0009 Phase B) — `cmd_emit_ui`'s own entry point
/// once `--manifest-path`/an auto-detected `Cargo.toml` resolves any.
/// No plain (components-free) sibling function exists: `cmd_emit_ui` is
/// the only caller and always comes through here, passing `&[]` when no
/// UI-plugin crate was linked — byte-for-byte the same checks
/// `typecheck_optional_main` alone would run (`typecheck_and_own_impl`'s
/// own `ui_components.is_empty()` branch).
fn typecheck_and_own_optional_main_with_ui_components(
    path: &str,
    components: &[nirdosha::ui_plugin::NativeUiComponent],
) -> Result<(nirdosha::ast::Program, String), String> {
    typecheck_and_own_impl(path, false, components)
}

/// Prints `typeck::ungated_fn_warnings` to stderr — non-fatal, unlike a
/// `TypeError` (`docs/ROADMAP.md` A10). Called only from `serve`/`emit-ui`,
/// the two commands where "reachable via `/api/<fn>`" is actually the
/// question being asked; `run`/`build`/`emit-llvm` never serve anything,
/// so warning about HTTP reachability there would be noise unrelated to
/// what those commands do.
fn print_ungated_fn_warnings(program: &nirdosha::ast::Program) {
    for w in nirdosha::typeck::ungated_fn_warnings(program) {
        eprintln!("{w}");
    }
    // `docs/WORKFLOW.md`'s "state ownership" section: same non-fatal,
    // reachability-shaped warning, for a workflow `state` with no
    // `owner` rather than a plain `fn` with no `requires(...)`.
    for w in nirdosha::typeck::workflow_owner_warnings(program) {
        eprintln!("{w}");
    }
}

fn typecheck_and_own_impl(
    path: &str,
    require_main: bool,
    ui_components: &[nirdosha::ui_plugin::NativeUiComponent],
) -> Result<(nirdosha::ast::Program, String), String> {
    let (program, src) = nirdosha::loader::load_program(path)?;
    let type_result = if require_main {
        nirdosha::typeck::typecheck(&program)
    } else if ui_components.is_empty() {
        nirdosha::typeck::typecheck_optional_main(&program)
    } else {
        nirdosha::typeck::typecheck_optional_main_with_ui_components(&program, ui_components)
    };
    if let Err(errors) = type_result {
        let joined = errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n");
        return Err(joined);
    }
    if let Err(errors) = nirdosha::ownership::check_ownership(&program) {
        let joined = errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n");
        return Err(joined);
    }
    // `validate <fn_name> { pre: ... post: ... }`'s build-time
    // "self-check and fail" gate (`docs/ROADMAP.md` Track F, F3;
    // `docs/NEXT_GEN.md` §F3) — here, not in `cmd_build`/`cmd_emit_llvm`
    // alongside `smt::analyze`, deliberately: those two are the
    // *compiled*-codegen path only, and codegen doesn't support `db`
    // yet, so a `db`-backed app (nearly every real one) can never reach
    // them at all. This is the one choke point every command that owns
    // a typechecked program actually goes through — `build`/`run`/
    // `serve`/`emit-ui`/`emit-llvm`/`typecheck` alike — so a declared
    // contract's build-time proof/counterexample-fail applies uniformly
    // regardless of which command is running. Only a real, *proven*
    // defect fails here (a genuine counterexample, or an unbound
    // identifier the contract references) — a contract this Tier-1
    // walker can't statically model at all is neither proved nor
    // disproved by this call; it's still enforced, just at runtime
    // instead (`interpreter.rs::call`'s own backstop) — see
    // `print_unsupported_validate_notes`, called separately by the
    // commands where "why isn't this proven" is worth surfacing.
    if let Err(errors) = nirdosha::contract_check::check_program_contracts(&program) {
        return Err(errors.join("\n"));
    }
    Ok((program, src))
}

/// Prints `contract_check::unsupported_validate_notes` to stderr —
/// non-fatal, the same "surface a previously-silent case" posture
/// `print_ungated_fn_warnings` already takes for an ungated `fn`.
/// Called from exactly the same two commands as that function, for the
/// same reason (`print_ungated_fn_warnings`'s own doc comment: `serve`/
/// `emit-ui` are where "why isn't this enforced up front" is actually
/// the question being asked). The real enforcement — the build-time
/// hard-fail above, and `interpreter.rs::call`'s runtime backstop — is
/// unconditional everywhere regardless of whether this notice gets
/// printed; only the informational "here's why it's not statically
/// proven" heads-up is scoped this narrowly.
fn print_unsupported_validate_notes(program: &nirdosha::ast::Program) {
    for note in nirdosha::contract_check::unsupported_validate_notes(program) {
        eprintln!("{note}");
    }
}

/// `--theme <path>` for `emit-ui`/`serve` — reads a JSON file matching
/// `ui_gen::Theme`'s shape (every field optional, see that struct's own
/// doc comment) and layers it over the baked-in MD3 tokens. `None` (no
/// flag given) keeps output byte-identical to before this flag existed.
fn load_theme(path: Option<&str>) -> Result<Option<nirdosha::ui_gen::Theme>, String> {
    let Some(path) = path else { return Ok(None) };
    let text = std::fs::read_to_string(path).map_err(|e| format!("error reading {path}: {e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("error parsing {path} as a theme JSON object: {e}"))
}

/// `nirdosha init <project-name> [--dest <path>] [--no-email] [--no-roles]
/// [--sms] [--push] [--force]` -- scaffolds `<dest>/<project-name>/`
/// (default `<dest>`: current directory) containing a starter
/// `<project-name>.nir` (`nirdosha::init::generate_source`), a bundled
/// copy of this very executable (`std::env::current_exe()`, copied so the
/// folder can be moved to another machine and run with no separate
/// `nirdosha` install -- same-OS/arch as wherever `init` ran, no cross-
/// compilation attempted), a `run.sh`/`run.bat` launcher for that copy
/// (whichever matches the host OS -- never both, since the other one
/// couldn't run against this binary anyway), and a placeholder
/// `jwks.json` so the launcher's placeholder `--jwks-file`/`--issuer`/
/// `--audience` flags start successfully with every `requires(role: ...)`
/// route still honestly 401ing until real IdP values replace them. This
/// is tooling-level, not a compiler concept: `typeck`/`codegen`/`serve`
/// still only ever know about the one `.nir` file this writes.
fn cmd_init(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut project_name: Option<String> = None;
    let mut dest = ".".to_string();
    let mut opts = nirdosha::init::InitOptions::default();
    let mut force = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dest" => dest = args.next().unwrap_or(dest),
            "--no-email" => opts.email = false,
            "--no-roles" => opts.roles = false,
            "--sms" => opts.sms = true,
            "--push" => opts.push = true,
            "--force" => force = true,
            other => project_name = Some(other.to_string()),
        }
    }
    let Some(name) = project_name else {
        eprintln!(
            "usage: nirdosha init <project-name> [--dest <path>] [--no-email] [--no-roles] [--sms] [--push] [--force]"
        );
        return ExitCode::FAILURE;
    };
    let source = match nirdosha::init::generate_source(&name, &opts) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    let project_dir = std::path::Path::new(&dest).join(&name);
    if let Err(e) = std::fs::create_dir_all(&project_dir) {
        eprintln!("error creating {}: {e}", project_dir.display());
        return ExitCode::FAILURE;
    }

    let nir_path = project_dir.join(format!("{name}.nir"));
    let jwks_path = project_dir.join("jwks.json");
    let exe_dest = project_dir.join(format!("nirdosha{}", std::env::consts::EXE_SUFFIX));
    // The bundled binary only ever works on the host's own OS/arch, so
    // only the launcher that could actually run against it is written --
    // a `run.bat` next to a Linux ELF binary would just be a trap.
    let (launcher_name, launcher_body) = if cfg!(windows) {
        ("run.bat", nirdosha::init::render_launcher_windows(&name))
    } else {
        ("run.sh", nirdosha::init::render_launcher_unix(&name))
    };
    let launcher_path = project_dir.join(launcher_name);

    if !force {
        let conflicts: Vec<String> = [&nir_path, &exe_dest, &launcher_path, &jwks_path]
            .into_iter()
            .filter(|p| p.exists())
            .map(|p| p.display().to_string())
            .collect();
        if !conflicts.is_empty() {
            eprintln!("refusing to overwrite existing file(s): {} (pass --force to overwrite)", conflicts.join(", "));
            return ExitCode::FAILURE;
        }
    }

    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error locating the running nirdosha executable to bundle: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::copy(&current_exe, &exe_dest) {
        eprintln!("error copying {} to {}: {e}", current_exe.display(), exe_dest.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&nir_path, &source) {
        eprintln!("error writing {}: {e}", nir_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&jwks_path, nirdosha::init::placeholder_jwks()) {
        eprintln!("error writing {}: {e}", jwks_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&launcher_path, &launcher_body) {
        eprintln!("error writing {}: {e}", launcher_path.display());
        return ExitCode::FAILURE;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `fs::write` doesn't set the exec bit -- `fs::copy` above
        // already preserved it on the bundled binary, but the launcher
        // script is brand new content and needs it set explicitly.
        if let Err(e) = std::fs::set_permissions(&launcher_path, std::fs::Permissions::from_mode(0o755)) {
            eprintln!("error making {} executable: {e}", launcher_path.display());
            return ExitCode::FAILURE;
        }
    }

    println!("wrote {}/", project_dir.display());
    if cfg!(windows) {
        println!("run it: cd {} && {launcher_name}", project_dir.display());
    } else {
        println!("run it: cd {} && ./{launcher_name}", project_dir.display());
    }
    ExitCode::SUCCESS
}

/// `--serve`'s own default port when no `--serve <port>` value is
/// given — matches `examples/features/51_compiled_serve.nir`'s own
/// hand-written demo port, so a `curl http://127.0.0.1:8080/...`
/// habit from that primitives-only example still works here.
const DEFAULT_SERVE_PORT: u16 = 8080;

fn cmd_build(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut opt = nirdosha::codegen::OptLevel::O2;
    let mut serve: Option<u16> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next(),
            // The generated IR is unoptimized either way (module doc) --
            // this only controls whether clang optimizes after. O2 is
            // the default: docs/goal.md row 5 is about hardware speed, and
            // `nirdosha build` should actually deliver on that unless
            // asked not to (debugging a miscompile without an optimizer
            // in the way is the reason to ask).
            "--opt0" => opt = nirdosha::codegen::OptLevel::O0,
            // Reviving compiled `nirdosha serve` (`rfcs/0010-landing-
            // and-serve-exposure.md`) -- an optional trailing port
            // number (`--serve 9000`) overrides `DEFAULT_SERVE_PORT`;
            // anything else (missing, or the next token doesn't parse
            // as a port) leaves the default in place and that token
            // free to be consumed as the usual positional `<file.nir>`/
            // other flag.
            "--serve" => {
                serve = Some(DEFAULT_SERVE_PORT);
                if let Some(next) = args.next() {
                    match next.parse::<u16>() {
                        Ok(port) => serve = Some(port),
                        Err(_) => input = Some(next),
                    }
                }
            }
            other => input = Some(other.to_string()),
        }
    }
    let (Some(path), Some(out)) = (input, output) else {
        eprintln!("usage: nirdosha build <file.nir> -o <out> [--opt0] [--serve [port]]");
        return ExitCode::FAILURE;
    };
    let (program, _src) = match typecheck_and_own(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    // RFC 0017, real minimal version (Phase 5,
    // `docs/research/2026-09-pending-verification-differentiation-
    // work.md`): if `<path>.guarantees.json` exists, `guarantee_check`
    // runs before codegen and a violation fails the build outright --
    // matching the RFC's own pipeline position (after typeck/ownership,
    // before codegen::build) and its "hard error" semantics for a
    // capability-ceiling/exported-contract/vocabulary violation. No
    // manifest present is not an error -- RFC 0017's own "additive
    // only" compatibility rule.
    let guarantee_registry = nirdosha::ast::TypeRegistry::build(&program);
    let guarantee_manifest = match nirdosha::guarantee_manifest::load(std::path::Path::new(&path)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(manifest) = &guarantee_manifest {
        let violations = nirdosha::guarantee_manifest::check(&program, &guarantee_registry, manifest);
        if !violations.is_empty() {
            eprintln!("build failed: {} guarantee-manifest violation(s) against {}:", violations.len(), nirdosha::guarantee_manifest::manifest_path_for(std::path::Path::new(&path)).display());
            for v in &violations {
                eprintln!("  - {}", v.describe());
            }
            return ExitCode::FAILURE;
        }
    }
    let smt_report = nirdosha::smt::analyze(&program);
    let result = match serve {
        Some(port) => {
            // Only meaningful once a program is actually served -- a
            // plain `nirdosha build` prints nothing new here.
            print_ungated_fn_warnings(&program);
            // The UI is generated *now*, at compile time, and baked
            // into the binary as a plain byte string
            // (`codegen::build_serve`) -- unlike the deleted
            // interpreted `serve.rs`, a compiled process has no
            // `Program` AST left at runtime to call `ui_gen::generate`
            // against. Demo mode only, deliberately, for this first cut
            // (`compiled_serve::ServeConfig::default()`'s own doc
            // comment has the same disclosure) -- real production
            // identity flags for `--serve` are real, separate
            // follow-up work.
            let registry = nirdosha::ast::TypeRegistry::build(&program);
            let effects = nirdosha::effects::infer_effects(&program, &registry);
            let ui_html = nirdosha::ui_gen::generate(&program, &effects, None, false, true, false, None).into_bytes();

            // RFC 0016's FAPI wiring used to auto-detect through
            // `.nir/hi.db` (if a project had one, and an active pack's
            // compliance profile declared `sender_constrained_tokens`,
            // this build turned on `compiled_serve`'s real DPoP
            // enforcement with no CLI flag needed). `.nir/hi.db`'s
            // reader/writer (`hi_graph`/`hi_plugin`) moved to the
            // standalone `nirdosha-hi` crate 2026-09-16 -- this compiler
            // binary no longer depends on it, so that auto-detection no
            // longer happens here. A real, disclosed feature reduction,
            // not silently dropped: DPoP enforcement itself
            // (`compiled_serve`) is unaffected, and `nirdosha-hi`'s own
            // `:publish` is the supported path for a project that wants
            // its installed packs' wiring requirements to shape a build.
            let require_sender_constrained_tokens = false;
            let opts = nirdosha::codegen::ServeCodegenOptions { port, ui_html, require_sender_constrained_tokens };
            nirdosha::codegen::build_serve(&program, &smt_report, std::path::Path::new(&out), opt, &opts)
        }
        None => nirdosha::codegen::build(&program, &smt_report, std::path::Path::new(&out), opt),
    };
    match result {
        Ok(()) => {
            println!("wrote {out}");
            // RFC 0017 §4's emitted guarantee bundle -- always written,
            // manifest or not (a manifest only adds the hard-error
            // enforcement above; the bundle itself is this project's
            // answer to "give me the generated code and I'll tell you
            // whether it satisfies these guarantees," so it travels
            // with every build, not only a manifest-governed one).
            // Best-effort like the FAPI sidecar above: a write failure
            // here is a warning, not a build failure -- the binary
            // already compiled successfully.
            match std::fs::read(&path) {
                Ok(source_bytes) => {
                    let source_hash = sha256_hex(&source_bytes);
                    let bundle = nirdosha::guarantee_manifest::build_bundle(&program, &guarantee_registry, &source_hash, guarantee_manifest.as_ref());
                    let bundle_path = format!("{out}.guarantees.json");
                    match serde_json::to_string_pretty(&bundle) {
                        Ok(text) => match std::fs::write(&bundle_path, text) {
                            Ok(()) => println!("wrote {bundle_path}"),
                            Err(e) => eprintln!("warning: failed to write {bundle_path}: {e}"),
                        },
                        Err(e) => eprintln!("warning: failed to render guarantee bundle: {e}"),
                    }
                }
                Err(e) => eprintln!("warning: could not re-read {path} to hash it for the guarantee bundle: {e}"),
            }
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_emit_llvm(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(path) = args.next() else {
        eprintln!("usage: nirdosha emit-llvm <file.nir>");
        return ExitCode::FAILURE;
    };
    let (program, _src) = match typecheck_and_own(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    let smt_report = nirdosha::smt::analyze(&program);
    match nirdosha::codegen::emit_llvm_ir(&program, &smt_report) {
        Ok(ir) => {
            print!("{ir}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}


/// `nirdosha verify <file.nir>` -- a standalone, machine-readable
/// verdict over the same gates `build`/`emit-llvm` already run
/// (`typecheck_and_own_impl`'s own pipeline: typecheck, ownership,
/// `validate` contracts), plus `smt::analyze`'s Tier-1 proof-obligation
/// counts, without requiring a working LLVM/clang toolchain and without
/// ever producing a binary. Exists so CI and an agent's own repair loop
/// can ask "does this pass?" as one call with a real, three-valued exit
/// code (`0` == `PROVED`, `1` == `DISPROVED`, `2` == `UNKNOWN` -- never
/// collapsed to a binary pass/fail, so a caller that only checks `$?`
/// can still tell "definitely wrong" from "couldn't be decided,"
/// exactly like the JSON `verdict` field below) and a JSON verdict on
/// stdout, instead of parsing `build`'s stderr text.
///
/// Unlike `build`, does not require `fn main()`
/// (`typecheck_optional_main`, not `typecheck` --
/// `typecheck_and_own_optional_main_with_ui_components`'s own
/// `require_main` parameter already draws exactly this line for
/// `emit-ui`): most of what an agent emits under a constrained grammar
/// is a tool/library fragment meant to be `use`d, not a runnable
/// program on its own, and every check here applies just as soundly
/// either way.
///
/// Stages run in the same dependency order `typecheck_and_own_impl`
/// enforces (each one assumes the previous succeeded) and stop at the
/// first failure -- a later stage stays `Skipped`, not silently
/// `Passed`, so the verdict never claims to have checked something it
/// never actually ran.
/// `nirdosha-master-plan.md` Part 3 Q1 2027's "Spec v1 published --
/// verdict schema + certificate format + repair protocol as an
/// in-toto predicate (compose with SLSA, don't compete)." Wraps any of
/// this compiler's three attestation-shaped JSON outputs (`verify`'s
/// `VerifyVerdict`, `certify`'s `Certificate`, `fix`'s `FixReport`) in
/// a real in-toto v1 Statement envelope
/// (<https://in-toto.io/Statement/v1>, verified against the published
/// spec directly, not guessed) -- the existing JSON becomes the
/// `predicate` body unchanged, addressed to the exact source file via
/// a real SHA-256 `subject` digest, under a nirdosha-owned
/// `predicateType` URI namespace (see `docs/SPEC_V1.md` for the full,
/// versioned type list). "Compose with SLSA, don't compete": this is
/// the same generic Statement/Predicate envelope SLSA provenance
/// attestations use, so a nirdosha attestation sits in the identical
/// slot in any in-toto-aware verification pipeline (`cosign verify-
/// attestation`, GitHub's own attestation store, etc.) instead of
/// inventing a separate, incompatible wrapper format.
fn wrap_in_toto(path: &str, source_hash: &str, predicate_kind: &str, predicate: serde_json::Value) -> serde_json::Value {
    const IN_TOTO_STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
    const NIRDOSHA_PREDICATE_NAMESPACE: &str = "https://nirdosha.dev/attestations";
    // in-toto's own `digest` map is `{"<algorithm>": "<hex, no prefix>"}`
    // -- `sha256_hex`'s own `"sha256:<hex>"` form is this project's
    // convention (`Certificate::source_hash` etc.), not in-toto's, so
    // the prefix is stripped here rather than baked into a second
    // hashing convention just for this wrapper.
    let hex = source_hash.strip_prefix("sha256:").unwrap_or(source_hash);
    serde_json::json!({
        "_type": IN_TOTO_STATEMENT_TYPE,
        "subject": [ { "name": path, "digest": { "sha256": hex } } ],
        "predicateType": format!("{NIRDOSHA_PREDICATE_NAMESPACE}/{predicate_kind}"),
        "predicate": predicate,
    })
}

fn cmd_verify(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut in_toto = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--in-toto" => in_toto = true,
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha verify <file.nir> [--in-toto]");
        return ExitCode::FAILURE;
    };

    let verdict = run_verify_pipeline(&path);
    let output = if in_toto {
        let source_hash = match std::fs::read(&path) {
            Ok(bytes) => sha256_hex(&bytes),
            Err(e) => {
                eprintln!("error reading {path} to compute its subject digest: {e}");
                return ExitCode::FAILURE;
            }
        };
        let predicate = serde_json::to_value(&verdict).expect("VerifyVerdict always serializes");
        wrap_in_toto(&path, &source_hash, "verify/v1", predicate)
    } else {
        serde_json::to_value(&verdict).expect("VerifyVerdict always serializes")
    };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));
    match verdict.verdict {
        ProofVerdict::Proved => {
            eprintln!("PROVED: {path} passed every check");
            ExitCode::SUCCESS
        }
        ProofVerdict::Disproved => {
            eprintln!("DISPROVED: {path} failed verification");
            ExitCode::FAILURE
        }
        ProofVerdict::Unknown => {
            eprintln!("UNKNOWN: {path} has at least one obligation Z3 couldn't decide -- not proved, not disproved");
            ExitCode::from(2)
        }
    }
}

/// `nirdosha fix <file.nir> [--apply]` -- `nirdosha-master-plan.md`
/// Part 3, Sprint 1 ("`nirdosha fix` -- byte-offset FixPatch,
/// fixability classes (auto / assisted / manual)", parity target:
/// Kōdo). Runs the exact same gate pipeline `nirdosha verify` does
/// (`run_verify_pipeline`, shared, not duplicated) and reports it
/// unchanged, plus -- with `--apply` -- writes every `Auto`-class patch
/// to the file and re-verifies to show whether it actually worked.
/// `Assisted`/`Manual` fixes are never applied automatically, by
/// design: `Auto` is this command's own bar for "safe without a human
/// in the loop," the same bar `rustc`'s `Applicability::MachineApplicable`
/// sets for `cargo fix` (see `Applicability`'s own doc comment).
fn cmd_fix(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut apply = false;
    let mut in_toto = false;
    for a in args.by_ref() {
        match a.as_str() {
            "--apply" => apply = true,
            "--in-toto" => in_toto = true,
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha fix <file.nir> [--apply] [--in-toto]");
        return ExitCode::FAILURE;
    };

    // Captured before the pipeline runs (and before `--apply` can
    // change the file on disk) -- the in-toto `subject` digest below
    // is deliberately the file's state at the *start* of this
    // invocation, the same "before" half `FixReport` itself already
    // reports against, not whatever `--apply` may have rewritten it to
    // by the time this function returns.
    let subject_hash = std::fs::read(&path).map(|bytes| sha256_hex(&bytes)).ok();

    let before = run_verify_pipeline(&path);

    let applied = if apply {
        match write_auto_patches(&path, &before) {
            Ok(applied) => applied,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Vec::new()
    };

    let after = if apply { Some(run_verify_pipeline(&path)) } else { None };
    let effective_verdict = after.as_ref().map(|v| v.verdict).unwrap_or(before.verdict);

    let report = FixReport { before, applied, after };
    let report_value = serde_json::to_value(&report).expect("FixReport always serializes");
    let output = match (in_toto, &subject_hash) {
        (true, Some(hash)) => wrap_in_toto(&path, hash, "fix/v1", report_value),
        (true, None) => {
            eprintln!("--in-toto needs the file's own bytes for its subject digest, and {path} couldn't be read a second time to compute one");
            return ExitCode::FAILURE;
        }
        (false, _) => report_value,
    };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));

    eprintln!(
        "{} auto fix(es) applied to {path}",
        if apply { report.applied.len().to_string() } else { "0 (pass --apply to write)".to_string() }
    );
    match effective_verdict {
        ProofVerdict::Proved => ExitCode::SUCCESS,
        ProofVerdict::Disproved => ExitCode::FAILURE,
        ProofVerdict::Unknown => ExitCode::from(2),
    }
}

/// `nirdosha explain [<code>]` -- `nirdosha-master-plan.md` Part 3
/// Sprint 1's "machine-learnable error index" (parity target: Kōdo,
/// Midspiral), over `explain::REGISTRY`. JSON on stdout either way
/// (an array of every entry's `code`/`title` with no argument, one full
/// entry object with one), matching `verify`/`fix`'s own stdout-JSON +
/// stderr-summary convention rather than `rustc --explain`'s
/// plain-text-only precedent -- this project's other two diagnostic
/// commands are both machine-first, and a caller that wants to render
/// `explanation`/`wrong`/`right` as prose can do that from the JSON
/// trivially, while the reverse (scraping structure back out of prose)
/// is real work this avoids imposing on every caller.
fn cmd_explain(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next() {
        None => {
            let index: Vec<serde_json::Value> = nirdosha::explain::REGISTRY
                .iter()
                .map(|e| serde_json::json!({ "code": e.code, "title": e.title }))
                .collect();
            println!("{}", serde_json::to_string_pretty(&index).expect("explain index always serializes"));
            eprintln!("{} error code(s) -- `nirdosha explain <code>` for the full entry", index.len());
            ExitCode::SUCCESS
        }
        Some(code) => match nirdosha::explain::lookup(&code) {
            Some(entry) => {
                let value = serde_json::json!({
                    "code": entry.code,
                    "title": entry.title,
                    "explanation": entry.explanation,
                    "wrong": entry.wrong,
                    "right": entry.right,
                });
                println!("{}", serde_json::to_string_pretty(&value).expect("explain entry always serializes"));
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("no such error code `{code}` -- run `nirdosha explain` with no argument for the full index");
                ExitCode::FAILURE
            }
        },
    }
}

/// `nirdosha certify <file.nir>` -- `nirdosha-master-plan.md` Part 3
/// Sprint 1's Certificate v0. Runs the identical gate pipeline
/// `verify`/`fix` both run (`run_verify_pipeline`) and wraps it in a
/// `Certificate` instead of the full `VerifyVerdict` JSON. Issues a
/// certificate for *every* verdict, `DISPROVED` included -- an honest
/// "this code is proven wrong, here is the conclusive evidence" is a
/// real, useful attestation (an audit trail, the same reason a court
/// record documents an acquittal and a conviction alike), not
/// something withheld until the code passes. Exit code mirrors
/// `verify`'s own three-way split (`0`/`1`/`2` for
/// `PROVED`/`DISPROVED`/`UNKNOWN`) so CI gating on `certify` behaves
/// identically to gating on `verify` directly.
fn cmd_certify(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut sign_key_path: Option<String> = None;
    let mut in_toto = false;
    let mut isolation_log_path: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sign" => {
                sign_key_path = args.next();
                if sign_key_path.is_none() {
                    eprintln!("--sign needs a private-key path -- usage: nirdosha certify <file.nir> [--sign <key.pk8>] [--isolation-log <ops.json>] [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            "--isolation-log" => {
                isolation_log_path = args.next();
                if isolation_log_path.is_none() {
                    eprintln!("--isolation-log needs a path -- usage: nirdosha certify <file.nir> [--sign <key.pk8>] [--isolation-log <ops.json>] [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            "--in-toto" => in_toto = true,
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha certify <file.nir> [--sign <key.pk8>] [--isolation-log <ops.json>] [--in-toto]");
        return ExitCode::FAILURE;
    };
    let source_bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pipeline = run_verify_pipeline(&path);
    let verdict = pipeline.verdict;
    let mut certificate = build_certificate(&source_bytes, pipeline);
    // `Certificate::isolation_violations`'s own doc comment: this is
    // what makes a certificate "re-attestable over time" real, not just
    // a schema field -- a caller holding a saved operation-history log
    // (`nirdosha_isolation_core::Op`, JSON array) can re-issue the
    // certificate with real, observed violations attached. Exactly the
    // same log format/check `nirdosha check-isolation` runs standalone
    // (see that command for why there's no *live* log-producing
    // mechanism yet -- a real, disclosed, separate gap).
    if let Some(log_path) = &isolation_log_path {
        match load_and_check_isolation_log(log_path) {
            Ok(anomalies) => certificate.isolation_violations = isolation_violations_from_anomalies(&anomalies),
            Err(e) => {
                eprintln!("error reading --isolation-log {log_path}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let evidence_tier = certificate.evidence_tier.clone();
    let source_hash = certificate.source_hash.clone();

    let certificate_value = match sign_key_path {
        None => serde_json::to_value(&certificate).expect("Certificate always serializes"),
        Some(key_path) => match sign_certificate(&certificate, &key_path) {
            Ok(signed) => serde_json::to_value(&signed).expect("SignedCertificate always serializes"),
            Err(e) => {
                eprintln!("signing failed: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let output = if in_toto { wrap_in_toto(&path, &source_hash, "certificate/v1", certificate_value) } else { certificate_value };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));

    match verdict {
        ProofVerdict::Proved => {
            eprintln!("PROVED: certificate issued for {path}, evidence_tier={evidence_tier}");
            ExitCode::SUCCESS
        }
        ProofVerdict::Disproved => {
            eprintln!("DISPROVED: certificate issued for {path} recording the failure, evidence_tier={evidence_tier}");
            ExitCode::FAILURE
        }
        ProofVerdict::Unknown => {
            eprintln!("UNKNOWN: certificate issued for {path}, evidence_tier={evidence_tier}");
            ExitCode::from(2)
        }
    }
}

/// Parses a saved operation-history log (a JSON array of
/// `nirdosha_isolation_core::Op` -- `{"txn":str,"resource":str,
/// "kind":"read"|"write","seq":u64}` per element) and runs the real
/// detector over it. Shared by `cmd_certify`'s `--isolation-log` and
/// `cmd_check_isolation` below, so both attach/report the identical
/// verdict for the identical log format -- one implementation, not two
/// that could quietly drift apart.
fn load_and_check_isolation_log(path: &str) -> Result<Vec<nirdosha_isolation_core::Anomaly>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let ops: Vec<nirdosha_isolation_core::Op> = serde_json::from_slice(&bytes).map_err(|e| format!("not a valid ops-log JSON array: {e}"))?;
    let mut checker = nirdosha_isolation_core::Checker::new();
    checker.load(ops);
    Ok(checker.check())
}

/// `nirdosha check-isolation <ops.json>` -- the CLI enforcement surface
/// `crates/runtime-kernels/src/kernel/isolation_check.rs`'s own module
/// doc has named as a real gap since RFC 0016 Phase 2 landed: the
/// checker was detection-only, wired into the live `db`/`transact`
/// path with no way to run it over a saved log on demand. This is that
/// command -- same detector (`nirdosha-isolation-core`, shared
/// verbatim with the live path, not a reimplementation), a real
/// 3-valued exit code matching `verify`'s own convention.
///
/// **The capture half is real now too (2026-09-15), closing what used
/// to be this doc comment's own disclosed gap**: a live compiled
/// `.nir` process can produce an ops-log for this command to check --
/// `runtime-kernels`' `isolation_check::maybe_start_capture` (opt-in
/// via `NIRDOSHA_ISOLATION_LOG_PATH`, same env-var-gated posture
/// `nfr.rs`'s own `NIRDOSHA_OBSERVABILITY_URL` already uses) writes the
/// running process's own `Checker::ops()` window to a file in this
/// exact `Vec<Op>` JSON shape, periodically, atomically. A saved log
/// can still come from any other caller's own tooling too (a test
/// harness, `certify --isolation-log`'s existing callers) -- this adds
/// a real producer, it doesn't require one.
///
/// **`--teach <hint>`**: Phase 4 item 2, "feed real incidents into
/// `hint_cache`" -- on a real anomaly found, records `<hint>` as this
/// process's `"transact_isolation_anomaly"` runtime lesson
/// (`hint_cache::RuntimeLessons`, `--teach`'s own doc comment there for
/// why this is a deliberate, human-confirmed act, not automatic
/// synthesis). `hi_llm.rs::generate_from_task_prompt` consults it
/// proactively for a future task whose own prompt looks `transact`-
/// shaped -- see that function's own doc comment for the real,
/// disclosed matching rule.
fn cmd_check_isolation(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut in_toto = false;
    let mut teach: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--in-toto" => in_toto = true,
            "--teach" => {
                teach = args.next();
                if teach.is_none() {
                    eprintln!("--teach needs a hint string -- usage: nirdosha check-isolation <ops.json> [--teach <hint>] [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha check-isolation <ops.json> [--teach <hint>] [--in-toto]");
        return ExitCode::FAILURE;
    };
    let anomalies = match load_and_check_isolation_log(&path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let clean = anomalies.is_empty();
    // `--teach`'s runtime-lessons hint cache (`hint_cache.rs`) moved to
    // `crates/nirdosha-hi` 2026-09-16 along with the rest of `hi` -- a
    // native incident can no longer directly seed `hi`'s corrective-
    // hint cache in-process (they're separate binaries now). The flag
    // still parses (so an existing script's invocation doesn't break),
    // it just can't record anywhere anymore.
    if !clean {
        if teach.is_some() {
            eprintln!("--teach: the runtime-lessons hint cache now lives in the standalone nirdosha-hi crate -- nothing recorded here");
        }
    }
    let verdict = if clean { "clean" } else { "anomaly_found" };
    let predicate = serde_json::json!({
        "verdict": verdict,
        "evidence_tier": nirdosha::verify_pipeline::IsolationViolation::EVIDENCE_TIER,
        "anomalies": isolation_violations_from_anomalies(&anomalies),
        "taught": false,
    });
    let output = if in_toto {
        let source_hash = match std::fs::read(&path) {
            Ok(bytes) => sha256_hex(&bytes),
            Err(e) => {
                eprintln!("error reading {path} to compute its subject digest: {e}");
                return ExitCode::FAILURE;
            }
        };
        wrap_in_toto(&path, &source_hash, "check-isolation/v1", predicate)
    } else {
        predicate
    };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));
    if clean {
        eprintln!("CLEAN: {path} -- no isolation anomaly detected in this log");
        ExitCode::SUCCESS
    } else {
        eprintln!("ANOMALY_FOUND: {path} -- {} anomal{} detected (evidence_tier=monitored: conclusive that these happened, not proof no others exist)", anomalies.len(), if anomalies.len() == 1 { "y" } else { "ies" });
        ExitCode::FAILURE
    }
}

/// `nirdosha check-drift <file.nir> <escalations.json>` -- Phase 4 item
/// 1 of `docs/research/2026-09-pending-verification-differentiation-
/// work.md`: "detect when a compile-time `nfr(...)` assumption diverges
/// from what the APM kernel actually observes in production, and
/// trigger re-verification instead of just firing an alert." Built for
/// real 2026-09-15.
///
/// Compares `<file.nir>`'s own declared `nfr(...)` commitments
/// (`nfr_commitments_from_program`) against a real, saved
/// `NIRDOSHA_OBSERVABILITY_URL` escalation log -- `<escalations.json>`
/// is a JSON array of `nfr.rs`'s own `send_escalation` wire shape
/// (`mcp_tools::NfrEscalation`), the *identical* format that channel
/// already POSTs, not a new one invented for this command. If any
/// commitment drifted (`mcp_tools::nfr_drift`), this **actually
/// re-verifies** `<file.nir>` (`run_verify_pipeline`, the same pipeline
/// `verify`/`certify` run) and includes that fresh verdict in the
/// output -- the "trigger re-verification instead of just firing an
/// alert" the RFC asks for, not a passive notification with nothing
/// downstream of it.
///
/// **`--teach <hint>`**: same Phase 4 item 2 mechanism `check-
/// isolation --teach` uses, recording `<hint>` as this process's
/// `"nfr_drift"` runtime lesson on a real finding.
fn cmd_check_drift(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut log_path: Option<String> = None;
    let mut in_toto = false;
    let mut teach: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--in-toto" => in_toto = true,
            "--teach" => {
                teach = args.next();
                if teach.is_none() {
                    eprintln!("--teach needs a hint string -- usage: nirdosha check-drift <file.nir> <escalations.json> [--teach <hint>] [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            other if path.is_none() => path = Some(other.to_string()),
            other => log_path = Some(other.to_string()),
        }
    }
    let (Some(path), Some(log_path)) = (path, log_path) else {
        eprintln!("usage: nirdosha check-drift <file.nir> <escalations.json> [--teach <hint>] [--in-toto]");
        return ExitCode::FAILURE;
    };
    let source_bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (program, _src) = match nirdosha::loader::load_program(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error loading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let escalations: Vec<NfrEscalation> = match std::fs::read(&log_path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("not a valid escalations JSON array: {e}"))) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error reading {log_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let commitments = nfr_commitments_from_program(&program);
    let findings = nfr_drift(&commitments, &escalations);
    let drifted = !findings.is_empty();

    // See `cmd_check_isolation`'s own `--teach` comment: the hint cache
    // this used to record into moved to `crates/nirdosha-hi`.
    if drifted && teach.is_some() {
        eprintln!("--teach: the runtime-lessons hint cache now lives in the standalone nirdosha-hi crate -- nothing recorded here");
    }

    // The actual "trigger re-verification" half -- only run when there
    // is real drift to react to; a clean comparison has nothing to
    // re-verify against that `verify`/`certify` wouldn't already cover
    // on their own.
    let reverification = if drifted { Some(run_verify_pipeline(&path)) } else { None };

    let verdict = if drifted { "drift_found" } else { "no_drift" };
    let predicate = serde_json::json!({
        "verdict": verdict,
        "declared_commitments": commitments.len(),
        "findings": findings,
        "reverification": reverification,
        "taught": teach.is_some() && drifted,
    });
    let output = if in_toto {
        wrap_in_toto(&path, &sha256_hex(&source_bytes), "check-drift/v1", predicate)
    } else {
        predicate
    };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));
    if drifted {
        eprintln!("DRIFT_FOUND: {path} -- {} declared nfr(...) commitment(s) diverged from observed reality in {log_path}; re-verification ran, see \"reverification\" above", findings.len());
        ExitCode::FAILURE
    } else {
        eprintln!("NO_DRIFT: {path} -- every declared nfr(...) commitment checked against {log_path} holds");
        ExitCode::SUCCESS
    }
}

/// `nirdosha check-guarantees <file.nir> [--manifest <path>]` -- RFC
/// 0017's on-demand check, `cmd_build`'s own hard-error enforcement
/// pulled out into its own command for a caller that wants the verdict
/// without actually building (mirrors `verify` existing alongside
/// `build`'s own typecheck/ownership gate for the identical reason).
/// `--manifest` defaults to `guarantee_manifest::manifest_path_for`'s
/// convention (`<file.nir>.guarantees.json`) when omitted; no manifest
/// found either way is reported as clean, never an error -- RFC 0017's
/// own "additive only" compatibility rule.
fn cmd_check_guarantees(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut manifest_path: Option<String> = None;
    let mut in_toto = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--in-toto" => in_toto = true,
            "--manifest" => {
                manifest_path = args.next();
                if manifest_path.is_none() {
                    eprintln!("--manifest needs a path -- usage: nirdosha check-guarantees <file.nir> [--manifest <path>] [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha check-guarantees <file.nir> [--manifest <path>] [--in-toto]");
        return ExitCode::FAILURE;
    };
    let (program, source) = match typecheck_and_own(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    let manifest = match &manifest_path {
        Some(explicit) => match std::fs::read(explicit) {
            Ok(bytes) => match serde_json::from_slice::<nirdosha::guarantee_manifest::GuaranteeManifest>(&bytes) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("{explicit} is not a valid guarantee manifest: {e}");
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("error reading {explicit}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => match nirdosha::guarantee_manifest::load(std::path::Path::new(&path)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let violations = match &manifest {
        Some(m) => nirdosha::guarantee_manifest::check(&program, &registry, m),
        None => Vec::new(),
    };
    let clean = violations.is_empty();
    let verdict = if manifest.is_none() { "no_manifest" } else if clean { "clean" } else { "violation_found" };
    let predicate = serde_json::json!({
        "verdict": verdict,
        "violations": violations.iter().map(|v| v.describe()).collect::<Vec<_>>(),
    });
    let output = if in_toto { wrap_in_toto(&path, &sha256_hex(source.as_bytes()), "check-guarantees/v1", predicate) } else { predicate };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));
    if clean {
        eprintln!(
            "{}",
            if manifest.is_none() { format!("NO_MANIFEST: {path} has no guarantee manifest -- nothing to check (RFC 0017 is additive-only)") } else { format!("CLEAN: {path} satisfies its guarantee manifest") }
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("VIOLATION_FOUND: {path} -- {} guarantee-manifest violation(s)", violations.len());
        ExitCode::FAILURE
    }
}

/// `nirdosha verify-binary <bundle.guarantees.json> --against <policy.json>`
/// -- RFC 0017 §6, the dual of `verify`: checks an already-emitted
/// guarantee bundle (`nirdosha build`'s own `<out>.guarantees.json`)
/// against an operator-supplied policy (the identical manifest JSON
/// shape `check-guarantees --manifest` reads) **with no recompilation**
/// -- the real "give me the generated code and I'll tell you whether it
/// satisfies these guarantees" answer for the binary side, not just the
/// source side `verify`/`certify` already covered.
fn cmd_verify_binary(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut bundle_path: Option<String> = None;
    let mut policy_path: Option<String> = None;
    let mut in_toto = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--in-toto" => in_toto = true,
            "--against" => {
                policy_path = args.next();
                if policy_path.is_none() {
                    eprintln!("--against needs a policy path -- usage: nirdosha verify-binary <bundle.guarantees.json> --against <policy.json> [--in-toto]");
                    return ExitCode::FAILURE;
                }
            }
            other => bundle_path = Some(other.to_string()),
        }
    }
    let (Some(bundle_path), Some(policy_path)) = (bundle_path, policy_path) else {
        eprintln!("usage: nirdosha verify-binary <bundle.guarantees.json> --against <policy.json> [--in-toto]");
        return ExitCode::FAILURE;
    };
    let bundle_bytes = match std::fs::read(&bundle_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {bundle_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bundle: serde_json::Value = match serde_json::from_slice(&bundle_bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{bundle_path} is not valid JSON: {e}");
            return ExitCode::FAILURE;
        }
    };
    let policy: nirdosha::guarantee_manifest::GuaranteeManifest = match std::fs::read(&policy_path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("not a valid guarantee manifest: {e}"))) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error reading {policy_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let violations = nirdosha::guarantee_manifest::check_bundle_against_policy(&bundle, &policy);
    let clean = violations.is_empty();
    let predicate = serde_json::json!({
        "verdict": if clean { "satisfies_policy" } else { "violation_found" },
        "bundle": bundle_path,
        "policy": policy_path,
        "violations": violations.iter().map(|v| v.describe()).collect::<Vec<_>>(),
    });
    let output = if in_toto { wrap_in_toto(&bundle_path, &sha256_hex(&bundle_bytes), "verify-binary/v1", predicate) } else { predicate };
    println!("{}", serde_json::to_string_pretty(&output).expect("this JSON value always serializes"));
    if clean {
        eprintln!("SATISFIES_POLICY: {bundle_path} satisfies {policy_path}");
        ExitCode::SUCCESS
    } else {
        eprintln!("VIOLATION_FOUND: {bundle_path} -- {} violation(s) against {policy_path}", violations.len());
        ExitCode::FAILURE
    }
}

/// `nirdosha keygen [-o <path>]` -- generates a real Ed25519 keypair
/// (`nirdosha_audit::crypto_backend::rand::SystemRandom`, the OS CSPRNG, not a fixed/test seed)
/// for `nirdosha certify --sign`. Writes the private key as raw
/// PKCS#8 DER to `<path>` (default `nirdosha_signing_key.pk8`) --
/// **keep this file secret**, anyone holding it can sign certificates
/// your key will be trusted for -- and the base64 public key to
/// `<path>.pub`, the thing you actually distribute/pin.
fn cmd_keygen(mut args: impl Iterator<Item = String>) -> ExitCode {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use nirdosha_audit::crypto_backend::signature::KeyPair;

    let mut out: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => {
                out = args.next();
                if out.is_none() {
                    eprintln!("-o needs a path -- usage: nirdosha keygen [-o key.pk8]");
                    return ExitCode::FAILURE;
                }
            }
            other => {
                eprintln!("unknown argument `{other}` -- usage: nirdosha keygen [-o key.pk8]");
                return ExitCode::FAILURE;
            }
        }
    }
    let out_path = out.unwrap_or_else(|| "nirdosha_signing_key.pk8".to_string());

    let rng = nirdosha_audit::crypto_backend::rand::SystemRandom::new();
    let pkcs8 = match nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::generate_pkcs8(&rng) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("key generation failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, pkcs8.as_ref()) {
        eprintln!("error writing {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    let keypair = nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("a key this function just generated always parses");
    let public_key_b64 = BASE64_STANDARD.encode(keypair.public_key().as_ref());
    let pub_path = format!("{out_path}.pub");
    if let Err(e) = std::fs::write(&pub_path, format!("{public_key_b64}\n")) {
        eprintln!("error writing {pub_path}: {e}");
        return ExitCode::FAILURE;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "private_key_path": out_path,
            "public_key_path": pub_path,
            "public_key": public_key_b64,
            "algorithm": "ed25519",
        }))
        .expect("this JSON value always serializes")
    );
    eprintln!("wrote {out_path} (private -- keep secret) and {pub_path} (public -- distribute/pin this)");
    ExitCode::SUCCESS
}

/// `nirdosha verify-certificate <certificate.json>` -- the other half
/// of `nirdosha certify --sign`/`nirdosha keygen`'s round trip: checks
/// a signed certificate's Ed25519 signature against its own embedded
/// `public_key`. Does **not** decide whether to *trust* that key
/// (`SignedCertificate`'s own doc comment: pinning which public keys
/// are acceptable is the verifier's operational policy, not this
/// command's job) -- it answers exactly one question, "is this
/// signature valid for this certificate and this embedded key,"
/// honestly, as its own field (`"valid": true`/`false`), never folded
/// into a generic success/failure exit code a caller might
/// misconstrue as "and therefore this key is trustworthy."
fn cmd_verify_certificate(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(path) = args.next() else {
        eprintln!("usage: nirdosha verify-certificate <certificate.json>");
        return ExitCode::FAILURE;
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let full: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{path} is not valid JSON: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (Some(signature_b64), Some(public_key_b64)) = (full.get("signature").and_then(|v| v.as_str()), full.get("public_key").and_then(|v| v.as_str())) else {
        eprintln!("{path} is not a signed certificate -- missing `signature`/`public_key` fields (did you mean to run `nirdosha certify --sign`?)");
        return ExitCode::FAILURE;
    };
    let certificate: Certificate = match serde_json::from_value(full.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{path}'s own certificate fields don't match Certificate v0's shape: {e}");
            return ExitCode::FAILURE;
        }
    };
    let canonical = serde_json::to_vec(&certificate).expect("Certificate always serializes");

    let valid = match nirdosha_audit::signing::verify_bytes(&canonical, public_key_b64, signature_b64) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{path}'s signature/public_key: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({ "valid": valid, "public_key": public_key_b64, "source_hash": certificate.source_hash }))
            .expect("this JSON value always serializes")
    );
    if valid {
        eprintln!("VALID: {path}'s signature matches its embedded public key");
        ExitCode::SUCCESS
    } else {
        eprintln!("INVALID: {path}'s signature does not match its embedded public key (or the certificate was modified after signing)");
        ExitCode::FAILURE
    }
}

/// `nirdosha equivalence <file.nir> <fn_a> <fn_b>` --
/// `nirdosha-master-plan.md` Part 3 Dec 2026's "Equivalence checking --
/// 'prove the agent's refactor is behavior-identical'" (parity target:
/// Velvet, Imandra). Loads and typechecks `file.nir` (equivalence
/// checking needs a well-typed program the same way `validate`
/// contract-checking does -- an ill-typed function has no meaningful
/// semantics to compare), then hands `fn_a`/`fn_b` to
/// `contract_check::check_equivalence`. JSON on stdout, matching
/// `verify`/`fix`/`certify`'s own convention; exit `0` for
/// `Equivalent`, `1` for a real, concrete `Different` counterexample,
/// `2` for `Unsupported` (today's real scope boundary, not a claim
/// either function actually differs) -- the same three-valued shape
/// `verify`'s own `PROVED`/`DISPROVED`/`UNKNOWN` already established,
/// for the identical reason: "couldn't check" must never collapse into
/// either a false pass or a false fail.
fn cmd_equivalence(mut args: impl Iterator<Item = String>) -> ExitCode {
    let (Some(path), Some(fn_a), Some(fn_b)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: nirdosha equivalence <file.nir> <fn_a> <fn_b>");
        return ExitCode::FAILURE;
    };
    let (program, _src) = match nirdosha::loader::load_program(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(errs) = nirdosha::typeck::typecheck_optional_main(&program) {
        for e in &errs {
            eprintln!("{e}");
        }
        return ExitCode::FAILURE;
    }

    let result = nirdosha::contract_check::check_equivalence(&program, &fn_a, &fn_b);
    let (value, exit) = match &result {
        nirdosha::contract_check::EquivalenceResult::Equivalent => (serde_json::json!({ "result": "EQUIVALENT", "fn_a": fn_a, "fn_b": fn_b }), ExitCode::SUCCESS),
        nirdosha::contract_check::EquivalenceResult::Different { bindings, result_a, result_b } => (
            serde_json::json!({
                "result": "DIFFERENT",
                "fn_a": fn_a,
                "fn_b": fn_b,
                "counterexample": bindings.iter().map(|(k, v)| (k.clone(), *v)).collect::<std::collections::BTreeMap<_, _>>(),
                "result_a": result_a,
                "result_b": result_b,
            }),
            ExitCode::FAILURE,
        ),
        nirdosha::contract_check::EquivalenceResult::Unsupported(msg) => (serde_json::json!({ "result": "UNSUPPORTED", "fn_a": fn_a, "fn_b": fn_b, "detail": msg }), ExitCode::from(2)),
        nirdosha::contract_check::EquivalenceResult::EngineLimit => (serde_json::json!({ "result": "ENGINE_LIMIT", "fn_a": fn_a, "fn_b": fn_b, "detail": "the solver fuel ran out before equivalence could be decided -- fail-closed per RFC 0016, not a verdict about the functions" }), ExitCode::from(2)),
    };
    println!("{}", serde_json::to_string_pretty(&value).expect("this JSON value always serializes"));
    match result {
        nirdosha::contract_check::EquivalenceResult::Equivalent => eprintln!("EQUIVALENT: `{fn_a}` and `{fn_b}` produce the same result for every input Z3 could check"),
        nirdosha::contract_check::EquivalenceResult::Different { .. } => eprintln!("DIFFERENT: `{fn_a}` and `{fn_b}` diverge -- see the counterexample above"),
        nirdosha::contract_check::EquivalenceResult::Unsupported(ref msg) => eprintln!("UNSUPPORTED: {msg}"),
        nirdosha::contract_check::EquivalenceResult::EngineLimit => eprintln!("ENGINE_LIMIT: the solver fuel ran out before deciding -- raise NIRDOSHA-equivalent fuel via `set_proof_fuel_rlimit` or simplify the functions"),
    }
    exit
}

/// A trusted identity's own record in a trust config -- `name` paired
/// with the Ed25519 public key that name is allowed to sign
/// attestations with. Which list it's registered in
/// (`TrustConfig::known_agents` vs `human_reviewers`) is itself part
/// of the trust claim: `nirdosha audit` checks an attestation's
/// claimed `role` against the *matching* list, so an agent identity
/// can never satisfy a "human-reviewed" gate by forging its own role
/// string -- the role comes from which list the name is actually
/// registered in, not from the attestation's own say-so.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct TrustedIdentity {
    name: String,
    public_key: String,
}

/// `nirdosha-master-plan.md` Part 3 Dec 2026's "known_agents/
/// human_reviewers trust config" -- the registry `nirdosha attest`
/// checks a reviewer name against before signing, and `nirdosha audit`
/// checks an attestation's claimed reviewer+role against before
/// trusting it.
#[derive(serde::Serialize, serde::Deserialize)]
struct TrustConfig {
    trust_config_version: String,
    known_agents: Vec<TrustedIdentity>,
    human_reviewers: Vec<TrustedIdentity>,
}

/// The part of an `Attestation` that gets signed -- kept as its own
/// struct (not just "`Attestation` minus its signature field") for the
/// same reason `Certificate`/`SignedCertificate` are two structs: the
/// signed bytes must be exactly reproducible by both the signer and a
/// later verifier parsing the full `Attestation` back and discarding
/// its signature fields, and a single struct that *also* carries
/// `signature`/`signature_algorithm` would make "sign everything except
/// these two fields" a fragile, easy-to-get-wrong manual exclusion
/// instead of "sign this struct, full stop."
#[derive(serde::Serialize, serde::Deserialize)]
struct AttestationCore {
    attestation_version: String,
    file_hash: String,
    reviewer: String,
    role: String,
    note: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Attestation {
    #[serde(flatten)]
    core: AttestationCore,
    signature_algorithm: String,
    signature: String,
}

/// `nirdosha-master-plan.md` Part 3 Dec 2026's "reviewer-forgery
/// prevention... an LLM can't fake `@reviewed_by`." Implemented as a
/// sidecar attestation over a file's content hash, signed with a real
/// Ed25519 key registered in a `TrustConfig` -- deliberately **not** a
/// new `.nir` source annotation (no `@reviewed_by(...)` token in the
/// grammar): `docs/GRAMMAR.md`'s own stated discipline treats every new
/// keyword as a breaking addition pre-1.0, worth an RFC, not something
/// to land as a side effect of a trust-reporting feature. An
/// attestation is exactly as strong as its signature: forging one
/// without the named reviewer's private key is exactly as hard as
/// forging any other Ed25519 signature, the same real cryptographic
/// guarantee `nirdosha certify --sign` already provides for a
/// certificate.
///
/// `nirdosha attest <file.nir> --reviewer <name> --role agent|human
/// --key <key.pk8> --trust-config <config.json> [--note <text>]
/// [-o <attestation.json>]` -- refuses to sign for a name that isn't
/// registered under the matching list in `--trust-config` (an
/// unregistered reviewer producing a "valid-looking" attestation
/// nobody would trust is a real usability trap this rejects up front,
/// not just something `audit` would catch later).
fn cmd_attest(mut args: impl Iterator<Item = String>) -> ExitCode {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    let mut path: Option<String> = None;
    let mut reviewer: Option<String> = None;
    let mut role: Option<String> = None;
    let mut key_path: Option<String> = None;
    let mut trust_config_path: Option<String> = None;
    let mut note: Option<String> = None;
    let mut out: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--reviewer" => reviewer = args.next(),
            "--role" => role = args.next(),
            "--key" => key_path = args.next(),
            "--trust-config" => trust_config_path = args.next(),
            "--note" => note = args.next(),
            "-o" => out = args.next(),
            other => path = Some(other.to_string()),
        }
    }
    let usage = "usage: nirdosha attest <file.nir> --reviewer <name> --role agent|human --key <key.pk8> --trust-config <config.json> [--note <text>] [-o <attestation.json>]";
    let (Some(path), Some(reviewer), Some(role), Some(key_path), Some(trust_config_path)) = (path, reviewer, role, key_path, trust_config_path) else {
        eprintln!("{usage}");
        return ExitCode::FAILURE;
    };
    let role = match role.as_str() {
        "agent" => "known_agent".to_string(),
        "human" => "human_reviewer".to_string(),
        other => {
            eprintln!("--role must be `agent` or `human`, got `{other}`");
            return ExitCode::FAILURE;
        }
    };

    let trust_config: TrustConfig = match std::fs::read_to_string(&trust_config_path).and_then(|s| serde_json::from_str(&s).map_err(std::io::Error::other)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error reading trust config {trust_config_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let registry = if role == "known_agent" { &trust_config.known_agents } else { &trust_config.human_reviewers };
    if !registry.iter().any(|identity| identity.name == reviewer) {
        eprintln!("`{reviewer}` is not registered as a `{role}` in {trust_config_path} -- add them first, or check --role");
        return ExitCode::FAILURE;
    }

    let source_bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pkcs8 = match std::fs::read(&key_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading private key {key_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let keypair = match nirdosha_audit::crypto_backend::signature::Ed25519KeyPair::from_pkcs8(&pkcs8) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{key_path} is not a valid Ed25519 PKCS#8 private key: {e}");
            return ExitCode::FAILURE;
        }
    };

    let core = AttestationCore { attestation_version: "0".to_string(), file_hash: sha256_hex(&source_bytes), reviewer, role, note };
    let canonical = serde_json::to_vec(&core).expect("AttestationCore always serializes");
    let signature = keypair.sign(&canonical);
    let attestation = Attestation { core: serde_json::from_slice(&canonical).expect("re-parsing what was just serialized cannot fail"), signature_algorithm: "ed25519".to_string(), signature: BASE64_STANDARD.encode(signature.as_ref()) };

    let printed = serde_json::to_string_pretty(&attestation).expect("Attestation always serializes");
    if let Some(out_path) = &out {
        if let Err(e) = std::fs::write(out_path, &printed) {
            eprintln!("error writing {out_path}: {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("{printed}");
    eprintln!("attestation for {path} signed by `{}` ({}){}", attestation.core.reviewer, attestation.core.role, out.map(|p| format!(" -- wrote {p}")).unwrap_or_default());
    ExitCode::SUCCESS
}

/// Verifies one `Attestation` against a `TrustConfig` and the file's
/// *current* real content hash -- four honest outcomes, never
/// collapsed into a single pass/fail:
/// - `UNTRUSTED_REVIEWER`: the claimed name isn't registered under the
///   matching list at all -- checked *before* touching the signature,
///   since an unregistered name can never be trusted regardless of
///   whether it signed correctly with *some* key.
/// - `FORGED_OR_TAMPERED`: the name is registered, but the signature
///   doesn't verify against that registered public key -- exactly the
///   case this whole feature exists to catch (`docs/PUBLIC_ROADMAP.md`'s
///   "an LLM can't fake `@reviewed_by`").
/// - `STALE`: the signature is genuinely valid, but `file_hash` no
///   longer matches the file's real current content -- the code moved
///   on since this review, and the attestation doesn't cover today's
///   version.
/// - `CURRENT`: valid signature, registered reviewer, matching hash --
///   this file, as it exists right now, really was reviewed by this
///   name.
fn audit_one_attestation(attestation: &Attestation, trust_config: &TrustConfig, current_file_hash: &str) -> serde_json::Value {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    let registry = if attestation.core.role == "known_agent" { &trust_config.known_agents } else { &trust_config.human_reviewers };
    let Some(identity) = registry.iter().find(|i| i.name == attestation.core.reviewer) else {
        return serde_json::json!({ "reviewer": attestation.core.reviewer, "role": attestation.core.role, "status": "UNTRUSTED_REVIEWER" });
    };

    let canonical = serde_json::to_vec(&attestation.core).expect("AttestationCore always serializes");
    let valid = (|| -> Option<bool> {
        let public_key_bytes = BASE64_STANDARD.decode(&identity.public_key).ok()?;
        let signature_bytes = BASE64_STANDARD.decode(&attestation.signature).ok()?;
        let public_key = nirdosha_audit::crypto_backend::signature::UnparsedPublicKey::new(&nirdosha_audit::crypto_backend::signature::ED25519, &public_key_bytes);
        Some(public_key.verify(&canonical, &signature_bytes).is_ok())
    })()
    .unwrap_or(false);

    if !valid {
        return serde_json::json!({ "reviewer": attestation.core.reviewer, "role": attestation.core.role, "status": "FORGED_OR_TAMPERED" });
    }
    if attestation.core.file_hash != current_file_hash {
        return serde_json::json!({ "reviewer": attestation.core.reviewer, "role": attestation.core.role, "status": "STALE", "attested_hash": attestation.core.file_hash, "current_hash": current_file_hash });
    }
    serde_json::json!({ "reviewer": attestation.core.reviewer, "role": attestation.core.role, "status": "CURRENT", "note": attestation.core.note })
}

/// `nirdosha audit <file.nir> --trust-config <config.json>
/// [--attestation <attestation.json>]...` --
/// `nirdosha-master-plan.md` Part 3 Dec 2026's "confidence/trust
/// propagation": combines `run_verify_pipeline`'s own formal verdict
/// with every attestation's real, checked status
/// (`audit_one_attestation`) into one report, rather than reporting
/// them side by side and leaving the reader to combine them. This is a
/// deliberately small first version of what the master plan's later
/// (Q1 2027) `nirdosha audit` item calls a "consolidated trust
/// report" -- named here as the same command because it already does
/// that job for the two signals that exist today (formal proof,
/// signed review); a later pass adds more inputs to the same report,
/// not a competing command.
fn cmd_audit(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut path: Option<String> = None;
    let mut trust_config_path: Option<String> = None;
    let mut attestation_paths: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--trust-config" => trust_config_path = args.next(),
            "--attestation" => {
                if let Some(p) = args.next() {
                    attestation_paths.push(p);
                }
            }
            other => path = Some(other.to_string()),
        }
    }
    let (Some(path), Some(trust_config_path)) = (path, trust_config_path) else {
        eprintln!("usage: nirdosha audit <file.nir> --trust-config <config.json> [--attestation <attestation.json>]...");
        return ExitCode::FAILURE;
    };
    let trust_config: TrustConfig = match std::fs::read_to_string(&trust_config_path).and_then(|s| serde_json::from_str(&s).map_err(std::io::Error::other)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error reading trust config {trust_config_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let source_bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let current_hash = sha256_hex(&source_bytes);

    let mut attestation_reports = Vec::new();
    for attestation_path in &attestation_paths {
        let attestation: Attestation = match std::fs::read_to_string(attestation_path).and_then(|s| serde_json::from_str(&s).map_err(std::io::Error::other)) {
            Ok(a) => a,
            Err(e) => {
                attestation_reports.push(serde_json::json!({ "attestation_file": attestation_path, "status": "UNREADABLE", "detail": e.to_string() }));
                continue;
            }
        };
        let mut report = audit_one_attestation(&attestation, &trust_config, &current_hash);
        report["attestation_file"] = serde_json::json!(attestation_path);
        attestation_reports.push(report);
    }

    let verdict = run_verify_pipeline(&path);
    let formally_proved = verdict.verdict == ProofVerdict::Proved;
    let currently_reviewed = attestation_reports.iter().any(|r| r["status"] == "CURRENT");
    let human_reviewed = attestation_reports.iter().any(|r| r["status"] == "CURRENT" && r["role"] == "human_reviewer");
    let any_red_flag = attestation_reports.iter().any(|r| r["status"] == "FORGED_OR_TAMPERED" || r["status"] == "UNTRUSTED_REVIEWER");

    // A small, honest rollup -- not a numeric score (a fabricated
    // "87% confidence" would claim more precision than two boolean
    // signals actually support). Order matters: a forged/untrusted
    // attestation is worse than having none at all, so it's checked
    // first regardless of what the formal verdict says.
    let trust_summary = if any_red_flag {
        "REJECTED_ATTESTATION_PRESENT"
    } else if formally_proved && human_reviewed {
        "PROVED_AND_HUMAN_REVIEWED"
    } else if formally_proved && currently_reviewed {
        "PROVED_AND_AGENT_REVIEWED"
    } else if formally_proved {
        "PROVED_ONLY"
    } else if human_reviewed {
        "HUMAN_REVIEWED_ONLY"
    } else if currently_reviewed {
        "AGENT_REVIEWED_ONLY"
    } else {
        "UNVERIFIED"
    };

    let report = serde_json::json!({
        "file": path,
        "verify_verdict": verdict.verdict,
        "attestations": attestation_reports,
        "trust_summary": trust_summary,
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("this JSON value always serializes"));
    eprintln!("{trust_summary}: {path}");
    if any_red_flag {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `nirdosha suggest-contracts <file.nir> <fn_name>` --
/// `nirdosha-master-plan.md` Part 3 Q1 2027's "LLM-assisted contract
/// inference" (parity target: Kōdo's `kodoc annotate --ai`, Certora
/// AutoProver). Asks a real LLM (`hi_llm::suggest_contract`, same
/// activation/client plumbing `nirdosha hi`'s Generate mode and
/// `crates/bench` both use) for a `validate` block, then -- this is
/// the part that matters -- **actually checks it**: splices the
/// suggestion into a scratch copy of the file and runs the identical
/// `run_verify_pipeline` every other command here uses, reporting the
/// real verdict alongside the suggested text. A suggestion is never
/// presented as trustworthy on the strength of an LLM having produced
/// it; `docs/PUBLIC_ROADMAP.md`'s Certora citation (AutoProver
/// "independently derived one invariant, missed a human-written one"
/// on Aave v4) is the concrete reason this command's own doc comment
/// insists on that distinction rather than assuming it's obvious.
///
/// Refuses up front if `fn_name` already has a `validate` block --
/// a function has exactly one, and silently overwriting an existing,
/// possibly hand-written contract is a worse failure mode than asking
/// the caller to remove it first.
fn cmd_suggest_contracts(mut args: impl Iterator<Item = String>) -> ExitCode {
    let (Some(path), Some(fn_name)) = (args.next(), args.next()) else {
        eprintln!("usage: nirdosha suggest-contracts <file.nir> <fn_name>");
        return ExitCode::FAILURE;
    };

    let (program, _src) = match nirdosha::loader::load_program(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    if !program.fns.iter().any(|f| f.name == fn_name) {
        eprintln!("no such function `{fn_name}` in {path}");
        return ExitCode::FAILURE;
    }
    if program.validates.iter().any(|v| v.fn_name == fn_name) {
        eprintln!("`{fn_name}` already has a `validate` block -- remove it first if you want a fresh suggestion (never overwritten automatically)");
        return ExitCode::FAILURE;
    }

    // `nirdosha suggest-contracts`'s LLM client (`Activation`/
    // `resolve_activation`/`LlmClient`/`suggest_contract`) used to live
    // in `hi_llm.rs`, which moved to the standalone `nirdosha-hi` crate
    // 2026-09-16 along with the rest of `hi` -- this compiler binary no
    // longer depends on it. This command is temporarily unavailable
    // pending a small shared `nirdosha-llm-client` crate (the one real
    // entanglement the `hi` extraction didn't yet resolve: `suggest-
    // contracts` is the only native-only feature that ever needed an
    // LLM call). Disclosed here, not silently broken -- every check
    // above this point (file/fn exist, no pre-existing `validate`
    // block) still runs and still reports its own real failure first.
    eprintln!("nirdosha suggest-contracts: temporarily unavailable -- its LLM client moved to the standalone nirdosha-hi crate on 2026-09-16 and hasn't been re-extracted into a shared crate this binary can use yet");
    ExitCode::FAILURE
}

fn cmd_emit_ast(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(path) = args.next() else {
        eprintln!("usage: nirdosha emit-ast <file.nir>");
        return ExitCode::FAILURE;
    };
    // `loader::load_program` resolves any `use "..."` (`docs/ROADMAP.md`
    // Track F, F2 piece 3) but — like the plain lex/parse this replaces
    // — never typechecks the *entry* file itself (only an imported
    // file, before its `pub` items are merged in, which has to be
    // well-typed for the merge to be sound at all): this command's own
    // "AST of a program that doesn't yet typecheck is still legitimate
    // to inspect" contract, above, is unaffected.
    let (program, _src) = match nirdosha::loader::load_program(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    match serde_json::to_string_pretty(&program) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to serialize AST: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `nirdosha emit-ui <file.nir> [-o out.html]` — derives a self-contained,
/// Material-styled HTML/JS app from the program's `struct` declarations
/// and `list_/create_/update_/delete_/get_<struct>` naming convention
/// (`ui_gen::generate`). Unlike `emit-ast`, this needs the *typed*
/// program (`typecheck_and_own`, same gate `build`/`emit-llvm` use) —
/// screen inference reads resolved struct fields and function
/// signatures, not raw syntax.
/// `nirdosha gen-crud <plan.json> --db <literal> [-o out.nir]` — see
/// `crud_gen`'s module doc for why this exists (replaces protobox's
/// placeholder-only Python `_stub_fns` with real, compiling persistence
/// bodies, deterministically, no LLM call).
fn cmd_gen_crud(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut plan_path: Option<String> = None;
    let mut db: Option<String> = None;
    let mut out: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => db = args.next(),
            "-o" => out = args.next(),
            other => plan_path = Some(other.to_string()),
        }
    }
    let (Some(plan_path), Some(db)) = (plan_path, db) else {
        eprintln!("usage: nirdosha gen-crud <plan.json> --db <db_connect literal> [-o out.nir]");
        return ExitCode::FAILURE;
    };
    let text = match std::fs::read_to_string(&plan_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error reading {plan_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let plan: nirdosha::crud_gen::ScreenPlan = match serde_json::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error parsing {plan_path} as a screen plan: {e}");
            return ExitCode::FAILURE;
        }
    };
    let source = match nirdosha::crud_gen::render_plan(&plan, &db, "") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &source) {
                eprintln!("error writing {path}: {e}");
                return ExitCode::FAILURE;
            }
        }
        None => print!("{source}"),
    }
    ExitCode::SUCCESS
}

/// `--manifest-path <Cargo.toml>` resolves explicitly; absent that, a
/// `Cargo.toml` sitting right next to the input `.nir` file is used
/// automatically (the common case for a project that has one at all —
/// no need to type the flag every time). Returns `Ok(None)` for "no
/// manifest, don't attempt discovery at all" (the byte-for-byte-
/// unchanged default for every `.nir` file with no Cargo project next
/// to it, and every existing test/example in this repo), never an
/// error just for that.
fn resolve_ui_component_manifest(explicit: Option<&str>, input_nir_path: &str) -> Option<std::path::PathBuf> {
    if let Some(p) = explicit {
        return Some(std::path::PathBuf::from(p));
    }
    let candidate = std::path::Path::new(input_nir_path).parent()?.join("Cargo.toml");
    candidate.is_file().then_some(candidate)
}

fn cmd_emit_ui(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut theme_path: Option<String> = None;
    let mut manifest_path: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next(),
            "--theme" => theme_path = args.next(),
            "--manifest-path" => manifest_path = args.next(),
            other => input = Some(other.to_string()),
        }
    }
    let Some(path) = input else {
        eprintln!("usage: nirdosha emit-ui <file.nir> [-o out.html] [--theme theme.json] [--manifest-path Cargo.toml]");
        return ExitCode::FAILURE;
    };
    // rfcs/0009 Phase B -- `ui_plugin::discover_components` (a real
    // `cargo metadata` call) only ever runs when a manifest was
    // actually found (explicit flag, or an auto-detected `Cargo.toml`
    // next to `path`); absent either, this is `vec![]` and every line
    // below behaves exactly as it did before this RFC existed.
    let components = match resolve_ui_component_manifest(manifest_path.as_deref(), &path) {
        Some(manifest) => match nirdosha::ui_plugin::discover_components(&manifest) {
            Ok(c) => c,
            Err(msg) => {
                eprintln!("error discovering UI-plugin components from {}: {msg}", manifest.display());
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };
    let (program, _src) = match typecheck_and_own_optional_main_with_ui_components(&path, &components) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    print_ungated_fn_warnings(&program);
    print_unsupported_validate_notes(&program);
    let theme = match load_theme(theme_path.as_deref()) {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    let registry = nirdosha::ast::TypeRegistry::build(&program);
    let effects = nirdosha::effects::infer_effects(&program, &registry);
    // `emit-ui` produces a static file, no server behind either
    // `/api/_demo_login` or `/auth/login` -- both false, same as
    // `identity_base: None`/`server_table_api: false` right above.
    let html = if components.is_empty() {
        nirdosha::ui_gen::generate(&program, &effects, None, false, false, false, theme.as_ref())
    } else {
        nirdosha::ui_gen::generate_with_ui_components(&program, &effects, None, false, false, false, theme.as_ref(), &components)
    };
    match output {
        Some(out) => match std::fs::write(&out, html) {
            Ok(()) => {
                println!("wrote {out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error writing {out}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            println!("{html}");
            ExitCode::SUCCESS
        }
    }
}

/// `nirdosha emit-catalog [-o out.json]` (rfcs/0009 Phase 0) -- prints
/// `catalog/std/0.1.json`, a hand-written documentation of the closed
/// layout/control/chart/theme vocabulary `ui_gen.rs`/`ui_gen_template.html`
/// already render. Baked in at compile time (`include_str!`), not read
/// from disk at runtime, the same "ships inside the binary" posture
/// `nirdosha.gbnf` has for the core grammar. Parsed and re-serialized
/// (rather than echoed byte-for-byte) purely so a hand-edit that breaks
/// JSON syntax fails loudly here instead of shipping silently malformed
/// output -- this command does not yet merge in anything from typeck or
/// a linked plugin (rfcs/0009 Phase B); it is std only, disclosed, not
/// hidden.
const STD_CATALOG_JSON: &str = include_str!("../catalog/std/0.1.json");

/// `nirdosha grammar-export` -- the actual, queryable "what is the
/// current grammar" system: runs the real parser (`nirdosha::
/// grammar_gen`) over `<root>/examples/**/*.nir` plus every
/// `nirdosha::capabilities` snippet, traces which productions fired,
/// and prints (or writes) the resulting EBNF + GBNF. Never hand-
/// transcribed -- see `grammar_gen.rs`'s own doc comment for exactly
/// what "mechanical" means here and its one disclosed gap (a handful
/// of lexical leaf productions the trace can't derive on its own).
fn cmd_grammar_export(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut root: Option<String> = None;
    let mut out_dir: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next(),
            "-o" => out_dir = args.next(),
            other => {
                eprintln!("unknown argument `{other}` -- usage: nirdosha grammar-export [--root <nirdosha repo checkout>] [-o <out_dir>]");
                return ExitCode::FAILURE;
            }
        }
    }
    let root = root.map(std::path::PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    let corpus = nirdosha::grammar_gen::default_corpus(&root);
    if corpus.is_empty() {
        eprintln!("no corpus found under {}/examples -- pass --root <path to a nirdosha repo checkout>", root.display());
        return ExitCode::FAILURE;
    }
    let report = nirdosha::grammar_gen::generate(&corpus);
    if !report.parse_errors.is_empty() {
        eprintln!("warning: {} corpus item(s) failed to parse against the current compiler:", report.parse_errors.len());
        for (label, err) in &report.parse_errors {
            eprintln!("  {label}: {err}");
        }
    }
    if !report.uncovered.is_empty() {
        eprintln!(
            "note: {} rule(s) this corpus never reached -- their grammar is not in this output (a disclosed gap, not a silent one): {:?}",
            report.uncovered.len(),
            report.uncovered
        );
    }
    match out_dir {
        Some(dir) => {
            let dir = std::path::PathBuf::from(dir);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("creating {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
            let ebnf_path = dir.join("GRAMMAR.generated.md");
            let gbnf_path = dir.join("nirdosha.generated.gbnf");
            if let Err(e) = std::fs::write(&ebnf_path, &report.ebnf) {
                eprintln!("writing {}: {e}", ebnf_path.display());
                return ExitCode::FAILURE;
            }
            if let Err(e) = std::fs::write(&gbnf_path, &report.gbnf) {
                eprintln!("writing {}: {e}", gbnf_path.display());
                return ExitCode::FAILURE;
            }
            println!("wrote {} ({} rules covered, {} uncovered)", ebnf_path.display(), report.covered.len(), report.uncovered.len());
            println!("wrote {}", gbnf_path.display());
        }
        None => {
            println!("{}", report.ebnf);
            println!("{}", report.gbnf);
        }
    }
    ExitCode::SUCCESS
}

fn cmd_emit_catalog(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut output: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next(),
            other => {
                eprintln!("unknown argument `{other}` -- usage: nirdosha emit-catalog [-o out.json]");
                return ExitCode::FAILURE;
            }
        }
    }
    let value: serde_json::Value = match serde_json::from_str(STD_CATALOG_JSON) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("internal error: catalog/std/0.1.json failed to parse: {e}");
            return ExitCode::FAILURE;
        }
    };
    let json = serde_json::to_string_pretty(&value).expect("a parsed serde_json::Value always re-serializes");
    match output {
        Some(out) => match std::fs::write(&out, &json) {
            Ok(()) => {
                println!("wrote {out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error writing {out}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            println!("{json}");
            ExitCode::SUCCESS
        }
    }
}

/// One field a `screen` block gates by role or claim, named by its
/// owning struct -- `ui_gen::GatedField`'s own `field_name` has no
/// struct context on its own, so `cmd_roles` attaches it here.
#[derive(serde::Serialize)]
struct RoleGatedFieldRef {
    #[serde(rename = "struct")]
    struct_name: String,
    field: String,
}

#[derive(serde::Serialize, Default)]
struct RoleReportEntry {
    functions: Vec<String>,
    view_fields: Vec<RoleGatedFieldRef>,
    edit_fields: Vec<RoleGatedFieldRef>,
}

#[derive(serde::Serialize)]
struct ClaimReportEntry {
    key: String,
    value: String,
    functions: Vec<String>,
    view_fields: Vec<RoleGatedFieldRef>,
    edit_fields: Vec<RoleGatedFieldRef>,
}

/// `nirdosha roles <file.nir>` (`ROADMAP.md` A6, "Roles -> functions/
/// fields report") -- pure static analysis, no new runtime concept: every
/// role/claim gate already computed by `typeck.rs`/`ui_gen.rs`, grouped
/// by the role/claim itself rather than by where it appears. Two real
/// sources, both already load-bearing elsewhere: a `fn`'s own
/// `requires(role/claim: ...)` (`FnDecl::requires`, same data
/// `ui_gen`'s private `fn_role_gate` reads), and a `screen` block's
/// field-level `view`/`edit` gates (`ui_gen::field_gates_for_struct`,
/// its one function already exposed outside that module for exactly
/// this "which fields does this gate touch" question -- see its own doc
/// comment). Workflow `state { owner: role(...) }` gates are
/// deliberately **not** folded in here: A6's own spec scopes this report
/// to "functions/fields," and a workflow state owner is neither -- the
/// existing `typeck::collect_role_claim_strings` (the demo-mode identity
/// catalog's own source) already covers that separately for whatever
/// wants the full role vocabulary instead of this report's narrower,
/// site-attributed shape.
fn cmd_roles(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next(),
            other => input = Some(other.to_string()),
        }
    }
    let Some(path) = input else {
        eprintln!("usage: nirdosha roles <file.nir> [-o out.json]");
        return ExitCode::FAILURE;
    };
    let (program, _src) = match typecheck_and_own(&path) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    let mut roles: std::collections::BTreeMap<String, RoleReportEntry> = std::collections::BTreeMap::new();
    let mut claims: Vec<ClaimReportEntry> = Vec::new();
    let mut claim_index: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    let claim_slot = |claims: &mut Vec<ClaimReportEntry>, index: &mut std::collections::HashMap<(String, String), usize>, key: &str, value: &str| -> usize {
        *index.entry((key.to_string(), value.to_string())).or_insert_with(|| {
            claims.push(ClaimReportEntry { key: key.to_string(), value: value.to_string(), functions: vec![], view_fields: vec![], edit_fields: vec![] });
            claims.len() - 1
        })
    };

    for f in &program.fns {
        match &f.requires {
            Some(nirdosha::ast::Requirement::Role(role)) => {
                roles.entry(role.clone()).or_default().functions.push(f.name.clone());
            }
            Some(nirdosha::ast::Requirement::Claim(key, value)) => {
                let idx = claim_slot(&mut claims, &mut claim_index, key, value);
                claims[idx].functions.push(f.name.clone());
            }
            None => {}
        }
    }

    for s in &program.structs {
        for gated in nirdosha::ui_gen::field_gates_for_struct(&program, &s.name) {
            for role in &gated.view_roles {
                roles.entry(role.clone()).or_default().view_fields.push(RoleGatedFieldRef { struct_name: s.name.clone(), field: gated.field_name.clone() });
            }
            for role in &gated.edit_roles {
                roles.entry(role.clone()).or_default().edit_fields.push(RoleGatedFieldRef { struct_name: s.name.clone(), field: gated.field_name.clone() });
            }
            if let Some((key, value)) = &gated.view_claim {
                let idx = claim_slot(&mut claims, &mut claim_index, key, value);
                claims[idx].view_fields.push(RoleGatedFieldRef { struct_name: s.name.clone(), field: gated.field_name.clone() });
            }
            if let Some((key, value)) = &gated.edit_claim {
                let idx = claim_slot(&mut claims, &mut claim_index, key, value);
                claims[idx].edit_fields.push(RoleGatedFieldRef { struct_name: s.name.clone(), field: gated.field_name.clone() });
            }
        }
    }

    let report = serde_json::json!({ "roles": roles, "claims": claims });
    let json = serde_json::to_string_pretty(&report).expect("built entirely from plain strings, always serializes");
    match output {
        Some(out) => match std::fs::write(&out, &json) {
            Ok(()) => {
                println!("wrote {out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error writing {out}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            println!("{json}");
            ExitCode::SUCCESS
        }
    }
}

