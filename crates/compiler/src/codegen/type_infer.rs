use super::*;
use super::layout::*;
use super::checks::*;

/// A short, identifier-safe token for `ty`, used to name a generic
/// struct/enum instantiation's own LLVM named type distinctly per
/// concrete instantiation — `Result(i64, str)` and `Result(str, str)`
/// need different LLVM types, since their layouts differ, even though
/// they share one declaration. Only ever needs to handle the *non-affine*
/// `Ty` subset: `llvm_ty`'s `Ty::Named` arm already rejects an
/// affine-containing instantiation before this can run on one, and no
/// affine type (`Box`/`Tcp`/etc.) can otherwise legally appear as a
/// generic type argument here. Returns the name *without* a leading `%`
/// sigil (so a nested `Ty::Named` argument mangles into the middle of
/// the string without an illegal embedded sigil) — callers that need the
/// real LLVM identifier prepend `%` themselves (`llvm_ty`'s `Ty::Named`
/// arm, `declare_named_type`).
/// True exactly for the broken placeholder `local_ty_of`'s own
/// constructor-call fallback invents when `ctor_ty` can't fully resolve
/// a *standalone* `Ok(..)`/`Err(..)` call (see that arm's own comment
/// for why it structurally never can: `Ok`'s payload only ever pins
/// down `Result`'s `T`, `Err`'s only ever pins down `E`, so without
/// either an enclosing expected type or a sibling branch supplying the
/// other one, one type parameter is always missing). `"Ok"`/`"Err"` are
/// never real registered struct/enum names, so a `Ty::Named` under
/// either literal name with no type arguments is never a genuine
/// resolved type — it's this specific failure signal, checked here
/// (not trusted blindly) so `match_result_ty`/`if_result_ty` can prefer
/// a sibling branch that *does* fully resolve instead.
pub(super) fn is_unresolved_ok_err_placeholder(ty: &Ty) -> bool {
    matches!(ty, Ty::Named(name, args) if args.is_empty() && (name == "Ok" || name == "Err"))
}


