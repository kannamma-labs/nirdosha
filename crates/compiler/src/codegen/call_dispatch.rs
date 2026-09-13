use super::*;
use super::layout::*;
use super::checks::*;

/// WGS84 ellipsoid constants — mirrors `interpreter.rs`'s own
/// `WGS84_A`/`WGS84_F`/`wgs84_e2()` exactly (same values, same derived
/// `e2` formula), needed independently here since codegen computes these
/// geometry builtins as inline IR rather than calling back into
/// `interpreter.rs`'s Rust functions.
pub(super) const WGS84_A: f64 = 6_378_137.0;

pub(super) const WGS84_F: f64 = 1.0 / 298.257_223_563;

pub(super) const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);


impl Codegen<'_> {
    pub(super) fn call(&mut self, name: &str, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        // `name` is a local variable holding a `Ty::Fn` value (an
        // acquired or passed-in first-class function), not a top-level
        // `fn` — `f(x)` where `f` is such a variable parses to the same
        // `Expr::Call("f", ...)` shape a direct call does
        // (`parser.rs::parse_call`), so this has to be checked before
        // anything below assumes `name` names a global. Only reached for
        // a non-aggregate `sig_ret` (`local_ty_of`'s own `Ty::Fn` fork
        // routes an aggregate-returning one to `call_ptr` instead).
        if let Some((Ty::Fn(params, ret), fn_slot)) = scopes.get(name) {
            let fn_ptr = self.fresh_reg(&format!("{name}.fnval"));
            writeln!(self.out, "  {fn_ptr} = load ptr, ptr {fn_slot}").unwrap();
            return self.call_indirect(&fn_ptr, &params, &ret, args, scopes);
        }
        // Row 11: a struct/variant constructor is always aggregate-valued
        // (`is_aggregate()` now covers `Ty::Named`), so a scalar `expr()`
        // result is the wrong shape for it — every well-typed caller
        // already routed it to `expr_ptr`/`expr_ptr_expected` via
        // `local_ty_of`'s `is_aggregate()` fork. This guard keeps the
        // scalar path a clean `CodegenError` (not the `sigs.get(name)
        // .expect(...)` panic a ctor name would otherwise hit below) if
        // some future construct defies that routing — the same
        // defense-in-depth shape `Expr::Ident`/`Expr::Assign`'s own
        // aggregate guards already use.
        if self.registry.is_struct(name) || self.registry.find_variant(name).is_some() {
            return unsupported(format!(
                "codegen doesn't support constructing `{name}` in this scalar expression position yet — a struct/enum value is aggregate; bind it via `let`, or pass/return it through a function call"
            ));
        }
        // `check_supported` (`check_expr`'s `Expr::Call` arm, above)
        // already rejected every builtin except `print`,
        // `STR_CRYPTO_BUILTINS`, `RAND_BUILTINS`, and `PHASE4_BUILTINS`/
        // `PHASE5_BUILTINS`' names before this ever runs -- explicit
        // `== "print"`/`== "sha256_hex"`/etc. checks here, not
        // `is_builtin`, state that invariant directly rather than
        // leaning on it silently.
        if name == "print" {
            for a in args {
                // `local_ty_of` picks the right `printf` format
                // string/vararg type: `double` for `f64`, the `{ptr,
                // i64}` two-word convention for `str`, `i1`-widened for
                // `bool` (a bare bool variable, a bool literal, and a
                // comparison result `x > y` all resolve to `Ty::Bool`
                // here identically -- `local_ty_of`'s `Expr::Binary` arm
                // already maps every comparison/`&&`/`||` operator to
                // `Ty::Bool`, so there's exactly one bool-shaped case to
                // handle, not several), a fixed `"()"` string for `unit`
                // (there's no `unit` *literal* syntax -- the only way to
                // produce a unit-typed argument is a call to a `-> unit`
                // function; its side effect still has to run, so `expr()`
                // below is still called unconditionally, its nominal
                // result just isn't a meaningful value to print), and
                // plain `i64` for everything else.
                let arg_ty = self.local_ty_of(a, scopes);
                if arg_ty.is_aggregate() {
                    // Printing a whole Vector/Matrix isn't built yet —
                    // this purely-syntactic pre-pass (`check_expr`) has
                    // no type info to catch it earlier, so it's caught
                    // here instead, with a specific reason rather than
                    // falling through to a scalar `expr()` call that
                    // would fail confusingly.
                    return unsupported(
                        "codegen doesn't support `print` on a Vector/Matrix argument yet — \
                         only integer/f64/str/bool/unit-typed arguments are supported so far",
                    );
                }
                let v = self.expr(a, scopes)?;
                if arg_ty == Ty::F64 {
                    writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.float_fmt, double {v})").unwrap();
                } else if arg_ty == Ty::Str || arg_ty == Ty::Json {
                    // `%.*s`, not `%s` — the buffer isn't guaranteed
                    // NUL-terminated by this design (`Ty::Str`'s note in
                    // `llvm_ty`), so the explicit length has to drive how
                    // many bytes `printf` reads, not a NUL scan. `Ty::Json`
                    // rides the same branch: it's the identical `{ptr,
                    // i64}` raw-JSON-text representation (`Ty::Json`'s own
                    // `llvm_ty` doc comment), so `print(a_json_value)` — a
                    // `db_query`/`list_<workflow>_overdue()` result, say —
                    // prints its real JSON text, not a wrong `i64`-shaped
                    // read of a two-word struct (which is what falling
                    // through to the `else` arm below would have done).
                    // Found by actually trying to `print` a `json` value,
                    // not designed in advance.
                    let ptr_reg = self.fresh_reg("str_print_ptr");
                    writeln!(self.out, "  {ptr_reg} = extractvalue {{ptr, i64}} {v}, 0").unwrap();
                    let len_reg = self.fresh_reg("str_print_len");
                    writeln!(self.out, "  {len_reg} = extractvalue {{ptr, i64}} {v}, 1").unwrap();
                    let len_i32 = self.fresh_reg("str_print_len_i32");
                    writeln!(self.out, "  {len_i32} = trunc i64 {len_reg} to i32").unwrap();
                    writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.str_fmt, i32 {len_i32}, ptr {ptr_reg})").unwrap();
                } else if arg_ty == Ty::Bool {
                    // Prints `1`/`0`, not `interpreter.rs`'s `render()`
                    // `"true"`/`"false"` — a real, honest cosmetic
                    // difference between the two execution paths (same
                    // class as `@.float_fmt`'s `%f`-vs-Rust-formatting
                    // note above), not a semantic one: both agree on
                    // which of the two boolean values it is.
                    let widened = self.fresh_reg("bool_as_i64");
                    writeln!(self.out, "  {widened} = zext i1 {v} to i64").unwrap();
                    writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.int_fmt, i64 {widened})").unwrap();
                } else if arg_ty == Ty::Unit {
                    // `v` (the call's nominal "result") carries no real
                    // data for a `void`-returning callee — deliberately
                    // ignored. `interpreter.rs`'s `render()` prints
                    // `"()"` for `Value::Unit`; match that exactly so
                    // interpreted/compiled output agree.
                    let _ = v;
                    writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.unit_fmt)").unwrap();
                } else {
                    writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.int_fmt, i64 {v})").unwrap();
                }
            }
            return Ok("0".to_string()); // print's own "value" is unit; never read
        }
        // `sha256_hex`/`constant_time_str_eq` — linked calls into
        // `runtime-kernels/src/lib.rs`'s from-scratch SHA-256 (`STR_CRYPTO_BUILTINS`'
        // doc comment on why these two don't go through
        // `call_builtin_scalar`/`call_builtin_agg` like `PHASE4`/
        // `PHASE5_BUILTINS` do).
        if name == "sha256_hex" {
            let a = self.expr(&args[0], scopes)?;
            let a_ptr = self.fresh_reg("sha256_a_ptr");
            writeln!(self.out, "  {a_ptr} = extractvalue {{ptr, i64}} {a}, 0").unwrap();
            let a_len = self.fresh_reg("sha256_a_len");
            writeln!(self.out, "  {a_len} = extractvalue {{ptr, i64}} {a}, 1").unwrap();
            // The 1-arg form passes a null `b_ptr`/`0` `b_len` -- the
            // kernel never dereferences `b_ptr` when `b_len` is 0 (its
            // own doc comment), so an absent second argument needs no
            // real buffer, just these two placeholder values.
            let (b_ptr, b_len) = if args.len() == 2 {
                let b = self.expr(&args[1], scopes)?;
                let b_ptr = self.fresh_reg("sha256_b_ptr");
                writeln!(self.out, "  {b_ptr} = extractvalue {{ptr, i64}} {b}, 0").unwrap();
                let b_len = self.fresh_reg("sha256_b_len");
                writeln!(self.out, "  {b_len} = extractvalue {{ptr, i64}} {b}, 1").unwrap();
                (b_ptr, b_len)
            } else {
                ("null".to_string(), "0".to_string())
            };
            // 64 bytes, always -- a hex-encoded SHA-256 digest is a
            // fixed size, never data-dependent. Heap-allocated and never
            // freed (`nir_sha256_hex`'s own doc comment on why: `Ty::Str`
            // isn't affine, so there's no scope-closing point to hook a
            // matching `nir_free` onto).
            let out_ptr = self.fresh_reg("sha256_out");
            writeln!(self.out, "  {out_ptr} = call ptr @nir_alloc(i64 64)").unwrap();
            writeln!(
                self.out,
                "  call void @nir_sha256_hex(ptr {a_ptr}, i64 {a_len}, ptr {b_ptr}, i64 {b_len}, ptr {out_ptr})"
            )
            .unwrap();
            let partial = self.fresh_reg("sha256_str_partial");
            writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {out_ptr}, 0").unwrap();
            let result = self.fresh_reg("sha256_str");
            writeln!(self.out, "  {result} = insertvalue {{ptr, i64}} {partial}, i64 64, 1").unwrap();
            return Ok(result);
        }
        if name == "constant_time_str_eq" {
            let a = self.expr(&args[0], scopes)?;
            let b = self.expr(&args[1], scopes)?;
            let a_ptr = self.fresh_reg("cteq_a_ptr");
            writeln!(self.out, "  {a_ptr} = extractvalue {{ptr, i64}} {a}, 0").unwrap();
            let a_len = self.fresh_reg("cteq_a_len");
            writeln!(self.out, "  {a_len} = extractvalue {{ptr, i64}} {a}, 1").unwrap();
            let b_ptr = self.fresh_reg("cteq_b_ptr");
            writeln!(self.out, "  {b_ptr} = extractvalue {{ptr, i64}} {b}, 0").unwrap();
            let b_len = self.fresh_reg("cteq_b_len");
            writeln!(self.out, "  {b_len} = extractvalue {{ptr, i64}} {b}, 1").unwrap();
            let raw = self.fresh_reg("cteq_raw");
            writeln!(
                self.out,
                "  {raw} = call i32 @nir_constant_time_str_eq(ptr {a_ptr}, i64 {a_len}, ptr {b_ptr}, i64 {b_len})"
            )
            .unwrap();
            return self.icmp("ne", "i32", &raw, "0");
        }
        // `str_slice`/`str_index_of` (`STR_BUILTINS`'s own doc comment) —
        // `str_slice` is pure pointer arithmetic on the existing `{ptr,
        // i64}` representation, no kernel call; `str_index_of` is the one
        // genuine byte-scan, linked to `nir_str_index_of`.
        if name == "str_slice" {
            let (s_ptr, s_len) = self.str_parts(&args[0], scopes)?;
            let start = self.expr(&args[1], scopes)?;
            let end = self.expr(&args[2], scopes)?;
            self.guard_str_bounds_ok(&start, &end, &s_len);
            let new_ptr = self.fresh_reg("slice_ptr");
            writeln!(self.out, "  {new_ptr} = getelementptr i8, ptr {s_ptr}, i64 {start}").unwrap();
            let new_len = self.fresh_reg("slice_len");
            writeln!(self.out, "  {new_len} = sub i64 {end}, {start}").unwrap();
            let partial = self.fresh_reg("slice_str_partial");
            writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {new_ptr}, 0").unwrap();
            let result = self.fresh_reg("slice_str");
            writeln!(self.out, "  {result} = insertvalue {{ptr, i64}} {partial}, i64 {new_len}, 1").unwrap();
            return Ok(result);
        }
        if name == "str_index_of" {
            let (hay_ptr, hay_len) = self.str_parts(&args[0], scopes)?;
            let (needle_ptr, needle_len) = self.str_parts(&args[1], scopes)?;
            let idx = self.fresh_reg("str_index_of");
            writeln!(
                self.out,
                "  {idx} = call i64 @nir_str_index_of(ptr {hay_ptr}, i64 {hay_len}, ptr {needle_ptr}, i64 {needle_len})"
            )
            .unwrap();
            return Ok(idx);
        }
        // `identity_expired(identity, now) -> bool` — no kernel at all,
        // unlike its `IDENTITY_BUILTINS` siblings: `VerifiedIdentity.
        // expires_at` is a plain `i64` field, so this is a GEP+load+
        // `icmp`, unconditionally inline, the same "no linked call
        // needed" shape `len(Vector)` already has for a different
        // reason (there it's compile-time-known; here it's one memory
        // read).
        if name == "identity_expired" {
            let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
            let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
            let (idx, _) = self.field_index_and_ty(&identity_ty, "expires_at").expect("VerifiedIdentity always has expires_at, ast::prelude_structs");
            let identity_llty = self.llvm_ty(&identity_ty)?;
            let field_ptr = self.fresh_reg("identity_expires_at_ptr");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {idx}").unwrap();
            let expires_at = self.fresh_reg("identity_expires_at");
            writeln!(self.out, "  {expires_at} = load i64, ptr {field_ptr}").unwrap();
            let now = self.expr(&args[1], scopes)?;
            return self.icmp("sgt", "i64", &now, &expires_at);
        }
        if name == "check_revocation" {
            return self.emit_check_revocation(args, scopes);
        }
        if name == "session_cookie" {
            return self.emit_session_cookie(args, scopes);
        }
        if name == "rand_seed" {
            // Every integer-typed `expr()` result is already `i64`
            // (module doc) regardless of `rand_seed`'s argument's own
            // declared width (`typeck.rs` accepts any integer type) --
            // no extra widening needed here.
            let seed = self.expr(&args[0], scopes)?;
            writeln!(self.out, "  call void @nir_rand_seed(i64 {seed})").unwrap();
            return Ok("0".to_string()); // rand_seed's own "value" is unit; never read
        }
        if name == "sleep_ms" {
            let ms = self.expr(&args[0], scopes)?;
            writeln!(self.out, "  call void @nir_sleep_ms(i64 {ms})").unwrap();
            return Ok("0".to_string()); // sleep_ms's own "value" is unit; never read
        }
        if name == "rand_f64" {
            let r = self.fresh_reg("rand_f64");
            writeln!(self.out, "  {r} = call double @nir_rand_f64()").unwrap();
            return Ok(r);
        }
        if name == "rand_gaussian" {
            let mean = self.expr(&args[0], scopes)?;
            let stddev = self.expr(&args[1], scopes)?;
            let r = self.fresh_reg("rand_gaussian");
            writeln!(self.out, "  {r} = call double @nir_rand_gaussian(double {mean}, double {stddev})").unwrap();
            return Ok(r);
        }
        // `dec_from_i64`/`dec_to_str` — linked calls into
        // `runtime-kernels/src/lib.rs`'s `rust_decimal`-backed kernels
        // (`DEC128_BUILTINS`' own doc comment). `dec128`'s LLVM shape is
        // the plain two-word value `{i64, i64}` (`llvm_ty`'s `Ty::Dec128`
        // arm) -- passed/returned by value here exactly like `Ty::Str`'s
        // own `{ptr, i64}` already is, never through a pointer.
        if name == "dec_from_i64" {
            // Every integer-typed `expr()` result is already `i64`
            // (module doc) regardless of the argument's own declared
            // width -- `scale`'s declared `u32` narrows the same way
            // `rand_seed`'s argument already does, no extra handling
            // needed beyond the narrow itself.
            let value = self.expr(&args[0], scopes)?;
            let scale64 = self.expr(&args[1], scopes)?;
            let scale32 = self.narrow_from_i64(&scale64, &Ty::U32)?;
            let r = self.fresh_reg("dec_from_i64");
            writeln!(self.out, "  {r} = call {{i64, i64}} @nir_dec128_from_i64(i64 {value}, i32 {scale32})").unwrap();
            return Ok(r);
        }
        if name == "dec_to_str" {
            let d = self.expr(&args[0], scopes)?;
            // 64 bytes, always -- `runtime-kernels/src/lib.rs`'s own
            // `nir_dec128_to_str` doc comment: a dec128's longest
            // possible `Display` string is well under this. Heap-
            // allocated and never freed, same as `sha256_hex`'s own
            // output buffer above, for the identical reason (`Ty::Str`
            // isn't affine, so there's no scope-closing point to hook a
            // matching `nir_free` onto).
            let out_ptr = self.fresh_reg("dec_to_str_out");
            writeln!(self.out, "  {out_ptr} = call ptr @nir_alloc(i64 64)").unwrap();
            let len = self.fresh_reg("dec_to_str_len");
            writeln!(self.out, "  {len} = call i64 @nir_dec128_to_str({{i64, i64}} {d}, ptr {out_ptr}, i64 64)").unwrap();
            // `nir_dec128_to_str` only ever returns `-1` if the 64-byte
            // buffer was too small, which the kernel's own doc comment
            // already argues never happens in practice -- `guard_io_ok`
            // (traps only on negative) is the honest backstop for that
            // "shouldn't happen but stay checked" case, same posture
            // every other linked kernel's unexpected-failure path gets.
            self.guard_io_ok(&len);
            let partial = self.fresh_reg("dec_to_str_partial");
            writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {out_ptr}, 0").unwrap();
            let result = self.fresh_reg("dec_to_str_result");
            writeln!(self.out, "  {result} = insertvalue {{ptr, i64}} {partial}, i64 {len}, 1").unwrap();
            return Ok(result);
        }
        if name == "dec_round" {
            let d = self.expr(&args[0], scopes)?;
            let scale64 = self.expr(&args[1], scopes)?;
            let scale32 = self.narrow_from_i64(&scale64, &Ty::U32)?;
            let r = self.fresh_reg("dec_round");
            writeln!(self.out, "  {r} = call {{i64, i64}} @nir_dec128_round({{i64, i64}} {d}, i32 {scale32})").unwrap();
            return Ok(r);
        }
        if name == "dec_scale" {
            let d = self.expr(&args[0], scopes)?;
            let r = self.fresh_reg("dec_scale");
            writeln!(self.out, "  {r} = call i64 @nir_dec128_scale({{i64, i64}} {d})").unwrap();
            return Ok(r);
        }
        if PHASE4_BUILTINS.contains(&name) || name == "det" || name == "rank" {
            return self.call_builtin_scalar(name, args, scopes);
        }
        // User-defined call. `typeck.rs` already required this to
        // resolve and every argument to either exactly match or (for a
        // literal) fit its parameter's declared type — the sigs table
        // is what lets codegen honor that at the LLVM level too, where
        // a call instruction's argument types must match the callee's
        // `define` exactly, byte for byte.
        let sig_params = self.sigs.get(name).expect("typeck.rs already resolved this call").params.clone();
        let sig_ret = self.sigs.get(name).expect("typeck.rs already resolved this call").ret.clone();
        if sig_ret.is_aggregate() {
            // Every well-behaved caller already checked the callee's
            // return type and used `expr_ptr()` (→ `call_ptr()`)
            // instead — same defense-in-depth shape as the `Expr::Ident`/
            // `Expr::Assign` guards above.
            return unsupported(format!(
                "codegen doesn't support calling `{name}` (which returns a Vector/Matrix) in \
                 this expression position yet"
            ));
        }

        let arg_vals = self.call_args(args, &sig_params, scopes)?;

        let ret_llty = self.llvm_ty(&sig_ret)?;
        if ret_llty == "void" {
            writeln!(self.out, "  call void @{name}({})", arg_vals.join(", ")).unwrap();
            Ok("0".to_string()) // unit result; never read by a well-typed caller
        } else {
            let r = self.fresh_reg("call_result");
            writeln!(self.out, "  {r} = call {ret_llty} @{name}({})", arg_vals.join(", ")).unwrap();
            // The call instruction itself is correctly typed at the
            // callee's *declared* return width (LLVM requires that) —
            // but every other `expr()` result for an integer type is
            // `i64` (module doc), and this one has to honor that same
            // invariant too, or a caller like `Stmt::Let`'s
            // `guard_in_range` (which always compares at `i64`) sees a
            // value narrower than it expects. Found the same way as the
            // `add i8` wraparound bug: by actually building `hello.nir`
            // after the i64-everywhere fix landed and reading clang's
            // "defined with type 'i32' but expected 'i64'" error, not by
            // re-reading the code and reasoning it through in advance.
            Ok(self.widen_to_i64(&r, &sig_ret))
        }
    }


    /// Shared argument-evaluation loop for both `call()` (scalar/void
    /// return) and `call_ptr()` (aggregate return) — every argument's
    /// handling depends only on its own declared parameter type, never
    /// on the callee's return type, so there's exactly one place this
    /// logic needs to live. Args are evaluated left to right, matching
    /// `interpreter.rs`'s evaluation order.
    pub(super) fn call_args(&mut self, args: &[Expr], sig_params: &[Ty], scopes: &mut Scopes) -> Result<Vec<String>, CodegenError> {
        let mut arg_vals = Vec::with_capacity(args.len());
        for (a, want) in args.iter().zip(sig_params.iter()) {
            if want.is_aggregate() {
                // Passed by pointer — no copy at the call site itself;
                // the callee's own prologue does the copy-in (`function`'s
                // doc comment), so the caller can hand over whichever
                // pointer `expr_ptr` already has (a variable's own
                // storage, or a fresh literal's). A constructor argument
                // is built with `want` (this parameter's declared type)
                // as its expected type via `expr_ptr_expected`, so an
                // `Option(i64)` parameter receives a `None`/`Some(5)`
                // argument constructed against exactly that instantiation.
                let ptr = self.expr_ptr_expected(a, want, scopes)?;
                arg_vals.push(format!("ptr {ptr}"));
                continue;
            }
            let llty = self.llvm_ty(want)?;
            let v = if let Some(n) = literal_value(a) {
                // A literal (or negated literal) that typeck already
                // proved fits `want`'s range — emit it directly at
                // `want`'s width as a bare constant, no instruction
                // needed (this is the fix for the bug found by actually
                // inspecting the first generated .ll file: an earlier
                // draft ran `-3` through a real `sub i64 0, 3` and then
                // tried to pass the resulting *i64* register where an
                // `i32` parameter was declared — a genuine LLVM type
                // mismatch, not a hypothetical one).
                n.to_string()
            } else {
                // Not a literal — typeck's exact-match rule guarantees
                // this expression's own *declared* Nirdosha type already
                // equals `want`, but `expr()` itself always hands back an
                // `i64` for any integer type (module doc), so it still
                // needs narrowing to `want`'s actual LLVM width before
                // it can be passed at a call site. Lossless: the value
                // was already proven to fit `want` when it was originally
                // bound (that's what `guard_in_range` did at its own
                // `let`/assign site) — this narrow can't newly overflow.
                let val64 = self.expr(a, scopes)?;
                if want.is_integer() {
                    self.narrow_from_i64(&val64, want)?
                } else {
                    val64
                }
            };
            arg_vals.push(format!("{llty} {v}"));
        }
        Ok(arg_vals)
    }


    /// Calls through an already-loaded function-pointer *value*
    /// (`fn_ptr` — a register holding the address, not a variable's own
    /// storage slot) — the shared implementation `call()`'s and
    /// `call_ptr()`'s own "`name` is a local `Ty::Fn` variable" branches
    /// both route through. The only real difference from an ordinary
    /// direct call (`call <ret> @name(...)`) is the callee operand (a
    /// loaded `ptr` register instead of a named global) and having to
    /// spell out the full `<ret>(<params>)` function type at the call
    /// site — LLVM requires this for any indirect call, since the
    /// callee isn't a named `@fn` whose own `define` already states it
    /// (the exact same `call <functy> <callee>(<args>)` shape this
    /// module already uses for `@printf`'s varargs signature).
    pub(super) fn call_indirect(
        &mut self,
        fn_ptr: &str,
        params: &[Ty],
        ret: &Ty,
        args: &[Expr],
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        let arg_vals = self.call_args(args, params, scopes)?;
        let param_lltys: Vec<String> =
            params.iter().map(|p| if p.is_aggregate() { Ok("ptr".to_string()) } else { self.llvm_ty(p) }).collect::<Result<_, _>>()?;
        if ret.is_aggregate() {
            let agg_llty = self.llvm_ty(ret)?;
            let dest = self.fresh_reg("indirect_call_result.addr");
            self.emit_alloca(&dest, &agg_llty);
            let mut all_param_lltys = vec!["ptr".to_string()];
            all_param_lltys.extend(param_lltys);
            let mut all_args = vec![format!("ptr {dest}")];
            all_args.extend(arg_vals);
            writeln!(self.out, "  call void ({}) {fn_ptr}({})", all_param_lltys.join(", "), all_args.join(", ")).unwrap();
            Ok(dest)
        } else {
            let ret_llty = self.llvm_ty(ret)?;
            if ret_llty == "void" {
                writeln!(self.out, "  call void ({}) {fn_ptr}({})", param_lltys.join(", "), arg_vals.join(", ")).unwrap();
                Ok("0".to_string())
            } else {
                let r = self.fresh_reg("indirect_call_result");
                writeln!(self.out, "  {r} = call {ret_llty} ({}) {fn_ptr}({})", param_lltys.join(", "), arg_vals.join(", ")).unwrap();
                Ok(self.widen_to_i64(&r, ret))
            }
        }
    }


    /// Every Phase-4 builtin that yields a plain scalar (`f64`/`i64`/
    /// `bool`) result — reached from `call()`'s `expr()` path (aggregate-
    /// returning builtins are `call_builtin_agg`'s job instead). Each
    /// mirrors its `interpreter.rs` implementation's exact accumulation
    /// order: "first term computed directly, then each remaining term
    /// folded in left-to-right" is bit-identical to `interpreter.rs`'s
    /// own `iter().sum()`/`.fold(0.0, ...)`-based versions specifically
    /// *because* `0.0 + x == x` and `f64::max(0.0, |x|) == |x|` are both
    /// exact (no rounding) for any finite `x` — not a coincidence this
    /// phase leans on, a real IEEE 754 identity.
    pub(super) fn call_builtin_scalar(&mut self, name: &str, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        match name {
            "dot" => {
                let a_ty = self.local_ty_of(&args[0], scopes);
                let (elem, len) = agg_elem_and_len(&a_ty);
                let (elem, len) = (elem.clone(), len);
                let elem_llty = self.llvm_ty(&elem)?;
                let is_float = elem == Ty::F64;
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let b_ptr = self.expr_ptr(&args[1], scopes)?;
                let a0 = self.agg_load_elem(&a_ptr, &elem_llty, &elem, 0);
                let b0 = self.agg_load_elem(&b_ptr, &elem_llty, &elem, 0);
                let mut acc = self.emit_mul(&a0, &b0, is_float);
                for i in 1..len {
                    let ai = self.agg_load_elem(&a_ptr, &elem_llty, &elem, i);
                    let bi = self.agg_load_elem(&b_ptr, &elem_llty, &elem, i);
                    let prod = self.emit_mul(&ai, &bi, is_float);
                    acc = self.emit_add(&acc, &prod, is_float);
                }
                Ok(acc)
            }
            "sum" => {
                let a_ty = self.local_ty_of(&args[0], scopes);
                let (elem, len) = agg_elem_and_len(&a_ty);
                let (elem, len) = (elem.clone(), len);
                let elem_llty = self.llvm_ty(&elem)?;
                let is_float = elem == Ty::F64;
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let mut acc = self.agg_load_elem(&a_ptr, &elem_llty, &elem, 0);
                for i in 1..len {
                    let v = self.agg_load_elem(&a_ptr, &elem_llty, &elem, i);
                    acc = self.emit_add(&acc, &v, is_float);
                }
                Ok(acc)
            }
            "len" => match self.local_ty_of(&args[0], scopes) {
                // A `Vector`'s length is baked into its `Ty`, known at
                // codegen time -- no load at all, genuinely O(1).
                Ty::Vector(_, n) => Ok(n.to_string()),
                // A `str`'s length is a runtime value (the second field
                // of its `{ptr, i64}` representation), not a compile-time
                // constant like Vector's -- one `extractvalue`, same as
                // `str_parts`'s own len half.
                Ty::Str => {
                    let (_, len) = self.str_parts(&args[0], scopes)?;
                    Ok(len)
                }
                _ => unreachable!("typeck.rs already proved this is a Vector or str"),
            },
            "norm" | "frobenius_norm" => {
                let (_, len) = agg_elem_and_len(&self.local_ty_of(&args[0], scopes));
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let x0 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let mut sum_sq = self.emit_mul(&x0, &x0, true);
                for i in 1..len {
                    let xi = self.agg_load_elem(&a_ptr, "double", &Ty::F64, i);
                    let sq = self.emit_mul(&xi, &xi, true);
                    sum_sq = self.emit_add(&sum_sq, &sq, true);
                }
                Ok(self.emit_call1("@llvm.sqrt.f64", &sum_sq))
            }
            "norm1" => {
                let (_, len) = agg_elem_and_len(&self.local_ty_of(&args[0], scopes));
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let x0 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let mut acc = self.emit_call1("@llvm.fabs.f64", &x0);
                for i in 1..len {
                    let xi = self.agg_load_elem(&a_ptr, "double", &Ty::F64, i);
                    let abs_xi = self.emit_call1("@llvm.fabs.f64", &xi);
                    acc = self.emit_add(&acc, &abs_xi, true);
                }
                Ok(acc)
            }
            "norm_inf" => {
                let (_, len) = agg_elem_and_len(&self.local_ty_of(&args[0], scopes));
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let x0 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let mut acc = self.emit_call1("@llvm.fabs.f64", &x0);
                for i in 1..len {
                    let xi = self.agg_load_elem(&a_ptr, "double", &Ty::F64, i);
                    let abs_xi = self.emit_call1("@llvm.fabs.f64", &xi);
                    acc = self.emit_call2("@llvm.maxnum.f64", &acc, &abs_xi);
                }
                Ok(acc)
            }
            "trace" => {
                let Ty::Matrix(elem, n, _) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a square Matrix")
                };
                let elem_llty = self.llvm_ty(&elem)?;
                let is_float = *elem == Ty::F64;
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let mut acc = self.agg_load_elem(&a_ptr, &elem_llty, &elem, 0);
                for i in 1..n {
                    let v = self.agg_load_elem(&a_ptr, &elem_llty, &elem, i * n + i);
                    acc = self.emit_add(&acc, &v, is_float);
                }
                Ok(acc)
            }
            "distance" => {
                let (_, len) = agg_elem_and_len(&self.local_ty_of(&args[0], scopes));
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let b_ptr = self.expr_ptr(&args[1], scopes)?;
                let a0 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let b0 = self.agg_load_elem(&b_ptr, "double", &Ty::F64, 0);
                let d0 = self.emit_sub(&a0, &b0);
                let mut sum_sq = self.emit_mul(&d0, &d0, true);
                for i in 1..len {
                    let ai = self.agg_load_elem(&a_ptr, "double", &Ty::F64, i);
                    let bi = self.agg_load_elem(&b_ptr, "double", &Ty::F64, i);
                    let di = self.emit_sub(&ai, &bi);
                    let sq = self.emit_mul(&di, &di, true);
                    sum_sq = self.emit_add(&sum_sq, &sq, true);
                }
                Ok(self.emit_call1("@llvm.sqrt.f64", &sum_sq))
            }
            "bearing" => {
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let b_ptr = self.expr_ptr(&args[1], scopes)?;
                let (lat1, lon1, lat2, lon2, deg) = self.bearing_deg(&a_ptr, &b_ptr);
                let _ = (lat1, lon1, lat2, lon2);
                Ok(deg)
            }
            "is_symmetric" | "is_diag" => {
                let Ty::Matrix(_, n, _) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a square Matrix(f64,_,_)")
                };
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let mut acc: Option<String> = None;
                for i in 0..n {
                    for j in 0..n {
                        // `is_diag`'s `i == j` cells are trivially true
                        // (`interpreter.rs`'s own `i == j ||` short-
                        // circuit) — a compile-time-known fact per
                        // unrolled iteration here, so skip emitting any
                        // comparison for them at all rather than
                        // computing `elems[i*n+i] == elems[i*n+i]`.
                        if name == "is_diag" && i == j {
                            continue;
                        }
                        let lhs = self.agg_load_elem(&a_ptr, "double", &Ty::F64, i * n + j);
                        let rhs = if name == "is_symmetric" {
                            self.agg_load_elem(&a_ptr, "double", &Ty::F64, j * n + i)
                        } else {
                            Self::float_const(0.0)
                        };
                        let eq = self.fcmp("oeq", &lhs, &rhs)?;
                        acc = Some(match acc {
                            None => eq,
                            Some(prev) => {
                                let out = self.fresh_reg("builtin_and");
                                writeln!(self.out, "  {out} = and i1 {prev}, {eq}").unwrap();
                                out
                            }
                        });
                    }
                }
                // A 1x1 Matrix is both symmetric and diagonal trivially --
                // `is_diag`'s loop skips its only (i==j) cell entirely, so
                // `acc` would otherwise stay `None`; `is_symmetric`'s only
                // cell compares `elems[0]` to itself, always `Some`. Both
                // are correctly `true` either way.
                Ok(acc.unwrap_or_else(|| "true".to_string()))
            }
            "is_square" => {
                let Ty::Matrix(_, r, c) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a Matrix")
                };
                // Genuinely O(1): a `Matrix`'s shape is baked into its
                // `Ty`, so this is decidable at codegen time -- no load,
                // no comparison instruction, unlike every other builtin
                // here.
                Ok(if r == c { "true".to_string() } else { "false".to_string() })
            }
            // Phase 5: genuine data-dependent control flow (partial-pivot
            // row selection) — a linked native `call` into
            // `runtime-kernels/src/lib.rs`'s staticlib instead of unrolled IR
            // (module doc's `PHASE5_BUILTINS` note, and `build()`'s
            // embedded-lib linking). Neither is fallible: `det` returns
            // `0.0` for a singular matrix (a real, legitimate answer,
            // matching `interpreter.rs::matrix_det`'s own contract) and
            // `rank`'s row-echelon reduction never fails outright.
            "det" => {
                let Ty::Matrix(_, n, _) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a square Matrix(f64,_,_)")
                };
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let out = self.fresh_reg("det_result");
                writeln!(self.out, "  {out} = call double @nir_det(ptr {a_ptr}, i64 {n})").unwrap();
                Ok(out)
            }
            "rank" => {
                let Ty::Matrix(_, rows, cols) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a Matrix(f64,_,_)")
                };
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let out = self.fresh_reg("rank_result");
                writeln!(self.out, "  {out} = call i64 @nir_rank(ptr {a_ptr}, i64 {rows}, i64 {cols})").unwrap();
                Ok(out)
            }
            _ => unreachable!("PHASE4_BUILTINS'/PHASE5_BUILTINS' aggregate-returning names go through call_builtin_agg instead"),
        }
    }


    /// `bearing`'s formula, factored out so `call_builtin_scalar`'s match
    /// arm stays short — returns `(lat1_rad, lon1_rad, lat2_rad, lon2_rad,
    /// result_deg)`; only the last is actually used by `bearing` itself,
    /// but returning the intermediates avoids a second, pointless
    /// abstraction boundary for a function with exactly one real caller.
    /// Mirrors `interpreter.rs::bearing_deg` exactly, including its
    /// `(deg + 360.0) % 360.0` final wrap (`frem`, IEEE remainder --
    /// matching Rust's `f64::rem` semantics `%` already uses there).
    pub(super) fn bearing_deg(&mut self, from_ptr: &str, to_ptr: &str) -> (String, String, String, String, String) {
        let deg_to_rad = Self::float_const(std::f64::consts::PI / 180.0);
        let rad_to_deg = Self::float_const(180.0 / std::f64::consts::PI);

        let lat1_deg = self.agg_load_elem(from_ptr, "double", &Ty::F64, 0);
        let lon1_deg = self.agg_load_elem(from_ptr, "double", &Ty::F64, 1);
        let lat2_deg = self.agg_load_elem(to_ptr, "double", &Ty::F64, 0);
        let lon2_deg = self.agg_load_elem(to_ptr, "double", &Ty::F64, 1);
        let lat1 = self.emit_mul(&lat1_deg, &deg_to_rad, true);
        let lon1 = self.emit_mul(&lon1_deg, &deg_to_rad, true);
        let lat2 = self.emit_mul(&lat2_deg, &deg_to_rad, true);
        let lon2 = self.emit_mul(&lon2_deg, &deg_to_rad, true);
        let dlon = self.emit_sub(&lon2, &lon1);

        let sin_dlon = self.emit_call1("@llvm.sin.f64", &dlon);
        let cos_lat2 = self.emit_call1("@llvm.cos.f64", &lat2);
        let y = self.emit_mul(&sin_dlon, &cos_lat2, true);

        let cos_lat1 = self.emit_call1("@llvm.cos.f64", &lat1);
        let sin_lat2 = self.emit_call1("@llvm.sin.f64", &lat2);
        let term1 = self.emit_mul(&cos_lat1, &sin_lat2, true);
        let sin_lat1 = self.emit_call1("@llvm.sin.f64", &lat1);
        let cos_dlon = self.emit_call1("@llvm.cos.f64", &dlon);
        let t2a = self.emit_mul(&sin_lat1, &cos_lat2, true);
        let term2 = self.emit_mul(&t2a, &cos_dlon, true);
        let x = self.emit_sub(&term1, &term2);

        let ang = self.emit_call2("@atan2", &y, &x);
        let deg = self.emit_mul(&ang, &rad_to_deg, true);
        let plus_360 = self.emit_add(&deg, &Self::float_const(360.0), true);
        let wrapped = self.fresh_reg("bearing_wrap");
        writeln!(self.out, "  {wrapped} = frem double {plus_360}, {}", Self::float_const(360.0)).unwrap();
        (lat1, lon1, lat2, lon2, wrapped)
    }


    /// Every Phase-4 builtin that yields a `Vector`/`Matrix` result —
    /// reached from `call_ptr()`'s `expr_ptr()` path. `builtin_result_ty`
    /// picks the destination's shape up front (the same function
    /// `local_ty_of` uses), so every arm below just fills it in.
    pub(super) fn call_builtin_agg(&mut self, name: &str, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = self.builtin_result_ty(name, args, scopes);
        let agg_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("builtin_result.addr");
        self.emit_alloca(&dest, &agg_llty);

        match name {
            "transpose" => {
                let Ty::Matrix(elem, rows, cols) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a Matrix")
                };
                let elem_llty = self.llvm_ty(&elem)?;
                let m_ptr = self.expr_ptr(&args[0], scopes)?;
                // Matches interpreter.rs: `for j in 0..cols { for i in
                // 0..rows { out.push(elems[i*cols+j]) } }` -- output is
                // row-major over the (cols, rows) transposed shape, so
                // out[j*rows+i] = elems[i*cols+j].
                for j in 0..cols {
                    for i in 0..rows {
                        let v = self.agg_load_elem(&m_ptr, &elem_llty, &elem, i * cols + j);
                        self.agg_store_elem(&dest, &elem_llty, &elem, j * rows + i, &v)?;
                    }
                }
            }
            "cross" => {
                let a_ty = self.local_ty_of(&args[0], scopes);
                let (elem, _) = agg_elem_and_len(&a_ty);
                let elem = elem.clone();
                let elem_llty = self.llvm_ty(&elem)?;
                let is_float = elem == Ty::F64;
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let b_ptr = self.expr_ptr(&args[1], scopes)?;
                // Matches interpreter.rs's three `term(i,j,k,l) = a[i]*b[j]
                // - a[k]*b[l]` calls exactly.
                for (out_i, (i, j, k, l)) in [(1usize, 2usize, 2usize, 1usize), (2, 0, 0, 2), (0, 1, 1, 0)].into_iter().enumerate() {
                    let ai = self.agg_load_elem(&a_ptr, &elem_llty, &elem, i);
                    let bj = self.agg_load_elem(&b_ptr, &elem_llty, &elem, j);
                    let p1 = self.emit_mul(&ai, &bj, is_float);
                    let ak = self.agg_load_elem(&a_ptr, &elem_llty, &elem, k);
                    let bl = self.agg_load_elem(&b_ptr, &elem_llty, &elem, l);
                    let p2 = self.emit_mul(&ak, &bl, is_float);
                    let c = if is_float {
                        self.emit_sub(&p1, &p2)
                    } else {
                        let out = self.fresh_reg("agg_sub");
                        writeln!(self.out, "  {out} = sub i64 {p1}, {p2}").unwrap();
                        out
                    };
                    self.agg_store_elem(&dest, &elem_llty, &elem, out_i, &c)?;
                }
            }
            "zeros" | "ones" => {
                let fill = Self::float_const(if name == "zeros" { 0.0 } else { 1.0 });
                let (_, len) = agg_elem_and_len(&result_ty);
                for i in 0..len {
                    self.agg_store_elem(&dest, "double", &Ty::F64, i, &fill)?;
                }
            }
            "identity" => {
                let Ty::Matrix(_, n, _) = result_ty else { unreachable!("builtin_result_ty always returns Matrix for identity") };
                let zero = Self::float_const(0.0);
                let one = Self::float_const(1.0);
                for i in 0..n {
                    for j in 0..n {
                        let v = if i == j { &one } else { &zero };
                        self.agg_store_elem(&dest, "double", &Ty::F64, i * n + j, v)?;
                    }
                }
            }
            "lla_to_ecef" => {
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let lat_deg = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let lon_deg = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 1);
                let alt = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 2);
                let (x, y, z) = self.lla_to_ecef_vals(&lat_deg, &lon_deg, &alt);
                self.agg_store_elem(&dest, "double", &Ty::F64, 0, &x)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 1, &y)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 2, &z)?;
            }
            "ecef_to_lla" => {
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let x = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let y = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 1);
                let z = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 2);
                let (lat, lon, alt) = self.ecef_to_lla_vals(&x, &y, &z);
                self.agg_store_elem(&dest, "double", &Ty::F64, 0, &lat)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 1, &lon)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 2, &alt)?;
            }
            "ecef_to_enu" | "enu_to_ecef" => {
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let ref_ptr = self.expr_ptr(&args[1], scopes)?;
                let a0 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 0);
                let a1 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 1);
                let a2 = self.agg_load_elem(&a_ptr, "double", &Ty::F64, 2);
                let ref_lat_deg = self.agg_load_elem(&ref_ptr, "double", &Ty::F64, 0);
                let ref_lon_deg = self.agg_load_elem(&ref_ptr, "double", &Ty::F64, 1);
                let ref_alt = self.agg_load_elem(&ref_ptr, "double", &Ty::F64, 2);
                let (ref_x, ref_y, ref_z) = self.lla_to_ecef_vals(&ref_lat_deg, &ref_lon_deg, &ref_alt);
                let deg_to_rad = Self::float_const(std::f64::consts::PI / 180.0);
                let ref_lat = self.emit_mul(&ref_lat_deg, &deg_to_rad, true);
                let ref_lon = self.emit_mul(&ref_lon_deg, &deg_to_rad, true);
                let r = self.enu_rotation_vals(&ref_lat, &ref_lon);

                let (out0, out1, out2) = if name == "ecef_to_enu" {
                    let d0 = self.emit_sub(&a0, &ref_x);
                    let d1 = self.emit_sub(&a1, &ref_y);
                    let d2 = self.emit_sub(&a2, &ref_z);
                    let d = [d0, d1, d2];
                    let mut outs: Vec<String> = Vec::with_capacity(3);
                    for k in 0..3 {
                        let t0 = self.emit_mul(&r[k * 3], &d[0], true);
                        let t1 = self.emit_mul(&r[k * 3 + 1], &d[1], true);
                        let t2 = self.emit_mul(&r[k * 3 + 2], &d[2], true);
                        let s01 = self.emit_add(&t0, &t1, true);
                        outs.push(self.emit_add(&s01, &t2, true));
                    }
                    (outs[0].clone(), outs[1].clone(), outs[2].clone())
                } else {
                    let enu = [a0, a1, a2];
                    let mut d: Vec<String> = Vec::with_capacity(3);
                    for j in 0..3 {
                        let t0 = self.emit_mul(&r[j], &enu[0], true);
                        let t1 = self.emit_mul(&r[3 + j], &enu[1], true);
                        let t2 = self.emit_mul(&r[6 + j], &enu[2], true);
                        let s01 = self.emit_add(&t0, &t1, true);
                        d.push(self.emit_add(&s01, &t2, true));
                    }
                    (self.emit_add(&ref_x, &d[0], true), self.emit_add(&ref_y, &d[1], true), self.emit_add(&ref_z, &d[2], true))
                };
                self.agg_store_elem(&dest, "double", &Ty::F64, 0, &out0)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 1, &out1)?;
                self.agg_store_elem(&dest, "double", &Ty::F64, 2, &out2)?;
            }
            "kf_predict_state" | "kf_predict_cov" => {
                // All four args evaluated left to right regardless of
                // which this specific half actually needs, matching the
                // interpreter's own call-argument evaluation (every
                // `Expr::Call` argument is evaluated before
                // `eval_builtin` dispatches on `name`, independent of
                // which arm ends up using which value).
                let x_ptr = self.expr_ptr(&args[0], scopes)?;
                let p_ptr = self.expr_ptr(&args[1], scopes)?;
                let f_ptr = self.expr_ptr(&args[2], scopes)?;
                let q_ptr = self.expr_ptr(&args[3], scopes)?;
                let Ty::Vector(_, n) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved x is Vector(f64,n)")
                };
                if name == "kf_predict_state" {
                    // `x' = F x` -- `P`/`Q` unused here, matching
                    // `interpreter.rs::kf_predict`'s own `x_new`
                    // computation exactly.
                    let x_new = self.mat_vec_mul_ptr_vals(&f_ptr, n, n, &x_ptr);
                    self.store_all_f64_vals(&dest, &x_new)?;
                } else {
                    // `P' = F P F^T + Q` -- `x` unused here.
                    let fp = self.mat_mul_ptr_vals(&f_ptr, n, n, &p_ptr, n);
                    let fpft = self.mat_mul_a_bt_vals(&fp, &f_ptr, n);
                    let q_vals = self.load_all_f64_vals(&q_ptr, n * n);
                    let p_new = self.vec_add_vals(&fpft, &q_vals);
                    self.store_all_f64_vals(&dest, &p_new)?;
                }
            }
            // Phase 5: `dest` is already allocated (shared setup above);
            // each arm below just hands it to the linked runtime call as
            // the out-pointer and traps via `guard_call_ok` on failure —
            // no unrolled IR, no separate `agg_*` computation, matching
            // `PHASE5_BUILTINS`' whole point (module doc).
            "inv" => {
                let Ty::Matrix(_, n, _) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a square Matrix(f64,_,_)")
                };
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let ok = self.fresh_reg("inv_ok");
                writeln!(self.out, "  {ok} = call i32 @nir_inv(ptr {a_ptr}, i64 {n}, ptr {dest})").unwrap();
                self.guard_call_ok(&ok);
            }
            "solve" => {
                let Ty::Matrix(_, n, _) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved this is a square Matrix(f64,_,_)")
                };
                let a_ptr = self.expr_ptr(&args[0], scopes)?;
                let b_ptr = self.expr_ptr(&args[1], scopes)?;
                let ok = self.fresh_reg("solve_ok");
                writeln!(self.out, "  {ok} = call i32 @nir_solve(ptr {a_ptr}, i64 {n}, ptr {b_ptr}, ptr {dest})").unwrap();
                self.guard_call_ok(&ok);
            }
            "kf_update_state" | "kf_update_cov" => {
                let Ty::Vector(_, n) = self.local_ty_of(&args[0], scopes) else {
                    unreachable!("typeck.rs already proved x is Vector(f64,n)")
                };
                let Ty::Vector(_, m) = self.local_ty_of(&args[2], scopes) else {
                    unreachable!("typeck.rs already proved z is Vector(f64,m)")
                };
                let x_ptr = self.expr_ptr(&args[0], scopes)?;
                let p_ptr = self.expr_ptr(&args[1], scopes)?;
                let z_ptr = self.expr_ptr(&args[2], scopes)?;
                let h_ptr = self.expr_ptr(&args[3], scopes)?;
                let r_ptr = self.expr_ptr(&args[4], scopes)?;
                let func = if name == "kf_update_state" { "@nir_kf_update_state" } else { "@nir_kf_update_cov" };
                let ok = self.fresh_reg("kf_update_ok");
                writeln!(
                    self.out,
                    "  {ok} = call i32 {func}(ptr {x_ptr}, ptr {p_ptr}, ptr {z_ptr}, ptr {h_ptr}, ptr {r_ptr}, i64 {n}, i64 {m}, ptr {dest})"
                )
                .unwrap();
                self.guard_call_ok(&ok);
            }
            _ => unreachable!("PHASE4_BUILTINS'/PHASE5_BUILTINS' scalar-returning names go through call_builtin_scalar instead"),
        }
        Ok(dest)
    }


    /// `lla_to_ecef`'s formula on already-loaded `lat_deg`/`lon_deg`/`alt`
    /// SSA values — factored out so `call_builtin_agg`'s own `lla_to_ecef`
    /// arm and `ecef_to_enu`/`enu_to_ecef`'s internal reference-point
    /// conversion (`interpreter.rs` calls the same Rust fn for both) share
    /// one implementation rather than two independently-written copies
    /// that could drift apart. Mirrors `interpreter.rs::lla_to_ecef`
    /// exactly. Returns `(x, y, z)`.
    pub(super) fn lla_to_ecef_vals(&mut self, lat_deg: &str, lon_deg: &str, alt: &str) -> (String, String, String) {
        let deg_to_rad = Self::float_const(std::f64::consts::PI / 180.0);
        let one = Self::float_const(1.0);
        let e2 = Self::float_const(WGS84_E2);
        let lat = self.emit_mul(lat_deg, &deg_to_rad, true);
        let lon = self.emit_mul(lon_deg, &deg_to_rad, true);
        let sin_lat = self.emit_call1("@llvm.sin.f64", &lat);
        let sin_lat_sq = self.emit_mul(&sin_lat, &sin_lat, true);
        let e2_sinsq = self.emit_mul(&e2, &sin_lat_sq, true);
        let one_minus = self.emit_sub(&one, &e2_sinsq);
        let sqrt_term = self.emit_call1("@llvm.sqrt.f64", &one_minus);
        let n = self.fresh_reg("wgs84_n");
        writeln!(self.out, "  {n} = fdiv double {}, {sqrt_term}", Self::float_const(WGS84_A)).unwrap();
        let cos_lat = self.emit_call1("@llvm.cos.f64", &lat);
        let cos_lon = self.emit_call1("@llvm.cos.f64", &lon);
        let sin_lon = self.emit_call1("@llvm.sin.f64", &lon);
        let n_plus_alt = self.emit_add(&n, alt, true);
        let np_cos_lat = self.emit_mul(&n_plus_alt, &cos_lat, true);
        let x = self.emit_mul(&np_cos_lat, &cos_lon, true);
        let y = self.emit_mul(&np_cos_lat, &sin_lon, true);
        let one_minus_e2 = self.emit_sub(&one, &e2);
        let n_1me2 = self.emit_mul(&n, &one_minus_e2, true);
        let n_1me2_plus_alt = self.emit_add(&n_1me2, alt, true);
        let z = self.emit_mul(&n_1me2_plus_alt, &sin_lat, true);
        (x, y, z)
    }


    /// `ecef_to_lla`'s formula — five fixed Newton-refinement iterations,
    /// unrolled (not data-dependent: always exactly 5, matching
    /// `interpreter.rs::ecef_to_lla`'s `for _ in 0..5`). Returns
    /// `(lat_deg, lon_deg, alt)`.
    pub(super) fn ecef_to_lla_vals(&mut self, x: &str, y: &str, z: &str) -> (String, String, String) {
        let e2 = Self::float_const(WGS84_E2);
        let one = Self::float_const(1.0);
        let lon = self.emit_call2("@atan2", y, x);
        let x2 = self.emit_mul(x, x, true);
        let y2 = self.emit_mul(y, y, true);
        let x2y2 = self.emit_add(&x2, &y2, true);
        let p = self.emit_call1("@llvm.sqrt.f64", &x2y2);
        let one_minus_e2 = self.emit_sub(&one, &e2);
        let p_1me2 = self.emit_mul(&p, &one_minus_e2, true);
        let mut lat = self.emit_call2("@atan2", z, &p_1me2);
        let mut alt = Self::float_const(0.0);
        for _ in 0..5 {
            let sin_lat = self.emit_call1("@llvm.sin.f64", &lat);
            let sin_lat_sq = self.emit_mul(&sin_lat, &sin_lat, true);
            let e2_sinsq = self.emit_mul(&e2, &sin_lat_sq, true);
            let one_minus = self.emit_sub(&one, &e2_sinsq);
            let sqrt_term = self.emit_call1("@llvm.sqrt.f64", &one_minus);
            let n = self.fresh_reg("wgs84_n");
            writeln!(self.out, "  {n} = fdiv double {}, {sqrt_term}", Self::float_const(WGS84_A)).unwrap();
            let cos_lat = self.emit_call1("@llvm.cos.f64", &lat);
            let p_over_cos = self.fresh_reg("p_over_cos");
            writeln!(self.out, "  {p_over_cos} = fdiv double {p}, {cos_lat}").unwrap();
            alt = self.emit_sub(&p_over_cos, &n);
            let n_plus_alt = self.emit_add(&n, &alt, true);
            let e2n = self.emit_mul(&e2, &n, true);
            let e2n_over = self.fresh_reg("e2n_over");
            writeln!(self.out, "  {e2n_over} = fdiv double {e2n}, {n_plus_alt}").unwrap();
            let inner = self.emit_sub(&one, &e2n_over);
            let p_inner = self.emit_mul(&p, &inner, true);
            lat = self.emit_call2("@atan2", z, &p_inner);
        }
        let rad_to_deg = Self::float_const(180.0 / std::f64::consts::PI);
        let lat_deg = self.emit_mul(&lat, &rad_to_deg, true);
        let lon_deg = self.emit_mul(&lon, &rad_to_deg, true);
        (lat_deg, lon_deg, alt)
    }


    /// The 3x3 ENU rotation matrix's 9 entries at `ref_lat`/`ref_lon`
    /// (already-loaded radians SSA values), row-major flattened
    /// (`r[i][j]` at index `i*3+j`) — mirrors `interpreter.rs::
    /// enu_rotation` exactly.
    pub(super) fn enu_rotation_vals(&mut self, ref_lat: &str, ref_lon: &str) -> [String; 9] {
        let sin_lat = self.emit_call1("@llvm.sin.f64", ref_lat);
        let cos_lat = self.emit_call1("@llvm.cos.f64", ref_lat);
        let sin_lon = self.emit_call1("@llvm.sin.f64", ref_lon);
        let cos_lon = self.emit_call1("@llvm.cos.f64", ref_lon);
        let neg_sin_lon = self.fresh_reg("neg");
        writeln!(self.out, "  {neg_sin_lon} = fneg double {sin_lon}").unwrap();
        let neg_sin_lat = self.fresh_reg("neg");
        writeln!(self.out, "  {neg_sin_lat} = fneg double {sin_lat}").unwrap();
        let r00 = neg_sin_lon;
        let r01 = cos_lon.clone();
        let r02 = Self::float_const(0.0);
        let r10 = self.emit_mul(&neg_sin_lat, &cos_lon, true);
        let r11 = self.emit_mul(&neg_sin_lat, &sin_lon, true);
        let r12 = cos_lat.clone();
        let r20 = self.emit_mul(&cos_lat, &cos_lon, true);
        let r21 = self.emit_mul(&cos_lat, &sin_lon, true);
        let r22 = sin_lat;
        [r00, r01, r02, r10, r11, r12, r20, r21, r22]
    }


    /// `A(ar x ac) * B(ac x bc)` on already-materialized pointers,
    /// flattened row-major as `ar*bc` SSA values (not yet stored anywhere)
    /// — the pointer-level analog of `agg_mul`'s Matrix*Matrix arm,
    /// needed because `kf_predict_cov` starts from raw argument pointers,
    /// not `Expr` nodes to feed `expr_ptr`/`agg_mul`. Same accumulation
    /// order as `interpreter.rs::mat_mul_f64` — bit-identical per the
    /// `0.0 + x == x` identity `call_builtin_scalar`'s doc comment
    /// already relies on.
    pub(super) fn mat_mul_ptr_vals(&mut self, a_ptr: &str, ar: usize, ac: usize, b_ptr: &str, bc: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(ar * bc);
        for i in 0..ar {
            for j in 0..bc {
                let a0 = self.agg_load_elem(a_ptr, "double", &Ty::F64, i * ac);
                let b0 = self.agg_load_elem(b_ptr, "double", &Ty::F64, j);
                let mut sum = self.emit_mul(&a0, &b0, true);
                for k in 1..ac {
                    let ak = self.agg_load_elem(a_ptr, "double", &Ty::F64, i * ac + k);
                    let bk = self.agg_load_elem(b_ptr, "double", &Ty::F64, k * bc + j);
                    let prod = self.emit_mul(&ak, &bk, true);
                    sum = self.emit_add(&sum, &prod, true);
                }
                out.push(sum);
            }
        }
        out
    }


    /// `A(n x n) * B^T` where `A` is already-computed values (`a_vals`,
    /// row-major) and `B` is a raw pointer read transposed in place
    /// (`B^T[k,j] = B[j,k]`) rather than materialized — `kf_predict_cov`'s
    /// `F P F^T` needs exactly this shape (`A = F P`, `B = F`). Bit-for-
    /// bit identical to `interpreter.rs::mat_mul_f64(fp, n, n,
    /// &mat_transpose_f64(f,n,n), n)`: `mat_transpose_f64` only
    /// rearranges data (no arithmetic), so reading `B` transposed in
    /// place here is the same accumulation, term for term.
    pub(super) fn mat_mul_a_bt_vals(&mut self, a_vals: &[String], b_ptr: &str, n: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                let a0 = a_vals[i * n].clone();
                let b0 = self.agg_load_elem(b_ptr, "double", &Ty::F64, j * n);
                let mut sum = self.emit_mul(&a0, &b0, true);
                for k in 1..n {
                    let ak = a_vals[i * n + k].clone();
                    let bk = self.agg_load_elem(b_ptr, "double", &Ty::F64, j * n + k);
                    let prod = self.emit_mul(&ak, &bk, true);
                    sum = self.emit_add(&sum, &prod, true);
                }
                out.push(sum);
            }
        }
        out
    }


    /// `A(ar x ac) * v` on a raw pointer — the pointer-level analog of
    /// `agg_mul`'s Matrix*Vector arm, for `kf_predict_state`'s `F x`.
    pub(super) fn mat_vec_mul_ptr_vals(&mut self, a_ptr: &str, ar: usize, ac: usize, v_ptr: &str) -> Vec<String> {
        let mut out = Vec::with_capacity(ar);
        for i in 0..ar {
            let a0 = self.agg_load_elem(a_ptr, "double", &Ty::F64, i * ac);
            let v0 = self.agg_load_elem(v_ptr, "double", &Ty::F64, 0);
            let mut sum = self.emit_mul(&a0, &v0, true);
            for k in 1..ac {
                let ak = self.agg_load_elem(a_ptr, "double", &Ty::F64, i * ac + k);
                let vk = self.agg_load_elem(v_ptr, "double", &Ty::F64, k);
                let prod = self.emit_mul(&ak, &vk, true);
                sum = self.emit_add(&sum, &prod, true);
            }
            out.push(sum);
        }
        out
    }


    /// Elementwise `a + b` on two already-loaded value lists — the
    /// pointer-level analog of `agg_elementwise`'s `Add` arm, for
    /// `kf_predict_cov`'s final `+ Q`.
    pub(super) fn vec_add_vals(&mut self, a: &[String], b: &[String]) -> Vec<String> {
        let mut out = Vec::with_capacity(a.len());
        for (x, y) in a.iter().zip(b.iter()) {
            out.push(self.emit_add(x, y, true));
        }
        out
    }


    /// Loads all `len` `f64` elements of a flat aggregate buffer into a
    /// `Vec<String>` of SSA values, in flat order — `kf_predict_cov`'s `Q`
    /// read.
    pub(super) fn load_all_f64_vals(&mut self, ptr: &str, len: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push(self.agg_load_elem(ptr, "double", &Ty::F64, i));
        }
        out
    }


    /// Stores a `Vec<String>` of already-computed `f64` values into a
    /// destination buffer at consecutive flat offsets — the write side of
    /// `load_all_f64_vals`/`mat_mul_ptr_vals`/`mat_vec_mul_ptr_vals`'s
    /// results.
    pub(super) fn store_all_f64_vals(&mut self, dest: &str, vals: &[String]) -> Result<(), CodegenError> {
        for (i, v) in vals.iter().enumerate() {
            self.agg_store_elem(dest, "double", &Ty::F64, i, v)?;
        }
        Ok(())
    }


    /// `expr_ptr`'s `Expr::Call` case — the sret side of the by-pointer
    /// return convention `function()` sets up: allocate the destination
    /// here (the caller's responsibility, matching a normal `let`'s own
    /// alloca), pass its address as the implicit first argument, and
    /// hand that same pointer back as this call expression's own
    /// "value" — exactly `expr_ptr`'s contract.
    pub(super) fn call_ptr(&mut self, name: &str, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        // Same "`name` may be a local `Ty::Fn` variable, not a global"
        // check `call()` opens with — see its own comment for why.
        if let Some((Ty::Fn(params, ret), fn_slot)) = scopes.get(name) {
            let fn_ptr = self.fresh_reg(&format!("{name}.fnval"));
            writeln!(self.out, "  {fn_ptr} = load ptr, ptr {fn_slot}").unwrap();
            return self.call_indirect(&fn_ptr, &params, &ret, args, scopes);
        }
        if PHASE4_BUILTINS.contains(&name)
            || matches!(name, "inv" | "solve" | "kf_update_state" | "kf_update_cov")
        {
            return self.call_builtin_agg(name, args, scopes);
        }
        if name == "check_role" {
            return self.emit_check_role(args, scopes);
        }
        if name == "dec_from_str" {
            return self.emit_dec_from_str(args, scopes);
        }
        if name == "oidc_validate_token" {
            return self.emit_oidc_validate_token(args, scopes);
        }
        if name == "mock_issue_token" {
            return self.emit_mock_issue_token(args, scopes);
        }
        if name == "extract_claim" {
            return self.emit_extract_claim(args, scopes);
        }
        if name == "check_role_path" {
            return self.emit_check_role_path(args, scopes);
        }
        if name == "extract_claim_path" {
            return self.emit_extract_claim_path(args, scopes);
        }
        if name == "check_revocation" {
            return self.emit_check_revocation(args, scopes);
        }
        if name == "create_application_session" {
            return self.emit_create_application_session(args, scopes);
        }
        if name == "session_cookie" {
            return self.emit_session_cookie(args, scopes);
        }
        if name == "verify_session" {
            return self.emit_verify_session(args, scopes);
        }
        if name == "new_refresh_token" {
            return self.emit_new_refresh_token(args, scopes);
        }
        if name == "exchange_refresh_token" {
            return self.emit_exchange_refresh_token(args, scopes);
        }
        if name == "validate_api_key" {
            return self.emit_validate_api_key(args, scopes);
        }
        if name == "db_connect" {
            return self.emit_db_connect(args, scopes);
        }
        if name == "env" {
            return self.emit_env(args, scopes);
        }
        if name == "db_execute" {
            return self.emit_db_execute(args, scopes);
        }
        if name == "db_query" {
            return self.emit_db_query(args, scopes);
        }
        if name == "json_parse" {
            return self.emit_json_parse(args, scopes);
        }
        if name == "json_get" {
            return self.emit_json_get(args, scopes);
        }
        if name == "json_array_get" {
            return self.emit_json_array_get(args, scopes);
        }
        if name == "json_array_len" {
            return self.emit_json_array_len(args, scopes);
        }
        if name == "json_get_str" {
            return self.emit_json_get_str(args, scopes);
        }
        if name == "json_get_i64" {
            return self.emit_json_get_i64(args, scopes);
        }
        if name == "json_get_f64" {
            return self.emit_json_get_f64(args, scopes);
        }
        if name == "json_get_bool" {
            return self.emit_json_get_bool(args, scopes);
        }
        if name == "json_set_str" {
            return self.emit_json_set_str(args, scopes);
        }
        if name == "mq_connect" {
            return self.emit_mq_connect(args, scopes);
        }
        if name == "mq_publish" {
            return self.emit_mq_publish(args, scopes);
        }
        if name == "mq_consume" {
            return self.emit_mq_consume(args, scopes);
        }
        if name == "http_get" {
            return self.emit_http_call("nir_http_get", args, false, scopes);
        }
        if name == "http_post" {
            return self.emit_http_call("nir_http_post", args, true, scopes);
        }
        if name == "https_get" {
            return self.emit_http_call("nir_https_get", args, false, scopes);
        }
        if name == "https_post" {
            return self.emit_http_call("nir_https_post", args, true, scopes);
        }
        if name == "call_via" {
            return self.emit_call_via(args, scopes);
        }
        if name == "__workflow_start" {
            return self.emit_workflow_start(args, scopes);
        }
        if name == "__workflow_advance" {
            return self.emit_workflow_advance(args, scopes);
        }
        if name == "__workflow_overdue" {
            return self.emit_workflow_overdue(args, scopes);
        }
        if name == "__workflow_pending_for_me" || name == "__workflow_submitted_by_me" || name == "__workflow_history" {
            return self.emit_workflow_unsupported_query(name);
        }
        if name == "send_email" {
            return self.emit_send_notification("email", args, scopes);
        }
        if name == "send_sms" {
            return self.emit_send_notification("sms", args, scopes);
        }
        if name == "send_push" {
            return self.emit_send_notification("push", args, scopes);
        }
        if name == "notify" {
            return self.emit_notify(args, scopes);
        }
        let sig_params = self.sigs.get(name).expect("typeck.rs already resolved this call").params.clone();
        let sig_ret = self.sigs.get(name).expect("typeck.rs already resolved this call").ret.clone();
        let arg_vals = self.call_args(args, &sig_params, scopes)?;

        let agg_llty = self.llvm_ty(&sig_ret)?;
        let dest = self.fresh_reg("call_result.addr");
        self.emit_alloca(&dest, &agg_llty);
        let mut all_args = vec![format!("ptr {dest}")];
        all_args.extend(arg_vals);
        writeln!(self.out, "  call void @{name}({})", all_args.join(", ")).unwrap();
        Ok(dest)
    }


    /// `dec_from_str(s) -> Result(dec128, str)` — `DEC128_BUILTINS`'s own
    /// doc comment: reuses `emit_result_merge`, the same generic
    /// tag-then-payload machinery `emit_env`/`emit_db_connect` (just
    /// below) already use, rather than hand-rolling the branch/merge
    /// shape the way `emit_check_role` did before that helper existed.
    /// `nir_dec128_from_str` returns its `Dec128Bits` payload directly
    /// (by value, not through an out-pointer) — the one real difference
    /// from `emit_db_connect`'s `handle_scratch`, since dec128 is already
    /// passed/returned by value everywhere in this backend (`llvm_ty`'s
    /// `Ty::Dec128` arm).
    pub(super) fn emit_dec_from_str(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Dec128, Ty::Str]);
        let (s_ptr, s_len) = self.str_parts(&args[0], scopes)?;
        let ok_scratch = self.fresh_reg("dec_from_str_ok_scratch");
        self.emit_alloca(&ok_scratch, "i32");
        let err_scratch = self.fresh_reg("dec_from_str_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let d = self.fresh_reg("dec_from_str_val");
        writeln!(
            self.out,
            "  {d} = call {{i64, i64}} @nir_dec128_from_str(ptr {s_ptr}, i64 {s_len}, ptr {ok_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let ok_loaded = self.fresh_reg("dec_from_str_ok");
        writeln!(self.out, "  {ok_loaded} = load i32, ptr {ok_scratch}").unwrap();
        let is_ok = self.icmp("ne", "i32", &ok_loaded, "0")?;
        let err_val = self.fresh_reg("dec_from_str_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{i64, i64}", &d, &err_val, "dec_from_str")
    }


}
