use super::*;
use super::layout::*;

impl Codegen<'_> {
    /// Every integer-typed value this backend hands around internally is
    /// `i64` — see the module doc's "why arithmetic is always computed
    /// at i64 width" note; this is the *load* side of that: a value just
    /// read out of a narrower-than-`i64` stack slot gets widened
    /// immediately, before it can participate in anything else. `zext`
    /// for an unsigned type (`Ty::is_unsigned`), `sext` for a signed
    /// one — the *only* place this backend needs that distinction at
    /// all (see `llvm_ty`'s `Ty::U8`/etc. arm for why nothing
    /// downstream does). A no-op for `bool` (stays `i1` throughout — it
    /// never enters the i64 scheme) or a value whose LLVM width is
    /// already 64 bits (`Ty::I64`, and also `Ty::U64`/`Ty::Usize`, which
    /// map to the same `i64` width — checked by comparing the actual
    /// LLVM type string, not the `Ty` variant, since `Ty::U64 !=
    /// Ty::I64` even though they compile to the identical width).
    pub(super) fn widen_to_i64(&mut self, val: &str, ty: &Ty) -> String {
        if !ty.is_integer() {
            return val.to_string();
        }
        let llty = self.llvm_ty(ty).expect("check_supported already validated this type");
        if llty == "i64" {
            return val.to_string();
        }
        let r = self.fresh_reg("widen");
        let op = if ty.is_unsigned() { "zext" } else { "sext" };
        writeln!(self.out, "  {r} = {op} {llty} {val} to i64").unwrap();
        r
    }


    /// The *store* side: narrows an `i64` value back down to `ty`'s
    /// actual declared width, right before it's written to a stack slot,
    /// passed as a call argument, or returned. Always lossless when
    /// called on a value that already passed `guard_in_range` for this
    /// same `ty` (that's the whole point of checking *before* narrowing,
    /// not after — see the module doc's "found by testing" note on why
    /// computing directly at the narrow width was wrong).
    pub(super) fn narrow_from_i64(&mut self, val: &str, ty: &Ty) -> Result<String, CodegenError> {
        let llty = self.llvm_ty(ty)?;
        if llty == "i64" {
            return Ok(val.to_string());
        }
        let r = self.fresh_reg("narrow");
        writeln!(self.out, "  {r} = trunc i64 {val} to {llty}").unwrap();
        Ok(r)
    }


    /// Converts `val` (of LLVM type `llty`) into the one `i64` machine
    /// word every `chan`/`spawn` payload crosses the kernel ABI boundary
    /// as (`runtime-kernels/src/lib.rs`'s "chan/spawn/join kernels"
    /// section). Every integer type is already carried at full `i64`
    /// width by the time it reaches here (module doc's own invariant,
    /// enforced by `widen_to_i64`) — only `double`/`ptr`/`i1` need a real
    /// conversion instruction.
    pub(super) fn to_i64_word(&mut self, val: &str, llty: &str) -> String {
        match llty {
            "ptr" => {
                let r = self.fresh_reg("word_ptrtoint");
                writeln!(self.out, "  {r} = ptrtoint ptr {val} to i64").unwrap();
                r
            }
            "double" => {
                let r = self.fresh_reg("word_bitcast");
                writeln!(self.out, "  {r} = bitcast double {val} to i64").unwrap();
                r
            }
            "i1" => {
                let r = self.fresh_reg("word_zext");
                writeln!(self.out, "  {r} = zext i1 {val} to i64").unwrap();
                r
            }
            _ => val.to_string(),
        }
    }


    /// The reverse of `to_i64_word` — unpacks a raw `i64` word (from
    /// `nir_chan_recv`/`nir_thread_join`) back to `llty`'s real shape.
    pub(super) fn from_i64_word(&mut self, val: &str, llty: &str) -> String {
        match llty {
            "ptr" => {
                let r = self.fresh_reg("word_inttoptr");
                writeln!(self.out, "  {r} = inttoptr i64 {val} to ptr").unwrap();
                r
            }
            "double" => {
                let r = self.fresh_reg("word_bitcast");
                writeln!(self.out, "  {r} = bitcast i64 {val} to double").unwrap();
                r
            }
            "i1" => {
                let r = self.fresh_reg("word_trunc");
                writeln!(self.out, "  {r} = trunc i64 {val} to i1").unwrap();
                r
            }
            _ => val.to_string(),
        }
    }


    /// Whether `ty` fits in the one `i64` machine word `chan`/`spawn`'s
    /// payload ABI uses today (`to_i64_word`/`from_i64_word`) — every
    /// plain scalar (`i8`/.../`i64`/`bool`/`f64`), every pointer-shaped
    /// handle (`box`/`ref`), and every existing `i64`-handle type
    /// (`tcp`/`file`/`thread`/`chan`/`sandbox`) qualify; `str`/`dec128`
    /// (two words) and any `Vector`/`Matrix`/struct/enum
    /// (`Ty::is_aggregate()`) don't — a real, disclosed narrower scope
    /// than `chan T`/`spawn`'s full type-level generality (each caller of
    /// this fn explains the gap at its own rejection site).
    pub(super) fn is_word_sized(&mut self, ty: &Ty) -> Result<bool, CodegenError> {
        if ty.is_aggregate() {
            return Ok(false);
        }
        let llty = self.llvm_ty(ty)?;
        Ok(matches!(llty.as_str(), "i1" | "i8" | "i16" | "i32" | "i64" | "double" | "ptr"))
    }


    /// `spawn name(args)`'s real codegen — see `runtime-kernels/src/
    /// lib.rs`'s "chan/spawn/join kernels" section for the kernel side
    /// this calls into. Every parameter and `name`'s return type must be
    /// word-sized (`is_word_sized`) — checked here, not `check_supported`'s
    /// structural pre-pass (which has no signature info), with a specific
    /// reason rather than a confusing fallback error.
    pub(super) fn spawn_thread(&mut self, name: &str, args: &[Expr], span: Span, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let _ = span;
        // Cloned out of `self.sigs` field-by-field (not the whole
        // `FnSig`, which doesn't derive `Clone`) — same pattern `call()`'s
        // own `sig_params`/`sig_ret` locals already use, for the same
        // reason: this function needs `&mut self` again immediately
        // after, and a borrow of `self.sigs` can't outlive that.
        let sig_params = self.sigs.get(name).expect("typeck.rs already resolved this call").params.clone();
        let sig_ret = self.sigs.get(name).expect("typeck.rs already resolved this call").ret.clone();
        for p in &sig_params {
            if !self.is_word_sized(p)? {
                return unsupported(format!(
                    "codegen doesn't support spawning `{name}` yet — its parameter type `{p:?}` \
                     isn't word-sized (only integers/bool/f64/box/ref/handles are supported as \
                     spawn arguments so far, not str/dec128/struct/enum/Vector/Matrix)"
                ));
            }
        }
        if sig_ret != Ty::Unit && !self.is_word_sized(&sig_ret)? {
            return unsupported(format!(
                "codegen doesn't support spawning `{name}` yet — its return type `{sig_ret:?}` \
                 isn't word-sized (only integers/bool/f64/box/ref/handles/unit are supported as \
                 spawn results so far, not str/dec128/struct/enum/Vector/Matrix)"
            ));
        }

        // The heap block `args` get marshaled into, laid out as one
        // anonymous LLVM struct field per parameter — an empty struct
        // (`{}`, `null` ctx pointer, never dereferenced) for a
        // zero-argument spawn.
        let param_lltys: Vec<String> = sig_params.iter().map(|p| self.llvm_ty(p)).collect::<Result<_, _>>()?;
        let ctx_llty = format!("{{{}}}", param_lltys.join(", "));
        let ctx_ptr = if sig_params.is_empty() {
            "null".to_string()
        } else {
            let size = format!("ptrtoint (ptr getelementptr ({ctx_llty}, ptr null, i32 1) to i64)");
            let p = self.fresh_reg("spawn_ctx");
            writeln!(self.out, "  {p} = call ptr @nir_alloc(i64 {size})").unwrap();
            p
        };
        for (i, (a, want)) in args.iter().zip(sig_params.iter()).enumerate() {
            let v = self.expr(a, scopes)?;
            let llty = self.llvm_ty(want)?;
            let v = if want.is_integer() { self.narrow_from_i64(&v, want)? } else { v };
            let field_ptr = self.fresh_reg("spawn_ctx_field");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {ctx_llty}, ptr {ctx_ptr}, i32 0, i32 {i}").unwrap();
            writeln!(self.out, "  store {llty} {v}, ptr {field_ptr}").unwrap();
        }

        let tramp_name = self.fresh_global("spawn_trampoline");
        self.emit_spawn_trampoline(&tramp_name, name, &param_lltys, &ctx_llty, &sig_ret)?;

        let handle = self.fresh_reg("thread_handle");
        writeln!(self.out, "  {handle} = call i64 @nir_thread_spawn(ptr {tramp_name}, ptr {ctx_ptr})").unwrap();
        self.guard_io_ok(&handle);
        Ok(handle)
    }


    /// Emits one top-level trampoline function into `self.trampolines` —
    /// the bridge `nir_thread_spawn`'s raw `extern "C" fn(*mut u8, *mut
    /// i64)` signature needs between the kernel (which knows nothing
    /// about `.nir` argument shapes) and `name`'s real, statically-known
    /// LLVM signature: unpack `ctx`'s fields, free `ctx` (its only job
    /// was carrying the args this far), call `name` for real, and write
    /// its result — widened/converted to one `i64` word by the same
    /// `widen_to_i64`/`to_i64_word` pair `expr()`/`chan` already use, or
    /// left untouched for a `unit`-returning spawn — through
    /// `result_slot`. Built by temporarily swapping `self.trampolines`
    /// into `self.out` (`trampolines`'s own doc comment explains why a
    /// second buffer is needed at all) so every ordinary instruction-
    /// emitting helper this needs just works unmodified, then swapping
    /// back.
    pub(super) fn emit_spawn_trampoline(
        &mut self,
        tramp_name: &str,
        callee_name: &str,
        param_lltys: &[String],
        ctx_llty: &str,
        ret_ty: &Ty,
    ) -> Result<(), CodegenError> {
        let ret_llty = self.llvm_ty(ret_ty)?;
        let saved_out = std::mem::take(&mut self.out);
        writeln!(self.out, "define void {tramp_name}(ptr %ctx, ptr %result_slot) {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        let mut call_args = Vec::with_capacity(param_lltys.len());
        for (i, llty) in param_lltys.iter().enumerate() {
            let field_ptr = self.fresh_reg("tramp_field");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {ctx_llty}, ptr %ctx, i32 0, i32 {i}").unwrap();
            let val = self.fresh_reg("tramp_arg");
            writeln!(self.out, "  {val} = load {llty}, ptr {field_ptr}").unwrap();
            call_args.push(format!("{llty} {val}"));
        }
        if !param_lltys.is_empty() {
            // Every argument was already copied into registers above —
            // the heap block `spawn_thread` allocated for them is only
            // ever needed for this one moment, so it's freed right here
            // rather than leaking one block per spawn forever.
            writeln!(self.out, "  call void @nir_free(ptr %ctx)").unwrap();
        }
        if ret_llty == "void" {
            writeln!(self.out, "  call void @{callee_name}({})", call_args.join(", ")).unwrap();
        } else {
            let r = self.fresh_reg("tramp_result");
            writeln!(self.out, "  {r} = call {ret_llty} @{callee_name}({})", call_args.join(", ")).unwrap();
            let widened = self.widen_to_i64(&r, ret_ty);
            let word = self.to_i64_word(&widened, &ret_llty);
            writeln!(self.out, "  store i64 {word}, ptr %result_slot").unwrap();
        }
        writeln!(self.out, "  ret void").unwrap();
        writeln!(self.out, "}}").unwrap();
        writeln!(self.out).unwrap();
        self.trampolines.push_str(&self.out);
        self.out = saved_out;
        Ok(())
    }


    /// Tear down one affine-typed value whose storage lives at `value_ptr`.
    /// Recurses into `box` contents, `struct` affine fields, and the live
    /// variant's affine payload fields of `enum`s. `value_ptr` is always a
    /// pointer to where the value is stored (a stack slot, a field address,
    /// or the heap address for the contents of a box), not the scalar value
    /// itself.
    pub(super) fn emit_affine_free(&mut self, value_ptr: &str, ty: &Ty) {
        match ty {
            Ty::Box(inner) => {
                let heap_ptr = self.fresh_reg("box.heap_ptr");
                writeln!(self.out, "  {heap_ptr} = load ptr, ptr {value_ptr}").unwrap();
                if self.registry.is_affine(inner) {
                    self.emit_affine_free(&heap_ptr, inner);
                }
                writeln!(self.out, "  call void @nir_free(ptr {heap_ptr})").unwrap();
            }
            Ty::Tcp | Ty::TcpListener => {
                let fd = self.fresh_reg("tcp.fd");
                writeln!(self.out, "  {fd} = load i64, ptr {value_ptr}").unwrap();
                writeln!(self.out, "  call i32 @nir_tcp_stop(i64 {fd})").unwrap();
            }
            // The real delivery of RFC 0006 Pillar 4's "no orphan
            // threads" for this backend: a `thread` binding a function
            // never explicitly `join`s is auto-joined right here, at
            // every scope-closing point this function's `FreeMap` already
            // walks for `box`/`tcp` — so a function genuinely cannot
            // return while something it spawned (and didn't hand off
            // elsewhere) is still outstanding, even though it isn't the
            // RFC prototype's own lexical-`Scope`-per-function mechanism
            // (`runtime-kernels/src/lib.rs`'s "chan/spawn/join kernels"
            // section doc comment has the honest gap: every `spawn` gets
            // its own one-job `Scope`, not one shared per spawning
            // function). The result is discarded — the same "this
            // binding's value was never going to be read again" posture
            // `Ty::Box`'s own free above already has.
            Ty::Thread(_) => {
                let handle = self.fresh_reg("thread.handle");
                writeln!(self.out, "  {handle} = load i64, ptr {value_ptr}").unwrap();
                writeln!(self.out, "  call i64 @nir_thread_join(i64 {handle})").unwrap();
            }
            Ty::Named(name, args) => {
                if let Some(fields) = self.registry.struct_fields(name) {
                    let type_params = self.registry.struct_type_params(name).unwrap_or(&[]);
                    let subst = zip_type_params(type_params, args);
                    let struct_llty = self.llvm_ty(ty).expect("check_supported already accepted this type");
                    for (i, f) in fields.iter().enumerate() {
                        let field_ty = substitute_ty(&f.ty, &subst);
                        if self.registry.is_affine(&field_ty) {
                            let field_ptr = self.fresh_reg("struct_field.addr");
                            writeln!(
                                self.out,
                                "  {field_ptr} = getelementptr inbounds {struct_llty}, ptr {value_ptr}, i32 0, i32 {i}"
                            )
                            .unwrap();
                            self.emit_affine_free(&field_ptr, &field_ty);
                        }
                    }
                } else if let Some(variants) = self.registry.enum_variants(name) {
                    let type_params = self.registry.enum_type_params(name).unwrap_or(&[]);
                    let subst = zip_type_params(type_params, args);
                    let enum_llty = self.llvm_ty(ty).expect("check_supported already accepted this type");
                    let tag_ptr = self.fresh_reg("enum_tag.addr");
                    writeln!(
                        self.out,
                        "  {tag_ptr} = getelementptr inbounds {enum_llty}, ptr {value_ptr}, i32 0, i32 0"
                    )
                    .unwrap();
                    let tag = self.fresh_reg("enum_tag");
                    writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
                    let payload = self.fresh_reg("enum_payload.addr");
                    writeln!(
                        self.out,
                        "  {payload} = getelementptr inbounds {enum_llty}, ptr {value_ptr}, i32 0, i32 1"
                    )
                    .unwrap();

                    let mut case_labels: Vec<String> = Vec::new();
                    for _ in variants.iter() {
                        case_labels.push(self.fresh_label("enum_free_arm"));
                    }
                    let default_label = self.fresh_label("enum_free_default");
                    let merge_label = self.fresh_label("enum_free_merge");

                    writeln!(self.out, "  switch i64 {tag}, label %{default_label} [").unwrap();
                    for (vidx, label) in case_labels.iter().enumerate() {
                        writeln!(self.out, "    i64 {vidx}, label %{label}").unwrap();
                    }
                    writeln!(self.out, "  ]").unwrap();

                    writeln!(self.out, "{default_label}:").unwrap();
                    writeln!(self.out, "  unreachable").unwrap();

                    for (variant, label) in variants.iter().zip(case_labels.iter()) {
                        writeln!(self.out, "{label}:").unwrap();
                        let mut word_off: u64 = 0;
                        for decl_ty in variant.payload.iter() {
                            let field_ty = substitute_ty(decl_ty, &subst);
                            if self.registry.is_affine(&field_ty) {
                                let field_ptr = self.fresh_reg("enum_payload_field.addr");
                                writeln!(
                                    self.out,
                                    "  {field_ptr} = getelementptr inbounds i64, ptr {payload}, i64 {word_off}"
                                )
                                .unwrap();
                                self.emit_affine_free(&field_ptr, &field_ty);
                            }
                            word_off += conservative_word_count(&field_ty, &self.registry);
                        }
                        writeln!(self.out, "  br label %{merge_label}").unwrap();
                    }

                    writeln!(self.out, "{merge_label}:").unwrap();
                }
            }
            _ => {}
        }
    }


    /// Emits teardown for every named binding in a `FreeMap` entry,
    /// looking each one up in the scope it's still live in. The binding's
    /// stack slot pointer is passed straight to `emit_affine_free`, which
    /// decides whether the slot contains a heap pointer (`box`), an fd
    /// (`tcp`), or a struct/enum value with affine fields. Callers are
    /// every scope-closing point `FreeMap`'s doc comment lists — always
    /// guarded by `!self.terminated` except `Stmt::Return`'s own call,
    /// which fires unconditionally since it's what's *about* to set
    /// `terminated`, not something already past it.
    pub(super) fn emit_frees_for_names(&mut self, names: &[String], scopes: &Scopes) {
        for name in names {
            let Some((ty, stack_ptr)) = scopes.get(name) else {
                // Only reachable for a name whose owning scope already
                // popped by the time this runs — doesn't happen for any
                // `FreeMap` entry as constructed (every entry is recorded
                // strictly before its own scope's pop), but a silent
                // no-op is the honest response to "nothing left to free"
                // rather than a panic over an invariant this function
                // itself doesn't own.
                continue;
            };
            self.emit_affine_free(&stack_ptr, &ty);
        }
    }


    /// Emits `nir_nfr_call_end` at one of this function's own return
    /// points — a no-op if this function has no `nfr(...)` at all
    /// (`current_fn_nfr_regs` is `None`). `was_err` is a literal `"0"`/
    /// `"1"` or an `i32`-typed SSA register — whichever the caller
    /// already computed (only the `Result`-returning, `error_rate_max`-
    /// declaring case needs a real one; every other return site just
    /// passes `"0"`, this function's own doc comment on `current_fn_nfr`).
    pub(super) fn emit_nfr_call_end(&mut self, was_err: &str) {
        if let Some((id, start)) = self.current_fn_nfr_regs.clone() {
            writeln!(self.out, "  call void @nir_nfr_call_end(i64 {id}, i64 {start}, i32 {was_err})").unwrap();
        }
    }


    /// Masks every `requires(...)`-annotated field of a struct value
    /// about to be returned — `src` is a pointer to the already-fully-
    /// constructed value (masking happens *in place*, before the caller
    /// copies it out, so the copied-out value is the masked one). A
    /// no-op for any `ret_ty` that isn't a struct with at least one
    /// masked field — the overwhelmingly common case, and deliberately
    /// checked as plain data lookups (no branch emitted at all) rather
    /// than emitting dead always-false checks for functions that never
    /// need any of this.
    pub(super) fn emit_field_masking(&mut self, src: &str, ret_ty: &Ty, scopes: &mut Scopes) -> Result<(), CodegenError> {
        let Ty::Named(name, args) = ret_ty else { return Ok(()) };
        let Some(fields) = self.registry.struct_fields(name) else { return Ok(()) };
        if !fields.iter().any(|f| f.mask_requires.is_some()) {
            return Ok(());
        }
        let type_params = self.registry.struct_type_params(name).unwrap_or(&[]);
        let subst = zip_type_params(type_params, args);
        let struct_llty = self.llvm_ty(ret_ty)?;
        for (i, field) in fields.iter().enumerate() {
            let Some(req) = field.mask_requires.clone() else { continue };
            let field_ty = substitute_ty(&field.ty, &subst);
            let authorized = self.emit_requirement_check(&req, scopes)?;
            let do_label = self.fresh_label("mask_do");
            let skip_label = self.fresh_label("mask_skip");
            writeln!(self.out, "  br i1 {authorized}, label %{skip_label}, label %{do_label}").unwrap();
            writeln!(self.out, "{do_label}:").unwrap();
            let field_ptr = self.fresh_reg("mask_field_ptr");
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {struct_llty}, ptr {src}, i32 0, i32 {i}").unwrap();
            let zero = self.emit_zero_value(&field_ty)?;
            let field_llty = self.llvm_ty(&field_ty)?;
            writeln!(self.out, "  store {field_llty} {zero}, ptr {field_ptr}").unwrap();
            writeln!(self.out, "  br label %{skip_label}").unwrap();
            writeln!(self.out, "{skip_label}:").unwrap();
        }
        Ok(())
    }


    /// Whether the *current function's own* proof parameter
    /// (`current_fn_role_view_param`/`current_fn_claim_view_param`)
    /// satisfies `req`, as a fresh `i1` SSA register — `"false"`
    /// (fail-closed) if this function has no such parameter at all, the
    /// same "absence of proof is not proof of absence of a requirement"
    /// posture `requires(...)`'s own fn-level gating already has.
    pub(super) fn emit_requirement_check(&mut self, req: &Requirement, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let (param_name, expected, field_name): (Option<String>, &str, &str) = match req {
            Requirement::Role(r) => (self.current_fn_role_view_param.clone(), r, "role"),
            Requirement::Claim(_, v) => (self.current_fn_claim_view_param.clone(), v, "value"),
        };
        let Some(param_name) = param_name else {
            return Ok("false".to_string());
        };
        let (view_ty, view_ptr) = scopes.get(&param_name).expect("scanned directly from this function's own params");
        self.emit_str_field_eq_check(&view_ty, &view_ptr, field_name, expected)
    }


    /// The shared core `emit_requirement_check` (a `RoleView`/`ClaimView`
    /// *parameter* of the current function) and `emit_acquire` (an
    /// arbitrary `proof` expression's own pointer) both reduce to: GEP to
    /// `field_name` on a value of type `view_ty` at `view_ptr`, load its
    /// `str`, and compare against the compile-time-known `expected`
    /// string via `nir_str_eq` — the one real runtime check either
    /// mechanism ever does. Returns a fresh `i1` SSA register.
    pub(super) fn emit_str_field_eq_check(
        &mut self,
        view_ty: &Ty,
        view_ptr: &str,
        field_name: &str,
        expected: &str,
    ) -> Result<String, CodegenError> {
        let (idx, _) = self
            .field_index_and_ty(view_ty, field_name)
            .expect("RoleView/ClaimView always declares this field, ast::prelude_structs");
        let view_llty = self.llvm_ty(view_ty)?;
        let field_ptr = self.fresh_reg("req_field_ptr");
        writeln!(self.out, "  {field_ptr} = getelementptr inbounds {view_llty}, ptr {view_ptr}, i32 0, i32 {idx}").unwrap();
        let field_val = self.fresh_reg("req_field_val");
        writeln!(self.out, "  {field_val} = load {{ptr, i64}}, ptr {field_ptr}").unwrap();
        let actual_ptr = self.fresh_reg("req_actual_ptr");
        writeln!(self.out, "  {actual_ptr} = extractvalue {{ptr, i64}} {field_val}, 0").unwrap();
        let actual_len = self.fresh_reg("req_actual_len");
        writeln!(self.out, "  {actual_len} = extractvalue {{ptr, i64}} {field_val}, 1").unwrap();
        let lit_global = self.fresh_global("req_expected");
        let escaped = llvm_escape_bytes(expected.as_bytes());
        writeln!(self.string_globals, "{lit_global} = private unnamed_addr constant [{} x i8] c\"{escaped}\"", expected.len()).unwrap();
        let eq = self.fresh_reg("req_eq");
        writeln!(
            self.out,
            "  {eq} = call i32 @nir_str_eq(ptr {actual_ptr}, i64 {actual_len}, ptr {lit_global}, i64 {})",
            expected.len()
        )
        .unwrap();
        let authorized = self.fresh_reg("req_authorized");
        writeln!(self.out, "  {authorized} = icmp ne i32 {eq}, 0").unwrap();
        Ok(authorized)
    }


    /// The zero value for a masked field's own type, as an operand ready
    /// to `store` — a bare literal for a true scalar, or a couple of
    /// `insertvalue` instructions (returning a fresh SSA register) for
    /// the two-word non-aggregate shapes (`str`, `dec128`). Never reached
    /// for an aggregate or affine type — `typeck.rs`'s
    /// `MaskRequiresNeedsScalarField` already rejects those.
    pub(super) fn emit_zero_value(&mut self, ty: &Ty) -> Result<String, CodegenError> {
        let llty = self.llvm_ty(ty)?;
        Ok(match llty.as_str() {
            "i1" => "false".to_string(),
            "double" => llvm_f64_literal(0.0),
            "ptr" => "null".to_string(),
            "{ptr, i64}" => {
                let partial = self.fresh_reg("mask_zero_str");
                writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr null, 0").unwrap();
                let full = self.fresh_reg("mask_zero_str");
                writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 0, 1").unwrap();
                full
            }
            "{i64, i64}" => {
                let partial = self.fresh_reg("mask_zero_dec");
                writeln!(self.out, "  {partial} = insertvalue {{i64, i64}} undef, i64 0, 0").unwrap();
                let full = self.fresh_reg("mask_zero_dec");
                writeln!(self.out, "  {full} = insertvalue {{i64, i64}} {partial}, i64 0, 1").unwrap();
                full
            }
            _ => "0".to_string(), // every remaining case is a plain integer width
        })
    }


}