impl Codegen<'_> {
    /// Resolve the concrete `Ty::Named(name, type_args)` a constructor call
    /// `name(args)` produces, *without* an expected-type context — the
    /// structural-inference fallback `expr_ptr`'s own `Expr::Call` ctor
    /// branch and `local_ty_of`'s `Expr::Call` ctor arm need (the four
    /// real construction sites — `Stmt::Let`/`Stmt::Return`/`call_args`/
    /// nested-`construct` — already have a concrete `expected` in hand and
    /// never call this; they call `construct` directly). Mirrors
    /// `typeck.rs::resolve_type_args`'s fall-back path: for a generic
    /// declaration, infer each type parameter from the corresponding
    /// argument's own type via `bind_type_params`, then collect. Returns
    /// `None` only for the genuinely-ambiguous case a zero-payload variant
    /// (`None`) reached with no enclosing type context at all — exactly
    /// the case the struct/enum codegen plan names as the one disclosed
    /// `CodegenError` to fail rather than guess on.
    pub(super) fn ctor_ty(&self, name: &str, args: &[Expr], scopes: &Scopes) -> Option<Ty> {
        if let Some(decl) = self.registry.struct_decl(name) {
            let type_params = decl.type_params.clone();
            let decl_tys: Vec<Ty> = decl.fields.iter().map(|f| f.ty.clone()).collect();
            let type_args = self.infer_type_args(&type_params, &decl_tys, args, scopes)?;
            Some(Ty::Named(name.to_string(), type_args))
        } else if let Some((enum_name, variant)) = self.registry.find_variant(name) {
            let type_params = self.registry.enum_type_params(&enum_name)?.to_vec();
            let decl_tys = variant.payload.clone();
            let type_args = self.infer_type_args(&type_params, &decl_tys, args, scopes)?;
            Some(Ty::Named(enum_name, type_args))
        } else {
            None
        }
    }


    /// The shared inference core of `ctor_ty` — `typeck.rs::
    /// resolve_type_args` minus its expected-type shortcut and its
    /// diagnostic path (codegen only ever sees an already-well-typed
    /// program, so a genuinely-ambiguous constructor is a real, disclosed
    /// `CodegenError` this returns `None` for, not a recoverable type
    /// error to report). Returns the fully-resolved `type_args` if every
    /// parameter was bound, `None` otherwise.
    pub(super) fn infer_type_args(&self, type_params: &[String], decl_tys: &[Ty], args: &[Expr], scopes: &Scopes) -> Option<Vec<Ty>> {
        if type_params.is_empty() {
            return Some(Vec::new());
        }
        let mut subst: HashMap<String, Ty> = HashMap::new();
        for (decl_ty, arg) in decl_tys.iter().zip(args.iter()) {
            let arg_ty = self.local_ty_of(arg, scopes);
            if arg_ty != Ty::Error {
                bind_type_params_owned(decl_ty, &arg_ty, type_params, &mut subst);
            }
        }
        type_params.iter().map(|p| subst.get(p).cloned()).collect()
    }


    /// `(field_index, substituted_field_type)` for `field` of the struct
    /// type `base_ty` — the one piece of information both `expr()`'s and
    /// `expr_ptr()`'s `Expr::FieldAccess` arms need, factored out so they
    /// share one resolution path. `base_ty` is always a concrete struct
    /// instantiation by the time this runs (`typeck.rs` already proved the
    /// base is a struct and the field exists); the `None` return is a
    /// defense-in-depth fallback, not an expected path.
    pub(super) fn field_index_and_ty(&self, base_ty: &Ty, field: &str) -> Option<(usize, Ty)> {
        if let Ty::Named(name, type_args) = base_ty
            && let Some(fields) = self.registry.struct_fields(name)
        {
            let type_params = self.registry.struct_type_params(name).unwrap_or(&[]);
            let subst = zip_type_params(type_params, type_args);
            for (i, f) in fields.iter().enumerate() {
                if f.name == field {
                    return Some((i, substitute_ty(&f.ty, &subst)));
                }
            }
        }
        None
    }


    /// `expr_ptr`'s `Expr::Call` ctor branch, and the shared backend the
    /// three expected-type-bearing construction sites (`Stmt::Let`/
    /// `Stmt::Return`/`call_args`, via `expr_ptr_expected`) route
    /// through. Allocates a fresh destination sized to `expected`'s real
    /// LLVM type, fills its fields (struct) or its tag + payload words
    /// (enum variant) from `args`, and returns the destination pointer —
    /// the `expr_ptr`-shaped result every aggregate value produces.
    ///
    /// `expected` is always a *concrete* `Ty::Named(decl_name, type_args)`
    /// at every call site: `Stmt::Let`/`Stmt::Return`/`call_args` hand
    /// over their own already-resolved declared/return/parameter type, and
    /// a nested constructor argument recurses with its field's own
    /// substituted type. So this is pure lookup + substitution, never
    /// inference — the inference the plan's design-decision 4 names is
    /// entirely `ctor_ty`'s job (the `expr_ptr`-reached fallback), not
    /// this method's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn construct(
        &mut self,
        name: &str,
        args: &[Expr],
        expected: &Ty,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        let dest_llty = self.llvm_ty(expected)?;
        let dest = self.fresh_reg("ctor.addr");
        self.emit_alloca(&dest, &dest_llty);

        if self.registry.is_struct(name) {
            self.construct_struct(name, args, expected, &dest, span, scopes)?;
        } else if let Some((enum_name, variant)) = self.registry.find_variant(name) {
            self.construct_variant(&enum_name, variant, args, expected, &dest, span, scopes)?;
        } else {
            unreachable!("typeck.rs already proved `{name}` is a struct or variant constructor")
        }
        Ok(dest)
    }


    /// The struct half of `construct` — stores each argument into its
    /// field slot via `getelementptr %Name, ptr dest, i32 0, i32 i`,
    /// recursing through `construct` for a nested constructor argument
    /// (so `Outer(Inner(1))` constructs the `Inner` into its own temp,
    /// then memcpys it into `Outer`'s field — a small extra copy, kept
    /// for simplicity over a "construct directly into the field slot"
    /// optimization that would need a different `construct` shape).
    pub(super) fn construct_struct(
        &mut self,
        name: &str,
        args: &[Expr],
        expected: &Ty,
        dest: &str,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        let decl_name = if let Ty::Named(n, _) = expected { n.as_str() } else { name };
        let fields = self.registry.struct_fields(decl_name).expect("just proved this is a struct");
        let type_params = self.registry.struct_type_params(decl_name).unwrap_or(&[]);
        let type_args = if let Ty::Named(_, a) = expected { a.clone() } else { Vec::new() };
        let subst = zip_type_params(type_params, &type_args);
        let base_llty = self.llvm_ty(expected)?;
        for (i, (arg, f)) in args.iter().zip(fields.iter()).enumerate() {
            let field_ty = substitute_ty(&f.ty, &subst);
            let field_ptr = self.fresh_reg("field.addr");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {base_llty}, ptr {dest}, i32 0, i32 {i}").unwrap();
            self.store_value_into(arg, &field_ty, &field_ptr, span, scopes)?;
        }
        Ok(())
    }


    /// The enum-variant half of `construct` — stores the variant's
    /// declaration-order index at the tag word (GEP field 0), then stores
    /// each payload argument at its 8-byte-aligned word offset inside the
    /// `[N x i64]` payload buffer (GEP field 1, then a word-granularity
    /// `getelementptr i64` into it). Word offsets are the cumulative
    /// `conservative_word_count` of the preceding payload fields in this
    /// variant — exactly the over-allocating, never-under-sizing scheme
    /// `declare_named_type`'s enum-layout doc explains.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn construct_variant(
        &mut self,
        enum_name: &str,
        variant: &Variant,
        args: &[Expr],
        expected: &Ty,
        dest: &str,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        let variants = self.registry.enum_variants(enum_name).expect("just proved this is an enum");
        let vidx = variants.iter().position(|v| v.name == variant.name).expect("typeck.rs proved this variant exists");
        let type_params = self.registry.enum_type_params(enum_name).unwrap_or(&[]);
        let type_args = if let Ty::Named(_, a) = expected { a.clone() } else { Vec::new() };
        let subst = zip_type_params(type_params, &type_args);
        let base_llty = self.llvm_ty(expected)?;

        // Tag word at GEP field 0.
        let tag_ptr = self.fresh_reg("tag.addr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {base_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 {vidx}, ptr {tag_ptr}").unwrap();

        // Payload buffer at GEP field 1 — `[N x i64]`, element-addressable
        // by a plain `getelementptr i64, ptr %payload, i64 <word_off>`.
        let payload = self.fresh_reg("payload.addr");
        writeln!(self.out, "  {payload} = getelementptr inbounds {base_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let mut word_off: u64 = 0;
        for (arg, decl_ty) in args.iter().zip(variant.payload.iter()) {
            let field_ty = substitute_ty(decl_ty, &subst);
            let field_ptr = self.fresh_reg("payfield.addr");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds i64, ptr {payload}, i64 {word_off}").unwrap();
            self.store_value_into(arg, &field_ty, &field_ptr, span, scopes)?;
            word_off += conservative_word_count(&field_ty, &self.registry);
        }
        Ok(())
    }


    /// Store `arg`'s value (a scalar) or whole value (an aggregate) into
    /// the slot at `field_ptr`, with the slot's own declared `field_ty`
    /// driving the scalar-vs-aggregate choice exactly the way `call_args`
    /// does for a call argument. A nested constructor argument recurses
    /// through `construct` (with `field_ty` as its expected type) instead
    /// of going through `expr_ptr`'s no-expected fallback — so a nested
    /// `Outer(Inner(1))` never hits `ctor_ty`'s ambiguous-case `None`.
    pub(super) fn store_value_into(
        &mut self,
        arg: &Expr,
        field_ty: &Ty,
        field_ptr: &str,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        if field_ty.is_aggregate() {
            if let Expr::Call(name, cargs, _) = arg
                && (self.registry.is_struct(name) || self.registry.find_variant(name).is_some())
            {
                let src = self.construct(name, cargs, field_ty, span, scopes)?;
                let bytes = agg_byte_size_operand(field_ty, &self.registry);
                writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {field_ptr}, ptr {src}, i64 {bytes}, i1 false)").unwrap();
                return Ok(());
            }
            let src = self.expr_ptr(arg, scopes)?;
            let bytes = agg_byte_size_operand(field_ty, &self.registry);
            writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {field_ptr}, ptr {src}, i64 {bytes}, i1 false)").unwrap();
        } else {
            let v = self.expr(arg, scopes)?;
            let v = if field_ty.is_integer() { self.narrow_from_i64(&v, field_ty)? } else { v };
            let field_llty = self.llvm_ty(field_ty)?;
            writeln!(self.out, "  store {field_llty} {v}, ptr {field_ptr}").unwrap();
        }
        Ok(())
    }


    /// The expected-type-bearing entry point the three aggregate-value
    /// call sites that already have a concrete type in hand (`Stmt::Let`'s
    /// own declared type, `Stmt::Return`'s `current_fn_ret`, `call_args`'
    /// per-argument `sig_params[i]`) route through instead of `expr_ptr` —
    /// so a constructor reached in one of those positions is built with
    /// its real expected type (never `ctor_ty`'s inference fallback), and
    /// every other aggregate expression is forwarded to `expr_ptr`
    /// unchanged. A constructor's `expected` is always a concrete
    /// `Ty::Named` here (a `let`/return/parameter type is fully resolved
    /// by `typeck.rs` before codegen runs), so `construct`'s "no
    /// inference" contract holds at every entry.
    pub(super) fn expr_ptr_expected(
        &mut self,
        e: &Expr,
        expected: &Ty,
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        if let Expr::Call(name, args, span) = e
            && (self.registry.is_struct(name) || self.registry.find_variant(name).is_some())
        {
            return self.construct(name, args, expected, *span, scopes);
        }
        // A real captured crash (`nirdosha_hi_*.log`/`.nir`) had a bare
        // `Ok`/`Err` reconstruction nested two `if`/`match` levels deep
        // inside an arm/branch whose own enclosing `let`/`return` *did*
        // have a concrete `expected` in hand -- but that `expected` was
        // dropped the moment this function fell through to plain
        // `expr_ptr` below for anything that wasn't a *direct*
        // constructor call, so a nested `if`/`match` re-derived its own
        // result type from scratch via `if_result_ty`/`match_result_ty`'s
        // own (necessarily weaker, no-context) inference instead of
        // just being told the answer it already had one call frame up.
        // Threading `expected` through here closes that gap at its
        // actual source, recursively, at any nesting depth -- not
        // another sibling-branch heuristic layered on top of it.
        match e {
            Expr::If { cond, then_block, else_block, span } => return self.if_expr(cond, then_block, else_block.as_deref(), *span, Some(expected), scopes),
            Expr::Match { scrutinee, arms, span } => return self.match_expr(scrutinee, arms, *span, Some(expected), scopes),
            _ => {}
        }
        self.expr_ptr(e, scopes)
    }


    /// A very small, codegen-local "what LLVM type does this produce"
    /// helper — used only where a caller (`return`) needs it and typeck
    /// isn't threaded through. Trusts the program is already well-typed
    /// (typeck.rs ran first), so it doesn't need to be a full inference
    /// pass, just enough to pick the right LLVM type keyword.
    pub(super) fn local_ty_of(&self, e: &Expr, scopes: &Scopes) -> Ty {
        match e {
            Expr::Int(_, _) => Ty::I64,
            Expr::Float(_, _) => Ty::F64,
            Expr::Bool(_, _) => Ty::Bool,
            Expr::Str(_, _) => Ty::Str,
            // A local variable first; if `name` isn't one, it's a bare
            // reference to an ordinary (ungated) top-level `fn` used as a
            // first-class value (`apply(double, 21)`, LANGUAGE.md §6a) —
            // `Ty::Fn`, not `Ty::I64`. A `requires`-gated fn's name has no
            // such reference at all (`TypeErrorKind::PrivilegedFnNotAcquired`,
            // already enforced before codegen runs), so `self.sigs.get`
            // finding one here always means an ungated fn.
            Expr::Ident(name, _) => scopes
                .get(name)
                .map(|(t, _)| t)
                .or_else(|| self.sigs.get(name).map(|s| Ty::Fn(s.params.clone(), Box::new(s.ret.clone()))))
                .unwrap_or(Ty::I64),
            Expr::Unary(UnOp::Not, _, _) => Ty::Bool,
            Expr::Unary(UnOp::Neg, inner, _) => self.local_ty_of(inner, scopes),
            Expr::Binary(op, l, r, _) => match op {
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq | BinOp::And | BinOp::Or => {
                    Ty::Bool
                }
                // `*`'s result shape depends on *both* operands (a
                // Matrix's own left-operand type is not always the
                // answer -- `Matrix * Vector` produces a `Vector`,
                // `scalar * Matrix` produces the `Matrix`) -- every other
                // arithmetic op here is same-shape-on-both-sides
                // (typeck-guaranteed), so the left operand's type alone
                // is still correct for those.
                BinOp::Mul => self.mul_result_ty(l, r, scopes),
                _ => self.local_ty_of(l, scopes),
            },
            Expr::Assign(name, _, _) => scopes.get(name).map(|(t, _)| t).unwrap_or(Ty::I64),
            Expr::Call(name, args, _)
                if PHASE4_BUILTINS.contains(&name.as_str()) || PHASE5_BUILTINS.contains(&name.as_str()) =>
            {
                self.builtin_result_ty(name, args, scopes)
            }
            // Not in `self.sigs` (that table is user-defined functions
            // only) -- without this arm, the fallback below would
            // wrongly report `Ty::I64` for these two builtins.
            Expr::Call(name, _, _) if name == "sha256_hex" => Ty::Str,
            Expr::Call(name, _, _) if name == "constant_time_str_eq" => Ty::Bool,
            Expr::Call(name, _, _) if name == "str_slice" => Ty::Str,
            Expr::Call(name, _, _) if name == "str_index_of" => Ty::I64,
            Expr::Call(name, _, _) if name == "rand_f64" || name == "rand_gaussian" => Ty::F64,
            Expr::Call(name, _, _) if name == "rand_seed" => Ty::Unit,
            Expr::Call(name, _, _) if name == "sleep_ms" => Ty::Unit,
            Expr::Call(name, _, _) if name == "dec_from_i64" || name == "dec_round" => Ty::Dec128,
            Expr::Call(name, _, _) if name == "dec_to_str" => Ty::Str,
            Expr::Call(name, _, _) if name == "dec_scale" => Ty::U32,
            Expr::Call(name, _, _) if name == "dec_from_str" => Ty::Named("Result".to_string(), vec![Ty::Dec128, Ty::Str]),
            Expr::Call(name, _, _) if name == "check_role" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("RoleView".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "oidc_validate_token" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("VerifiedIdentity".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "mock_issue_token" => {
                Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "extract_claim" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("ClaimView".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "identity_expired" => Ty::Bool,
            Expr::Call(name, _, _) if name == "check_role_path" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("RoleView".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "extract_claim_path" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("ClaimView".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "check_revocation" => Ty::Bool,
            Expr::Call(name, _, _) if name == "create_application_session" => Ty::Named("ApplicationSession".to_string(), vec![]),
            Expr::Call(name, _, _) if name == "session_cookie" => Ty::Str,
            Expr::Call(name, _, _) if name == "verify_session" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("VerifiedIdentity".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "new_refresh_token" => Ty::Named("RefreshTokenHandle".to_string(), vec![]),
            Expr::Call(name, _, _) if name == "exchange_refresh_token" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("VerifiedIdentity".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "validate_api_key" => {
                Ty::Named("Result".to_string(), vec![Ty::Named("VerifiedIdentity".to_string(), vec![]), Ty::Str])
            }
            Expr::Call(name, _, _) if name == "db_connect" => {
                Ty::Named("Result".to_string(), vec![Ty::Db, Ty::Str])
            }
            // RFC 0011 §1 -- same `Result(str, str)` shape as `json_get_str`
            // below; needed here (not just in `emit_env`'s own dispatch)
            // so a `match env(...)` scrutinee is correctly routed through
            // the aggregate (`expr_ptr`) codegen path instead of falling
            // through to `fn call`'s scalar-only dispatch and hitting its
            // "typeck.rs already resolved this call" `expect` panic.
            Expr::Call(name, _, _) if name == "env" => Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]),
            Expr::Call(name, _, _) if name == "db_execute" => {
                Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "db_query" => {
                Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_parse" || name == "json_get" || name == "json_array_get" => {
                Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_get_str" => {
                Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_get_i64" || name == "json_array_len" => {
                Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_get_f64" => {
                Ty::Named("Result".to_string(), vec![Ty::F64, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_get_bool" => {
                Ty::Named("Result".to_string(), vec![Ty::Bool, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "json_set_str" => {
                Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "mq_connect" => {
                Ty::Named("Result".to_string(), vec![Ty::Mq, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "mq_publish" => {
                Ty::Named("Result".to_string(), vec![Ty::Unit, Ty::Str])
            }
            Expr::Call(name, _, _) if name == "mq_consume" => {
                Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str])
            }
            Expr::Call(name, _, _)
                if name == "http_get" || name == "http_post" || name == "https_get" || name == "https_post" || name == "call_via" =>
            {
                Ty::Named("Result".to_string(), vec![Ty::Named("HttpResponse".to_string(), vec![]), Ty::Str])
            }
            // `workflow` Layer 1 (`WORKFLOW_BUILTINS`/`NOTIFY_BUILTINS`'
            // own doc comments) — every one of these returns
            // `Result(_, WorkflowActionError)`, matching `typeck.rs`'s
            // own `workflow_result_of` calls for the same names exactly.
            Expr::Call(name, _, _) if name == "__workflow_start" => workflow_result_of(Ty::I64),
            Expr::Call(name, _, _) if name == "__workflow_advance" => workflow_result_of(Ty::Bool),
            Expr::Call(name, _, _) if name == "__workflow_overdue" => workflow_result_of(Ty::Json),
            Expr::Call(name, _, _)
                if name == "__workflow_pending_for_me" || name == "__workflow_submitted_by_me" || name == "__workflow_history" =>
            {
                workflow_result_of(Ty::Json)
            }
            Expr::Call(name, _, _) if name == "send_email" || name == "send_sms" || name == "send_push" || name == "notify" => {
                workflow_result_of(Ty::Bool)
            }
            // Row 11: a struct/variant constructor call produces a
            // `Ty::Named` value (the struct's own type, or the owning
            // enum's), not the `Ty::I64` the `self.sigs.get` fallback
            // below would wrongly hand back for a name that isn't a user
            // fn. `ctor_ty`'s `None` (a zero-payload generic variant
            // reached with no expected type, e.g. a bare `None`
            // expression statement) falls back to a placeholder `Named`
            // so `is_aggregate()` routing still picks the right
            // `expr_ptr`/`expr` fork — the real construction path
            // (`expr_ptr_expected`) always has the true expected type and
            // never reaches this `None` branch for a real program.
            Expr::Call(name, args, _) if self.registry.is_struct(name) || self.registry.find_variant(name).is_some() => {
                self.ctor_ty(name, args, scopes).unwrap_or_else(|| Ty::Named(name.to_string(), Vec::new()))
            }
            // `name` here can be a local variable holding an acquired/
            // passed-in `Ty::Fn` value, not just a top-level `fn` —
            // `f(x)` where `f: fn(i64) -> i64` is a parameter parses to
            // the same `Expr::Call("f", ...)` shape a direct call does
            // (`parser.rs::parse_call` doesn't distinguish), so `scopes`
            // is checked first; `self.sigs` (top-level fns only) would
            // otherwise never find `f` and wrongly fall back to `Ty::I64`.
            Expr::Call(name, _, _) => match scopes.get(name) {
                Some((Ty::Fn(_, ret), _)) => *ret,
                _ => self.sigs.get(name).map(|s| s.ret.clone()).unwrap_or(Ty::I64),
            },
            // Row 11: `base.field`'s type is `base`'s struct type's
            // substituted field type — factored through
            // `field_index_and_ty` so `expr()`/`expr_ptr()`'s own
            // `Expr::FieldAccess` arms share one resolution path with
            // this `local_ty_of` arm. The `Ty::I64` fallback is
            // defense-in-depth (typeck already proved the base is a struct
            // and the field exists), not an expected path.
            Expr::FieldAccess(base, field, _) => {
                self.field_index_and_ty(&self.local_ty_of(base, scopes), field)
                    .map(|(_, ty)| ty)
                    .unwrap_or(Ty::I64)
            }
            Expr::ArrayLit(elements, _) => self.array_lit_ty(elements, scopes),
            Expr::Index(base, _, _) => match self.local_ty_of(base, scopes) {
                Ty::Vector(elem, _) | Ty::Matrix(elem, _, _) => *elem,
                _ => Ty::I64,
            },
            // Needed so a directly-nested call (e.g. `stop(connect(...))`,
            // with no intervening `let` to record the declared type in
            // `scopes`) still reports the right `Ty` to `Expr::StopSandbox`/
            // `Expr::Send`/`Expr::Recv`'s own `local_ty_of` dispatch —
            // mirrors `typeck.rs`'s `infer` arms for these exactly.
            Expr::Connect(_, _, _) => Ty::Tcp,
            Expr::Listen(_, _) => Ty::TcpListener,
            Expr::Accept(_, _) => Ty::Tcp,
            Expr::Recv(target, _) => match self.local_ty_of(target, scopes) {
                Ty::Tcp => Ty::Str,
                Ty::Channel(inner) => *inner,
                _ => Ty::I64,
            },
            // Mirrors `typeck::infer_spawn`/the `Expr::Join` arm of
            // `infer` exactly — needed for the same "directly-nested,
            // no intervening `let`" case `Expr::Recv`'s own comment
            // above explains (e.g. `join spawn worker(x)` with nothing
            // ever bound to a name).
            Expr::Spawn(name, _, _) => Ty::Thread(Box::new(self.sigs.get(name).map(|s| s.ret.clone()).unwrap_or(Ty::I64))),
            // `acquire name(proof)` -> `Result(Ty::Fn(params, ret), str)` —
            // `name`'s own declared signature, unchanged; `acquire` only
            // ever gates *whether* the value is obtained, never its shape.
            Expr::Acquire(name, _, _) => {
                let sig = self.sigs.get(name);
                let fn_ty = Ty::Fn(
                    sig.map(|s| s.params.clone()).unwrap_or_default(),
                    Box::new(sig.map(|s| s.ret.clone()).unwrap_or(Ty::I64)),
                );
                Ty::Named("Result".to_string(), vec![fn_ty, Ty::Str])
            }
            Expr::Join(inner, _) => match self.local_ty_of(inner, scopes) {
                Ty::Thread(t) => *t,
                _ => Ty::I64,
            },
            Expr::Box(inner, _) => Ty::Box(Box::new(self.local_ty_of(inner, scopes))),
            Expr::Froze(inner, _) => Ty::Froze(Box::new(self.local_ty_of(inner, scopes))),
            Expr::Ref(inner, _) => Ty::Ref(Box::new(self.local_ty_of(inner, scopes))),
            // `*e` unwraps exactly one pointer level — `ownership.rs`
            // already proved `e`'s type is `Box`/`Ref`/`Froze` for any
            // program that reaches codegen, and (per its own move-
            // checking) that unwrapping affine content out of a shared
            // `Ref`/`Froze` never typechecks in the first place, so this
            // never needs to reject anything itself, only report the
            // unwrapped type.
            Expr::Deref(inner, _) => match self.local_ty_of(inner, scopes) {
                Ty::Box(t) | Ty::Ref(t) | Ty::Froze(t) => *t,
                _ => Ty::I64,
            },
            // Aggregate-result `if`/`match`: typeck already proved every
            // branch/arm body has the same type, so the first one is
            // usually representative -- `if_result_ty`/`match_result_ty`
            // handle the cases where it isn't (see their own comments).
            // This is only needed so that nested aggregate control flow
            // (e.g. an `if` inside an `if` branch) can resolve its own
            // result type.
            Expr::If { then_block, else_block, .. } => self.if_result_ty(then_block, else_block.as_deref(), scopes),
            // "The first arm is representative" breaks for exactly one
            // shape: an arm body that's itself a bare `Ok(..)`/`Err(..)`
            // reconstruction of the `Result` prelude type (`match
            // <result> { Ok(r) => Ok(r), Err(e) => Err(e) }` -- an
            // entirely ordinary "pass a Result through" pattern). Neither
            // variant's own payload determines the *other* type
            // parameter (`Ok(r)`'s payload only ever pins down `T`, never
            // `E`), so `ctor_ty` (via the `Expr::Call` arm below) can't
            // resolve one standalone and used to fall through to its own
            // `Ty::Named("Ok", [])`/`Ty::Named("Err", [])` placeholder --
            // registered under neither name, so `declare_named_type`'s
            // `unreachable!` fired on it the moment this match's result
            // type was needed. See `match_result_ty` for the fix.
            Expr::Match { scrutinee, arms, .. } => self.match_result_ty(&self.local_ty_of(scrutinee, scopes), arms, scopes),
            _ => Ty::I64,
        }
    }


    /// `local_ty_of`'s `Expr::Match` case: `arms[0].body`'s type
    /// represents the whole match, *unless* it's a bare `Ok`/`Err`
    /// reconstruction whose missing type parameter only a sibling arm
    /// can supply -- see the call site's own comment for the failure
    /// this fixes. Looks at every arm together specifically to catch
    /// that case; falls back to the first arm alone (the previous,
    /// still-correct-for-every-other-shape behavior) otherwise.
    pub(super) fn match_result_ty(&self, scrutinee_ty: &Ty, arms: &[MatchArm], scopes: &Scopes) -> Ty {
        // The scrutinee being a `Result(t, e)` is ground truth for what
        // `Ok`/`Err` mean in any arm matching it -- consulted below
        // whenever an arm's own pattern binding (`v` in `Ok(v) => ...`)
        // isn't in `scopes` yet at this point: real per-arm codegen (and
        // its `scopes.push()` of that binding) happens *after* this
        // return type is already needed, further down in `match_expr`.
        let result_args = match scrutinee_ty {
            Ty::Named(name, args) if name == "Result" && args.len() == 2 => Some((&args[0], &args[1])),
            _ => None,
        };
        // The overwhelmingly common shape a bare reconstruction arm
        // has: `Ok(v) => Ok(v)`/`Err(e) => Err(e)`, the payload
        // expression is exactly the arm's own pattern binding,
        // unchanged. **This proves only the one type parameter that
        // variant's payload actually touches** (`Ok`'s only ever `T`,
        // `Err`'s only ever `E`) -- it is *not* license to assume the
        // *other* parameter also matches the scrutinee's, which a
        // sibling arm changing it (`Err(e) => Err(0)` turning a `str`
        // scrutinee-error into an `i64` one, an ordinary "normalize the
        // error type" pattern) makes outright false. So this returns
        // just the one proven component, `(is_ok, component)`, for the
        // combine step below to use -- never a whole `Result(T, E)` on
        // a single passthrough arm's say-so alone.
        let passthrough_component = |arm: &MatchArm| -> Option<(bool, Ty)> {
            if let Expr::Call(name, args, _) = arm.body.as_ref() {
                if (name == "Ok" || name == "Err") && args.len() == 1 {
                    if let (Expr::Ident(id, _), Some(bound), Some((t, e))) = (&args[0], arm.bindings.first(), result_args) {
                        if id == bound {
                            return Some((name == "Ok", if name == "Ok" { t.clone() } else { e.clone() }));
                        }
                    }
                }
            }
            None
        };
        // Phase 1: any arm whose body is *not* this passthrough shape
        // might still fully self-resolve via plain `local_ty_of` --
        // e.g. a nested `if`/`match` that itself bottoms out in
        // something concrete (recursing into this same function, whose
        // own combine step may resolve it). A bare `Ok(x)`/`Err(y)`
        // wrapping something other than its own binding (`Err(DbError
        // (e))`) is *not* skipped here but never actually resolves this
        // way either (`ctor_ty` structurally can't bind `Ok`/`Err`'s
        // *other* type parameter from any single call, passthrough or
        // not) -- it always falls through to phase 2's `args[0]`-only
        // extraction below, which sidesteps that limitation entirely by
        // never asking `ctor_ty` about the outer `Ok`/`Err` call at all.
        // Every arm is tried, not just the first, because typeck already
        // proved they all agree; which one (if any) is resolvable this
        // way varies per arm.
        for arm in arms {
            if passthrough_component(arm).is_some() {
                continue;
            }
            let ty = self.arm_body_ty(scrutinee_ty, arm, scopes);
            if !is_unresolved_ok_err_placeholder(&ty) {
                return ty;
            }
        }
        // Phase 2: every arm was either the passthrough shape or
        // independently unresolvable alone. The remaining case this can
        // still recover, and the one every real crash this fixed
        // actually hit: an `Ok`-shaped arm supplies `T` (whether via
        // scrutinee substitution for a passthrough, or `local_ty_of` on
        // its own payload expression for anything else), a *different*
        // `Err`-shaped arm supplies `E`, and combining the two (which
        // neither alone could) is fully correct, not a guess — typeck
        // already proved this match produces exactly one `Result(T, E)`.
        let mut ok_ty = None;
        let mut err_ty = None;
        for arm in arms {
            if let Some((is_ok, component)) = passthrough_component(arm) {
                if is_ok {
                    ok_ty.get_or_insert(component);
                } else {
                    err_ty.get_or_insert(component);
                }
                continue;
            }
            if let Expr::Call(name, args, _) = arm.body.as_ref() {
                if name == "Ok" && args.len() == 1 {
                    ok_ty.get_or_insert_with(|| self.local_ty_of(&args[0], scopes));
                } else if name == "Err" && args.len() == 1 {
                    err_ty.get_or_insert_with(|| self.local_ty_of(&args[0], scopes));
                }
            }
        }
        match (ok_ty, err_ty) {
            (Some(t), Some(e)) => Ty::Named("Result".to_string(), vec![t, e]),
            _ => self.arm_body_ty(scrutinee_ty, &arms[0], scopes),
        }
    }


    /// `local_ty_of(&arm.body, scopes)`, but with `arm`'s own pattern
    /// bindings (`v` in `Ok(v) => ...`) visible first — a real bug found
    /// compiling `Ok(v) => v.subject` against `Result(VerifiedIdentity,
    /// str)` for the first time (`verify_session`'s natural usage): at
    /// the point a match's own result type is needed, the real per-arm
    /// binding (`match_enum`'s own codegen loop, further down) hasn't
    /// run yet, so a naive `local_ty_of(&arm.body, scopes)` resolves
    /// `v.subject`'s field type against whatever stale/absent binding
    /// `scopes` already had for that name — producing a genuine `'{
    /// ptr, i64 }' but expected 'i64'`-style invalid-IR bug, not a
    /// hypothetical one. Fixed by probing on a scratch *clone* of
    /// `scopes` (this function only ever has `&Scopes`, and widening it
    /// to `&mut Scopes` would ripple into `local_ty_of`'s every other
    /// caller) with the arm's own bindings pushed, substituted the same
    /// way `construct_variant` substitutes a payload's declared type —
    /// discarded immediately after, never observed by the real,
    /// value-carrying binding `match_enum`'s own loop does later.
    pub(super) fn arm_body_ty(&self, scrutinee_ty: &Ty, arm: &MatchArm, scopes: &Scopes) -> Ty {
        let Ty::Named(enum_name, type_args) = scrutinee_ty else { return self.local_ty_of(&arm.body, scopes) };
        if !self.registry.is_enum(enum_name) {
            return self.local_ty_of(&arm.body, scopes);
        }
        let Some(variants) = self.registry.enum_variants(enum_name) else { return self.local_ty_of(&arm.body, scopes) };
        let Some(variant) = variants.iter().find(|v| v.name == arm.variant) else { return self.local_ty_of(&arm.body, scopes) };
        let type_params = self.registry.enum_type_params(enum_name).unwrap_or(&[]);
        let subst = zip_type_params(type_params, type_args);
        let mut probe = scopes.clone();
        probe.push();
        for (name, decl_ty) in arm.bindings.iter().zip(variant.payload.iter()) {
            let field_ty = substitute_ty(decl_ty, &subst);
            // A placeholder value string: this scope frame only exists
            // to answer a type question, never to emit real IR against.
            probe.define(name, field_ty, "undef".to_string());
        }
        self.local_ty_of(&arm.body, &probe)
    }


    /// Mirrors `typeck::infer_array_lit`'s Vector-vs-Matrix shape rule
    /// exactly (minus its diagnostic paths, irrelevant here — codegen
    /// only ever sees an already-well-typed program): element 0's own
    /// type decides everything. A plain scalar element 0 makes the whole
    /// literal a `Vector`; a same-shaped-`Vector` element 0 (of scalars,
    /// not itself nested) makes it a `Matrix`, flattened row-major.
    pub(super) fn array_lit_ty(&self, elements: &[Expr], scopes: &Scopes) -> Ty {
        let t0 = self.local_ty_of(&elements[0], scopes);
        match &t0 {
            Ty::Vector(inner, n) if !matches!(inner.as_ref(), Ty::Vector(..) | Ty::Matrix(..)) => {
                Ty::Matrix(inner.clone(), elements.len(), *n)
            }
            _ => Ty::Vector(Box::new(t0), elements.len()),
        }
    }


    /// Mirrors `typeck::infer_mul`'s shape-resolution table (minus its
    /// diagnostics -- codegen only ever sees an already-well-typed
    /// program, so `Vector * Vector` and other illegal shapes never
    /// reach this): scalar × `Matrix` (either order) keeps the `Matrix`'s
    /// shape, `Matrix * Vector` produces a `Vector` sized to the
    /// `Matrix`'s row count, `Matrix * Matrix` produces a `Matrix` sized
    /// `(rows-of-left, cols-of-right)`, and two matching scalars keep
    /// their shared scalar type.
    pub(super) fn mul_result_ty(&self, lhs: &Expr, rhs: &Expr, scopes: &Scopes) -> Ty {
        let lt = self.local_ty_of(lhs, scopes);
        let rt = self.local_ty_of(rhs, scopes);
        match (&lt, &rt) {
            (s, Ty::Matrix(elem, r, c)) if !s.is_aggregate() => Ty::Matrix(elem.clone(), *r, *c),
            (Ty::Matrix(elem, r, c), s) if !s.is_aggregate() => Ty::Matrix(elem.clone(), *r, *c),
            (Ty::Matrix(m_elem, r, _c), Ty::Vector(..)) => Ty::Vector(m_elem.clone(), *r),
            (Ty::Matrix(l_elem, r1, _c1), Ty::Matrix(_, _r2, c2)) => Ty::Matrix(l_elem.clone(), *r1, *c2),
            _ => lt,
        }
    }


    /// Phase 4's `local_ty_of` analog for a builtin call — mirrors each
    /// builtin's `typeck.rs` signature (minus diagnostics, same "already
    /// well-typed" trust every other `local_ty_of` arm relies on), needed
    /// because `call_ptr`/`call_builtin_agg` have to allocate a
    /// correctly-shaped destination before they know what a `let`'s own
    /// declared type annotation says (they're reached through `expr_ptr`,
    /// which doesn't see that outer context). `zeros`/`ones`/`identity`
    /// read their shape off `literal_value` (`ast.rs`), the same
    /// literal-only rule `typeck::literal_dimension` enforces.
    pub(super) fn builtin_result_ty(&self, name: &str, args: &[Expr], scopes: &Scopes) -> Ty {
        match name {
            "transpose" => match self.local_ty_of(&args[0], scopes) {
                Ty::Matrix(elem, r, c) => Ty::Matrix(elem, c, r),
                _ => Ty::F64,
            },
            "dot" => match self.local_ty_of(&args[0], scopes) {
                Ty::Vector(elem, _) => *elem,
                _ => Ty::F64,
            },
            "cross" => self.local_ty_of(&args[0], scopes),
            "zeros" | "ones" => {
                if args.len() == 1 {
                    let n = literal_value(&args[0]).unwrap_or(0) as usize;
                    Ty::Vector(Box::new(Ty::F64), n)
                } else {
                    let r = literal_value(&args[0]).unwrap_or(0) as usize;
                    let c = literal_value(&args[1]).unwrap_or(0) as usize;
                    Ty::Matrix(Box::new(Ty::F64), r, c)
                }
            }
            "identity" => {
                let n = literal_value(&args[0]).unwrap_or(0) as usize;
                Ty::Matrix(Box::new(Ty::F64), n, n)
            }
            "sum" => match self.local_ty_of(&args[0], scopes) {
                Ty::Vector(elem, _) | Ty::Matrix(elem, _, _) => *elem,
                _ => Ty::F64,
            },
            "len" => Ty::I64,
            "norm" | "norm1" | "norm_inf" | "frobenius_norm" | "distance" | "bearing" => Ty::F64,
            "trace" => match self.local_ty_of(&args[0], scopes) {
                Ty::Matrix(elem, _, _) => *elem,
                _ => Ty::F64,
            },
            "is_symmetric" | "is_diag" | "is_square" => Ty::Bool,
            "lla_to_ecef" | "ecef_to_lla" | "ecef_to_enu" | "enu_to_ecef" => Ty::Vector(Box::new(Ty::F64), 3),
            "kf_predict_state" => match self.local_ty_of(&args[0], scopes) {
                Ty::Vector(elem, n) => Ty::Vector(elem, n),
                _ => Ty::F64,
            },
            "kf_predict_cov" => match self.local_ty_of(&args[0], scopes) {
                Ty::Vector(elem, n) => Ty::Matrix(elem, n, n),
                _ => Ty::F64,
            },
            // Phase 5: `inv` keeps its square-matrix operand's own shape;
            // `solve`/`kf_update_state` return a `Vector` shaped like the
            // state/RHS vector operand; `kf_update_cov` returns a square
            // `Matrix` sized off that same vector's length; `det`/`rank`
            // are scalar (handled by the catch-all fallthrough via their
            // absence here would be wrong -- listed explicitly instead).
            "det" => Ty::F64,
            "inv" => self.local_ty_of(&args[0], scopes),
            "solve" => self.local_ty_of(&args[1], scopes),
            "rank" => Ty::I64,
            "kf_update_state" => self.local_ty_of(&args[0], scopes),
            "kf_update_cov" => match self.local_ty_of(&args[0], scopes) {
                Ty::Vector(elem, n) => Ty::Matrix(elem, n, n),
                _ => Ty::F64,
            },
            _ => unreachable!("PHASE4_BUILTINS/PHASE5_BUILTINS and this match must stay in sync"),
        }
    }


    /// The type an if-expression's value slot needs to be, so it can
    /// correctly hold a `bool` (`i1`) result and not just an integer one
    /// — the fix for the gap this function used to have (a hardcoded
    /// `i64` result slot, wrong for a genuinely `bool`-valued `if` whose
    /// branches both fall through). `typeck::check_if` already proved
    /// both branches agree in type at any real value-position use, so
    /// inspecting only the `then` branch's trailing type is sound *in
    /// principle* — except `local_ty_of` can independently fail to
    /// resolve one specific branch's own trailing type even when the
    /// branches truly do agree (a bare `Ok`/`Err` reconstruction; see
    /// `is_unresolved_ok_err_placeholder`), which is exactly why this is
    /// a thin wrapper `if_result_ty` (below) actually calls, not the
    /// whole story by itself anymore.
    pub(super) fn block_trailing_ty(&self, block: &Block, scopes: &Scopes) -> Ty {
        match block.stmts.last() {
            Some(Stmt::Expr(e)) => self.local_ty_of(e, scopes),
            _ => Ty::Unit,
        }
    }


    /// `if_expr`'s own `result_ty` — `block_trailing_ty(then_block)`
    /// when that resolves to a genuine type, but tries the `else`
    /// branch (and, failing that, combines a bare `Ok`/`Err` on one
    /// side with the other's) before giving up, exactly mirroring
    /// `match_result_ty`'s own three-tier fallback and for the identical
    /// reason: a real captured crash (`nirdosha_hi_*.log`/`.nir`) had
    /// `if cond { Err(DbError(msg)) } else { match ... }` as a `let`'s
    /// RHS — `then`'s own trailing type alone is the unresolvable
    /// placeholder, while `else`'s (a nested `match`, itself already
    /// correctly resolved via `match_result_ty`) is not. `ElseBranch::If`
    /// (an `else if` chain) isn't given the same sibling-combining
    /// treatment — a real, disclosed, narrower gap than the one this
    /// fixes, not a regression: `then`'s own type is still tried first,
    /// so this is never worse than `block_trailing_ty` alone was.
    pub(super) fn if_result_ty(&self, then_block: &Block, else_block: Option<&ElseBranch>, scopes: &Scopes) -> Ty {
        let then_ty = self.block_trailing_ty(then_block, scopes);
        if !is_unresolved_ok_err_placeholder(&then_ty) {
            return then_ty;
        }
        let else_block_body = match else_block {
            Some(ElseBranch::Block(b)) => b.stmts.last().and_then(|s| match s {
                Stmt::Expr(e) => Some(e),
                _ => None,
            }),
            _ => None,
        };
        if let Some(else_body) = else_block_body {
            let else_ty = self.local_ty_of(else_body, scopes);
            if !is_unresolved_ok_err_placeholder(&else_ty) {
                return else_ty;
            }
            // Both sides independently unresolvable alone -- the same
            // last-resort combine `match_result_ty` falls back to:
            // correct, not a guess, since typeck already proved both
            // branches produce the same `Result(T, E)`.
            let then_body = match then_block.stmts.last() {
                Some(Stmt::Expr(e)) => Some(e),
                _ => None,
            };
            let bare_payload = |e: Option<&Expr>, variant: &str| -> Option<Ty> {
                match e {
                    Some(Expr::Call(name, args, _)) if name == variant && args.len() == 1 => Some(self.local_ty_of(&args[0], scopes)),
                    _ => None,
                }
            };
            let ok_ty = bare_payload(then_body, "Ok").or_else(|| bare_payload(Some(else_body), "Ok"));
            let err_ty = bare_payload(then_body, "Err").or_else(|| bare_payload(Some(else_body), "Err"));
            if let (Some(t), Some(e)) = (ok_ty, err_ty) {
                return Ty::Named("Result".to_string(), vec![t, e]);
            }
        }
        then_ty
    }


}
