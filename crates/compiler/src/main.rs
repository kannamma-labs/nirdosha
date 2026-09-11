use std::process::ExitCode;

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
        "mcp" => cmd_mcp(args),
        "emit-llvm" => cmd_emit_llvm(args),
        "emit-ast" => cmd_emit_ast(args),
        "emit-ui" => cmd_emit_ui(args),
        "emit-catalog" => cmd_emit_catalog(args),
        "hi" => cmd_hi(args),
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
    eprintln!("                                      compile to a native binary (LLVM, -O2 by default)");
    eprintln!("  nirdosha verify <file.nir>          typecheck/ownership/contract-check only, no LLVM/clang");
    eprintln!("                                      needed -- 3-valued JSON verdict on stdout, exit 0/1/2");
    eprintln!("  nirdosha fix <file.nir> [--apply]   same checks as verify, plus a byte-offset FixPatch per");
    eprintln!("                                      obligation where one exists (auto/assisted/manual);");
    eprintln!("                                      --apply writes every `auto` patch to the file in place");
    eprintln!("  nirdosha explain [<code>]           print the machine-learnable error index (JSON on stdout);");
    eprintln!("                                      with no <code>, lists every NIR-code and its title");
    eprintln!("  nirdosha certify <file.nir>         same checks as verify, wrapped in a deterministic, hash-pinned");
    eprintln!("                                      Certificate v0 (source_hash/grammar_hash/evidence_tier/...)");
    eprintln!("  nirdosha mcp                        run an MCP server on stdio (JSON-RPC, newline-delimited) --");
    eprintln!("                                      exposes verify_code/get_grammar/fix/describe as MCP tools;");
    eprintln!("                                      launch via an MCP client's config, not interactively");
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
            let opts = nirdosha::codegen::ServeCodegenOptions { port, ui_html };
            nirdosha::codegen::build_serve(&program, &smt_report, std::path::Path::new(&out), opt, &opts)
        }
        None => nirdosha::codegen::build(&program, &smt_report, std::path::Path::new(&out), opt),
    };
    match result {
        Ok(()) => {
            println!("wrote {out}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// `nirdosha hi` -- one command, one app (rfcs/0013-nirdosha-realm.md,
/// rfcs/0014-generative-build-console.md). Bare `hi`, no subcommand,
/// opens the native build-mode window directly: a live 3D graph over
/// `.nir/hi.db`, with its own bottom-of-window `:ask`/`:impact` console
/// baked into the page itself (`hi_graph.html`) -- there is no separate
/// terminal front end to launch first and hand off from. The named
/// subcommands below are `.nir/hi.db`'s standalone CLI surface, for
/// CI/scripting use that never needs a window at all: `ingest`/`link`
/// aren't things the window's own startup runs on its own, and
/// `sync`/`impact` are exactly what bare `hi` already does/shows, just
/// callable without opening anything.
fn cmd_hi(mut args: impl Iterator<Item = String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error resolving the current directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(sub) = args.next() else {
        return cmd_hi_window(&cwd);
    };
    let conn = match nirdosha::hi_graph::open(&cwd) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    match sub.as_str() {
        "ingest" => {
            let Some(doc) = args.next() else {
                eprintln!("usage: nirdosha hi ingest <doc.md>");
                return ExitCode::FAILURE;
            };
            match nirdosha::hi_graph::ingest_document(&conn, std::path::Path::new(&doc)) {
                Ok(n) => {
                    println!("ingested {n} new chunk(s) from {doc}");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "sync" => {
            let files: Vec<String> = args.collect();
            match nirdosha::hi_graph::sync(&conn, &cwd, &files) {
                Ok(r) => {
                    println!(
                        "synced {} file(s): {} unit(s) seen, {} added, {} changed, {} edge(s) flagged possibly_stale",
                        r.files_scanned, r.units_seen, r.units_added, r.units_changed, r.edges_flagged
                    );
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "link" => {
            let (Some(req_id), Some(target)) = (args.next(), args.next()) else {
                eprintln!("usage: nirdosha hi link <requirement-id> <fn|struct|enum|screen:name>");
                return ExitCode::FAILURE;
            };
            match nirdosha::hi_graph::link(&conn, &req_id, &target) {
                Ok(()) => {
                    println!("linked {req_id} <-> {target}");
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "impact" => {
            let Some(target) = args.next() else {
                eprintln!("usage: nirdosha hi impact <target>");
                return ExitCode::FAILURE;
            };
            match nirdosha::hi_graph::impact(&conn, &target) {
                Ok(report) => {
                    print!("{}", format_impact_report(&target, &report));
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        "serve" => {
            // The headless/network-reachable fallback rfcs/0014's own
            // "no network port at all" section documents -- not the
            // default build-mode transport (that's the wry custom-
            // protocol handler bare `hi` opens), but a real surface for
            // scripting/CI/remote-dev-box use. Drop this validating
            // connection before handing the directory to the server,
            // which opens its own per-request connections (see
            // hi_server.rs's own doc comment on why:
            // rusqlite::Connection isn't Sync).
            drop(conn);
            match nirdosha::hi_server::serve(&cwd) {
                Ok(handle) => {
                    println!("hi API listening on http://127.0.0.1:{} (Ctrl+C to stop)", handle.port);
                    loop {
                        std::thread::park();
                    }
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("unknown `hi` subcommand `{other}` -- usage: nirdosha hi [ingest|sync|link|impact|serve] ...");
            ExitCode::FAILURE
        }
    }
}

/// Bare `nirdosha hi`: auto-scaffolds/syncs `.nir/hi.db` (best-effort --
/// a sync problem degrades to a logged warning, never blocks the window
/// from opening, same "must never be the thing that crashes" posture
/// `hi_graph.rs`'s own doc comment describes), then opens the native
/// build-mode window and blocks until it's closed. `NIRDOSHA_HI_DISABLE=1`
/// skips the scaffold/sync step entirely (the window still opens, just
/// against whatever `.nir/hi.db` already has, or none at all).
fn cmd_hi_window(cwd: &std::path::Path) -> ExitCode {
    if !nirdosha::hi_graph::is_disabled(&|k| std::env::var(k).ok()) {
        match nirdosha::hi_graph::open(cwd) {
            Ok(conn) => {
                if let Err(e) = nirdosha::hi_graph::sync(&conn, cwd, &[]) {
                    eprintln!("hi: sync failed, continuing with a possibly-stale graph: {e}");
                }
            }
            Err(e) => eprintln!("hi: couldn't open .nir/hi.db, continuing without it ({}=1 to silence this): {e}", nirdosha::hi_graph::HI_DISABLE_VAR),
        }
    }
    match nirdosha::hi_window::open(cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// One rendering of a bounded impact walk (rfcs/0013), flagged nodes
/// listed first (`hi_graph::impact` already sorts them that way).
fn format_impact_report(target: &str, report: &nirdosha::hi_graph::ImpactReport) -> String {
    if report.hits.is_empty() {
        return format!("no reachable nodes from `{target}` -- try `nirdosha hi link` or `nirdosha hi sync` first.\n");
    }
    let mut out = format!("impact of `{target}` ({} node(s){}):\n", report.hits.len(), if report.partial { ", partial -- bound reached" } else { "" });
    for h in &report.hits {
        let flag = h.flag.as_deref().map(|f| format!("  [{f}]")).unwrap_or_default();
        // `source_ref`/`line`/`col` are only ever set on a `CodeUnit`
        // node (`Requirement`/`Document`/`Chunk` have nothing to point
        // at) -- printed only when present, same "NULL means nothing to
        // show" convention the rest of this report already follows.
        let location = match (&h.source_ref, h.line, h.col) {
            (Some(path), Some(line), Some(col)) => format!("  ({path}:{line}:{col})"),
            _ => String::new(),
        };
        out.push_str(&format!("  depth {} {} {} `{}`{location}{flag}\n", h.depth, h.kind, h.edge_kind, h.title.as_deref().unwrap_or(&h.node_id)));
    }
    out
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

#[derive(serde::Serialize)]
struct VerifyDiagnostic {
    line: usize,
    col: usize,
    message: String,
    /// Same meaning as `ContractObligation::fix` -- `None` unless this
    /// specific diagnostic kind has real fix analysis attached (v1:
    /// only `typeck::TypeErrorKind::UnknownVar`, an edit-distance typo
    /// correction against the names actually in scope -- see
    /// `fix_unbound_identifier`).
    fix: Option<Fix>,
    /// `nirdosha explain <code>`'s index key (`explain::REGISTRY`) --
    /// `None` unless this specific diagnostic's site can identify one
    /// of the twelve `AGENTS.md`-numbered rules (or `NIR0013`,
    /// unbound-identifier) with certainty; see `explain.rs`'s own doc
    /// comment for exactly which sites do today and why the rest
    /// honestly don't yet.
    code: Option<&'static str>,
}

#[derive(serde::Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum StageStatus {
    Passed,
    Failed,
    Skipped,
}

/// The top-level verdict `nirdosha verify` reports, and the one
/// `ContractsResult` reports for its own stage -- three-valued on
/// purpose, never collapsed to pass/fail. `StageStatus` above answers
/// "did this stage run and complete" (a pipeline-mechanics question,
/// genuinely binary: typecheck either finds no error or it does); this
/// answers "what do we actually know about the code's correctness" (an
/// epistemic question, and not binary at all): `Unsupported` -- Z3
/// couldn't model a predicate, not "it found no problem" -- used to be
/// folded into an overall `Passed` verdict, which reported confidence
/// this pipeline never earned. A caller (CI, an agent's own repair
/// loop) needs a real, distinguishable third answer for "we don't know"
/// so it doesn't treat unmodeled code as proved safe. Exit codes follow
/// the same three-way split (0/1/2, `cmd_verify` below), not just this
/// JSON field, for the same reason: a caller that only inspects `$?`
/// must be able to see the difference too.
#[derive(serde::Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProofVerdict {
    Proved,
    Disproved,
    Unknown,
}

#[derive(serde::Serialize)]
struct StageResult {
    status: StageStatus,
    errors: Vec<VerifyDiagnostic>,
}

impl StageResult {
    fn skipped() -> Self {
        StageResult { status: StageStatus::Skipped, errors: vec![] }
    }
}

/// `nirdosha fix`'s three fixability classes (`nirdosha-master-plan.md`
/// Part 3, Sprint 1: "fixability classes (auto / assisted / manual)",
/// parity target: Kōdo). Modeled directly on `rustc`'s own
/// `Applicability` enum (`MachineApplicable`/`MaybeIncorrect`/
/// `HasPlaceholders`/`Unspecified`, the real precedent `rustfix`/
/// `cargo fix` already ship against) rather than invented from scratch:
/// `Auto` == `MachineApplicable` (safe to apply without review -- the
/// only class `nirdosha fix --apply` ever writes to disk on its own);
/// `Assisted` == `MaybeIncorrect`/`HasPlaceholders` collapsed into one
/// (a real fix exists, or a shape of one does, but it needs a judgment
/// call `nirdosha fix` can't make safely by itself); `Manual` ==
/// `Unspecified` (no mechanical fix known for this diagnostic kind at
/// all, today).
#[derive(serde::Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Applicability {
    Auto,
    Assisted,
    Manual,
}

/// A byte-offset text replacement -- `[start_byte, end_byte)` in the
/// original source file, replaced with `replacement`. Byte offsets, not
/// line/column: a patch applier needs to slice and splice the exact
/// original bytes, and `token::Span::byte`'s own doc comment names the
/// same reason `rustc`'s `Span`/`BytePos` is byte-addressed rather than
/// line/column-addressed. Only ever present when `Fix::applicability`
/// is `Auto` or `Assisted` -- a `Manual` fix has no patch to offer by
/// definition, so its `Fix::patch` is always `None`, not a patch that
/// happens to be a no-op.
#[derive(serde::Serialize)]
struct FixPatch {
    start_byte: usize,
    end_byte: usize,
    replacement: String,
}

#[derive(serde::Serialize)]
struct Fix {
    applicability: Applicability,
    /// Present for `Auto` (always) and `Assisted` (when a concrete
    /// replacement text exists, even if it needs review before
    /// trusting it); absent for `Manual`, and absent for an `Assisted`
    /// fix that only has *guidance* to offer, not literal replacement
    /// text (e.g. "wrap this in a `struct Text` or a real `enum`,
    /// your call" -- two shapes, no single patch is honest to propose).
    patch: Option<FixPatch>,
    /// Human-readable explanation of what this fix does and why this
    /// applicability class, not a fix format's own vocabulary --
    /// printed as-is by any caller that doesn't want to interpret
    /// `applicability` itself.
    rationale: String,
}

#[derive(serde::Serialize)]
struct ContractObligation {
    fn_name: String,
    status: &'static str,
    detail: Option<String>,
    /// `None` means no automated-fix analysis produced anything for
    /// this obligation's kind yet -- not the same claim as `Manual`
    /// (which is a considered "no mechanical fix exists"). `nirdosha
    /// fix` v1 only analyzes `unbound_identifier` obligations (a real,
    /// bounded, edit-distance typo correction against the function's
    /// own parameter names); every other obligation kind is `None`
    /// here, honestly, rather than a blanket `Manual` that implies more
    /// analysis happened than actually did.
    fix: Option<Fix>,
}

#[derive(serde::Serialize)]
struct ContractsResult {
    /// Whether this stage ran at all -- `Skipped` only if an earlier
    /// stage (load/typecheck/ownership) already failed, `Passed`
    /// otherwise, even when `verdict` below is `Disproved` or
    /// `Unknown`: this stage genuinely *ran*, it's `verdict` that says
    /// what it found, not whether it executed. Never set to `Failed` by
    /// this stage -- an earlier revision conflated "ran and found a
    /// counterexample" with `StageStatus::Failed`, which left no room
    /// for `Unsupported` to mean anything but a silent pass; `verdict`
    /// is the fix, this field's meaning is now consistent with `load`/
    /// `typecheck`/`ownership`'s.
    status: StageStatus,
    verdict: ProofVerdict,
    proved: usize,
    unsupported: usize,
    failed: usize,
    obligations: Vec<ContractObligation>,
}

/// `smt::analyze`'s per-span proof counts (Tier 1: overflow/division/
/// array-bounds obligations `codegen.rs` elides a runtime trap for once
/// proven) -- informational, not a pass/fail gate on its own. A span
/// missing from these counts isn't a defect: it's still enforced, just
/// at runtime instead (the same "proved vs. still-safely-checked"
/// distinction `contract_check.rs`'s `Unsupported` already draws for
/// `validate` blocks).
#[derive(serde::Serialize)]
struct ProofObligations {
    proven_in_range: usize,
    proven_nonzero_divisor: usize,
    proven_index_bounds: usize,
}

#[derive(serde::Serialize)]
struct VerifyVerdict {
    source: String,
    /// Three-valued, replacing what used to be a `status: StageStatus`
    /// field here (`Passed`/`Failed` only) -- called out explicitly
    /// per `docs/STABILITY_AND_RELEASES.md`'s rule for this exact JSON
    /// schema, not a silent rename: the old field could not represent
    /// "Z3 couldn't decide," so a file with an unmodeled `validate`
    /// predicate and nothing else wrong reported the same `Passed` a
    /// file with a fully proved contract did. `verdict` is `Unknown`
    /// in that case instead, never folded into `Proved`.
    verdict: ProofVerdict,
    load: StageResult,
    typecheck: StageResult,
    ownership: StageResult,
    contracts: ContractsResult,
    proof_obligations: ProofObligations,
}

/// Levenshtein edit distance -- standard textbook dynamic-programming
/// form (a single rolling `prev_row`, `O(len(a) * len(b))` time,
/// `O(min(len(a), len(b)))` space via the shorter string as columns).
/// Used only for `fix_unbound_identifier`'s typo suggestions, over
/// short identifier strings (a handful of characters, never a whole
/// file), so the naive DP form is the right amount of engineering --
/// no need for the banded/early-exit variants a spell-checker over a
/// large dictionary would want.
/// Recovers `NIR0012` (reserved word used where an identifier was
/// required) from `loader::load_program`'s already-formatted `String`
/// error, the only signal available at this call site today --
/// `ParseError` doesn't carry a structured code field, and threading
/// one through would mean widening `loader::load_program`'s
/// `Result<_, String>` across every one of its own callers
/// (`build`/`emit-llvm`/`emit-ast`/`gen-crud`/...), a much larger
/// change than one diagnostic's classification justifies today. Safe
/// *because* the substring it looks for is fully deterministic: only
/// `parser.rs`'s `expect_ident` ever produces the literal prefix
/// `"expected identifier, found "`, and only `Display for Tok`
/// (`token.rs`, round-trip tested) ever renders a reserved word as
/// `"the reserved keyword `...`"`/`"the reserved type name `...`"` --
/// so this can't false-positive on an unrelated "expected identifier"
/// site or an unrelated reserved-word mention. A heuristic, not a
/// guess: narrowed by construction to the one message shape that means
/// this rule, nothing looser.
fn classify_load_error_code(msg: &str) -> Option<&'static str> {
    const PREFIX: &str = "expected identifier, found ";
    let after = msg.find(PREFIX)? + PREFIX.len();
    let found = &msg[after..];
    if found.starts_with("the reserved keyword ") || found.starts_with("the reserved type name ") {
        Some("NIR0012")
    } else {
        None
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev_row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur_row = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let insert = cur_row[j] + 1;
            let delete = prev_row[j + 1] + 1;
            let substitute = prev_row[j] + cost;
            cur_row.push(insert.min(delete).min(substitute));
        }
        prev_row = cur_row;
    }
    prev_row[b.len()]
}

/// `nirdosha fix`'s one real, tested fixability analysis for v1
/// (`nirdosha-master-plan.md` Part 3 Sprint 1): a `validate` predicate
/// referencing a name that's neither `result` nor one of the function's
/// own parameters, most often a plain typo (`ammount` for `amount`).
/// Modeled on `rustc`'s own identifier-typo suggestions
/// (`find_best_match_for_name`), which use the same edit-distance
/// technique and the same "only suggest when unambiguous" discipline --
/// a real precedent for exactly this shape of fix, not a heuristic
/// invented here from nothing.
///
/// - Exactly one candidate strictly closer than every other, and within
///   a relative threshold -- `Auto`: the typo is unambiguous, the patch
///   is the obviously-intended one, safe to apply without a human in
///   the loop (matches `rustc`'s own `MachineApplicable` bar for typo
///   fixes).
/// - More than one candidate tied at the closest distance -- `Assisted`:
///   a real fix is one of these names, but which one is a judgment call
///   this analysis can't make safely; no single patch, the rationale
///   lists every tied candidate.
/// - Nothing within the threshold -- `Manual`: this isn't a typo of any
///   parameter name close enough to guess at; the rationale lists what
///   *is* available so a human or an agent's repair loop has the real
///   options in front of it, not just "no".
fn fix_unbound_identifier(name: &str, span: nirdosha::token::Span, candidates: &[String]) -> Fix {
    // A relative threshold, not a flat constant -- `1/3` of the name's
    // own length (floor, minimum 1) roughly matches how many characters
    // a plausible single typo (one substitution/transposition/drop)
    // changes in a short identifier, without also matching two
    // genuinely different short names to each other (e.g. `a` and `b`
    // are distance 1 but not a typo of each other -- a flat threshold
    // of 1 would still suggest one for the other; `a`'s own length-based
    // threshold is 1 too, so this doesn't fully solve that single-char
    // case, but it's the same tradeoff `rustc`'s own suggestion
    // threshold makes, not an oversight unique to this implementation).
    let threshold = (name.chars().count() / 3).max(1);

    let mut distances: Vec<(usize, &String)> =
        candidates.iter().map(|c| (levenshtein(name, c), c)).filter(|(d, _)| *d <= threshold).collect();
    distances.sort_by_key(|(d, name)| (*d, name.to_string()));

    let start_byte = span.byte;
    let end_byte = span.byte + name.len();

    match distances.as_slice() {
        [] => Fix {
            applicability: Applicability::Manual,
            patch: None,
            rationale: format!(
                "`{name}` isn't within edit distance {threshold} of any available name ({}) -- not a plausible typo of one of them, needs a real decision about what this predicate should reference",
                candidates.join(", ")
            ),
        },
        [(only_dist, only_name)] => Fix {
            applicability: Applicability::Auto,
            patch: Some(FixPatch { start_byte, end_byte, replacement: (*only_name).clone() }),
            rationale: format!("`{name}` is edit distance {only_dist} from `{only_name}`, the only candidate this close -- almost certainly a typo"),
        },
        [(best_dist, _), (tied_dist, _), ..] if best_dist == tied_dist => {
            let tied: Vec<&str> = distances.iter().filter(|(d, _)| d == best_dist).map(|(_, n)| n.as_str()).collect();
            Fix {
                applicability: Applicability::Assisted,
                patch: None,
                rationale: format!(
                    "`{name}` is equally close (edit distance {best_dist}) to more than one candidate ({}) -- one of these is almost certainly intended, but which one needs a human or an agent's own judgment, not a guess",
                    tied.join(", ")
                ),
            }
        }
        [(best_dist, best_name), ..] => Fix {
            applicability: Applicability::Auto,
            patch: Some(FixPatch { start_byte, end_byte, replacement: (*best_name).clone() }),
            rationale: format!("`{name}` is edit distance {best_dist} from `{best_name}`, strictly closer than every other candidate -- almost certainly a typo"),
        },
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
fn cmd_verify(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(path) = args.next() else {
        eprintln!("usage: nirdosha verify <file.nir>");
        return ExitCode::FAILURE;
    };

    let verdict = run_verify_pipeline(&path);
    println!("{}", serde_json::to_string_pretty(&verdict).expect("VerifyVerdict always serializes"));
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

/// The shared gate pipeline behind both `nirdosha verify` and `nirdosha
/// fix` -- load, typecheck, ownership, `validate` contract-check (with
/// `nirdosha fix`'s per-obligation `Fix` analysis always attached, per
/// `ContractObligation::fix`'s own doc comment: computing it is cheap
/// and it's additive to the JSON schema, so `verify` callers get it
/// too, not just `fix` ones), plus `smt::analyze`'s Tier-1 counts.
/// Extracted out of `cmd_verify` so `cmd_fix` runs the exact same
/// checks instead of a second, maintained-separately copy that could
/// drift from what `verify` actually checks.
fn run_verify_pipeline(path: &str) -> VerifyVerdict {
    let mut load = StageResult { status: StageStatus::Passed, errors: vec![] };
    let mut typecheck = StageResult::skipped();
    let mut ownership = StageResult::skipped();
    let mut contracts = ContractsResult {
        status: StageStatus::Skipped,
        verdict: ProofVerdict::Proved,
        proved: 0,
        unsupported: 0,
        failed: 0,
        obligations: vec![],
    };
    let mut proof_obligations = ProofObligations { proven_in_range: 0, proven_nonzero_divisor: 0, proven_index_bounds: 0 };

    let program = match nirdosha::loader::load_program(path) {
        Ok((program, _src)) => Some(program),
        Err(msg) => {
            load.status = StageStatus::Failed;
            let code = classify_load_error_code(&msg);
            load.errors.push(VerifyDiagnostic { line: 0, col: 0, message: msg, fix: None, code });
            None
        }
    };

    let program = program.and_then(|program| match nirdosha::typeck::typecheck_optional_main(&program) {
        Ok(()) => {
            typecheck.status = StageStatus::Passed;
            Some(program)
        }
        Err(errs) => {
            typecheck.status = StageStatus::Failed;
            typecheck.errors = errs
                .iter()
                .map(|e| {
                    let (fix, code) = match &e.kind {
                        nirdosha::typeck::TypeErrorKind::UnknownVar { name, candidates } => {
                            (Some(fix_unbound_identifier(name, e.span, candidates)), Some("NIR0013"))
                        }
                        nirdosha::typeck::TypeErrorKind::StrInFnSignature { .. } => (None, Some("NIR0002")),
                        _ => (None, None),
                    };
                    VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix, code }
                })
                .collect();
            None
        }
    });

    let program = program.and_then(|program| match nirdosha::ownership::check_ownership(&program) {
        Ok(()) => {
            ownership.status = StageStatus::Passed;
            Some(program)
        }
        Err(errs) => {
            ownership.status = StageStatus::Failed;
            ownership.errors = errs
                .iter()
                .map(|e| VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix: None, code: None })
                .collect();
            None
        }
    });

    if let Some(program) = &program {
        contracts.status = StageStatus::Passed;
        for outcome in nirdosha::contract_check::run_program_validates(program) {
            use nirdosha::contract_check::ContractCheckResult;
            match outcome.result {
                ContractCheckResult::Proved => {
                    contracts.proved += 1;
                    contracts.obligations.push(ContractObligation { fn_name: outcome.fn_name, status: "proved", detail: None, fix: None });
                }
                ContractCheckResult::Unsupported(msg) => {
                    contracts.unsupported += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "unsupported",
                        detail: Some(msg),
                        fix: None,
                    });
                }
                ContractCheckResult::Counterexample { violated_predicate, bindings, result } => {
                    contracts.failed += 1;
                    let bindings_str = bindings.iter().map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join(", ");
                    let detail = format!(
                        "`{violated_predicate}` is violated when {bindings_str} (fn returns {})",
                        result.map(|r| r.to_string()).unwrap_or_else(|| "<uncomputed>".to_string())
                    );
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "counterexample",
                        detail: Some(detail),
                        fix: None,
                    });
                }
                ContractCheckResult::UnboundIdentifier { name, span, candidates } => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "unbound_identifier",
                        detail: Some(name.clone()),
                        fix: Some(fix_unbound_identifier(&name, span, &candidates)),
                    });
                }
                ContractCheckResult::NoSuchFunction(name) => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "no_such_function",
                        detail: Some(name),
                        fix: None,
                    });
                }
                ContractCheckResult::PredicateParseError(msg) => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "predicate_parse_error",
                        detail: Some(msg),
                        fix: None,
                    });
                }
            }
        }

        // `failed` beats `unsupported`, which beats an empty verdict of
        // `Proved` -- a real counterexample is more informative than "Z3
        // couldn't decide," so a file that's both definitely wrong
        // somewhere and unmodeled somewhere else is reported as
        // `Disproved`, not `Unknown`: an agent's repair loop has a
        // concrete counterexample to act on either way, and hiding it
        // behind "unknown" would be a strictly worse answer.
        contracts.verdict = if contracts.failed > 0 {
            ProofVerdict::Disproved
        } else if contracts.unsupported > 0 {
            ProofVerdict::Unknown
        } else {
            ProofVerdict::Proved
        };

        let smt_report = nirdosha::smt::analyze(program);
        proof_obligations.proven_in_range = smt_report.proven_in_range.len();
        proof_obligations.proven_nonzero_divisor = smt_report.proven_nonzero_divisor.len();
        proof_obligations.proven_index_bounds = smt_report.proven_index_bounds.len();
    }

    // A hard pipeline failure (load/typecheck/ownership) is always
    // `Disproved`, not `Unknown` -- there's no uncertainty in "this
    // doesn't typecheck." Only `contracts.verdict` can introduce
    // `Unknown`, and only when nothing else already disproved the file.
    let pipeline_failed =
        [load.status, typecheck.status, ownership.status].iter().any(|s| *s == StageStatus::Failed);
    let verdict_value = if pipeline_failed || contracts.verdict == ProofVerdict::Disproved {
        ProofVerdict::Disproved
    } else if contracts.verdict == ProofVerdict::Unknown {
        ProofVerdict::Unknown
    } else {
        ProofVerdict::Proved
    };

    VerifyVerdict {
        source: path.to_string(),
        verdict: verdict_value,
        load,
        typecheck,
        ownership,
        contracts,
        proof_obligations,
    }
}

/// One `Auto`-class patch `nirdosha fix --apply` actually wrote to
/// disk -- distinct from `Fix`/`FixPatch` above (which describe a
/// *proposed* patch, applied or not): this is the record of one that
/// really was, so `FixReport.applied` is a true audit trail, not a
/// re-derivation of `before`'s obligations filtered by applicability.
///
/// `site`/`detail` are deliberately generic strings, not `fn_name`/
/// `&'static str` (an earlier revision had both, then only ever
/// collected `contracts.obligations`' `Auto` patches -- silently
/// dropping every `Auto` fix attached to a `load`/`typecheck`/
/// `ownership` `VerifyDiagnostic` instead, e.g. `fix_unbound_identifier`
/// firing from `typeck.rs`, because `--apply`'s collector never looked
/// there. Caught by hand-running `nirdosha fix --apply` end to end on a
/// real typo: the JSON proposed the correct patch, `--apply` wrote
/// zero of them). `site` is the function name for a `ContractObligation`
/// or `"<line>:<col>"` for a `VerifyDiagnostic`; `detail` is the
/// obligation's status tag or the diagnostic's own message -- both
/// human-readable provenance, not data the applier itself branches on.
#[derive(serde::Serialize)]
struct AppliedPatch {
    site: String,
    detail: String,
    start_byte: usize,
    end_byte: usize,
    replacement: String,
}

#[derive(serde::Serialize)]
struct FixReport {
    /// The verdict before any patch was applied -- identical shape to
    /// `nirdosha verify`'s own output, `fix` fields included, whether
    /// or not `--apply` was given.
    before: VerifyVerdict,
    /// Every `Auto`-class patch actually written to disk. Always empty
    /// without `--apply` -- `nirdosha fix` on its own only *reports*
    /// patches, matching `rustc`'s own separation between emitting
    /// suggestions and a separate tool (`rustfix`/`cargo fix`)
    /// applying them; `--apply` is that second step folded into the
    /// same command instead of a second binary, not a different
    /// analysis.
    applied: Vec<AppliedPatch>,
    /// Re-verification after applying `applied`'s patches -- `None`
    /// unless `--apply` was given. This is the honest check that the
    /// patches this command just wrote actually improved the verdict,
    /// not an assumption that generating a patch means it worked.
    after: Option<VerifyVerdict>,
}

/// Collects every `Auto`-class patch across all four `before` stages
/// (`load`/`typecheck`/`ownership`'s `VerifyDiagnostic`s, plus
/// `contracts.obligations`) and writes them to `path`, highest byte
/// offset first -- applying low-to-high would shift every later
/// patch's own byte offsets out from under it the moment an earlier
/// one changed the file's length; high-to-low never does, since
/// nothing after the current patch's end has been touched yet by the
/// time it's applied. Extracted out of `cmd_fix` so the MCP `fix` tool
/// (`mcp_fix`) shares this exact algorithm instead of a second,
/// maintained-separately copy -- the real bug `AppliedPatch`'s own doc
/// comment describes (an earlier revision only ever scanned
/// `contracts.obligations`, silently dropping every `Auto` patch
/// attached to a stage diagnostic instead) was exactly this kind of
/// drift, and it only had one call site to go wrong in at the time.
fn write_auto_patches(path: &str, before: &VerifyVerdict) -> Result<Vec<AppliedPatch>, String> {
    let mut patches: Vec<(String, String, &FixPatch)> = Vec::new();
    for diag in before.load.errors.iter().chain(before.typecheck.errors.iter()).chain(before.ownership.errors.iter()) {
        if let Some(Fix { applicability: Applicability::Auto, patch: Some(p), .. }) = &diag.fix {
            patches.push((format!("{}:{}", diag.line, diag.col), diag.message.clone(), p));
        }
    }
    for ob in &before.contracts.obligations {
        if let Some(Fix { applicability: Applicability::Auto, patch: Some(p), .. }) = &ob.fix {
            patches.push((ob.fn_name.clone(), ob.status.to_string(), p));
        }
    }
    patches.sort_by(|a, b| b.2.start_byte.cmp(&a.2.start_byte));

    let mut applied = Vec::new();
    if patches.is_empty() {
        return Ok(applied);
    }

    let mut src = std::fs::read_to_string(path).map_err(|e| format!("error reading {path} to apply patches: {e}"))?;
    for (site, detail, patch) in &patches {
        if patch.start_byte > src.len() || patch.end_byte > src.len() || patch.start_byte > patch.end_byte {
            // A patch computed against a stale byte range (shouldn't
            // happen -- `before` was just read from this same file --
            // but a corrupt/concurrently-modified file is a real
            // possibility this must not silently misapply against) is
            // skipped, not forced.
            continue;
        }
        src.replace_range(patch.start_byte..patch.end_byte, &patch.replacement);
        applied.push(AppliedPatch {
            site: site.clone(),
            detail: detail.clone(),
            start_byte: patch.start_byte,
            end_byte: patch.end_byte,
            replacement: patch.replacement.clone(),
        });
    }
    std::fs::write(path, &src).map_err(|e| format!("error writing patched file: {e}"))?;
    Ok(applied)
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
    for a in args.by_ref() {
        match a.as_str() {
            "--apply" => apply = true,
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: nirdosha fix <file.nir> [--apply]");
        return ExitCode::FAILURE;
    };

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
    println!("{}", serde_json::to_string_pretty(&report).expect("FixReport always serializes"));

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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

/// Certificate v0 (`nirdosha-master-plan.md` Part 3 Sprint 1, parity
/// target: Velvet, Kōdo) -- a deterministic, hash-pinned JSON
/// attestation, reproducible by any third party holding the same
/// source file and the same compiler version: no timestamp, no
/// absolute path, no random nonce anywhere in this struct, only
/// content hashes and counts. `#[derive(Serialize)]` on a plain struct
/// (not a `serde_json::Map`) is itself part of that determinism --
/// serde always serializes struct fields in declaration order, never
/// resorted, so the same `Certificate` value always produces the same
/// JSON bytes.
///
/// `evidence_tier` is the field `docs/PUBLIC_ROADMAP.md`'s master-plan
/// notes say to ship now "so the format never breaks": all four tiers
/// (`proved`/`checked`/`sampled`/`unknown`) are valid values in this
/// schema from v0 onward, even though this compiler can only ever
/// produce `proved` or `unknown` today -- `checked` (sandbox-validated
/// behavioral evidence) and `sampled` (property-based/fuzz evidence)
/// both require infrastructure that doesn't exist yet (the MicroVM
/// sandbox tier, post-seed; see the master plan's "language-agnostic
/// ladder"). A certificate a future `nirdosha check` emits can set
/// either of those without this struct's shape ever changing.
#[derive(serde::Serialize)]
struct Certificate {
    certificate_version: &'static str,
    source_hash: String,
    grammar_hash: String,
    toolchain_version: &'static str,
    /// `"proved"` | `"checked"` | `"sampled"` | `"unknown"` -- see this
    /// struct's own doc comment for why the type is `&'static str`
    /// (an open, forward-declared vocabulary) rather than a 2-variant
    /// enum that would have to grow a breaking variant later.
    evidence_tier: &'static str,
    verdict_summary: VerdictSummary,
    proof_obligations: ProofObligations,
}

#[derive(serde::Serialize)]
struct VerdictSummary {
    verdict: ProofVerdict,
    contracts_proved: usize,
    contracts_unsupported: usize,
    contracts_failed: usize,
}

/// Builds a `Certificate` from a source file's raw bytes and the
/// pipeline result already run against it -- takes `VerifyVerdict` by
/// value and destructures it (`..` drops `load`/`typecheck`/
/// `ownership`/`source`) rather than borrowing, since nothing after
/// this call needs the full verdict and a certificate is deliberately
/// a compact summary of it, not a re-export of every diagnostic
/// `verify`'s own JSON already carries.
fn build_certificate(source_bytes: &[u8], pipeline: VerifyVerdict) -> Certificate {
    let VerifyVerdict { verdict, contracts, proof_obligations, .. } = pipeline;
    // `Skipped` means contract-check never ran at all (a load/typecheck/
    // ownership failure stopped the pipeline first) -- no Z3-backed
    // evidence was ever produced, so `unknown` is the honest tier, not
    // `proved`/`checked` implying analysis that didn't happen. Once
    // contract-check does run, both `Proved` and `Disproved` are a
    // *conclusive* Z3 answer -- `evidence_tier` describes the kind of
    // evidence (formal, either way), `verdict_summary.verdict` separately
    // describes the outcome (pass or fail). Only `Unknown` (Z3 couldn't
    // model at least one obligation) leaves `evidence_tier` at
    // `"unknown"` too: an inconclusive stage produced no full proof.
    let evidence_tier = if contracts.status == StageStatus::Skipped {
        "unknown"
    } else {
        match verdict {
            ProofVerdict::Proved | ProofVerdict::Disproved => "proved",
            ProofVerdict::Unknown => "unknown",
        }
    };
    Certificate {
        certificate_version: "0",
        source_hash: sha256_hex(source_bytes),
        grammar_hash: sha256_hex(NIRDOSHA_GBNF.as_bytes()),
        toolchain_version: env!("CARGO_PKG_VERSION"),
        evidence_tier,
        verdict_summary: VerdictSummary {
            verdict,
            contracts_proved: contracts.proved,
            contracts_unsupported: contracts.unsupported,
            contracts_failed: contracts.failed,
        },
        proof_obligations,
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
    let Some(path) = args.next() else {
        eprintln!("usage: nirdosha certify <file.nir>");
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
    let certificate = build_certificate(&source_bytes, pipeline);
    println!("{}", serde_json::to_string_pretty(&certificate).expect("Certificate always serializes"));
    match verdict {
        ProofVerdict::Proved => {
            eprintln!("PROVED: certificate issued for {path}, evidence_tier={}", certificate.evidence_tier);
            ExitCode::SUCCESS
        }
        ProofVerdict::Disproved => {
            eprintln!("DISPROVED: certificate issued for {path} recording the failure, evidence_tier={}", certificate.evidence_tier);
            ExitCode::FAILURE
        }
        ProofVerdict::Unknown => {
            eprintln!("UNKNOWN: certificate issued for {path}, evidence_tier={}", certificate.evidence_tier);
            ExitCode::from(2)
        }
    }
}

/// A `.nir` source file that exists only for the duration of one MCP
/// tool call -- `run_verify_pipeline`/`write_auto_patches`/
/// `loader::load_program` all take a filesystem path (import
/// resolution, `write_auto_patches`'s own read-modify-write, and
/// `db_connect`-relative paths in the source itself all need a real
/// file on disk), but an MCP tool call only ever carries source text
/// inline (`mcp_tools_list`'s own schemas -- every tool takes `source`,
/// never `path`, the same choice Kōdo's MCP tools make). Rather than
/// fork the pipeline into a path-based and a source-based variant,
/// every MCP handler below materializes `source` to one of these and
/// reuses the exact same, already-tested pipeline `verify`/`fix` run
/// against a real file. `Drop` removes it unconditionally, not a
/// manual `remove_file` at the end of each handler, so a long-running
/// `nirdosha mcp` process serving many calls never accumulates temp
/// files -- including on an early `?`-return from a handler.
struct TempNirFile(std::path::PathBuf);

impl TempNirFile {
    fn write(source: &str) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("nirdosha_mcp_{}_{n}.nir", std::process::id()));
        std::fs::write(&p, source).map_err(|e| format!("failed to stage source for verification: {e}"))?;
        Ok(Self(p))
    }

    fn path_str(&self) -> &str {
        self.0.to_str().expect("temp_dir()-rooted path is always valid UTF-8 on every platform this ships for")
    }
}

impl Drop for TempNirFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn require_str_arg<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    arguments.get(name).and_then(|v| v.as_str()).ok_or_else(|| format!("Missing required parameter '{name}'"))
}

/// `verify_code` -- runs `run_verify_pipeline` (the exact pipeline
/// `nirdosha verify`/`nirdosha fix` both run) against inline source and
/// returns the identical `VerifyVerdict` JSON shape, `source` replaced
/// with `"<inline>"` since the real value (a temp path) is an
/// implementation detail no caller should key off of.
fn mcp_verify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let temp = TempNirFile::write(source)?;
    let verdict = run_verify_pipeline(temp.path_str());
    let mut value = serde_json::to_value(&verdict).expect("VerifyVerdict always serializes");
    value["source"] = serde_json::json!("<inline>");
    Ok(value)
}

/// Baked into the binary at compile time (`include_str!`), the same
/// "ships inside the binary" posture `STD_CATALOG_JSON` documents for
/// `emit-catalog` -- `nirdosha.gbnf` is checked into the repo at the
/// crate root and re-generated by `crates/grammar_export`'s own tests
/// against `llama-cpp-gbnf`, never hand-edited independently of the
/// grammar it's exported from.
const NIRDOSHA_GBNF: &str = include_str!("../nirdosha.gbnf");

/// `get_grammar` -- returns the full LL(1) Nirdosha grammar in GBNF
/// form (constrained-decoding target for llama.cpp/vLLM-style grammar-
/// constrained generation). No arguments; the grammar is one fixed
/// artifact per compiler version, not parameterized per call.
fn mcp_get_grammar(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "format": "gbnf", "grammar": NIRDOSHA_GBNF }))
}

