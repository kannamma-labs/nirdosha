use super::*;
use super::layout::*;

impl Codegen<'_> {
    pub(super) fn while_loop(&mut self, span: Span, cond: &Expr, body: &Block, scopes: &mut Scopes) -> Result<(), CodegenError> {
        let cond_label = self.fresh_label("while_cond");
        let body_label = self.fresh_label("while_body");
        let after_label = self.fresh_label("while_after");

        writeln!(self.out, "  br label %{cond_label}").unwrap();
        writeln!(self.out, "{cond_label}:").unwrap();
        self.terminated = false;
        let c = self.expr(cond, scopes)?;
        writeln!(self.out, "  br i1 {c}, label %{body_label}, label %{after_label}").unwrap();

        writeln!(self.out, "{body_label}:").unwrap();
        self.terminated = false;
        scopes.push();
        self.stmts(&body.stmts, scopes)?;
        // Once per iteration (this IR runs every time around the loop),
        // not once total — a `box` allocated fresh each iteration reuses
        // the same hoisted stack slot (`entry_allocas`) but gets a brand
        // new heap block from `nir_alloc` every time, so the *previous*
        // iteration's block has to be freed here before it's overwritten,
        // or it leaks on every iteration but the last.
        if !self.terminated
            && let Some(names) = self.free_map.at_while_end.get(&span).cloned()
        {
            self.emit_frees_for_names(&names, scopes);
        }
        scopes.pop();
        if !self.terminated {
            writeln!(self.out, "  br label %{cond_label}").unwrap();
        }

        writeln!(self.out, "{after_label}:").unwrap();
        self.terminated = false;
        Ok(())
    }


    /// Evaluates `e`, returning an LLVM value operand (a register name
    /// like `%foo.3`, or a literal like `5`/`true`) ready to drop
    /// directly into another instruction.
    pub(super) fn expr(&mut self, e: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        match e {
            Expr::Int(n, _) => Ok(n.to_string()),
            Expr::Bool(b, _) => Ok(if *b { "true".to_string() } else { "false".to_string() }),
            // LLVM's own hex float literal format (`0x` + 16 hex digits
            // of the IEEE 754 binary64 bit pattern) — not a plain
            // decimal like `3.14`: a decimal literal only round-trips
            // exactly if LLVM's parser and Rust's `f64` formatter agree
            // on every rounding decision, which isn't guaranteed. The
            // bit pattern is unambiguous by construction.
            Expr::Float(f, _) => Ok(format!("0x{:016X}", f.to_bits())),
            Expr::Str(s, _) => {
                let bytes = s.as_bytes();
                let global = self.fresh_global("str");
                let escaped = llvm_escape_bytes(bytes);
                writeln!(
                    self.string_globals,
                    "{global} = private unnamed_addr constant [{} x i8] c\"{escaped}\\00\"",
                    bytes.len() + 1
                )
                .unwrap();
                // `{ptr, i64}` built via `insertvalue` from `undef` — a
                // first-class LLVM struct value, exactly like any other
                // `expr()` result, not routed through `expr_ptr()`'s
                // alloca-and-pointer convention (`Ty::Str` isn't
                // `is_aggregate()` — see `llvm_ty`'s note).
                let partial = self.fresh_reg("str_val");
                writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {global}, 0").unwrap();
                let full = self.fresh_reg("str_val");
                writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {}, 1", bytes.len()).unwrap();
                Ok(full)
            }
            Expr::Ident(name, _) => {
                let Some((ty, ptr)) = scopes.get(name) else {
                    // Not a local variable — the one other thing an
                    // `Ident` can name is a bare reference to an ordinary
                    // (ungated) top-level `fn` used as a first-class
                    // value (`apply(double, 21)`, LANGUAGE.md §6a). A
                    // function's own address is a compile-time-known
                    // constant operand in LLVM IR (`ptr @name`) — no
                    // `load` needed, unlike a real variable's storage.
                    // `typeck.rs`'s `PrivilegedFnNotAcquired` already
                    // guarantees a `requires`-gated fn's name never
                    // reaches here directly.
                    self.sigs.get(name).expect("typeck.rs already proved this resolves (local var or top-level fn)");
                    return Ok(format!("@{name}"));
                };
                if ty.is_aggregate() {
                    // Every well-behaved caller checks `is_aggregate()`
                    // first and calls `expr_ptr()` instead — this is a
                    // defense-in-depth guard against a construct this
                    // phase doesn't support routing correctly yet (e.g.
                    // an aggregate value inside an `if`/`while`
                    // expression's value slot), so it fails as a clean
                    // `CodegenError`, not a silently wrong `load` of an
                    // array through a scalar-typed instruction.
                    return unsupported(format!(
                        "codegen doesn't support using `{name}` (a Vector/Matrix) in this \
                         expression position yet — bind it via `let`, or pass/return it \
                         through a function call"
                    ));
                }
                if ty == Ty::Unit {
                    // No data to load — `Ty::Unit`'s own `llvm_ty` is
                    // `void`, and `load void` is invalid LLVM IR. Only
                    // reachable via a match arm that binds a `Ty::Unit`
                    // payload and then references it directly (e.g.
                    // `Ok(u) => u`) — the same placeholder value every
                    // other unit-shaped result in this file already
                    // uses ("its own value is unit; never [meaningfully]
                    // read").
                    return Ok("0".to_string());
                }
                let llty = self.llvm_ty(&ty)?;
                let reg = self.fresh_reg(&format!("{name}.val"));
                writeln!(self.out, "  {reg} = load {llty}, ptr {ptr}").unwrap();
                Ok(self.widen_to_i64(&reg, &ty))
            }
            Expr::Unary(UnOp::Neg, inner, _) => {
                // `inner` is already i64 (every integer-typed `expr()`
                // result is) — no need to consult its declared width at
                // all anymore, which used to be `local_ty_of`'s job here.
                // `f64` has no such invariant (its `expr()` result is
                // always genuinely `double`), so it's the one case here
                // that still needs `local_ty_of` to pick the right
                // instruction.
                let v = self.expr(inner, scopes)?;
                if self.local_ty_of(inner, scopes) == Ty::F64 {
                    let r = self.fresh_reg("fneg");
                    writeln!(self.out, "  {r} = fneg double {v}").unwrap();
                    return Ok(r);
                }
                let r = self.fresh_reg("neg");
                writeln!(self.out, "  {r} = sub i64 0, {v}").unwrap();
                Ok(r)
            }
            Expr::Unary(UnOp::Not, inner, _) => {
                let v = self.expr(inner, scopes)?;
                let r = self.fresh_reg("not");
                writeln!(self.out, "  {r} = xor i1 {v}, true").unwrap();
                Ok(r)
            }
            Expr::Binary(op, lhs, rhs, span) => self.binary(*op, lhs, rhs, *span, scopes),
            Expr::Call(name, args, _) => self.call(name, args, scopes),
            Expr::If { cond, then_block, else_block, span } => self.if_expr(cond, then_block, else_block.as_deref(), *span, None, scopes),
            Expr::Assign(name, rhs, span) => {
                let (ty, ptr) = scopes.get(name).expect("typeck.rs already proved this resolves");
                if ty.is_aggregate() {
                    // Same defense-in-depth reasoning as `Expr::Ident`
                    // above — every real caller (`Stmt::Expr`) already
                    // forks to `expr_ptr()`'s own `Expr::Assign` arm for
                    // an aggregate target before ever reaching here.
                    return unsupported(format!(
                        "codegen doesn't support assigning to `{name}` (a Vector/Matrix) in \
                         this expression position yet"
                    ));
                }
                let val = self.expr(rhs, scopes)?; // i64 (or i1 for bool)
                let val = self.guard_in_range(&val, &ty, *span)?; // checked at i64 width
                let store_val = if ty.is_integer() { self.narrow_from_i64(&val, &ty)? } else { val.clone() };
                let llty = self.llvm_ty(&ty)?;
                writeln!(self.out, "  store {llty} {store_val}, ptr {ptr}").unwrap();
                // `val` (still i64/i1), not `store_val` (narrow) — every
                // other `expr()` result is i64 for an integer type, and
                // an assignment-expression's own value has to match that
                // convention so a caller combining it further doesn't
                // need to know it came from an assignment specifically.
                Ok(val)
            }
            Expr::Acquire(_, _, _) | Expr::SpawnSandbox(_, _, _) => {
                unreachable!("check_supported already rejected this program")
            }
            // `transact { ... }` — always `Ty::Bool`-valued, never
            // aggregate, so it belongs here in `expr()`, not `expr_ptr()`
            // (unlike `Expr::Acquire`/`Expr::If`/`Expr::Match`, which can
            // be either and so are dispatched from both). `network_retry`/
            // `network_timeout` are already rejected in `check_expr`'s
            // pre-pass if present — `emit_transact`'s own doc comment has
            // the full scope.
            Expr::Transact { precheck, network, verify, commit, compensate, log, .. } => {
                self.emit_transact(precheck, network, verify, commit, compensate, log, scopes)
            }
            // `chan`'s own construction — one opaque `i64` handle,
            // identical for every payload type `T` (`llvm_ty`'s own
            // `Ty::Channel` arm), so there's nothing here that needs to
            // know what `T` is.
            Expr::Chan(_span) => {
                let handle = self.fresh_reg("chan_new");
                writeln!(self.out, "  {handle} = call i64 @nir_chan_new()").unwrap();
                Ok(handle)
            }
            // `spawn name(args)` — see `runtime-kernels/src/lib.rs`'s
            // "chan/spawn/join kernels" section for the real
            // `nir_thread_spawn` mechanics this lowers to: `args` are
            // marshaled into a heap-allocated context block, a fresh
            // per-call-site trampoline function unpacks that block, calls
            // `name` for real, and writes its (widened-to-one-word)
            // result back into a kernel-owned slot the eventual `join`
            // reads. `typeck.rs::infer_spawn` already proved `args` type-
            // check exactly like a call to `name` and rejected spawning a
            // builtin — this only adds the narrower, disclosed-here
            // restriction that every parameter and the return type must
            // be word-sized (no `str`/`dec128`/struct/enum/Vector/Matrix
            // yet — `is_word_sized`'s own doc comment).
            Expr::Spawn(name, args, span) => self.spawn_thread(name, args, *span, scopes),
            // `join t` — blocks on `t`'s own dedicated `Scope` (never any
            // other spawn's), then unpacks its one-word result back to
            // `T`'s real shape. `typeck.rs` already proved `t: thread<T>`.
            Expr::Join(inner, _span) => {
                let thread_ty = self.local_ty_of(inner, scopes);
                let ret_ty = match thread_ty {
                    Ty::Thread(t) => *t,
                    other => unreachable!("typeck.rs already restricted join's operand to thread, got {other:?}"),
                };
                let handle = self.expr(inner, scopes)?;
                let raw = self.fresh_reg("join_raw");
                writeln!(self.out, "  {raw} = call i64 @nir_thread_join(i64 {handle})").unwrap();
                if ret_ty == Ty::Unit {
                    Ok("0".to_string()) // join's own value is unit; never read
                } else {
                    let ret_llty = self.llvm_ty(&ret_ty)?;
                    Ok(self.from_i64_word(&raw, &ret_llty))
                }
            }
            // `open(path, mode)` — `path`/`mode` are both `str`, matching
            // `nir_file_open`'s `{ptr, len}` x2 signature exactly
            // (`runtime-kernels/src/lib.rs`). `-1` (bad mode string, or a real
            // I/O failure `interpreter.rs`'s own `Expr::Open` would
            // return as `Err`) traps via `guard_io_ok`, the same
            // "checker can't see this coming, so trap at runtime"
            // treatment `nir_tcp_connect`'s own failure path already
            // gets.
            Expr::Open(path, mode, _span) => {
                let (path_ptr, path_len) = self.str_parts(path, scopes)?;
                let (mode_ptr, mode_len) = self.str_parts(mode, scopes)?;
                let fd = self.fresh_reg("file_open_fd");
                writeln!(self.out, "  {fd} = call i64 @nir_file_open(ptr {path_ptr}, i64 {path_len}, ptr {mode_ptr}, i64 {mode_len})").unwrap();
                self.guard_io_ok(&fd);
                Ok(fd)
            }
            // Row 11: `base.field` where the field is a plain scalar —
            // GEP to the field's index within `base`'s real named struct
            // type, then `load` the field's own LLVM type, exactly the
            // shape every other scalar `expr()` arm has. An
            // aggregate-typed field (a nested struct/Vector/Matrix) never
            // reaches `expr()` — `local_ty_of` reports it as
            // `is_aggregate()`, so `Stmt::Let`/`Stmt::Expr`/`call_args`
            // all route it to `expr_ptr()`'s own `Expr::FieldAccess` arm
            // instead; the guard below keeps this arm a clean
            // `CodegenError` rather than a wrong-shaped `load` if some
            // future construct defies that routing.
            Expr::FieldAccess(base, field, span) => {
                let base_ty = self.local_ty_of(base, scopes);
                let (idx, field_ty) = self
                    .field_index_and_ty(&base_ty, field)
                    .expect("typeck.rs already proved this base is a struct with this field");
                if field_ty.is_aggregate() {
                    return unsupported(format!(
                        "codegen doesn't support an aggregate-typed field access `.{field}` at {span:?} in this scalar expression position yet — bind it via `let` instead (same pre-existing gap `if`/`match` themselves have for an aggregate result)"
                    ));
                }
                let base_ptr = self.expr_ptr(base, scopes)?;
                let base_llty = self.llvm_ty(&base_ty)?;
                let gep = self.fresh_reg("fieldptr");
                writeln!(self.out, "  {gep} = getelementptr inbounds {base_llty}, ptr {base_ptr}, i32 0, i32 {idx}").unwrap();
                let field_llty = self.llvm_ty(&field_ty)?;
                let loaded = self.fresh_reg("fieldval");
                writeln!(self.out, "  {loaded} = load {field_llty}, ptr {gep}").unwrap();
                Ok(self.widen_to_i64(&loaded, &field_ty))
            }
            // Row 11: `match scrutinee { ... }` with a scalar result —
            // the overwhelmingly common real case (`structs_enums.nir`'s
            // own `area()`). An aggregate-result `match` is deliberately
            // out of scope here, the same pre-existing gap `expr_ptr`'s
            // own `_ => unsupported(...)` already covers for `if` — it
            // fails cleanly via `expr_ptr_expected`'s `expr_ptr` fallback
            // rather than being silently absent.
            Expr::Match { scrutinee, arms, span } => self.match_expr(scrutinee, arms, *span, None, scopes),
            // `box e` — heap-allocate `e`'s type's own byte size
            // (`ty_byte_size`, covers every `Ty` a `box` can wrap, not
            // just aggregates), copy `e`'s value in, return the heap
            // pointer as this expression's own value. `Ty::Box` is a
            // single pointer word (`llvm_ty`), so — like `Ty::Str` — it's
            // an ordinary `expr()` result, never routed through
            // `expr_ptr()`'s sret convention.
            //
            // **Allocation only — the `free` half lives elsewhere, not
            // because it's missing.** This arm just calls `nir_alloc`;
            // the matching `nir_free` is emitted later, at whichever
            // scope-closing point actually owns this box's last use
            // (`ownership.rs`'s `FreeMap`, consumed by
            // `emit_frees_for_names`/`emit_affine_free`) — a real, working
            // free, not a placeholder (confirmed in generated IR for a
            // simple `let`-bound box). Keeping allocation and free as two
            // separate emission sites mirrors how the rest of this file
            // separates "construct a value" from "clean it up when its
            // scope ends."
            Expr::Box(inner, _) => {
                let inner_ty = self.local_ty_of(inner, scopes);
                let size = ty_byte_size(&inner_ty, &self.registry);
                let heap_ptr = self.fresh_reg("box_heap");
                writeln!(self.out, "  {heap_ptr} = call ptr @nir_alloc(i64 {size})").unwrap();
                if inner_ty.is_aggregate() {
                    let src = self.expr_ptr(inner, scopes)?;
                    writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {heap_ptr}, ptr {src}, i64 {size}, i1 false)").unwrap();
                } else {
                    let v = self.expr(inner, scopes)?;
                    let llty = self.llvm_ty(&inner_ty)?;
                    let v = if inner_ty.is_integer() { self.narrow_from_i64(&v, &inner_ty)? } else { v };
                    writeln!(self.out, "  store {llty} {v}, ptr {heap_ptr}").unwrap();
                }
                Ok(heap_ptr)
            }
            // `froze e` — identical construction to `Expr::Box` above
            // (same heap layout, `Ty::Froze`'s own `llvm_ty` arm), only
            // the resulting *type* differs (non-affine, freely copyable
            // instead of affine) — see `Ty::Froze`'s own doc comment for
            // why this is genuinely the same allocation, never freed.
            Expr::Froze(inner, _) => {
                let inner_ty = self.local_ty_of(inner, scopes);
                let size = ty_byte_size(&inner_ty, &self.registry);
                let heap_ptr = self.fresh_reg("froze_heap");
                writeln!(self.out, "  {heap_ptr} = call ptr @nir_alloc(i64 {size})").unwrap();
                if inner_ty.is_aggregate() {
                    let src = self.expr_ptr(inner, scopes)?;
                    writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {heap_ptr}, ptr {src}, i64 {size}, i1 false)").unwrap();
                } else {
                    let v = self.expr(inner, scopes)?;
                    let llty = self.llvm_ty(&inner_ty)?;
                    let v = if inner_ty.is_integer() { self.narrow_from_i64(&v, &inner_ty)? } else { v };
                    writeln!(self.out, "  store {llty} {v}, ptr {heap_ptr}").unwrap();
                }
                Ok(heap_ptr)
            }
            // `&x` — parser-enforced identifier-only (`typeck.rs` asserts
            // this), so codegen never has to evaluate an arbitrary
            // expression here: `x`'s own storage pointer (its `let`/param
            // alloca, or its own heap pointer if `x: box T`) already *is*
            // the reference's value. No new allocation, no copy, no load
            // — the cheapest possible case, exactly as cheap as the
            // language's own "not affine, freely copyable" treatment of
            // `Ty::Ref` implies it should be.
            Expr::Ref(inner, _) => match inner.as_ref() {
                Expr::Ident(name, _) => {
                    let (_, ptr) = scopes.get(name).expect("typeck.rs already proved this resolves");
                    Ok(ptr)
                }
                _ => unreachable!("parser only ever produces Expr::Ref with an Ident operand"),
            },
            // `*e` — `e`'s own type is always `Box`/`Ref` here
            // (`ownership.rs` already proved this typechecks), so `e`
            // itself is a plain pointer value fetched via `expr()` (never
            // `expr_ptr()` — `Ty::Box`/`Ty::Ref` are never
            // `is_aggregate()`). What's pointed to may or may not be an
            // aggregate; if it is, this arm hands back — this exact
            // pointer, unchanged, no copy — to whichever caller wants an
            // `expr_ptr()`-shaped result instead (see `expr_ptr`'s own
            // `Expr::Deref` arm), matching the interpreter's own
            // `Value::Boxed(inner) | Value::Ref(inner) => *inner`: a
            // dereference exposes the same storage, it doesn't clone it.
            Expr::Deref(inner, span) => {
                let ptr = self.expr(inner, scopes)?;
                let result_ty = self.local_ty_of(e, scopes);
                if result_ty.is_aggregate() {
                    return unsupported(format!(
                        "an aggregate-typed `*` result at {span:?} needs `expr_ptr()`'s \
                         pointer-returning path, not `expr()` — this indicates a caller bug, \
                         not a language-level limitation"
                    ));
                }
                let llty = self.llvm_ty(&result_ty)?;
                let reg = self.fresh_reg("deref_val");
                writeln!(self.out, "  {reg} = load {llty}, ptr {ptr}").unwrap();
                Ok(self.widen_to_i64(&reg, &result_ty))
            }
            Expr::Connect(host, port, _span) => {
                let (host_ptr, host_len) = self.str_parts(host, scopes)?;
                let port_v = self.expr(port, scopes)?;
                let fd = self.fresh_reg("tcp_connect_fd");
                writeln!(self.out, "  {fd} = call i64 @nir_tcp_connect(ptr {host_ptr}, i64 {host_len}, i64 {port_v})").unwrap();
                self.guard_io_ok(&fd);
                Ok(fd)
            }
            Expr::Listen(port, _span) => {
                let port_v = self.expr(port, scopes)?;
                let fd = self.fresh_reg("tcp_listen_fd");
                writeln!(self.out, "  {fd} = call i64 @nir_tcp_listen(i64 {port_v})").unwrap();
                self.guard_io_ok(&fd);
                Ok(fd)
            }
            Expr::Accept(listener, _span) => {
                let listener_fd = self.expr(listener, scopes)?;
                let fd = self.fresh_reg("tcp_accept_fd");
                writeln!(self.out, "  {fd} = call i64 @nir_tcp_accept(i64 {listener_fd})").unwrap();
                self.guard_io_ok(&fd);
                Ok(fd)
            }
            // `send`/`recv` share one AST node with `chan`'s I/O.
            // `check_expr`'s structural pre-pass can't tell a `Ty::Channel`
            // operand from a `Ty::Tcp`/`Ty::File` one (no type info), so
            // this — the one place that can see `local_ty_of` — is where
            // the real, type-directed dispatch lives, same "type-oblivious
            // pre-pass, real check at IR-gen time" precedent `print`'s
            // aggregate rejection already established (module doc). A
            // `Ty::Channel` payload additionally has to be word-sized
            // (`is_word_sized`'s own doc comment) — real, disclosed, still-
            // open future work for `str`/`dec128`/struct/enum payloads.
            Expr::Send(target, value, _span) => match self.local_ty_of(target, scopes) {
                Ty::Tcp => {
                    let fd = self.expr(target, scopes)?;
                    let (ptr, len) = self.str_parts(value, scopes)?;
                    let n = self.fresh_reg("tcp_send_n");
                    writeln!(self.out, "  {n} = call i64 @nir_tcp_send(i64 {fd}, ptr {ptr}, i64 {len})").unwrap();
                    self.guard_io_ok(&n);
                    Ok("0".to_string()) // send's own value is unit; never read
                }
                Ty::File => {
                    let fd = self.expr(target, scopes)?;
                    let (ptr, len) = self.str_parts(value, scopes)?;
                    let n = self.fresh_reg("file_write_n");
                    writeln!(self.out, "  {n} = call i64 @nir_file_write(i64 {fd}, ptr {ptr}, i64 {len})").unwrap();
                    self.guard_io_ok(&n);
                    Ok("0".to_string()) // send's own value is unit; never read
                }
                Ty::Channel(inner) => {
                    if !self.is_word_sized(&inner)? {
                        return unsupported(format!(
                            "codegen doesn't support sending a `{inner:?}` over `chan` yet — only \
                             word-sized payloads (integers, bool, f64, box/ref, or another handle) \
                             are supported so far, not str/dec128/struct/enum/Vector/Matrix"
                        ));
                    }
                    let handle = self.expr(target, scopes)?;
                    let inner_llty = self.llvm_ty(&inner)?;
                    let val = self.expr(value, scopes)?;
                    let val64 = self.to_i64_word(&val, &inner_llty);
                    let rc = self.fresh_reg("chan_send_rc");
                    writeln!(self.out, "  {rc} = call i64 @nir_chan_send(i64 {handle}, i64 {val64})").unwrap();
                    // Only reachable if every receiver for `handle` was
                    // already dropped -- never happens today (nothing
                    // ever removes a channel's table entry, `nir_chan_new`'s
                    // own doc comment), kept as a real, checked trap
                    // rather than a silently-ignored return value.
                    self.guard_io_ok(&rc);
                    Ok("0".to_string()) // send's own value is unit; never read
                }
                _ => unreachable!("typeck.rs already restricted send's first operand to tcp/chan/file"),
            },
            Expr::Recv(target, _span) => match self.local_ty_of(target, scopes) {
                Ty::Tcp => {
                    let fd = self.expr(target, scopes)?;
                    // One read syscall into a fixed 64KiB buffer, matching
                    // `interpreter.rs`'s `read_tcp` exactly (module doc's
                    // "one chunk, not a message boundary" scope note) —
                    // entry-block-hoisted like every other alloca in this
                    // file, not heap-allocated (`box`'s allocator doesn't
                    // exist yet, and isn't needed here: the buffer's
                    // lifetime is exactly this expression's).
                    let buf = self.fresh_reg("tcp_recv_buf");
                    self.emit_alloca(&buf, "[65536 x i8]");
                    let buf_ptr = self.fresh_reg("tcp_recv_ptr");
                    writeln!(self.out, "  {buf_ptr} = getelementptr [65536 x i8], ptr {buf}, i64 0, i64 0").unwrap();
                    let n = self.fresh_reg("tcp_recv_n");
                    writeln!(self.out, "  {n} = call i64 @nir_tcp_recv(i64 {fd}, ptr {buf_ptr}, i64 65536)").unwrap();
                    // `0` (peer closed) is an error here too, exactly like
                    // `read_tcp`'s `n == 0` check — not a valid empty
                    // read, so it shares the same negative-or-zero trap as
                    // a real I/O failure (`guard_io_ok` only special-cases
                    // "negative", so recv needs its own explicit `<= 0`
                    // check instead of reusing it verbatim).
                    self.guard_recv_ok(&n);
                    let partial = self.fresh_reg("tcp_recv_str");
                    writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {buf_ptr}, 0").unwrap();
                    let full = self.fresh_reg("tcp_recv_str");
                    writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {n}, 1").unwrap();
                    Ok(full)
                }
                Ty::File => {
                    // One read syscall into a fixed 64KiB buffer, matching
                    // `interpreter.rs::read_file` exactly. Unlike `Ty::Tcp`
                    // above, `0` is valid EOF here, not an error --
                    // `guard_io_ok` (traps only on negative), not
                    // `guard_recv_ok` (traps on `<= 0` too), matches that.
                    let fd = self.expr(target, scopes)?;
                    let buf = self.fresh_reg("file_recv_buf");
                    self.emit_alloca(&buf, "[65536 x i8]");
                    let buf_ptr = self.fresh_reg("file_recv_ptr");
                    writeln!(self.out, "  {buf_ptr} = getelementptr [65536 x i8], ptr {buf}, i64 0, i64 0").unwrap();
                    let n = self.fresh_reg("file_recv_n");
                    writeln!(self.out, "  {n} = call i64 @nir_file_read(i64 {fd}, ptr {buf_ptr}, i64 65536)").unwrap();
                    self.guard_io_ok(&n);
                    let partial = self.fresh_reg("file_recv_str");
                    writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {buf_ptr}, 0").unwrap();
                    let full = self.fresh_reg("file_recv_str");
                    writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {n}, 1").unwrap();
                    Ok(full)
                }
                Ty::Channel(inner) => {
                    if !self.is_word_sized(&inner)? {
                        return unsupported(format!(
                            "codegen doesn't support receiving a `{inner:?}` from `chan` yet — only \
                             word-sized payloads (integers, bool, f64, box/ref, or another handle) \
                             are supported so far, not str/dec128/struct/enum/Vector/Matrix"
                        ));
                    }
                    let handle = self.expr(target, scopes)?;
                    let raw = self.fresh_reg("chan_recv_raw");
                    writeln!(self.out, "  {raw} = call i64 @nir_chan_recv(i64 {handle})").unwrap();
                    let inner_llty = self.llvm_ty(&inner)?;
                    Ok(self.from_i64_word(&raw, &inner_llty))
                }
                _ => unreachable!("typeck.rs already restricted recv's operand to tcp/chan/file"),
            },
            // `stop` shares one AST node across `sandbox`/`tcp`/
            // `tcp_listener` — `sandbox` is still unsupported (Phase E),
            // `tcp`/`tcp_listener` both close via the same `nir_tcp_stop`
            // (a plain fd close serves either uniformly).
            Expr::StopSandbox(inner, _span) => match self.local_ty_of(inner, scopes) {
                Ty::Tcp | Ty::TcpListener => {
                    let fd = self.expr(inner, scopes)?;
                    writeln!(self.out, "  call i32 @nir_tcp_stop(i64 {fd})").unwrap();
                    Ok("0".to_string()) // stop's own value is unit for tcp/tcp_listener; never read
                }
                Ty::Sandbox => unsupported("codegen doesn't support `sandbox` yet — interpreter-only for now"),
                Ty::File => {
                    let fd = self.expr(inner, scopes)?;
                    writeln!(self.out, "  call i32 @nir_file_stop(i64 {fd})").unwrap();
                    Ok("0".to_string())
                }
                Ty::Db => {
                    let handle = self.expr(inner, scopes)?;
                    writeln!(self.out, "  call i32 @nir_db_stop(i64 {handle})").unwrap();
                    Ok("0".to_string())
                }
                Ty::Mq => {
                    let handle = self.expr(inner, scopes)?;
                    writeln!(self.out, "  call i32 @nir_mq_stop(i64 {handle})").unwrap();
                    Ok("0".to_string())
                }
                _ => unreachable!("typeck.rs already restricted stop's operand to sandbox/tcp/tcp_listener/file/db/mq"),
            },
            // `v[i]` / `m[i, j]` — always yields a scalar element, so this
            // belongs in `expr()`, not `expr_ptr()`, even though the base
            // is an aggregate reached via `expr_ptr()`. Indices are
            // arbitrary runtime integer expressions (`typeck.rs`'s
            // `Expr::Index` arm only checks `is_integer()`, no literal
            // restriction) — real `getelementptr` with a runtime offset,
            // not an unroll-only shortcut, matching the plan's design
            // decision 4.
            Expr::Index(base, indices, span) => {
                let base_ty = self.local_ty_of(base, scopes);
                let base_ptr = self.expr_ptr(base, scopes)?;
                let (elem, offset) = match &base_ty {
                    Ty::Vector(elem, n) => {
                        let idx = self.expr(&indices[0], scopes)?;
                        self.guard_index_in_bounds(&idx, *n, *span)?;
                        ((**elem).clone(), idx)
                    }
                    Ty::Matrix(elem, r, c) => {
                        let i = self.expr(&indices[0], scopes)?;
                        self.guard_index_in_bounds(&i, *r, *span)?;
                        let j = self.expr(&indices[1], scopes)?;
                        self.guard_index_in_bounds(&j, *c, *span)?;
                        // Row-major flat offset `i*C + j` — matching
                        // `interpreter.rs`'s `Value::Matrix` layout
                        // exactly (module doc / design decision 1).
                        let scaled = self.fresh_reg("idx_row_scaled");
                        writeln!(self.out, "  {scaled} = mul i64 {i}, {c}").unwrap();
                        let flat = self.fresh_reg("idx_flat");
                        writeln!(self.out, "  {flat} = add i64 {scaled}, {j}").unwrap();
                        ((**elem).clone(), flat)
                    }
                    _ => unreachable!("typeck.rs already proved this is indexable (Vector/Matrix only)"),
                };
                let elem_llty = self.llvm_ty(&elem)?;
                let gep = self.fresh_reg("idx_gep");
                writeln!(self.out, "  {gep} = getelementptr {elem_llty}, ptr {base_ptr}, i64 {offset}").unwrap();
                let loaded = self.fresh_reg("idx_val");
                writeln!(self.out, "  {loaded} = load {elem_llty}, ptr {gep}").unwrap();
                Ok(self.widen_to_i64(&loaded, &elem))
            }
            // Genuinely reachable now that `check_supported` accepts
            // `ArrayLit` (this phase) — but only ever in a scalar
            // expression context, since `array_lit_ty` always types it
            // `Vector`/`Matrix`. A real, if narrow, unsupported case
            // (e.g. as an `if` expression's value slot) rather than a
            // dead catch-all, so it has to fail cleanly, not panic.
            Expr::ArrayLit(_, _) => unsupported(
                "codegen doesn't support a Vector/Matrix literal in this expression position \
                 yet — bind it via `let`, or pass/return it through a function call",
            ),
        }
    }


    /// The aggregate-value twin of `expr()` — returns a pointer to a
    /// stack-allocated `Vector`/`Matrix` value instead of a bare SSA
    /// register, since an aggregate can't live in one (module doc /
    /// design decision 1 of the Vector/Matrix codegen plan). Only ever
    /// called where the caller already knows (typically via
    /// `local_ty_of(e, scopes).is_aggregate()`) that `e`'s type is
    /// `Vector`/`Matrix`. Deliberately narrow this phase — only the
    /// forms actually needed to bind, reassign, and pass/return an
    /// aggregate value exist yet; anything else (e.g. an aggregate-typed
    /// `if` expression) fails as a clean `CodegenError`, a real scope
    /// boundary for a later phase, not a gap found by accident.
    pub(super) fn expr_ptr(&mut self, e: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        match e {
            Expr::Ident(name, _) => {
                // No copy here — this is *the* variable's own storage.
                // A caller that's about to bind a new name or overwrite
                // an existing one (`Stmt::Let`, this fn's own
                // `Expr::Assign` arm) is responsible for copying out of
                // this pointer itself; a caller that's just forwarding
                // the value onward (a call argument, matching the
                // callee's own copy-in prologue) is not.
                let (_, ptr) = scopes.get(name).expect("typeck.rs already proved this resolves");
                Ok(ptr)
            }
            Expr::ArrayLit(elements, _) => self.array_lit(elements, scopes),
            // Row 11: a constructor call reached *without* an expected
            // type in hand (a bare constructor expression statement, or
            // any other `expr_ptr`-reached position `expr_ptr_expected`
            // didn't intercept). Resolve the expected type by
            // structural inference from the arguments' own types
            // (`ctor_ty`, the same fall-back `typeck.rs::resolve_type_args`
            // uses when no context is available); the one genuinely-
            // ambiguous case — a zero-payload variant like `None` with no
            // enclosing type context — is the disclosed `CodegenError`
            // the struct/enum codegen plan names, not a guess.
            Expr::Call(name, args, span) => {
                if self.registry.is_struct(name) || self.registry.find_variant(name).is_some() {
                    let expected = self.ctor_ty(name, args, scopes).ok_or_else(|| CodegenError {
                        message: format!(
                            "codegen can't infer the concrete type of `{name}(...)` here without an enclosing type context — give it one (a `let` annotation, a `return` in a typed function, or a typed call argument); this is the one genuinely-ambiguous construction case (a zero-payload variant reached with no expected type)"
                        ),
                    })?;
                    self.construct(name, args, &expected, *span, scopes)
                } else {
                    self.call_ptr(name, args, scopes)
                }
            }
            // Row 11: `base.field` where the field is itself an aggregate
            // (a nested struct, a `Vector`/`Matrix` field) — GEP to the
            // field's index within `base`'s real named struct type and
            // return that pointer directly, no load, no copy (the same
            // "expose the storage, don't clone it" shape `expr_ptr`'s
            // `Expr::Ident` and `Expr::Deref` arms already use). A
            // scalar field reached here is returned as a pointer to the
            // scalar slot — valid for any `expr_ptr` consumer that just
            // forwards the pointer onward, and never the path a scalar
            // `let`/argument takes (those route through `expr()`'s own
            // `Expr::FieldAccess` arm via `local_ty_of`'s scalar result).
            Expr::FieldAccess(base, field, _) => {
                let base_ty = self.local_ty_of(base, scopes);
                let (idx, _field_ty) = self
                    .field_index_and_ty(&base_ty, field)
                    .expect("typeck.rs already proved this base is a struct with this field");
                let base_ptr = self.expr_ptr(base, scopes)?;
                let base_llty = self.llvm_ty(&base_ty)?;
                let gep = self.fresh_reg("fieldptr");
                writeln!(self.out, "  {gep} = getelementptr inbounds {base_llty}, ptr {base_ptr}, i32 0, i32 {idx}").unwrap();
                Ok(gep)
            }
            Expr::Assign(name, rhs, _) => {
                let src = self.expr_ptr(rhs, scopes)?;
                let (ty, dst) = scopes.get(name).expect("typeck.rs already proved this resolves");
                let bytes = agg_byte_size_operand(&ty, &self.registry);
                writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {dst}, ptr {src}, i64 {bytes}, i1 false)").unwrap();
                Ok(dst)
            }
            // Every Vector/Matrix-*producing* binary operator (elementwise
            // `+`/`-`/`.*`/`./`, and `*` in its three legal shapes) --
            // `==`/`!=` on aggregate operands still yield a scalar `bool`
            // and stay on the `expr()`/`binary()` path (`agg_eq`), not
            // here.
            Expr::Binary(op, l, r, span) => self.agg_binary(*op, l, r, *span, scopes),
            // `*e` where the unwrapped payload is itself an aggregate
            // (`box Vector(f64,3)`, `&Matrix(f64,2,2)`, ...) — `e`'s own
            // type (`Box`/`Ref`) is never `is_aggregate()`, so `e` itself
            // is fetched via the ordinary `expr()` path (a bare pointer
            // value), then handed straight back as this dereference's
            // own pointer-returning result. No allocation, no copy —
            // dereferencing exposes the same storage the box/ref already
            // points at, it doesn't clone it (matches the interpreter's
            // `Value::Boxed(inner) | Value::Ref(inner) => *inner`, and
            // `expr()`'s own `Expr::Deref` arm for the non-aggregate
            // case — same value, different result shape only).
            Expr::Deref(inner, _) => self.expr(inner, scopes),
            // Aggregate-result `if`/`match` — `expr_ptr` is reached only
            // when the caller already knows this expression has an aggregate
            // type, so `if_expr`/`match_expr` will allocate a slot and
            // return its pointer.
            Expr::If { cond, then_block, else_block, span } => {
                self.if_expr(cond, then_block, else_block.as_deref(), *span, None, scopes)
            }
            Expr::Match { scrutinee, arms, span } => self.match_expr(scrutinee, arms, *span, None, scopes),
            // `acquire name(proof)` — always aggregate-valued
            // (`Result(Ty::Fn(..), str)`), so it belongs here, not in
            // `expr()`.
            Expr::Acquire(name, proof, _) => self.emit_acquire(name, proof, scopes),
            _ => unsupported(
                "codegen doesn't support this aggregate expression form yet — only \
                 identifiers, literals, assignment, binary operators, dereferencing a boxed/\
                 borrowed aggregate, function calls/returns, `if`, and `match` are supported so far \
                 (indexing reads a scalar element via `expr()`, and builtins land in a later \
                 phase)",
            ),
        }
    }


    /// `expr_ptr`'s `Expr::ArrayLit` case: allocate a fresh destination
    /// sized to the literal's own inferred shape, then fill it — element
    /// by element for a `Vector` (each a plain scalar `expr()`), row by
    /// row for a `Matrix` (each row's own pointer via `expr_ptr`,
    /// `memcpy`'d into the right flat offset — row-major, matching
    /// `interpreter.rs`'s `Value::Matrix` exactly).
    pub(super) fn array_lit(&mut self, elements: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let ty = self.array_lit_ty(elements, scopes);
        let agg_llty = self.llvm_ty(&ty)?;
        let dest = self.fresh_reg("arraylit.addr");
        self.emit_alloca(&dest, &agg_llty);

        match &ty {
            Ty::Vector(elem, _) => {
                let elem_llty = self.llvm_ty(elem)?;
                for (i, e) in elements.iter().enumerate() {
                    let v = self.expr(e, scopes)?;
                    let v = if elem.is_integer() { self.narrow_from_i64(&v, elem)? } else { v };
                    let gep = self.fresh_reg("arraylit.elem");
                    writeln!(self.out, "  {gep} = getelementptr {agg_llty}, ptr {dest}, i64 0, i64 {i}").unwrap();
                    writeln!(self.out, "  store {elem_llty} {v}, ptr {gep}").unwrap();
                }
            }
            Ty::Matrix(elem, _rows, cols) => {
                let row_bytes = *cols as u64 * elem_byte_size(elem);
                for (i, row_expr) in elements.iter().enumerate() {
                    let row_ptr = self.expr_ptr(row_expr, scopes)?;
                    let row_dest = self.fresh_reg("arraylit.row");
                    writeln!(self.out, "  {row_dest} = getelementptr {agg_llty}, ptr {dest}, i64 0, i64 {}", i * cols).unwrap();
                    writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {row_dest}, ptr {row_ptr}, i64 {row_bytes}, i1 false)").unwrap();
                }
            }
            _ => unreachable!("array_lit_ty always returns Vector or Matrix"),
        }
        Ok(dest)
    }


    /// Calls a `transact` slot's callee — always a plain top-level `fn`,
    /// never a builtin (`typeck::infer_transact_slot`'s own doc comment)
    /// — and hands back its value plus its declared return type,
    /// regardless of whether that type is aggregate or scalar. Every
    /// other call site in this file already picks `call()` vs.
    /// `call_ptr()` based on the *caller's* own known context (an
    /// expected type, an aggregate check done ahead of time); a
    /// `transact` slot has neither — `precheck`/`verify`'s return type is
    /// fixed at `bool` but `commit`/`compensate`/`log`'s is genuinely
    /// unconstrained (`docs/TRANSACT.md`'s own decision) — so this is the
    /// one call site in `emit_transact` that has to branch on
    /// `sig_ret.is_aggregate()` itself, mirroring `call()`'s and
    /// `call_ptr()`'s own near-identical tails rather than forcing every
    /// slot through one or the other ahead of time.
    pub(super) fn emit_transact_call(&mut self, slot: &TransactSlot, scopes: &mut Scopes) -> Result<(String, Ty), CodegenError> {
        let sig_params = self.sigs.get(&slot.name).expect("typeck.rs already resolved this call").params.clone();
        let sig_ret = self.sigs.get(&slot.name).expect("typeck.rs already resolved this call").ret.clone();
        let arg_vals = self.call_args(&slot.args, &sig_params, scopes)?;
        if sig_ret.is_aggregate() {
            let agg_llty = self.llvm_ty(&sig_ret)?;
            let dest = self.fresh_reg("transact_call_result_addr");
            self.emit_alloca(&dest, &agg_llty);
            let mut all_args = vec![format!("ptr {dest}")];
            all_args.extend(arg_vals);
            writeln!(self.out, "  call void @{}({})", slot.name, all_args.join(", ")).unwrap();
            Ok((dest, sig_ret))
        } else {
            let ret_llty = self.llvm_ty(&sig_ret)?;
            if ret_llty == "void" {
                writeln!(self.out, "  call void @{}({})", slot.name, arg_vals.join(", ")).unwrap();
                Ok(("0".to_string(), sig_ret))
            } else {
                let r = self.fresh_reg("transact_call_result");
                writeln!(self.out, "  {r} = call {ret_llty} @{}({})", slot.name, arg_vals.join(", ")).unwrap();
                Ok((self.widen_to_i64(&r, &sig_ret), sig_ret))
            }
        }
    }


    /// Calls `callee_name` once with `arg_operands` (already-formatted
    /// `"<llty> <val>"` operands — from `call_args`, or from
    /// `emit_replay_decode_operands` for a replay trampoline's decoded
    /// values) and returns a real `i1`: `true` unconditionally for a
    /// non-`Result` return type (there's nothing to retry against — the
    /// call either ran or trapped, same as any ordinary call in this
    /// file), or, when `sig_ret` is `Result(_, _)`, `true` only once an
    /// attempt's tag word reads `0` (`Ok`) — retrying with doubling
    /// backoff (`nir_sleep_ms`, `TRANSACT_RETRY_BASE_BACKOFF_MS` ×
    /// 2^attempt) up to `TRANSACT_RETRY_MAX_ATTEMPTS` times, `false` if
    /// every attempt came back `Err`. The caller decides what
    /// "exhausted" means — for the live `transact` path, that's leaving
    /// the row `commit_pending`/`compensate_pending` for a later replay
    /// to finish, never a hard abort (`emit_transact`'s own doc comment
    /// on why compiled traps can't be the retry-on-failure mechanism
    /// here).
    ///
    /// Emits its own internal `header`/`body`/`ok`/`err`/`done` blocks
    /// (a private control-flow region, same shape `guard_in_range`'s
    /// trap check already uses) — callers must not assume the current
    /// block is still whatever it was before this call returns; merge
    /// results via a stored-to `alloca` slot afterward, never a `phi`
    /// keyed on a label this function didn't hand back (`emit_transact`/
    /// `emit_transact_replay_trampoline` both follow this already).
    pub(super) fn emit_call_with_retry(&mut self, callee_name: &str, sig_ret: &Ty, arg_operands: &[String], label_prefix: &str) -> Result<String, CodegenError> {
        let is_result = matches!(sig_ret, Ty::Named(n, targs) if n == "Result" && targs.len() == 2);
        if !is_result {
            let ret_llty = self.llvm_ty(sig_ret)?;
            if sig_ret.is_aggregate() {
                let dest = self.fresh_reg(&format!("{label_prefix}_call_dest"));
                self.emit_alloca(&dest, &ret_llty);
                let mut all = vec![format!("ptr {dest}")];
                all.extend(arg_operands.iter().cloned());
                writeln!(self.out, "  call void @{callee_name}({})", all.join(", ")).unwrap();
            } else if ret_llty == "void" {
                writeln!(self.out, "  call void @{callee_name}({})", arg_operands.join(", ")).unwrap();
            } else {
                writeln!(self.out, "  call {ret_llty} @{callee_name}({})", arg_operands.join(", ")).unwrap();
            }
            return Ok("1".to_string());
        }

        let result_llty = self.llvm_ty(sig_ret)?;
        let success_slot = self.fresh_reg(&format!("{label_prefix}_retry_success_addr"));
        self.emit_alloca(&success_slot, "i1");
        writeln!(self.out, "  store i1 false, ptr {success_slot}").unwrap();
        let counter_slot = self.fresh_reg(&format!("{label_prefix}_retry_counter_addr"));
        self.emit_alloca(&counter_slot, "i64");
        writeln!(self.out, "  store i64 0, ptr {counter_slot}").unwrap();

        let header = self.fresh_label(&format!("{label_prefix}_retry_header"));
        let body = self.fresh_label(&format!("{label_prefix}_retry_body"));
        let ok_label = self.fresh_label(&format!("{label_prefix}_retry_ok"));
        let err_label = self.fresh_label(&format!("{label_prefix}_retry_err"));
        let done = self.fresh_label(&format!("{label_prefix}_retry_done"));

        writeln!(self.out, "  br label %{header}").unwrap();
        writeln!(self.out, "{header}:").unwrap();
        let counter = self.fresh_reg(&format!("{label_prefix}_retry_counter"));
        writeln!(self.out, "  {counter} = load i64, ptr {counter_slot}").unwrap();
        let under_max = self.fresh_reg(&format!("{label_prefix}_retry_under_max"));
        writeln!(self.out, "  {under_max} = icmp slt i64 {counter}, {TRANSACT_RETRY_MAX_ATTEMPTS}").unwrap();
        writeln!(self.out, "  br i1 {under_max}, label %{body}, label %{done}").unwrap();

        writeln!(self.out, "{body}:").unwrap();
        let call_dest = self.fresh_reg(&format!("{label_prefix}_call_dest"));
        self.emit_alloca(&call_dest, &result_llty);
        let mut all = vec![format!("ptr {call_dest}")];
        all.extend(arg_operands.iter().cloned());
        writeln!(self.out, "  call void @{callee_name}({})", all.join(", ")).unwrap();
        let tag_ptr = self.fresh_reg(&format!("{label_prefix}_tag_ptr"));
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {call_dest}, i32 0, i32 0").unwrap();
        let tag = self.fresh_reg(&format!("{label_prefix}_tag"));
        writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
        let is_ok = self.fresh_reg(&format!("{label_prefix}_is_ok"));
        writeln!(self.out, "  {is_ok} = icmp eq i64 {tag}, 0").unwrap();
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i1 true, ptr {success_slot}").unwrap();
        writeln!(self.out, "  br label %{done}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        let next_counter = self.fresh_reg(&format!("{label_prefix}_retry_next"));
        writeln!(self.out, "  {next_counter} = add i64 {counter}, 1").unwrap();
        writeln!(self.out, "  store i64 {next_counter}, ptr {counter_slot}").unwrap();
        let backoff = self.fresh_reg(&format!("{label_prefix}_retry_backoff"));
        writeln!(self.out, "  {backoff} = shl i64 {TRANSACT_RETRY_BASE_BACKOFF_MS}, {counter}").unwrap();
        writeln!(self.out, "  call void @nir_sleep_ms(i64 {backoff})").unwrap();
        writeln!(self.out, "  br label %{header}").unwrap();

        writeln!(self.out, "{done}:").unwrap();
        let result = self.fresh_reg(&format!("{label_prefix}_retry_result"));
        writeln!(self.out, "  {result} = load i1, ptr {success_slot}").unwrap();
        Ok(result)
    }


    /// The mirror image of `emit_db_binds`, for a replay trampoline: GEPs
    /// `params.len()` `NirBindValue`s out of `arr_ptr` (already decoded
    /// by `nir_transact_decode_args`) and builds the `"<llty> <val>"`
    /// call-argument operands `emit_call_with_retry` needs, per real
    /// LLVM parameter type (`is_transact_scalar`'s four shapes — the
    /// only ones a `transact` `commit`/`compensate` callee can declare,
    /// so this never sees anything else).
    pub(super) fn emit_replay_decode_operands(&mut self, arr_ptr: &str, params: &[Ty], label_prefix: &str) -> Result<Vec<String>, CodegenError> {
        let arr_llty = format!("[{} x {NIR_BIND_VALUE_LLTY}]", params.len().max(1));
        let mut operands = Vec::with_capacity(params.len());
        for (i, ty) in params.iter().enumerate() {
            let elem_ptr = self.fresh_reg(&format!("{label_prefix}_elem_ptr"));
            writeln!(self.out, "  {elem_ptr} = getelementptr inbounds {arr_llty}, ptr {arr_ptr}, i32 0, i32 {i}").unwrap();
            match ty {
                Ty::Bool => {
                    let field_ptr = self.fresh_reg(&format!("{label_prefix}_i_ptr"));
                    writeln!(self.out, "  {field_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 1").unwrap();
                    let val = self.fresh_reg(&format!("{label_prefix}_i_val"));
                    writeln!(self.out, "  {val} = load i64, ptr {field_ptr}").unwrap();
                    let b = self.fresh_reg(&format!("{label_prefix}_bool_val"));
                    writeln!(self.out, "  {b} = icmp ne i64 {val}, 0").unwrap();
                    operands.push(format!("i1 {b}"));
                }
                Ty::Str => {
                    let sptr_field = self.fresh_reg(&format!("{label_prefix}_sptr_ptr"));
                    writeln!(self.out, "  {sptr_field} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 3").unwrap();
                    let sptr = self.fresh_reg(&format!("{label_prefix}_sptr"));
                    writeln!(self.out, "  {sptr} = load ptr, ptr {sptr_field}").unwrap();
                    let slen_field = self.fresh_reg(&format!("{label_prefix}_slen_ptr"));
                    writeln!(self.out, "  {slen_field} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 4").unwrap();
                    let slen = self.fresh_reg(&format!("{label_prefix}_slen"));
                    writeln!(self.out, "  {slen} = load i64, ptr {slen_field}").unwrap();
                    let partial = self.fresh_reg(&format!("{label_prefix}_str_partial"));
                    writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {sptr}, 0").unwrap();
                    let full = self.fresh_reg(&format!("{label_prefix}_str_full"));
                    writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {slen}, 1").unwrap();
                    operands.push(format!("{{ptr, i64}} {full}"));
                }
                Ty::F64 => {
                    let field_ptr = self.fresh_reg(&format!("{label_prefix}_f_ptr"));
                    writeln!(self.out, "  {field_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 2").unwrap();
                    let val = self.fresh_reg(&format!("{label_prefix}_f_val"));
                    writeln!(self.out, "  {val} = load double, ptr {field_ptr}").unwrap();
                    operands.push(format!("double {val}"));
                }
                other if other.is_integer() => {
                    let field_ptr = self.fresh_reg(&format!("{label_prefix}_i_ptr"));
                    writeln!(self.out, "  {field_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 1").unwrap();
                    let val = self.fresh_reg(&format!("{label_prefix}_i_val"));
                    writeln!(self.out, "  {val} = load i64, ptr {field_ptr}").unwrap();
                    operands.push(format!("i64 {val}"));
                }
                other => {
                    return unsupported(format!(
                        "transact replay trampoline: unsupported commit/compensate parameter type {other:?} \
                         (typeck.rs::infer_transact already restricts these to is_transact_scalar's four shapes)"
                    ));
                }
            }
        }
        Ok(operands)
    }


    /// Generates one top-level replay trampoline for a single `transact`
    /// site (same `self.trampolines`/temporarily-swapped-`self.out`
    /// mechanism `emit_spawn_trampoline` already established), matching
    /// `kernel::transact::ReplayFn`'s real ABI exactly: `extern "C"
    /// fn(*const u8, i64, i32) -> i32`. Decodes the row's JSON-encoded
    /// args (`nir_transact_decode_args`) into one shared, worst-case-
    /// sized `NirBindValue` buffer (`max(commit's arity, compensate's
    /// arity)` — the two paths never run in the same call, so sharing
    /// one buffer is safe and simpler than allocating two), then
    /// dispatches to whichever of `commit.name`/`compensate.name` the
    /// caller's `is_commit` flag selects, through the exact same
    /// `emit_call_with_retry` bounded-retry logic the live path uses —
    /// replaying a `commit` that itself needs a few attempts to succeed
    /// gets the same treatment as it would have gotten live.
    /// `kernel::transact::nir_transact_replay_all` (not this function)
    /// is what marks the row `committed`/`compensated` on a real `1`
    /// return, or leaves it pending (logged, not silently dropped) on
    /// `0` — this trampoline only ever reports which one happened.
    pub(super) fn emit_transact_replay_trampoline(&mut self, site_id: i64, commit: &TransactSlot, compensate: &Option<TransactSlot>) -> Result<String, CodegenError> {
        let tramp_name = self.fresh_global(&format!("transact_replay_{site_id}"));
        // `.params.clone()`/`.ret.clone()`, not `.clone()` on the whole
        // `&FnSig` — cloning the reference itself (always available,
        // regardless of whether `FnSig` implements `Clone`, since every
        // reference is trivially `Clone`/`Copy`) would silently keep the
        // borrow of `self.sigs` alive for this whole function, colliding
        // with every `&mut self` call below it. Same pattern
        // `emit_transact_call` already uses.
        let commit_params = self.sigs.get(&commit.name).expect("typeck.rs already resolved this call").params.clone();
        let commit_ret = self.sigs.get(&commit.name).expect("typeck.rs already resolved this call").ret.clone();
        let compensate_params_ret = compensate.as_ref().map(|c| {
            let s = self.sigs.get(&c.name).expect("typeck.rs already resolved this call");
            (s.params.clone(), s.ret.clone())
        });
        let max_arity = commit_params.len().max(compensate_params_ret.as_ref().map(|(p, _)| p.len()).unwrap_or(0)).max(1);

        let saved_out = std::mem::take(&mut self.out);
        // `emit_alloca`/`emit_call_with_retry` (called below, and shared
        // with the *live* `emit_transact` path) both write through
        // `self.entry_allocas`, a per-function scratch buffer normally
        // owned and spliced back in by `Codegen::function`'s own
        // "capture position, clear, ..., splice back" dance
        // (`entry_allocas`'s own doc comment). This trampoline is a
        // second, hand-built `define` outside that machinery (the same
        // `self.out`-swap trick `emit_spawn_trampoline` already uses) —
        // without saving/restoring `self.entry_allocas` too, this
        // function's own allocas would either land in the *enclosing*
        // live function's entry block (if some are already queued there
        // mid-codegen) or silently vanish into a buffer nothing ever
        // splices for this `define` at all — a real bug, caught by
        // actually compiling a `transact` program and reading clang's
        // own "use of undefined value" error, not reasoned about.
        let saved_entry_allocas = std::mem::take(&mut self.entry_allocas);
        writeln!(self.out, "define i32 {tramp_name}(ptr %args_json_ptr, i64 %args_json_len, i32 %is_commit) {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        let alloca_splice_pos = self.out.len();

        let arr_llty = format!("[{max_arity} x {NIR_BIND_VALUE_LLTY}]");
        let arr_ptr = self.fresh_reg("replay_arr");
        self.emit_alloca(&arr_ptr, &arr_llty);
        let decoded_n = self.fresh_reg("replay_decoded_n");
        writeln!(self.out, "  {decoded_n} = call i64 @nir_transact_decode_args(ptr %args_json_ptr, i64 %args_json_len, ptr {arr_ptr}, i64 {max_arity})").unwrap();
        let _ = decoded_n; // real decode-failure handling would need a third result branch -- not reachable for a row this same binary logged, only for a hand-corrupted log file.

        let result_slot = self.fresh_reg("replay_result_addr");
        self.emit_alloca(&result_slot, "i32");

        let is_commit_b = self.fresh_reg("replay_is_commit_b");
        writeln!(self.out, "  {is_commit_b} = icmp ne i32 %is_commit, 0").unwrap();
        let do_commit = self.fresh_label("replay_do_commit");
        let do_compensate = self.fresh_label("replay_do_compensate");
        let done = self.fresh_label("replay_done");
        writeln!(self.out, "  br i1 {is_commit_b}, label %{do_commit}, label %{do_compensate}").unwrap();

        writeln!(self.out, "{do_commit}:").unwrap();
        let commit_operands = self.emit_replay_decode_operands(&arr_ptr, &commit_params, "replay_commit")?;
        let commit_success = self.emit_call_with_retry(&commit.name, &commit_ret, &commit_operands, "replay_commit")?;
        let commit_result = self.fresh_reg("replay_commit_result_i32");
        writeln!(self.out, "  {commit_result} = zext i1 {commit_success} to i32").unwrap();
        writeln!(self.out, "  store i32 {commit_result}, ptr {result_slot}").unwrap();
        writeln!(self.out, "  br label %{done}").unwrap();

        writeln!(self.out, "{do_compensate}:").unwrap();
        if let Some(c) = compensate {
            let (comp_params, comp_ret) = compensate_params_ret.expect("compensate is Some, so compensate_params_ret was computed above");
            let comp_operands = self.emit_replay_decode_operands(&arr_ptr, &comp_params, "replay_compensate")?;
            let comp_success = self.emit_call_with_retry(&c.name, &comp_ret, &comp_operands, "replay_compensate")?;
            let comp_result = self.fresh_reg("replay_compensate_result_i32");
            writeln!(self.out, "  {comp_result} = zext i1 {comp_success} to i32").unwrap();
            writeln!(self.out, "  store i32 {comp_result}, ptr {result_slot}").unwrap();
        } else {
            // A row can only be `compensate_pending` if the live path
            // itself marked it so, which only happens inside the `if let
            // Some(c) = compensate` branch of `emit_transact` -- so this
            // arm is unreachable for any row this binary's own live path
            // produced. Still real, compiled code (not `unreachable`):
            // a hand-edited/corrupted log row is the only way here, and
            // reporting failure (leaving it `compensate_pending`) is the
            // same safe response replay already gives any row it can't
            // finish, not a crash.
            writeln!(self.out, "  store i32 0, ptr {result_slot}").unwrap();
        }
        writeln!(self.out, "  br label %{done}").unwrap();

        writeln!(self.out, "{done}:").unwrap();
        let result = self.fresh_reg("replay_result");
        writeln!(self.out, "  {result} = load i32, ptr {result_slot}").unwrap();
        writeln!(self.out, "  ret i32 {result}").unwrap();
        writeln!(self.out, "}}").unwrap();
        writeln!(self.out).unwrap();

        self.out.insert_str(alloca_splice_pos, &self.entry_allocas);
        self.trampolines.push_str(&self.out);
        self.out = saved_out;
        self.entry_allocas = saved_entry_allocas;
        Ok(tramp_name)
    }


    /// `transact { precheck?/network/verify/commit/compensate?/log? }` —
    /// `docs/TRANSACT.md`, compiled for real 2026-09, Layer 1 only (the
    /// same scope the now-deleted interpreter itself shipped *first*,
    /// before retry/timeout/durability/replay — "layers, not a syntax
    /// spec," the doc's own section title). Real, compiled control flow:
    /// `txn_id` generated (`nir_transact_gen_txn_id`), `precheck` (if
    /// present) aborting the whole block to `false` immediately on a
    /// `false` return, `network`/`verify` becoming real implicit local
    /// bindings later slots' arguments resolve through the ordinary
    /// `Expr::Ident`/`scopes` mechanism (no special-casing needed there —
    /// `call_args`'s existing argument evaluation already handles it),
    /// `verify`'s result choosing `commit` or `compensate`, `log` (if
    /// present) always running last. The whole expression is a real
    /// `i1` value, `true`/`false` exactly as documented.
    ///
    /// **Deliberately not attempted here, and not simply deferred by
    /// oversight — architecturally blocked by the compiled trap model,
    /// found while designing this, not assumed in advance:** `network`'s
    /// `retry`/`timeout` modifiers (rejected explicitly in
    /// `check_expr`'s pre-pass, with a specific reason, before this
    /// function is ever reached). The interpreter's own retry-on-trap
    /// semantics relied on catching an internal `RuntimeError` before it
    /// ever unwound; a compiled trap calls `abort()` directly (every
    /// `guard_*` in this file), an unrecoverable process exit — and
    /// `network`'s own declared return type is restricted to a bare
    /// scalar by `Ty::is_transact_scalar` (never `Result(_, _)`), so
    /// there is no non-trapping failure signal for `network` to react to
    /// either, unlike `commit`/`compensate` (whose return types are
    /// genuinely unconstrained). This is a real, narrower gap, not a
    /// missing feature: no bounded amount of engineering time closes it
    /// without either giving compiled traps a catchable/unwinding
    /// semantics (a much larger, unrelated language change) or changing
    /// `is_transact_scalar`'s already-locked durability-boundary rule.
    ///
    /// **Also deferred, named, not attempted**: `commit`/`compensate`'s
    /// own retry-with-backoff-then-trap-on-exhaustion (theoretically
    /// possible when their return type happens to be `Result(_, _)`,
    /// since that one *is* unconstrained — but genuinely new, untested
    /// control flow this pass didn't have budget to build and verify
    /// carefully in the same round as the rest of Layer 1); the
    /// durability log (`transact_log.rs`, deleted along with the
    /// interpreter) and crash replay — a real, separate, much larger
    /// follow-up, not a small addition to this function.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_transact(
        &mut self,
        precheck: &Option<TransactSlot>,
        network: &TransactSlot,
        verify: &TransactSlot,
        commit: &TransactSlot,
        compensate: &Option<TransactSlot>,
        log: &Option<TransactSlot>,
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        scopes.push();

        let result_slot = self.fresh_reg("transact_result_addr");
        self.emit_alloca(&result_slot, "i1");

        // `txn_id` — generated before `precheck` even runs, in scope for
        // every slot (`typeck::infer_transact`'s own ordering), even
        // though only `network` is required to actually reference it.
        let txn_id_slot = self.fresh_reg("transact_txn_id_addr");
        self.emit_alloca(&txn_id_slot, "{ptr, i64}");
        writeln!(self.out, "  call void @nir_transact_gen_txn_id(ptr {txn_id_slot})").unwrap();
        scopes.define("txn_id", Ty::Str, txn_id_slot.clone());

        let after_precheck_label = self.fresh_label("transact_after_precheck");
        if let Some(p) = precheck {
            let (precheck_val, _) = self.emit_transact_call(p, scopes)?;
            let precheck_true = self.fresh_label("transact_precheck_true");
            let precheck_false = self.fresh_label("transact_precheck_false");
            writeln!(self.out, "  br i1 {precheck_val}, label %{precheck_true}, label %{precheck_false}").unwrap();
            writeln!(self.out, "{precheck_false}:").unwrap();
            writeln!(self.out, "  store i1 false, ptr {result_slot}").unwrap();
            writeln!(self.out, "  br label %{after_precheck_label}").unwrap();
            writeln!(self.out, "{precheck_true}:").unwrap();
        }

        // `network` — always a bare scalar return (`Ty::is_transact_scalar`,
        // never aggregate), so this is always the plain `store <llty>`
        // path, never a `memcpy`.
        let (network_val, network_ty) = self.emit_transact_call(network, scopes)?;
        let network_llty = self.llvm_ty(&network_ty)?;
        let network_slot = self.fresh_reg("transact_network_addr");
        self.emit_alloca(&network_slot, &network_llty);
        writeln!(self.out, "  store {network_llty} {network_val}, ptr {network_slot}").unwrap();
        scopes.define("network", network_ty, network_slot);

        // `verify` — its own return type is already forced to `bool` by
        // `typeck.rs`, so `verify_val` is directly usable as a branch
        // condition with no extra `icmp`/widening needed.
        let (verify_val, verify_ty) = self.emit_transact_call(verify, scopes)?;
        let verify_slot = self.fresh_reg("transact_verify_addr");
        self.emit_alloca(&verify_slot, "i1");
        writeln!(self.out, "  store i1 {verify_val}, ptr {verify_slot}").unwrap();
        scopes.define("verify", verify_ty, verify_slot);

        // A compile-time-unique id for this call site — used both as the
        // durability log's own `site_id` column and as the key
        // `nir_transact_register_replay_site` registers this site's
        // generated trampoline under. Assigned by position in
        // `self.transact_sites` (one push per `emit_transact` call,
        // program-wide codegen order — stable within one build, not
        // across a rebuild that adds/removes/reorders `transact` sites;
        // see `docs/adr/0009-transact-durability-and-replay.md`'s
        // disclosed fingerprint-guard gap).
        let site_id = self.transact_sites.len() as i64;

        // `txn_id`'s raw `ptr`/`i64` parts — every `nir_transact_*` durability
        // call below takes these, not the `{ptr, i64}` struct value itself.
        let txn_id_ptr_gep = self.fresh_reg("transact_txn_id_ptr_gep");
        writeln!(self.out, "  {txn_id_ptr_gep} = getelementptr inbounds {{ptr, i64}}, ptr {txn_id_slot}, i32 0, i32 0").unwrap();
        let txn_id_ptr_val = self.fresh_reg("transact_txn_id_ptr");
        writeln!(self.out, "  {txn_id_ptr_val} = load ptr, ptr {txn_id_ptr_gep}").unwrap();
        let txn_id_len_gep = self.fresh_reg("transact_txn_id_len_gep");
        writeln!(self.out, "  {txn_id_len_gep} = getelementptr inbounds {{ptr, i64}}, ptr {txn_id_slot}, i32 0, i32 1").unwrap();
        let txn_id_len_val = self.fresh_reg("transact_txn_id_len");
        writeln!(self.out, "  {txn_id_len_val} = load i64, ptr {txn_id_len_gep}").unwrap();

        // Durable row created right here, not at the top of the function
        // — a `precheck`-rejected transact never reaches this point, so
        // it never gets a row at all (nothing for replay to ever act on
        // anyway). Best-effort, like `log` below: `nir_transact_log_init`
        // already aborted the whole program at startup (`emit_c_main`) if
        // the log couldn't be opened, so a failure return here would mean
        // a `txn_id` collision or a mid-run I/O error — rare, and not
        // something a live `abort()` should escalate to, matching this
        // function's existing "durability is a best-effort side channel
        // to the live control flow, not a gate on it" posture throughout.
        writeln!(self.out, "  call i32 @nir_transact_begin(ptr {txn_id_ptr_val}, i64 {txn_id_len_val}, i64 {site_id})").unwrap();

        let commit_label = self.fresh_label("transact_commit");
        let compensate_label = self.fresh_label("transact_compensate");
        let after_verify_label = self.fresh_label("transact_after_verify");
        writeln!(self.out, "  br i1 {verify_val}, label %{commit_label}, label %{compensate_label}").unwrap();

        writeln!(self.out, "{commit_label}:").unwrap();
        {
            let (binds_ptr, binds_len) = self.emit_db_binds(&commit.args, scopes)?;
            writeln!(self.out, "  call i32 @nir_transact_mark_commit_pending(ptr {txn_id_ptr_val}, i64 {txn_id_len_val}, ptr {binds_ptr}, i64 {binds_len})").unwrap();
            let sig_params = self.sigs.get(&commit.name).expect("typeck.rs already resolved this call").params.clone();
            let sig_ret = self.sigs.get(&commit.name).expect("typeck.rs already resolved this call").ret.clone();
            let arg_operands = self.call_args(&commit.args, &sig_params, scopes)?;
            let success = self.emit_call_with_retry(&commit.name, &sig_ret, &arg_operands, "transact_commit")?;
            let mark_committed_label = self.fresh_label("transact_mark_committed");
            let after_commit_label = self.fresh_label("transact_after_commit");
            writeln!(self.out, "  br i1 {success}, label %{mark_committed_label}, label %{after_commit_label}").unwrap();
            writeln!(self.out, "{mark_committed_label}:").unwrap();
            writeln!(self.out, "  call i32 @nir_transact_mark_committed(ptr {txn_id_ptr_val}, i64 {txn_id_len_val})").unwrap();
            writeln!(self.out, "  br label %{after_commit_label}").unwrap();
            writeln!(self.out, "{after_commit_label}:").unwrap();
            // `success == false` here means every live retry attempt came
            // back `Err` — the row is left `commit_pending` (never
            // marked), recoverable by the next `nir_transact_replay_all`
            // rather than lost. The live `transact` expression's own `i1`
            // result still reports `true` unconditionally below, matching
            // this function's pre-existing, unchanged semantics: it
            // reflects *which branch `verify` chose*, not whether
            // `commit`'s own side effect is confirmed durable yet.
        }
        writeln!(self.out, "  store i1 true, ptr {result_slot}").unwrap();
        writeln!(self.out, "  br label %{after_verify_label}").unwrap();

        writeln!(self.out, "{compensate_label}:").unwrap();
        if let Some(c) = compensate {
            let (binds_ptr, binds_len) = self.emit_db_binds(&c.args, scopes)?;
            writeln!(self.out, "  call i32 @nir_transact_mark_compensate_pending(ptr {txn_id_ptr_val}, i64 {txn_id_len_val}, ptr {binds_ptr}, i64 {binds_len})").unwrap();
            let sig_params = self.sigs.get(&c.name).expect("typeck.rs already resolved this call").params.clone();
            let sig_ret = self.sigs.get(&c.name).expect("typeck.rs already resolved this call").ret.clone();
            let arg_operands = self.call_args(&c.args, &sig_params, scopes)?;
            let success = self.emit_call_with_retry(&c.name, &sig_ret, &arg_operands, "transact_compensate")?;
            let mark_compensated_label = self.fresh_label("transact_mark_compensated");
            let after_compensate_label = self.fresh_label("transact_after_compensate");
            writeln!(self.out, "  br i1 {success}, label %{mark_compensated_label}, label %{after_compensate_label}").unwrap();
            writeln!(self.out, "{mark_compensated_label}:").unwrap();
            writeln!(self.out, "  call i32 @nir_transact_mark_compensated(ptr {txn_id_ptr_val}, i64 {txn_id_len_val})").unwrap();
            writeln!(self.out, "  br label %{after_compensate_label}").unwrap();
            writeln!(self.out, "{after_compensate_label}:").unwrap();
        }
        writeln!(self.out, "  store i1 false, ptr {result_slot}").unwrap();
        writeln!(self.out, "  br label %{after_verify_label}").unwrap();

        writeln!(self.out, "{after_verify_label}:").unwrap();
        if let Some(l) = log {
            // Best-effort per `docs/TRANSACT.md` — but unlike the deleted
            // interpreter, a trap inside `log` is *not* swallowed here:
            // every compiled trap is an unconditional `abort()`, the same
            // as anywhere else in this language. A real, disclosed
            // difference, not a silent one.
            self.emit_transact_call(l, scopes)?;
        }
        writeln!(self.out, "  br label %{after_precheck_label}").unwrap();

        writeln!(self.out, "{after_precheck_label}:").unwrap();
        let result = self.fresh_reg("transact_result");
        writeln!(self.out, "  {result} = load i1, ptr {result_slot}").unwrap();

        // The replay trampoline is generated unconditionally (even for a
        // program whose durability log never actually records a pending
        // row for this site at runtime) and registered under `site_id` —
        // `emit_c_main`'s prologue calls `nir_transact_register_replay_site`
        // for every entry in `self.transact_sites` before ever calling
        // `nir_main`, so replay always has a trampoline for every site
        // this build knows about, no runtime conditionality needed here.
        let tramp_name = self.emit_transact_replay_trampoline(site_id, commit, compensate)?;
        self.transact_sites.push((site_id, tramp_name));

        scopes.pop();
        Ok(result)
    }


    /// `acquire name(proof)` — `docs/LANGUAGE.md` §6a, compiled for real
    /// 2026-09. `name` must be a `requires`-gated top-level fn
    /// (`typeck.rs` already proved this; `FnSig::requires` is this
    /// codegen's own copy of `FnDecl::requires`, looked up here since
    /// `Codegen` doesn't otherwise keep a reference to the whole
    /// `Program`). Builds a real `Result(Ty::Fn(params, ret), str)` by
    /// hand, the same tag-then-payload shape `emit_check_role` already
    /// uses: `Ok(f)` stores the target function's own address (`ptr
    /// @name` — a compile-time-known constant operand, no instruction
    /// needed) as the payload; `Err(reason)` stores a fresh string
    /// literal naming exactly which requirement `proof` failed to
    /// prove. The actual check is `emit_str_field_eq_check` against
    /// `proof`'s own `role`/`value` field — the same runtime comparison
    /// `emit_requirement_check` does for a *parameter*-shaped proof,
    /// just against an arbitrary expression's pointer instead.
    pub(super) fn emit_acquire(&mut self, name: &str, proof: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let params = self.sigs.get(name).expect("typeck.rs already resolved this acquire target").params.clone();
        let ret = self.sigs.get(name).expect("typeck.rs already resolved this acquire target").ret.clone();
        let req = self
            .sigs
            .get(name)
            .expect("typeck.rs already resolved this acquire target")
            .requires
            .clone()
            .expect("typeck.rs only allows acquire on a requires-gated fn");
        let fn_ty = Ty::Fn(params, Box::new(ret));
        let result_ty = Ty::Named("Result".to_string(), vec![fn_ty, Ty::Str]);

        let (proof_ty, field_name, expected): (Ty, &str, String) = match &req {
            Requirement::Role(r) => (Ty::Named("RoleView".to_string(), vec![]), "role", r.clone()),
            Requirement::Claim(_, v) => (Ty::Named("ClaimView".to_string(), vec![]), "value", v.clone()),
        };
        let proof_ptr = self.expr_ptr_expected(proof, &proof_ty, scopes)?;
        let authorized = self.emit_str_field_eq_check(&proof_ty, &proof_ptr, field_name, &expected)?;

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("acquire_result.addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("acquire_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("acquire_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("acquire_ok");
        let err_label = self.fresh_label("acquire_err");
        let merge_label = self.fresh_label("acquire_merge");
        writeln!(self.out, "  br i1 {authorized}, label %{ok_label}, label %{err_label}").unwrap();

        // `Ok(f)` — variant 0. The target function's own address is a
        // compile-time constant operand (`ptr @name`), not a value that
        // needs computing — functions are already global values of
        // pointer type in LLVM IR.
        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store ptr @{name}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        // `Err("...")` — variant 1. Same "the payload's first two words
        // are directly a `str` value" shape `emit_check_role`'s own
        // `Err` arm uses.
        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let msg = match &req {
            Requirement::Role(r) => format!("{name} requires role \"{r}\", which the given proof doesn't have"),
            Requirement::Claim(c, v) => format!("{name} requires claim \"{c}\"=\"{v}\", which the given proof doesn't have"),
        };
        let msg_global = self.fresh_global("acquire_err_msg");
        writeln!(
            self.string_globals,
            "{msg_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
            msg.len(),
            llvm_escape_bytes(msg.as_bytes())
        )
        .unwrap();
        let msg_partial = self.fresh_reg("acquire_err_msg_partial");
        writeln!(self.out, "  {msg_partial} = insertvalue {{ptr, i64}} undef, ptr {msg_global}, 0").unwrap();
        let msg_full = self.fresh_reg("acquire_err_msg_full");
        writeln!(self.out, "  {msg_full} = insertvalue {{ptr, i64}} {msg_partial}, i64 {}, 1", msg.len()).unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {msg_full}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    pub(super) fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span, scopes: &mut Scopes) -> Result<String, CodegenError> {
        if op == BinOp::And || op == BinOp::Or {
            return self.short_circuit(op, lhs, rhs, scopes);
        }

        // `==`/`!=` are the one pair of operators typeck.rs allows on
        // `Vector`/`Matrix` operands -- structural equality, not the
        // scalar `fcmp`/`icmp` this function does for everything else.
        // Caught here, before the `is_float` dispatch below (which
        // assumes a bare scalar SSA value on both sides), rather than in
        // `expr()`'s own `Expr::Binary` arm, so every other call site of
        // `binary()` stays untouched.
        if matches!(op, BinOp::Eq | BinOp::NotEq) {
            let lhs_ty = self.local_ty_of(lhs, scopes);
            if lhs_ty.is_aggregate() {
                return self.agg_eq(op, lhs, rhs, &lhs_ty, scopes);
            }
            // `str` is the other non-`is_float`/non-plain-integer operand
            // shape `==`/`!=` has to special-case before the `is_float`
            // dispatch below (which assumes a bare scalar `fcmp`/`icmp`-
            // ready SSA value on both sides, not a `{ptr, i64}` struct).
            if lhs_ty == Ty::Str {
                return self.str_eq(op, lhs, rhs, scopes);
            }
        }

        // `dec128` arithmetic and comparisons — linked calls into
        // `runtime-kernels/src/lib.rs`'s `rust_decimal`-backed kernels
        // (`DEC128_BUILTINS`'s doc comment covers the builtin-call half;
        // this is the operator half). Checked before the `is_float`
        // dispatch below, which assumes a bare scalar `i64`/`double` SSA
        // value on both sides, not a `{i64, i64}` struct.
        if self.local_ty_of(lhs, scopes) == Ty::Dec128 {
            let l = self.expr(lhs, scopes)?;
            let r = self.expr(rhs, scopes)?;
            if let Some(kernel) = match op {
                BinOp::Add => Some("nir_dec128_add"),
                BinOp::Sub => Some("nir_dec128_sub"),
                BinOp::Mul | BinOp::ElemMul => Some("nir_dec128_mul"),
                BinOp::Div | BinOp::ElemDiv => Some("nir_dec128_div"),
                _ => None,
            } {
                let out = self.fresh_reg("dec128_binop");
                writeln!(self.out, "  {out} = call {{i64, i64}} @{kernel}({{i64, i64}} {l}, {{i64, i64}} {r})").unwrap();
                return Ok(out);
            }
            // Every comparison is `nir_dec128_cmp`'s real total
            // ordering (`Decimal: Ord`, `runtime-kernels/src/lib.rs`'s own doc
            // comment) compared against `0` — matches
            // `interpreter.rs`'s own `Eq`/`NotEq`/`Lt`/`Gt`/`LtEq`/
            // `GtEq` arm, which is just `Decimal`'s own `<`/`>`/etc.
            // operators underneath.
            let cmp = self.fresh_reg("dec128_cmp");
            writeln!(self.out, "  {cmp} = call i32 @nir_dec128_cmp({{i64, i64}} {l}, {{i64, i64}} {r})").unwrap();
            return match op {
                BinOp::Eq => self.icmp("eq", "i32", &cmp, "0"),
                BinOp::NotEq => self.icmp("ne", "i32", &cmp, "0"),
                BinOp::Lt => self.icmp("slt", "i32", &cmp, "0"),
                BinOp::Gt => self.icmp("sgt", "i32", &cmp, "0"),
                BinOp::LtEq => self.icmp("sle", "i32", &cmp, "0"),
                BinOp::GtEq => self.icmp("sge", "i32", &cmp, "0"),
                _ => unreachable!("BinOp has no other variants besides arithmetic/comparison/And/Or, and And/Or short-circuit before this function even runs"),
            };
        }

        // Arithmetic and ordering comparisons only ever apply to numeric
        // operands (typeck.rs's `unify_operands` rejects `bool` for all
        // of these except `==`/`!=`) — every arm below except Eq/NotEq
        // dispatches on whether the operands are `f64` or plain integer;
        // `l`/`r` are `i64` (or `i1`, for the Eq/NotEq-on-bool case) for
        // the integer case, or a genuine `double` for the float case —
        // see the module doc and `Ty::F64`'s codegen note in `llvm_ty`.
        // typeck.rs's `unify_operands`/`infer_hadamard` already proved
        // both operands share exactly one type, so checking `lhs` alone
        // is enough to know which.
        let is_float = self.local_ty_of(lhs, scopes) == Ty::F64;
        let l = self.expr(lhs, scopes)?;
        let r = self.expr(rhs, scopes)?;
        match op {
            BinOp::Add => {
                let out = self.fresh_reg("add");
                if is_float {
                    writeln!(self.out, "  {out} = fadd double {l}, {r}").unwrap();
                } else {
                    writeln!(self.out, "  {out} = add i64 {l}, {r}").unwrap();
                }
                Ok(out)
            }
            BinOp::Sub => {
                let out = self.fresh_reg("sub");
                if is_float {
                    writeln!(self.out, "  {out} = fsub double {l}, {r}").unwrap();
                } else {
                    writeln!(self.out, "  {out} = sub i64 {l}, {r}").unwrap();
                }
                Ok(out)
            }
            // `.*` on plain scalars is legal too (`infer_hadamard`'s doc
            // comment: two matching scalars are trivially "the same
            // shape") and means exactly the same thing as `*` there --
            // `scalar_binop`'s own `Int`/`Float` arms already treat
            // `Mul`/`ElemMul` identically, so this does too.
            BinOp::Mul | BinOp::ElemMul => {
                let out = self.fresh_reg("mul");
                if is_float {
                    writeln!(self.out, "  {out} = fmul double {l}, {r}").unwrap();
                } else {
                    writeln!(self.out, "  {out} = mul i64 {l}, {r}").unwrap();
                }
                Ok(out)
            }
            // Same story as `ElemMul` above -- `scalar_binop` treats
            // `Div`/`ElemDiv` identically, so `2 ./ 3` on plain scalars
            // takes this arm too.
            BinOp::Div | BinOp::ElemDiv => {
                if is_float {
                    // IEEE 754 division by zero saturates to inf/-inf/NaN
                    // rather than trapping (`Ty::F64`'s doc comment,
                    // matching the interpreter exactly) -- no guard to
                    // emit here at all, audited or not.
                    let out = self.fresh_reg("fdiv");
                    writeln!(self.out, "  {out} = fdiv double {l}, {r}").unwrap();
                    return Ok(out);
                }
                self.guard_nonzero_divisor(&r, span);
                let out = self.fresh_reg("sdiv");
                writeln!(self.out, "  {out} = sdiv i64 {l}, {r}").unwrap();
                Ok(out)
            }
            // `%` -- truncating remainder, same is_float split as `Div`
            // just above (typeck.rs's own `BinOp::Rem` arm already
            // rejected `Dec128` before codegen ever sees one, so there's
            // no third dispatch to handle here the way the dec128 block
            // at the top of this function has for `Add`/`Sub`/`Mul`/`Div`).
            BinOp::Rem => {
                if is_float {
                    // Same "no guard, saturates to NaN" reasoning `Div`'s
                    // float arm already gives: IEEE 754 `frem`-by-zero is
                    // NaN, never a trap.
                    let out = self.fresh_reg("frem");
                    writeln!(self.out, "  {out} = frem double {l}, {r}").unwrap();
                    return Ok(out);
                }
                self.guard_nonzero_divisor(&r, span);
                let out = self.fresh_reg("srem");
                writeln!(self.out, "  {out} = srem i64 {l}, {r}").unwrap();
                Ok(out)
            }
            // `==`/`!=` are the one pair typeck.rs allows on `bool`
            // operands too — pick i1/i64/double based on the *operand's*
            // declared type.
            BinOp::Eq | BinOp::NotEq => {
                if is_float {
                    let cond = if op == BinOp::Eq { "oeq" } else { "one" };
                    return self.fcmp(cond, &l, &r);
                }
                let cmp_ty = if self.local_ty_of(lhs, scopes) == Ty::Bool { "i1" } else { "i64" };
                let cond = if op == BinOp::Eq { "eq" } else { "ne" };
                self.icmp(cond, cmp_ty, &l, &r)
            }
            BinOp::Lt if is_float => self.fcmp("olt", &l, &r),
            BinOp::Gt if is_float => self.fcmp("ogt", &l, &r),
            BinOp::LtEq if is_float => self.fcmp("ole", &l, &r),
            BinOp::GtEq if is_float => self.fcmp("oge", &l, &r),
            BinOp::Lt => self.icmp("slt", "i64", &l, &r),
            BinOp::Gt => self.icmp("sgt", "i64", &l, &r),
            BinOp::LtEq => self.icmp("sle", "i64", &l, &r),
            BinOp::GtEq => self.icmp("sge", "i64", &l, &r),
            BinOp::And | BinOp::Or => unreachable!("handled above"),
        }
    }


    /// Evaluates a `str`-typed expression and extracts its `(ptr, i64
    /// len)` fields — the marshaling every `tcp` kernel call needs for a
    /// `str` argument, factored out since `connect`'s host and `send`'s
    /// payload both need exactly this.
    pub(super) fn str_parts(&mut self, e: &Expr, scopes: &mut Scopes) -> Result<(String, String), CodegenError> {
        let v = self.expr(e, scopes)?;
        let ptr = self.fresh_reg("str_ptr");
        writeln!(self.out, "  {ptr} = extractvalue {{ptr, i64}} {v}, 0").unwrap();
        let len = self.fresh_reg("str_len");
        writeln!(self.out, "  {len} = extractvalue {{ptr, i64}} {v}, 1").unwrap();
        Ok((ptr, len))
    }


    /// Recursively emit a structural equality comparison for two values
    /// of the same type `ty`, pointed to by `l_ptr` and `r_ptr`. Returns an
    /// `i1` SSA register that is true iff the values are equal. Mirrors
    /// `Value::PartialEq` in the interpreter: content equality for `box`/
    /// `&`, field-by-field for structs, tag-then-payload for enums, and
    /// elementwise for `Vector`/`Matrix`.
    pub(super) fn emit_deep_eq(&mut self, l_ptr: &str, r_ptr: &str, ty: &Ty, span: Span) -> Result<String, CodegenError> {
        match ty {
            Ty::Unit => {
                let one = self.fresh_reg("unit_eq");
                writeln!(self.out, "  {one} = add i1 0, 1").unwrap();
                Ok(one)
            }
            Ty::Bool | Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::Usize => {
                let llty = self.llvm_ty(ty)?;
                let l = self.fresh_reg("l");
                let r = self.fresh_reg("r");
                writeln!(self.out, "  {l} = load {llty}, ptr {l_ptr}").unwrap();
                writeln!(self.out, "  {r} = load {llty}, ptr {r_ptr}").unwrap();
                self.icmp("eq", &llty, &l, &r)
            }
            Ty::F64 => {
                let l = self.fresh_reg("l");
                let r = self.fresh_reg("r");
                writeln!(self.out, "  {l} = load double, ptr {l_ptr}").unwrap();
                writeln!(self.out, "  {r} = load double, ptr {r_ptr}").unwrap();
                self.fcmp("oeq", &l, &r)
            }
            Ty::Str => {
                let l_ptr_reg = self.fresh_reg("str_l_ptr");
                let l_len = self.fresh_reg("str_l_len");
                let r_ptr_reg = self.fresh_reg("str_r_ptr");
                let r_len = self.fresh_reg("str_r_len");
                writeln!(self.out, "  {l_ptr_reg} = load ptr, ptr {l_ptr}").unwrap();
                writeln!(self.out, "  {l_len} = load i64, ptr getelementptr ({{ptr, i64}}, ptr {l_ptr}, i32 0, i32 1)").unwrap();
                writeln!(self.out, "  {r_ptr_reg} = load ptr, ptr {r_ptr}").unwrap();
                writeln!(self.out, "  {r_len} = load i64, ptr getelementptr ({{ptr, i64}}, ptr {r_ptr}, i32 0, i32 1)").unwrap();
                let raw = self.fresh_reg("str_eq_raw");
                writeln!(
                    self.out,
                    "  {raw} = call i32 @nir_str_eq(ptr {l_ptr_reg}, i64 {l_len}, ptr {r_ptr_reg}, i64 {r_len})"
                )
                .unwrap();
                self.icmp("ne", "i32", &raw, "0")
            }
            Ty::Box(inner) | Ty::Ref(inner) => {
                let l = self.fresh_reg("box_l");
                let r = self.fresh_reg("box_r");
                writeln!(self.out, "  {l} = load ptr, ptr {l_ptr}").unwrap();
                writeln!(self.out, "  {r} = load ptr, ptr {r_ptr}").unwrap();
                self.emit_deep_eq(&l, &r, inner, span)
            }
            Ty::Vector(elem, n) | Ty::Matrix(elem, _, n) => {
                let len = if matches!(ty, Ty::Vector(..)) { *n } else { n * n };
                let elem_llty = self.llvm_ty(elem)?;
                let mut acc: Option<String> = None;
                for i in 0..len {
                    let l_elem = self.agg_elem_ptr(l_ptr, &elem_llty, i);
                    let r_elem = self.agg_elem_ptr(r_ptr, &elem_llty, i);
                    let eq = self.emit_deep_eq(&l_elem, &r_elem, elem, span)?;
                    acc = Some(match acc {
                        None => eq,
                        Some(prev) => {
                            let out = self.fresh_reg("agg_eq_and");
                            writeln!(self.out, "  {out} = and i1 {prev}, {eq}").unwrap();
                            out
                        }
                    });
                }
                let all_eq = acc.unwrap_or_else(|| {
                    let one = self.fresh_reg("empty_agg_eq");
                    writeln!(self.out, "  {one} = add i1 0, 1").unwrap();
                    one
                });
                Ok(all_eq)
            }
            Ty::Named(name, args) => {
                if let Some(fields) = self.registry.struct_fields(name) {
                    let type_params = self.registry.struct_type_params(name).unwrap_or(&[]);
                    let subst = zip_type_params(type_params, args);
                    let struct_ty = Ty::Named(name.to_string(), args.to_vec());
                    let struct_llty = self.llvm_ty(&struct_ty)?;
                    let mut acc: Option<String> = None;
                    for (i, f) in fields.iter().enumerate() {
                        let field_ty = substitute_ty(&f.ty, &subst);
                        let l_field = self.fresh_reg("l_field");
                        let r_field = self.fresh_reg("r_field");
                        writeln!(self.out, "  {l_field} = getelementptr inbounds {struct_llty}, ptr {l_ptr}, i32 0, i32 {i}").unwrap();
                        writeln!(self.out, "  {r_field} = getelementptr inbounds {struct_llty}, ptr {r_ptr}, i32 0, i32 {i}").unwrap();
                        let eq = self.emit_deep_eq(&l_field, &r_field, &field_ty, span)?;
                        acc = Some(match acc {
                            None => eq,
                            Some(prev) => {
                                let out = self.fresh_reg("struct_eq_and");
                                writeln!(self.out, "  {out} = and i1 {prev}, {eq}").unwrap();
                                out
                            }
                        });
                    }
                    let all_eq = acc.unwrap_or_else(|| {
                        let one = self.fresh_reg("empty_struct_eq");
                        writeln!(self.out, "  {one} = add i1 0, 1").unwrap();
                        one
                    });
                    Ok(all_eq)
                } else if let Some(variants) = self.registry.enum_variants(name) {
                    let type_params = self.registry.enum_type_params(name).unwrap_or(&[]);
                    let subst = zip_type_params(type_params, args);
                    let enum_ty = Ty::Named(name.to_string(), args.to_vec());
                    let enum_llty = self.llvm_ty(&enum_ty)?;

                    let l_tag_ptr = self.fresh_reg("l_tag_addr");
                    let r_tag_ptr = self.fresh_reg("r_tag_addr");
                    writeln!(self.out, "  {l_tag_ptr} = getelementptr inbounds {enum_llty}, ptr {l_ptr}, i32 0, i32 0").unwrap();
                    writeln!(self.out, "  {r_tag_ptr} = getelementptr inbounds {enum_llty}, ptr {r_ptr}, i32 0, i32 0").unwrap();
                    let l_tag = self.fresh_reg("l_tag");
                    let r_tag = self.fresh_reg("r_tag");
                    writeln!(self.out, "  {l_tag} = load i64, ptr {l_tag_ptr}").unwrap();
                    writeln!(self.out, "  {r_tag} = load i64, ptr {r_tag_ptr}").unwrap();
                    let tag_eq = self.icmp("eq", "i64", &l_tag, &r_tag)?;

                    let neq_label = self.fresh_label("enum_eq_neq");
                    let cmp_label = self.fresh_label("enum_eq_cmp");
                    let merge_label = self.fresh_label("enum_eq_merge");
                    writeln!(self.out, "  br i1 {tag_eq}, label %{cmp_label}, label %{neq_label}").unwrap();
                    self.terminated = true;

                    writeln!(self.out, "{neq_label}:").unwrap();
                    self.terminated = false;
                    writeln!(self.out, "  br label %{merge_label}").unwrap();
                    self.terminated = true;

                    writeln!(self.out, "{cmp_label}:").unwrap();
                    self.terminated = false;
                    let l_payload = self.fresh_reg("l_payload");
                    let r_payload = self.fresh_reg("r_payload");
                    writeln!(self.out, "  {l_payload} = getelementptr inbounds {enum_llty}, ptr {l_ptr}, i32 0, i32 1").unwrap();
                    writeln!(self.out, "  {r_payload} = getelementptr inbounds {enum_llty}, ptr {r_ptr}, i32 0, i32 1").unwrap();

                    let default_label = self.fresh_label("enum_eq_default");
                    let mut case_labels: Vec<String> = Vec::new();
                    let mut phi_entries: Vec<(String, String)> = Vec::new();
                    for (vidx, _v) in variants.iter().enumerate() {
                        let label = self.fresh_label(&format!("enum_eq_v{vidx}"));
                        case_labels.push(label.clone());
                    }
                    writeln!(self.out, "  switch i64 {l_tag}, label %{default_label} [").unwrap();
                    // case labels are emitted below, inside the switch brackets
                    for (vidx, _v) in variants.iter().enumerate() {
                        writeln!(self.out, "    i64 {vidx}, label %{}", case_labels[vidx]).unwrap();
                    }
                    writeln!(self.out, "  ]").unwrap();
                    self.terminated = true;

                    writeln!(self.out, "{default_label}:").unwrap();
                    self.terminated = false;
                    writeln!(self.out, "  unreachable").unwrap();
                    self.terminated = true;

                    for (vidx, v) in variants.iter().enumerate() {
                        let label = &case_labels[vidx];
                        writeln!(self.out, "{label}:").unwrap();
                        self.terminated = false;
                        let mut acc: Option<String> = None;
                        let mut word_off: u64 = 0;
                        for decl_ty in &v.payload {
                            let field_ty = substitute_ty(decl_ty, &subst);
                            let l_field = self.fresh_reg("l_field");
                            let r_field = self.fresh_reg("r_field");
                            writeln!(self.out, "  {l_field} = getelementptr inbounds i64, ptr {l_payload}, i64 {word_off}").unwrap();
                            writeln!(self.out, "  {r_field} = getelementptr inbounds i64, ptr {r_payload}, i64 {word_off}").unwrap();
                            let eq = self.emit_deep_eq(&l_field, &r_field, &field_ty, span)?;
                            acc = Some(match acc {
                                None => eq,
                                Some(prev) => {
                                    let out = self.fresh_reg("enum_field_and");
                                    writeln!(self.out, "  {out} = and i1 {prev}, {eq}").unwrap();
                                    out
                                }
                            });
                            word_off += conservative_word_count(&field_ty, &self.registry);
                        }
                        let variant_eq = acc.unwrap_or_else(|| {
                            let one = self.fresh_reg("empty_variant_eq");
                            writeln!(self.out, "  {one} = add i1 0, 1").unwrap();
                            one
                        });
                        writeln!(self.out, "  br label %{merge_label}").unwrap();
                        self.terminated = true;
                        phi_entries.push((variant_eq, label.clone()));
                    }

                    writeln!(self.out, "{merge_label}:").unwrap();
                    self.terminated = false;
                    let result = self.fresh_reg("enum_eq");
                    let mut phi = format!("  {result} = phi i1 [ 0, %{neq_label} ]");
                    for (val, label) in &phi_entries {
                        phi.push_str(&format!(", [ {val}, %{label} ]"));
                    }
                    writeln!(self.out, "{phi}").unwrap();
                    Ok(result)
                } else {
                    Err(CodegenError {
                        message: format!("equality not supported for named type `{name}` in compiled code"),
                    })
                }
            }
            _ => Err(CodegenError {
                message: format!("equality not supported for `{ty:?}` in compiled code"),
            }),
        }
    }


    /// Structural `==`/`!=` on aggregate values (`Vector`/`Matrix`/
    /// `struct`/`enum`) — delegates to `emit_deep_eq` for the recursive,
    /// type-driven comparison. Produces a scalar `bool`, so this is reached
    /// from `binary()` (`expr()`'s path), not `expr_ptr()`.
    pub(super) fn agg_eq(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, ty: &Ty, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let l_ptr = self.expr_ptr(lhs, scopes)?;
        let r_ptr = self.expr_ptr(rhs, scopes)?;
        let eq = self.emit_deep_eq(&l_ptr, &r_ptr, ty, lhs.span())?;
        if op == BinOp::Eq {
            Ok(eq)
        } else {
            let out = self.fresh_reg("agg_neq");
            writeln!(self.out, "  {out} = xor i1 {eq}, true").unwrap();
            Ok(out)
        }
    }


    /// `str`'s `==`/`!=` — a linked call into `nir_str_eq` (length check +
    /// byte compare, `runtime-kernels/src/lib.rs`), the same "reuse proven Rust
    /// code via a call" choice as `det`/`inv`/etc., not hand-emitted IR.
    /// `binary()`'s `Eq`/`NotEq` intercept routes here before the
    /// `is_float` dispatch that assumes a scalar operand.
    pub(super) fn str_eq(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let l = self.expr(lhs, scopes)?;
        let r = self.expr(rhs, scopes)?;
        let l_ptr = self.fresh_reg("str_eq_lptr");
        writeln!(self.out, "  {l_ptr} = extractvalue {{ptr, i64}} {l}, 0").unwrap();
        let l_len = self.fresh_reg("str_eq_llen");
        writeln!(self.out, "  {l_len} = extractvalue {{ptr, i64}} {l}, 1").unwrap();
        let r_ptr = self.fresh_reg("str_eq_rptr");
        writeln!(self.out, "  {r_ptr} = extractvalue {{ptr, i64}} {r}, 0").unwrap();
        let r_len = self.fresh_reg("str_eq_rlen");
        writeln!(self.out, "  {r_len} = extractvalue {{ptr, i64}} {r}, 1").unwrap();
        let raw = self.fresh_reg("str_eq_raw");
        writeln!(self.out, "  {raw} = call i32 @nir_str_eq(ptr {l_ptr}, i64 {l_len}, ptr {r_ptr}, i64 {r_len})").unwrap();
        // `nir_str_eq` returns 1 (equal) / 0 (not equal) — `Eq` wants
        // "raw != 0", `NotEq` wants "raw == 0".
        let cond = if op == BinOp::Eq { "ne" } else { "eq" };
        self.icmp(cond, "i32", &raw, "0")
    }


    /// `expr_ptr`'s `Expr::Binary` case — every Vector/Matrix-*producing*
    /// binary operator (elementwise `+`/`-`/`.*`/`./`, and `*` in its
    /// three legal shapes), fully unrolled at codegen time since every
    /// shape involved is a compile-time literal (typeck's
    /// `literal_dimension` rule) — no runtime loop, no data-dependent
    /// control flow anywhere in this phase.
    pub(super) fn agg_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span, scopes: &mut Scopes) -> Result<String, CodegenError> {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::ElemMul | BinOp::ElemDiv => self.agg_elementwise(op, lhs, rhs, span, scopes),
            BinOp::Mul => self.agg_mul(lhs, rhs, scopes),
            _ => unreachable!("typeck.rs never types another binary op as Vector/Matrix-producing"),
        }
    }


    /// Elementwise `+`/`-`/`.*`/`./` — same shape on both sides
    /// (typeck-guaranteed, not re-checked here), one scalar instruction
    /// per element. Integer `./` traps on a zero divisor per-element,
    /// same as scalar `/`/`./` (`guard_nonzero_divisor`); float `./`
    /// saturates, never traps, matching `scalar_binop`'s `Float` arm.
    pub(super) fn agg_elementwise(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let ty = self.local_ty_of(lhs, scopes);
        let agg_llty = self.llvm_ty(&ty)?;
        let dest = self.fresh_reg("agg_elemwise.addr");
        self.emit_alloca(&dest, &agg_llty);

        let l_ptr = self.expr_ptr(lhs, scopes)?;
        let r_ptr = self.expr_ptr(rhs, scopes)?;
        let (elem, len) = agg_elem_and_len(&ty);
        let elem = elem.clone();
        let elem_llty = self.llvm_ty(&elem)?;
        let is_float = elem == Ty::F64;

        for i in 0..len {
            let l = self.agg_load_elem(&l_ptr, &elem_llty, &elem, i);
            let r = self.agg_load_elem(&r_ptr, &elem_llty, &elem, i);
            let out = match op {
                BinOp::Add => self.emit_add(&l, &r, is_float),
                BinOp::Sub => {
                    let out = self.fresh_reg("agg_sub");
                    if is_float {
                        writeln!(self.out, "  {out} = fsub double {l}, {r}").unwrap();
                    } else {
                        writeln!(self.out, "  {out} = sub i64 {l}, {r}").unwrap();
                    }
                    out
                }
                BinOp::ElemMul => self.emit_mul(&l, &r, is_float),
                BinOp::ElemDiv => {
                    if is_float {
                        let out = self.fresh_reg("agg_fdiv");
                        writeln!(self.out, "  {out} = fdiv double {l}, {r}").unwrap();
                        out
                    } else {
                        self.guard_nonzero_divisor(&r, span);
                        let out = self.fresh_reg("agg_sdiv");
                        writeln!(self.out, "  {out} = sdiv i64 {l}, {r}").unwrap();
                        out
                    }
                }
                _ => unreachable!("agg_binary only dispatches elementwise ops here"),
            };
            self.agg_store_elem(&dest, &elem_llty, &elem, i, &out)?;
        }
        Ok(dest)
    }


    /// `*` in its three legal aggregate-producing shapes. Loop nesting
    /// and accumulation order match `interpreter.rs::eval_binary`'s
    /// `Matrix`/`Vector` `Mul` arms exactly (module doc, design decision
    /// 3) — first term computed directly, then each remaining term
    /// folded in left-to-right — so floating-point summation order is
    /// bit-identical to the interpreter's output, not just mathematically
    /// equivalent.
    pub(super) fn agg_mul(&mut self, lhs: &Expr, rhs: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let lt = self.local_ty_of(lhs, scopes);
        let rt = self.local_ty_of(rhs, scopes);
        let result_ty = self.mul_result_ty(lhs, rhs, scopes);
        let agg_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("agg_mul.addr");
        self.emit_alloca(&dest, &agg_llty);

        match (&lt, &rt) {
            // scalar * Matrix, either order -- elementwise scale.
            (s, mt @ Ty::Matrix(..)) if !s.is_aggregate() => {
                let scalar = self.expr(lhs, scopes)?;
                let m_ptr = self.expr_ptr(rhs, scopes)?;
                self.agg_scale(&scalar, &m_ptr, mt, &dest)?;
            }
            (mt @ Ty::Matrix(..), s) if !s.is_aggregate() => {
                let m_ptr = self.expr_ptr(lhs, scopes)?;
                let scalar = self.expr(rhs, scopes)?;
                self.agg_scale(&scalar, &m_ptr, mt, &dest)?;
            }
            // Matrix * Vector -- unrolled dot-product-per-row. Matches
            // interpreter.rs: `sum = m[i,0]*v[0]; for k in 1..cols: sum
            // += m[i,k]*v[k]`.
            (Ty::Matrix(m_elem, rows, cols), Ty::Vector(..)) => {
                let m_ptr = self.expr_ptr(lhs, scopes)?;
                let v_ptr = self.expr_ptr(rhs, scopes)?;
                let elem_llty = self.llvm_ty(m_elem)?;
                let is_float = **m_elem == Ty::F64;
                let cols = *cols;
                for i in 0..*rows {
                    let m0 = self.agg_load_elem(&m_ptr, &elem_llty, m_elem, i * cols);
                    let v0 = self.agg_load_elem(&v_ptr, &elem_llty, m_elem, 0);
                    let mut sum = self.emit_mul(&m0, &v0, is_float);
                    for k in 1..cols {
                        let mk = self.agg_load_elem(&m_ptr, &elem_llty, m_elem, i * cols + k);
                        let vk = self.agg_load_elem(&v_ptr, &elem_llty, m_elem, k);
                        let prod = self.emit_mul(&mk, &vk, is_float);
                        sum = self.emit_add(&sum, &prod, is_float);
                    }
                    self.agg_store_elem(&dest, &elem_llty, m_elem, i, &sum)?;
                }
            }
            // Matrix * Matrix -- unrolled triple-nested accumulation.
            // Matches interpreter.rs: `sum = a[i,0]*b[0,j]; for k in
            // 1..ac: sum += a[i,k]*b[k,j]`.
            (Ty::Matrix(l_elem, r1, c1), Ty::Matrix(_, _r2, c2)) => {
                let a_ptr = self.expr_ptr(lhs, scopes)?;
                let b_ptr = self.expr_ptr(rhs, scopes)?;
                let elem_llty = self.llvm_ty(l_elem)?;
                let is_float = **l_elem == Ty::F64;
                let (ac, bc) = (*c1, *c2);
                for i in 0..*r1 {
                    for j in 0..bc {
                        let a0 = self.agg_load_elem(&a_ptr, &elem_llty, l_elem, i * ac);
                        let b0 = self.agg_load_elem(&b_ptr, &elem_llty, l_elem, j);
                        let mut sum = self.emit_mul(&a0, &b0, is_float);
                        for k in 1..ac {
                            let ak = self.agg_load_elem(&a_ptr, &elem_llty, l_elem, i * ac + k);
                            let bk = self.agg_load_elem(&b_ptr, &elem_llty, l_elem, k * bc + j);
                            let prod = self.emit_mul(&ak, &bk, is_float);
                            sum = self.emit_add(&sum, &prod, is_float);
                        }
                        self.agg_store_elem(&dest, &elem_llty, l_elem, i * bc + j, &sum)?;
                    }
                }
            }
            _ => unreachable!("typeck::infer_mul already restricted the legal shapes"),
        }
        Ok(dest)
    }


    /// `agg_mul`'s scalar × `Matrix` case (either operand order already
    /// normalized by the caller) — elementwise scale, unrolled.
    pub(super) fn agg_scale(&mut self, scalar: &str, m_ptr: &str, mat_ty: &Ty, dest: &str) -> Result<(), CodegenError> {
        let (elem, len) = agg_elem_and_len(mat_ty);
        let elem = elem.clone();
        let elem_llty = self.llvm_ty(&elem)?;
        let is_float = elem == Ty::F64;
        for i in 0..len {
            let m = self.agg_load_elem(m_ptr, &elem_llty, &elem, i);
            let out = self.emit_mul(&m, scalar, is_float);
            self.agg_store_elem(dest, &elem_llty, &elem, i, &out)?;
        }
        Ok(())
    }


    /// `&&`/`||` as real branches, not eager `and`/`or` — see module doc.
    pub(super) fn short_circuit(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ptr = self.fresh_reg("logic_result.addr");
        self.emit_alloca(&result_ptr, "i1");

        let l = self.expr(lhs, scopes)?;
        let rhs_label = self.fresh_label("logic_rhs");
        let short_label = self.fresh_label("logic_short");
        let merge_label = self.fresh_label("logic_merge");

        if op == BinOp::And {
            writeln!(self.out, "  br i1 {l}, label %{rhs_label}, label %{short_label}").unwrap();
        } else {
            writeln!(self.out, "  br i1 {l}, label %{short_label}, label %{rhs_label}").unwrap();
        }

        writeln!(self.out, "{short_label}:").unwrap();
        writeln!(self.out, "  store i1 {l}, ptr {result_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{rhs_label}:").unwrap();
        let r = self.expr(rhs, scopes)?;
        writeln!(self.out, "  store i1 {r}, ptr {result_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let out = self.fresh_reg("logic_val");
        writeln!(self.out, "  {out} = load i1, ptr {result_ptr}").unwrap();
        Ok(out)
    }


    pub(super) fn if_expr(
        &mut self,
        cond: &Expr,
        then_block: &Block,
        else_block: Option<&ElseBranch>,
        span: Span,
        expected: Option<&Ty>,
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        let c = self.expr(cond, scopes)?;
        let then_label = self.fresh_label("if_then");
        let else_label = self.fresh_label("if_else");
        let merge_label = self.fresh_label("if_merge");

        // `expected` -- when a caller one frame up (`expr_ptr_expected`)
        // already has a concrete type in hand -- is authoritative and
        // used directly, no inference needed at all; `if_result_ty`'s
        // own (necessarily weaker, no-context) sibling-branch inference
        // is only a fallback for when this `if` is reached with no such
        // context (a bare statement, or nested inside a scalar `expr()`
        // dispatch).
        let result_ty = expected.cloned().unwrap_or_else(|| self.if_result_ty(then_block, else_block, scopes));
        // `unit` has no LLVM value to hold at all (`alloca void` isn't
        // legal IR) — a `unit`-valued if is only ever run for its
        // branches' side effects, so there's no slot to allocate, only
        // both branches to execute.
        let slot = if result_ty == Ty::Unit {
            None
        } else {
            let llty = self.llvm_ty(&result_ty)?;
            let ptr = self.fresh_reg("if_result.addr");
            self.emit_alloca(&ptr, &llty);
            Some((ptr, llty))
        };
        let is_aggregate = result_ty.is_aggregate();

        writeln!(self.out, "  br i1 {c}, label %{then_label}, label %{else_label}").unwrap();

        writeln!(self.out, "{then_label}:").unwrap();
        self.terminated = false;
        scopes.push();
        if let Some((ptr, _)) = &slot {
            self.block_value_to_slot(then_block, &ptr, &result_ty, scopes)?;
        } else {
            self.block_side_effects(then_block, scopes)?;
        }
        // Each branch is its own scope with its own independent affine
        // ownership — only the branch that actually runs at runtime frees
        // what it itself still owns there; the other branch's own
        // (different) still-owned set, if any, is a separate `FreeMap`
        // entry keyed by the same `if`'s span plus the other bool.
        if !self.terminated
            && let Some(names) = self.free_map.at_if_branch_end.get(&(span, true)).cloned()
        {
            self.emit_frees_for_names(&names, scopes);
        }
        scopes.pop();
        let then_terminated = self.terminated;
        if !self.terminated {
            writeln!(self.out, "  br label %{merge_label}").unwrap();
        }

        writeln!(self.out, "{else_label}:").unwrap();
        self.terminated = false;
        match else_block {
            Some(ElseBranch::Block(b)) => {
                scopes.push();
                if let Some((ptr, _)) = &slot {
                    self.block_value_to_slot(b, &ptr, &result_ty, scopes)?;
                } else {
                    self.block_side_effects(b, scopes)?;
                }
                if !self.terminated
                    && let Some(names) = self.free_map.at_if_branch_end.get(&(span, false)).cloned()
                {
                    self.emit_frees_for_names(&names, scopes);
                }
                scopes.pop();
            }
            Some(ElseBranch::If(e2)) => {
                // An `else if` produces a single value; route through the
                // same `if_expr` so nested scalar/aggregate handling is
                // uniform. The slot is shared with the outer `if`.
                if let Some((ptr, _)) = &slot {
                    if result_ty.is_aggregate() {
                        let src = self.expr_ptr(e2, scopes)?;
                        if !self.terminated {
                            let bytes = agg_byte_size_operand(&result_ty, &self.registry);
                            writeln!(
                                self.out,
                                "  call void @llvm.memcpy.p0.p0.i64(ptr {ptr}, ptr {src}, i64 {bytes}, i1 false)"
                            )
                            .unwrap();
                        }
                    } else {
                        let v = self.expr(e2, scopes)?;
                        if !self.terminated {
                            let llty = self.llvm_ty(&result_ty)?;
                            writeln!(self.out, "  store {llty} {v}, ptr {ptr}").unwrap();
                        }
                    }
                } else {
                    // unit-valued else-if: evaluate for side effects only.
                    self.expr(e2, scopes)?;
                }
            }
            None => {}
        };
        let else_terminated = self.terminated;
        if !self.terminated {
            writeln!(self.out, "  br label %{merge_label}").unwrap();
        }

        // The merge block is only reachable if at least one branch falls
        // through to it — if both branches unconditionally `return`,
        // there's nothing to merge, and the merge block would be dead
        // (valid but pointless) IR. Emit it regardless for simplicity;
        // `terminated` correctly reflects "both branches returned" so
        // the caller (a `let`/`return` around this `if`) won't try to
        // use a value that was never actually produced on any live path.
        writeln!(self.out, "{merge_label}:").unwrap();
        self.terminated = then_terminated && else_terminated;
        if self.terminated {
            writeln!(self.out, "  unreachable").unwrap();
            return Ok("0".to_string());
        }
        match slot {
            Some((ptr, llty)) => {
                if is_aggregate {
                    // The caller asked for a pointer (we're on the
                    // `expr_ptr` path); the slot itself is the value.
                    Ok(ptr)
                } else {
                    let out = self.fresh_reg("if_val");
                    writeln!(self.out, "  {out} = load {llty}, ptr {ptr}").unwrap();
                    Ok(out)
                }
            }
            None => Ok("0".to_string()), // unit; never meaningfully read
        }
    }


    /// `match scrutinee { ... }` — Row 11's destructuring expression,
    /// shaped exactly like `if_expr`'s slot-allocate/branch/merge
    /// structure, generalized from 2 branches to N. Two scrutinee shapes,
    /// exactly matching `typeck.rs`'s own `check_match`/
    /// `check_literal_match` split and `interpreter.rs`'s own `Expr::Match`
    /// eval:
    ///
    /// **Enum-variant arms** — the scrutinee is a `Ty::Named` enum value
    /// (`is_aggregate()` now), fetched via `expr_ptr`; the variant tag is
    /// loaded from GEP field 0 and dispatched with a real LLVM `switch`
    /// over the declaration-order variant indices (`enum_variants` order,
    /// the same order `declare_named_type`/`construct_variant` use).
    /// `%default` is a single `unreachable` block — `typeck.rs`'s
    /// exhaustiveness check already guarantees every variant is covered,
    /// the same "typeck already proved this" trust the interpreter's own
    /// `.expect(...)` at this exact point already relies on. Each arm
    /// binds its payload fields into fresh stack slots (mirroring the
    /// function-prologue param-binding pattern), evaluates its body into
    /// the shared result slot, and `br`s to the merge block.
    ///
    /// **Literal-pattern arms** — a `str`/`i64`/`bool` scrutinee (never
    /// an enum). `i64`/`bool` use a real LLVM `switch` (native integer
    /// dispatch, `bool` widened to `i64` for a uniform switch type);
    /// `str` has no native switch, so codegen emits a sequential
    /// `nir_str_eq`-then-`br` chain per arm (first match wins, falling
    /// through to the next), ending at the mandatory trailing `_` arm's
    /// block unconditionally — `typeck.rs::check_literal_match` already
    /// guarantees exactly one trailing wildcard and no duplicate literal
    /// arms.
    ///
    /// An aggregate-*result* `match` (an arm body producing a struct/
    /// enum/`Vector`/`Matrix`) is deliberately out of scope, the same
    /// pre-existing gap `expr_ptr`'s own `_ => unsupported(...)` already
    /// covers for `if`: it fails cleanly via `expr_ptr_expected`'s
    /// `expr_ptr` fallback rather than being silently absent. The
    /// overwhelmingly common real case — a scalar result (`area()`'s
    /// `f64`, an `Option`-unwrap's `i64`, a `str`-dispatch's `str`) — goes
    /// through `if_expr`'s already-proven slot/merge mechanism unchanged.
    pub(super) fn match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        span: Span,
        expected: Option<&Ty>,
        scopes: &mut Scopes,
    ) -> Result<String, CodegenError> {
        let scrutinee_ty = self.local_ty_of(scrutinee, scopes);
        // `expected` -- when a caller one frame up (`expr_ptr_expected`)
        // already has a concrete type in hand -- is authoritative and
        // used directly, no inference needed at all; `match_result_ty`
        // (see its own doc comment, and `arm_body_ty`'s for the
        // binding-visibility fix it applies per arm) is only a fallback
        // for when this `match` is reached with no such context (a bare
        // statement, or nested inside a scalar `expr()` dispatch).
        let result_ty = expected.cloned().unwrap_or_else(|| self.match_result_ty(&scrutinee_ty, arms, scopes));
        let merge_label = self.fresh_label("match_merge");
        let slot = if result_ty == Ty::Unit {
            None
        } else {
            let llty = self.llvm_ty(&result_ty)?;
            let ptr = self.fresh_reg("match_result.addr");
            self.emit_alloca(&ptr, &llty);
            Some((ptr, llty))
        };
        let is_aggregate = result_ty.is_aggregate();

        match &scrutinee_ty {
            Ty::Named(enum_name, type_args) if self.registry.is_enum(enum_name) => {
                self.match_enum(scrutinee, enum_name, type_args, arms, &result_ty, slot.as_ref(), &merge_label, span, scopes)?
            }
            Ty::Str | Ty::I64 | Ty::Bool => {
                self.match_literal(scrutinee, &scrutinee_ty, arms, &result_ty, slot.as_ref(), &merge_label, span, scopes)?
            }
            _ => unreachable!("typeck.rs already restricted a match scrutinee to an enum or str/i64/bool"),
        }

        writeln!(self.out, "{merge_label}:").unwrap();
        // The merge block is only reachable if at least one arm falls
        // through to it; if every arm unconditionally `return`s, the
        // match itself is a definite-return and `terminated` reflects
        // that so a caller doesn't read a never-produced value.
        if self.terminated {
            writeln!(self.out, "  unreachable").unwrap();
            return Ok("0".to_string());
        }
        match slot {
            Some((ptr, llty)) => {
                if is_aggregate {
                    Ok(ptr)
                } else {
                    let out = self.fresh_reg("match_val");
                    writeln!(self.out, "  {out} = load {llty}, ptr {ptr}").unwrap();
                    Ok(out)
                }
            }
            None => Ok("0".to_string()),
        }
    }


    /// The enum-variant-arms half of `match_expr` — loads the tag word
    /// and `switch`es on it, one case per arm in declaration order. See
    /// `match_expr`'s doc for the overall shape.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn match_enum(
        &mut self,
        scrutinee: &Expr,
        enum_name: &str,
        type_args: &[Ty],
        arms: &[MatchArm],
        result_ty: &Ty,
        slot: Option<&(String, String)>,
        merge_label: &str,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        let scrutinee_ptr = self.expr_ptr(scrutinee, scopes)?;
        let enum_ty = Ty::Named(enum_name.to_string(), type_args.to_vec());
        let enum_llty = self.llvm_ty(&enum_ty)?;
        let tag_ptr = self.fresh_reg("tag.addr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {enum_llty}, ptr {scrutinee_ptr}, i32 0, i32 0").unwrap();
        let tag = self.fresh_reg("tag");
        writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
        let payload = self.fresh_reg("payload.addr");
        writeln!(self.out, "  {payload} = getelementptr inbounds {enum_llty}, ptr {scrutinee_ptr}, i32 0, i32 1").unwrap();

        let variants = self.registry.enum_variants(enum_name).expect("typeck.rs proved this is an enum");
        let type_params = self.registry.enum_type_params(enum_name).unwrap_or(&[]);
        let subst = zip_type_params(type_params, type_args);

        let default_label = self.fresh_label("match_default");
        // Build the `switch` cases: one per arm, keyed by the variant's
        // declaration-order index. `typeck.rs`'s exhaustiveness check
        // guarantees every variant is covered exactly once, so `default`
        // is genuinely unreachable.
        let mut cases: Vec<String> = Vec::new();
        let mut arm_labels: Vec<String> = Vec::new();
        for arm in arms {
            let vidx = variants.iter().position(|v| v.name == arm.variant).expect("typeck.rs proved every match arm names a declared variant");
            let label = self.fresh_label("match_arm");
            cases.push(format!("i64 {vidx}, label %{label}"));
            arm_labels.push(label);
        }
        writeln!(self.out, "  switch i64 {tag}, label %{default_label} [").unwrap();
        for c in &cases {
            writeln!(self.out, "    {c}").unwrap();
        }
        writeln!(self.out, "  ]").unwrap();
        // The unreachable default — exhaustiveness is typeck's
        // guarantee, the same trust the interpreter's `.expect` relies on.
        writeln!(self.out, "{default_label}:").unwrap();
        writeln!(self.out, "  unreachable").unwrap();

        let mut any_fell_through = false;
        for (arm_idx, (arm, arm_label)) in arms.iter().zip(arm_labels.iter()).enumerate() {
            writeln!(self.out, "{arm_label}:").unwrap();
            self.terminated = false;
            scopes.push();
            let variant = variants.iter().find(|v| v.name == arm.variant).expect("just proved this variant exists");
            let mut word_off: u64 = 0;
            for (name, decl_ty) in arm.bindings.iter().zip(variant.payload.iter()) {
                let field_ty = substitute_ty(decl_ty, &subst);
                // A `Ty::Unit` payload (e.g. `mq_publish`'s own
                // `Result(unit, str)`) carries no data at all —
                // `llvm_ty(Unit)` is `void`, and `alloca void`/`load
                // void` are both invalid LLVM IR, so this skips the
                // whole alloca/load/store dance every other field type
                // gets below. The placeholder `"0"` is never actually
                // dereferenced as a pointer — `Expr::Ident`'s own
                // `Ty::Unit` short-circuit returns it directly instead
                // of loading through it, the same "its own value is
                // unit; never [meaningfully] read" convention every
                // other unit-shaped result in this file already uses.
                // Found by actually compiling `Ok(u) => true` against a
                // real `Result(unit, str)`, not designed in advance.
                if field_ty == Ty::Unit {
                    scopes.define(name, Ty::Unit, "0".to_string());
                    word_off += conservative_word_count(&field_ty, &self.registry);
                    continue;
                }
                let field_ptr = self.fresh_reg("armfield.addr");
                writeln!(self.out, "  {field_ptr} = getelementptr inbounds i64, ptr {payload}, i64 {word_off}").unwrap();
                // Every binding gets its own stack slot — the same
                // "even a scalar gets an alloca" convention `function`'s
                // param-binding uses — so a later reassignment inside the
                // arm body has real storage to store into.
                let slot_llty = self.llvm_ty(&field_ty)?;
                let slot_ptr = self.fresh_reg(&format!("{name}.addr"));
                self.emit_alloca(&slot_ptr, &slot_llty);
                if field_ty.is_aggregate() {
                    let bytes = agg_byte_size_operand(&field_ty, &self.registry);
                    writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {slot_ptr}, ptr {field_ptr}, i64 {bytes}, i1 false)").unwrap();
                } else {
                    let loaded = self.fresh_reg(&format!("{name}.val"));
                    writeln!(self.out, "  {loaded} = load {slot_llty}, ptr {field_ptr}").unwrap();
                    let loaded = self.widen_to_i64(&loaded, &field_ty);
                    let stored = if field_ty.is_integer() { self.narrow_from_i64(&loaded, &field_ty)? } else { loaded };
                    writeln!(self.out, "  store {slot_llty} {stored}, ptr {slot_ptr}").unwrap();
                }
                scopes.define(name, field_ty.clone(), slot_ptr);
                word_off += conservative_word_count(&field_ty, &self.registry);
            }
            let fell_through = if result_ty.is_aggregate() {
                // `expr_ptr_expected`, not plain `expr_ptr` — this arm's
                // body already has a real, concrete expected type in
                // hand (`result_ty`, already resolved from `arms[0]`
                // above), so a bare `Err(SomeVariant(...))`-shaped body
                // (which `ctor_ty`'s own no-context inference can't
                // disambiguate — which `Result(T, E)` instantiation does
                // this `Err` belong to?) resolves correctly instead of
                // hitting that ambiguity error. Found by actually
                // compiling a `workflow`-generated `Err(NoSuchTransition())`
                // arm, not designed in advance — the same "found by
                // testing" precedent this file's own `Ty::Unit`-payload
                // fix (just above, in this same per-arm loop) already
                // set.
                let src = self.expr_ptr_expected(&arm.body, result_ty, scopes)?;
                if let Some((ptr, _)) = slot {
                    let bytes = agg_byte_size_operand(result_ty, &self.registry);
                    writeln!(
                        self.out,
                        "  call void @llvm.memcpy.p0.p0.i64(ptr {ptr}, ptr {src}, i64 {bytes}, i1 false)"
                    )
                    .unwrap();
                }
                !self.terminated
            } else {
                let body_val = self.expr(&arm.body, scopes)?;
                if !self.terminated {
                    if let Some((ptr, llty)) = slot {
                        let stored = if result_ty.is_integer() { self.narrow_from_i64(&body_val, result_ty)? } else { body_val.clone() };
                        writeln!(self.out, "  store {llty} {stored}, ptr {ptr}").unwrap();
                    }
                }
                !self.terminated
            };
            // Affine payload bindings declared inside this arm are freed
            // here, before we leave the arm's scope; the match-arm-end
            // FreeMap field is populated in Part 2.
            if !self.terminated {
                if let Some(names) = self.free_map.at_match_arm_end.get(&(span, arm_idx)).cloned() {
                    self.emit_frees_for_names(&names, scopes);
                }
            }
            scopes.pop();
            if fell_through {
                writeln!(self.out, "  br label %{merge_label}").unwrap();
                any_fell_through = true;
            }
        }
        self.terminated = !any_fell_through;
        Ok(())
    }


    /// The literal-pattern-arms half of `match_expr` — a `str`/`i64`/
    /// `bool` scrutinee matched against literal-value arms plus a
    /// mandatory trailing `_`. `i64`/`bool` use a real LLVM `switch`;
    /// `str` uses a sequential `nir_str_eq`-then-`br` chain (no native
    /// string switch). See `match_expr`'s doc for the overall shape.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn match_literal(
        &mut self,
        scrutinee: &Expr,
        scrutinee_ty: &Ty,
        arms: &[MatchArm],
        result_ty: &Ty,
        slot: Option<&(String, String)>,
        merge_label: &str,
        span: Span,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        // `typeck.rs::check_literal_match` guarantees the last arm is the
        // `_` wildcard and no earlier arm duplicates a literal.
        let (literal_arms, wildcard) = arms.split_at(arms.len() - 1);
        let wildcard = &wildcard[0];

        // Emit one arm's body into the shared result slot, then `br` to
        // the merge block (unless the body itself terminated). Shared by
        // every arm across both the int/bool `switch` and the `str` chain.
        // `scopes` is left untouched here (no payload bindings exist for
        // a literal arm — `check_literal_match` keeps `bindings` empty);
        // the caller wraps its own `scopes.push()`/`pop()` where needed.
        let emit_arm_body = |cg: &mut Codegen<'_>,
                             arm_idx: usize,
                             body: &Expr,
                             scopes: &mut Scopes|
         -> Result<bool, CodegenError> {
            if result_ty.is_aggregate() {
                // `expr_ptr_expected`, not plain `expr_ptr` — same fix,
                // same reason, as `match_enum`'s own per-arm body
                // evaluation just above.
                let src = cg.expr_ptr_expected(body, result_ty, scopes)?;
                if !cg.terminated {
                    if let Some((ptr, _)) = slot {
                        let bytes = agg_byte_size_operand(result_ty, &cg.registry);
                        writeln!(
                            cg.out,
                            "  call void @llvm.memcpy.p0.p0.i64(ptr {ptr}, ptr {src}, i64 {bytes}, i1 false)"
                        )
                        .unwrap();
                    }
                }
            } else {
                let body_val = cg.expr(body, scopes)?;
                if !cg.terminated {
                    if let Some((ptr, llty)) = slot {
                        let stored = if result_ty.is_integer() { cg.narrow_from_i64(&body_val, result_ty)? } else { body_val.clone() };
                        writeln!(cg.out, "  store {llty} {stored}, ptr {ptr}").unwrap();
                    }
                }
            }
            if !cg.terminated {
                if let Some(names) = cg.free_map.at_match_arm_end.get(&(span, arm_idx)).cloned() {
                    cg.emit_frees_for_names(&names, scopes);
                }
                writeln!(cg.out, "  br label %{merge_label}").unwrap();
            }
            Ok(!cg.terminated)
        };

        let mut any_fell_through = false;

        match scrutinee_ty {
            Ty::I64 | Ty::Bool => {
                let raw = self.expr(scrutinee, scopes)?;
                let tag = if *scrutinee_ty == Ty::Bool {
                    let t = self.fresh_reg("match_bool");
                    writeln!(self.out, "  {t} = zext i1 {raw} to i64").unwrap();
                    t
                } else {
                    raw
                };
                let arm_labels: Vec<String> = literal_arms.iter().map(|_| self.fresh_label("match_arm")).collect();
                let default_label = self.fresh_label("match_default");
                let mut cases: Vec<String> = Vec::new();
                for (a, label) in literal_arms.iter().zip(arm_labels.iter()) {
                    let lit = match a.pattern.as_ref().expect("literal arm always has a pattern") {
                        LiteralPattern::Int(n) => format!("i64 {n}"),
                        LiteralPattern::Bool(b) => format!("i64 {}", if *b { 1 } else { 0 }),
                        LiteralPattern::Wildcard | LiteralPattern::Str(_) => {
                            unreachable!("check_literal_match keeps int/bool arms int/bool-typed")
                        }
                    };
                    cases.push(format!("{lit}, label %{label}"));
                }
                writeln!(self.out, "  switch i64 {tag}, label %{default_label} [").unwrap();
                for c in &cases {
                    writeln!(self.out, "    {c}").unwrap();
                }
                writeln!(self.out, "  ]").unwrap();
                // The default is the wildcard arm — every non-listed
                // value falls through to it, exactly the `_` semantics.
                for (arm_idx, (a, label)) in literal_arms.iter().zip(arm_labels.iter()).enumerate() {
                    writeln!(self.out, "{label}:").unwrap();
                    self.terminated = false;
                    let fell = emit_arm_body(self, arm_idx, &a.body, scopes)?;
                    any_fell_through |= fell;
                }
                writeln!(self.out, "{default_label}:").unwrap();
                self.terminated = false;
                let wildcard_idx = literal_arms.len();
                let fell = emit_arm_body(self, wildcard_idx, &wildcard.body, scopes)?;
                any_fell_through |= fell;
            }
            Ty::Str => {
                // No native string switch — a sequential `nir_str_eq`-
                // then-`br` chain, first match wins, falling through to
                // the next comparison; after the last literal arm, the
                // fall-through goes straight to the wildcard arm.
                let s = self.expr(scrutinee, scopes)?;
                let s_ptr = self.fresh_reg("match_str_ptr");
                writeln!(self.out, "  {s_ptr} = extractvalue {{ptr, i64}} {s}, 0").unwrap();
                let s_len = self.fresh_reg("match_str_len");
                writeln!(self.out, "  {s_len} = extractvalue {{ptr, i64}} {s}, 1").unwrap();

                let arm_labels: Vec<String> = literal_arms.iter().map(|_| self.fresh_label("match_arm")).collect();
                let wildcard_label = self.fresh_label("match_arm");
                // `cmp_label[0]` is the first comparison's block; the
                // initial `br` targets it. Each comparison falls through
                // to the next comparison's block (or the wildcard's, after
                // the last literal arm).
                let cmp_labels: Vec<String> = literal_arms.iter().map(|_| self.fresh_label("match_str_cmp")).collect();
                let fallthrough_targets: Vec<String> = cmp_labels
                    .iter()
                    .skip(1)
                    .cloned()
                    .chain(std::iter::once(wildcard_label.clone()))
                    .collect();

                writeln!(self.out, "  br label %{cmp0}", cmp0 = cmp_labels[0]).unwrap();
                for (i, (a, arm_label)) in literal_arms.iter().zip(arm_labels.iter()).enumerate() {
                    let LiteralPattern::Str(lit) = a.pattern.as_ref().expect("str arm") else { unreachable!() };
                    let cmp_label = &cmp_labels[i];
                    let next_label = &fallthrough_targets[i];
                    writeln!(self.out, "{cmp_label}:").unwrap();
                    self.terminated = false;
                    // The literal's backing global — same emission shape
                    // `Expr::Str` uses, but the literal lives in `a.pattern`,
                    // not an `Expr::Str`, so emit it inline here.
                    let global = self.fresh_global("str");
                    let bytes = lit.as_bytes();
                    let escaped = llvm_escape_bytes(bytes);
                    writeln!(self.string_globals, "{global} = private unnamed_addr constant [{} x i8] c\"{escaped}\\00\"", bytes.len() + 1).unwrap();
                    let lit_ptr = self.fresh_reg("match_lit_ptr");
                    writeln!(self.out, "  {lit_ptr} = getelementptr [{} x i8], ptr {global}, i64 0, i64 0", bytes.len() + 1).unwrap();
                    let lit_len = bytes.len() as i64;
                    let raw = self.fresh_reg("match_streq");
                    writeln!(self.out, "  {raw} = call i32 @nir_str_eq(ptr {s_ptr}, i64 {s_len}, ptr {lit_ptr}, i64 {lit_len})").unwrap();
                    let is_eq = self.icmp("ne", "i32", &raw, "0")?;
                    writeln!(self.out, "  br i1 {is_eq}, label %{arm_label}, label %{next_label}").unwrap();
                    // The arm body block.
                    writeln!(self.out, "{arm_label}:").unwrap();
                    self.terminated = false;
                    let fell = emit_arm_body(self, i, &a.body, scopes)?;
                    any_fell_through |= fell;
                }
                // Wildcard arm — the unconditional catch-all.
                writeln!(self.out, "{wildcard_label}:").unwrap();
                self.terminated = false;
                let wildcard_idx = literal_arms.len();
                let fell = emit_arm_body(self, wildcard_idx, &wildcard.body, scopes)?;
                any_fell_through |= fell;
            }
            _ => unreachable!("caller already restricted scrutinee_ty to str/i64/bool"),
        }
        self.terminated = !any_fell_through;
        Ok(())
    }


    /// Run `block` for side effects only and return whether it falls
    /// through (so the caller knows whether to emit a branch to the merge
    /// label). Used by unit-valued `if`/`match` arms.
    pub(super) fn block_side_effects(&mut self, block: &Block, scopes: &mut Scopes) -> Result<(), CodegenError> {
        match block.stmts.split_last() {
            None => {}
            Some((last, rest)) => {
                self.stmts(rest, scopes)?;
                if !self.terminated {
                    match last {
                        Stmt::Expr(e) => {
                            self.expr(e, scopes)?;
                        }
                        other => {
                            self.stmt(other, scopes)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }


    /// Evaluate `block` and store its trailing value into `slot_ptr`.
    /// Used by `if_expr`/`match_expr` for both scalar and aggregate result
    /// slots.
    pub(super) fn block_value_to_slot(
        &mut self,
        block: &Block,
        slot_ptr: &str,
        result_ty: &Ty,
        scopes: &mut Scopes,
    ) -> Result<(), CodegenError> {
        match block.stmts.split_last() {
            None => Ok(()),
            Some((last, rest)) => {
                self.stmts(rest, scopes)?;
                if self.terminated {
                    return Ok(());
                }
                match last {
                    Stmt::Expr(e) => {
                        if result_ty.is_aggregate() {
                            // `expr_ptr_expected`, not plain `expr_ptr` --
                            // mirrors `match_enum`'s own per-arm body
                            // compilation (see its own comment): this
                            // branch's trailing expression already has a
                            // real, concrete expected type in hand
                            // (`result_ty`), so a bare `Err(SomeVariant
                            // (...))`-shaped branch value (which `ctor_ty`'s
                            // own no-context inference can't disambiguate)
                            // resolves correctly instead of hitting that
                            // ambiguity error -- `match_enum` already
                            // needed this exact fix for its own arms; `if`/
                            // `else` branches (this function) share the
                            // identical shape and had been missed.
                            let src = self.expr_ptr_expected(e, result_ty, scopes)?;
                            let bytes = agg_byte_size_operand(result_ty, &self.registry);
                            writeln!(
                                self.out,
                                "  call void @llvm.memcpy.p0.p0.i64(ptr {slot_ptr}, ptr {src}, i64 {bytes}, i1 false)"
                            )
                            .unwrap();
                        } else {
                            let v = self.expr(e, scopes)?;
                            let llty = self.llvm_ty(result_ty)?;
                            writeln!(self.out, "  store {llty} {v}, ptr {slot_ptr}").unwrap();
                        }
                    }
                    other => {
                        self.stmt(other, scopes)?;
                    }
                }
                Ok(())
            }
        }
    }


}
