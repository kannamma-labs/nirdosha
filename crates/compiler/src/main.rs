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
        "emit-llvm" => cmd_emit_llvm(args),
        "emit-ast" => cmd_emit_ast(args),
        "emit-ui" => cmd_emit_ui(args),
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
    eprintln!("  nirdosha emit-llvm <file.nir>       print the generated LLVM IR");
    eprintln!("  nirdosha emit-ast <file.nir>        print the parsed AST as JSON (docs/goal.md row 9)");
    eprintln!("  nirdosha emit-ui <file.nir> [-o out.html]");
    eprintln!("                                      derive a Material-styled web UI from struct/fn conventions");
}

/// Load (resolving any `use "..."` — `docs/ROADMAP.md` Track F, F2 piece 3)
/// -> typecheck -> ownership-check, shared by `build` and `emit-llvm` —
/// same static gates `nirdosha::run` applies before ever interpreting,
/// applied here before ever generating code. Codegen's own
/// `check_supported` (a third, narrower gate — signed-integer/bool/unit
/// only, no `box`/`&`/`*`) runs separately, inside `codegen::build`/
/// `emit_llvm_ir` themselves, since it's specific to this backend, not a
/// property of the language generally. Returns the entry file's own
/// source alongside the (possibly multi-file-merged) `Program` — the
/// one caller that still needs it directly is `cmd_sandbox_worker`
/// (`Interpreter::new`'s own `source` argument); every other caller
/// just uses the `Program`.
fn typecheck_and_own(path: &str) -> Result<(nirdosha::ast::Program, String), String> {
    typecheck_and_own_impl(path, true)
}

/// Same as `typecheck_and_own`, but does not require a `fn main()` — for
/// commands that never execute an entrypoint (`serve`, `emit-ui`,
/// `--sandbox-worker`; see `typeck::typecheck_optional_main`'s doc
/// comment for why each of those doesn't need one).
fn typecheck_and_own_optional_main(path: &str) -> Result<(nirdosha::ast::Program, String), String> {
    typecheck_and_own_impl(path, false)
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

fn typecheck_and_own_impl(path: &str, require_main: bool) -> Result<(nirdosha::ast::Program, String), String> {
    let (program, src) = nirdosha::loader::load_program(path)?;
    let type_result =
        if require_main { nirdosha::typeck::typecheck(&program) } else { nirdosha::typeck::typecheck_optional_main(&program) };
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

fn cmd_build(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut opt = nirdosha::codegen::OptLevel::O2;
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
            other => input = Some(other.to_string()),
        }
    }
    let (Some(path), Some(out)) = (input, output) else {
        eprintln!("usage: nirdosha build <file.nir> -o <out> [--opt0]");
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
    match nirdosha::codegen::build(&program, &smt_report, std::path::Path::new(&out), opt) {
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

fn cmd_emit_ui(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut theme_path: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next(),
            "--theme" => theme_path = args.next(),
            other => input = Some(other.to_string()),
        }
    }
    let Some(path) = input else {
        eprintln!("usage: nirdosha emit-ui <file.nir> [-o out.html] [--theme theme.json]");
        return ExitCode::FAILURE;
    };
    let (program, _src) = match typecheck_and_own_optional_main(&path) {
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
    let html = nirdosha::ui_gen::generate(&program, &effects, None, false, false, false, theme.as_ref());
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