/// `fix` -- same pipeline as `verify_code`, plus `write_auto_patches`
/// (the exact algorithm `nirdosha fix --apply` uses, shared not
/// duplicated) when `apply: true`. There's no file for the CLI's own
/// `--apply` to write back to and hand the caller a path for, so this
/// returns the patched source text directly in `patched_source`
/// instead -- the MCP-native equivalent of `--apply` actually rewriting
/// the file on disk.
fn mcp_fix(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let apply = arguments.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let temp = TempNirFile::write(source)?;

    let before = run_verify_pipeline(temp.path_str());
    let applied = if apply { write_auto_patches(temp.path_str(), &before)? } else { Vec::new() };

    let mut before_value = serde_json::to_value(&before).expect("VerifyVerdict always serializes");
    before_value["source"] = serde_json::json!("<inline>");
    let mut result = serde_json::json!({ "before": before_value, "applied": applied });

    if apply {
        let patched = std::fs::read_to_string(temp.path_str()).map_err(|e| format!("failed to read patched source back: {e}"))?;
        let after = run_verify_pipeline(temp.path_str());
        let mut after_value = serde_json::to_value(&after).expect("VerifyVerdict always serializes");
        after_value["source"] = serde_json::json!("<inline>");
        result["after"] = after_value;
        result["patched_source"] = serde_json::json!(patched);
    }
    Ok(result)
}

/// `describe` -- parses (does not require it to typecheck, the same
/// "AST of a program that doesn't yet typecheck is still legitimate to
/// inspect" contract `cmd_emit_ast` already documents) inline source
/// and returns a curated structural summary via `describe_program`.
fn mcp_describe(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let temp = TempNirFile::write(source)?;
    let (program, _src) = nirdosha::loader::load_program(temp.path_str())?;
    Ok(describe_program(&program))
}

/// A curated structural summary of a parsed `Program` -- every fn's
/// signature (never its body), every struct's fields, every enum's
/// variants, every top-level `validate` block's contracts, as real
/// structured JSON (`Ty`/`Expr`/`Requirement`/`NfrSpec`/`Effect` all
/// derive `Serialize` already -- no need for Kōdo's own `{:?}`-string
/// shortcut here). Deliberately not the same thing `emit-ast` already
/// gives: that's the full, span-carrying AST (every expression, every
/// statement, meant for round-tripping); this is the signature-level
/// summary an agent actually wants when it asks "what does this file
/// declare" -- modeled on Kōdo's own `kodo.describe` MCP tool
/// (functions/types/meta, bodies omitted).
fn describe_program(program: &nirdosha::ast::Program) -> serde_json::Value {
    let functions: Vec<serde_json::Value> = program
        .fns
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name,
                "params": f.params.iter().map(|p| serde_json::json!({ "name": p.name, "type": p.ty })).collect::<Vec<_>>(),
                "return_type": f.ret,
                "effects": f.declared_effects,
                "requires": f.requires,
                "nfr": f.nfr,
                "exported": f.exported,
            })
        })
        .collect();
    let structs: Vec<serde_json::Value> = program
        .structs
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "fields": s.fields.iter().map(|f| serde_json::json!({ "name": f.name, "type": f.ty })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let enums: Vec<serde_json::Value> = program
        .enums
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "variants": e.variants.iter().map(|v| serde_json::json!({ "name": v.name, "payload": v.payload })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let validates: Vec<serde_json::Value> = program
        .validates
        .iter()
        .map(|v| {
            serde_json::json!({
                "fn_name": v.fn_name,
                "entries": v.entries.iter().map(|(key, expr)| serde_json::json!({ "key": key, "expr": expr })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "functions": functions, "structs": structs, "enums": enums, "validates": validates })
}

/// Wraps one tool handler's structured output in the MCP `tools/call`
/// result shape -- `content[0].text` is the same JSON serialized as
/// text (the spec's own documented backward-compatibility rule for
/// `structuredContent`: "a tool that returns structured content SHOULD
/// also return the serialized JSON in a TextContent block"), and
/// `isError` stays `false` here unconditionally: a `DISPROVED`/
/// `UNKNOWN` verdict, or an `Assisted`/`Manual` (unfixable) diagnostic,
/// is a normal, successful, informative answer to the question asked
/// -- not a tool failure. `isError: true` is reserved for
/// `mcp_tools_call` never reaching this function at all (a missing
/// argument or unknown tool name is a *protocol* error instead, per
/// the spec's own two-tier error model -- see `mcp_tools_call`).
fn mcp_tool_ok(structured: serde_json::Value) -> serde_json::Value {
    let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".to_string());
    serde_json::json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured,
        "isError": false,
    })
}

/// `tools/list` -- one entry per tool `mcp_tools_call` dispatches to,
/// `nirdosha-master-plan.md` Part 3 Sprint 1's exact four: `verify_code`,
/// `get_grammar`, `fix`, `describe` (parity target: Acutis, Imandra,
/// Kōdo). Every `inputSchema` is plain JSON Schema, per spec.
fn mcp_tools_list() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {
                "name": "verify_code",
                "title": "Verify Nirdosha source",
                "description": "Run nirdosha verify's full gate pipeline (load, typecheck, ownership, validate contracts with Z3 proof obligations) against inline .nir source. Returns the same three-valued PROVED/DISPROVED/UNKNOWN verdict JSON `nirdosha verify` prints on stdout.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "Nirdosha (.nir) source code to verify" } },
                    "required": ["source"],
                },
            },
            {
                "name": "get_grammar",
                "title": "Get the Nirdosha GBNF grammar",
                "description": "Returns the full LL(1) Nirdosha grammar in GBNF form, for constrained decoding (llama.cpp/vLLM-style grammar-constrained generation) -- the exact grammar nirdosha.gbnf ships inside the compiler binary.",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "name": "fix",
                "title": "Propose or apply automated fixes",
                "description": "Runs the same pipeline as verify_code, plus a byte-offset FixPatch for any diagnostic that has one, classified auto/assisted/manual. With apply: true, writes every auto-class patch into the source and returns the patched text alongside a re-verified verdict.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "description": "Nirdosha (.nir) source code to check and, optionally, patch" },
                        "apply": { "type": "boolean", "description": "If true, apply every auto-class patch and return patched_source. Defaults to false (report only, nothing patched)." },
                    },
                    "required": ["source"],
                },
            },
            {
                "name": "describe",
                "title": "Describe a Nirdosha source file's structure",
                "description": "Parses the given source (does not require it to typecheck) and returns a curated structural summary: every fn's name/params/return type/declared effects/requires/nfr, every struct's fields, every enum's variants, and every top-level validate block's pre/post contracts.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "Nirdosha (.nir) source code to describe" } },
                    "required": ["source"],
                },
            },
        ],
    })
}

/// `tools/call` -- dispatches `params.name` to one of the four MCP
/// handlers above with `params.arguments`. A missing `name`, an
/// unknown tool name, or a missing required argument are all *protocol*
/// errors (JSON-RPC `-32602 Invalid params`, matching the phrasing
/// Kōdo's own `missing_param_error` uses) per the spec's own
/// distinction between protocol errors ("Unknown tools", "Invalid
/// arguments") and tool-execution errors (`isError: true` in a
/// successful result) -- see `mcp_tool_ok`'s doc comment for why every
/// path that reaches a handler at all comes back `isError: false`.
fn mcp_tools_call(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| (-32602, "Missing required parameter 'name'".to_string()))?;
    let empty = serde_json::json!({});
    let arguments = params.get("arguments").unwrap_or(&empty);
    let outcome = match name {
        "verify_code" => mcp_verify_code(arguments),
        "get_grammar" => mcp_get_grammar(arguments),
        "fix" => mcp_fix(arguments),
        "describe" => mcp_describe(arguments),
        other => return Err((-32602, format!("Unknown tool: {other}"))),
    };
    outcome.map(mcp_tool_ok).map_err(|message| (-32602, message))
}

/// Routes one already-parsed JSON-RPC message. Returns `None` for a
/// notification (`id` absent from the original request) -- per the MCP
/// stdio transport spec the server must never write a response for
/// one, `notifications/initialized` above all (sent right after
/// `initialize`, never expecting an answer; every tool call here is
/// already independently stateless, so there's no session flag to set
/// in response to it either).
fn mcp_dispatch(method: &str, params: &serde_json::Value, id: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let id = id?.clone();
    let result = match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "nirdosha", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(mcp_tools_list()),
        "tools/call" => mcp_tools_call(params),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

/// Writes one MCP message to `stdout` -- messages are newline-delimited
/// and **must not** contain an embedded newline (MCP stdio transport
/// spec), so this is `to_string` (compact), never `to_string_pretty`,
/// as a correctness requirement, not a style choice.
fn write_mcp_message(stdout: &mut impl std::io::Write, value: &serde_json::Value) {
    let _ = writeln!(stdout, "{}", serde_json::to_string(value).expect("an MCP response always serializes"));
    let _ = stdout.flush();
}

/// `nirdosha mcp` -- `nirdosha-master-plan.md` Part 3 Sprint 1's MCP
/// server (parity target: Acutis, Imandra, Kōdo), stdio transport
/// (JSON-RPC 2.0, newline-delimited -- the transport MCP clients
/// **SHOULD** support, and the only one that makes sense for a server
/// an MCP client launches as a subprocess rather than one serving many
/// remote clients). Reads one JSON-RPC message per line from stdin
/// until stdin closes (the client's own documented shutdown sequence:
/// close stdin, wait, `SIGTERM`, `SIGKILL` -- this loop's `for line in
/// ...lines()` ending is exactly what "stdin closes" looks like from
/// here), writes at most one response per request to stdout, and
/// writes nothing at all for a notification. Every tool call
/// (`mcp_tools_call`) reuses the identical, already-tested `verify`/
/// `fix` pipeline the CLI commands run -- this is a second transport
/// for the same logic, not a second implementation of it.
fn cmd_mcp(_args: impl Iterator<Item = String>) -> ExitCode {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in std::io::BufRead::lines(stdin.lock()) {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                write_mcp_message(
                    &mut stdout,
                    &serde_json::json!({ "jsonrpc": "2.0", "id": serde_json::Value::Null, "error": { "code": -32700, "message": format!("parse error: {e}") } }),
                );
                continue;
            }
        };
        let id = request.get("id");
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let empty_params = serde_json::Value::Null;
        let params = request.get("params").unwrap_or(&empty_params);
        if let Some(response) = mcp_dispatch(method, params, id) {
            write_mcp_message(&mut stdout, &response);
        }
    }
    ExitCode::SUCCESS
}

/// docs/goal.md row 9: hands back the parsed `Program` as JSON, the same
/// `Serialize`/`Deserialize`-derived shape `typeck.rs::validate_fragment`
/// expects a single `Expr` fragment in (see its doc comment) — an agent
/// or tool can round-trip a whole program's structure, or splice one
/// fragment back in for isolated re-validation. Deliberately parse-only,
/// not `typecheck_and_own`'s full pipeline: the AST of a program that
/// doesn't yet typecheck is still a legitimate thing to want to inspect
/// (e.g. debugging *why* generation went wrong), so this doesn't gate on
/// it the way `build`/`emit-llvm` do.
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

