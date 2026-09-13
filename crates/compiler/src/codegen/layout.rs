use super::*;

/// Escapes raw bytes into LLVM's `c"..."` string-constant syntax:
/// printable ASCII passes through unchanged except `"`/`\` (which would
/// otherwise terminate/escape the constant early), everything else
/// becomes a `\XX` two-hex-digit byte escape — the same scheme LLVM's own
/// IR parser expects, used here since a `str` literal's already-escape-
/// resolved bytes (the lexer/parser already turned `\n`/`\t`/etc. into
/// real bytes — see `Expr::Str`'s doc) can contain anything, not just the
/// hand-picked printable text `@.int_fmt`/`@.float_fmt` use.
/// LLVM's own hex-float literal format for a `double` constant operand —
/// factored out of `Expr::Float`'s own codegen (see that arm's doc
/// comment for why this exact bit-pattern format, not a plain decimal
/// literal, is the only representation guaranteed to round-trip).
pub(super) fn llvm_f64_literal(f: f64) -> String {
    format!("0x{:016X}", f.to_bits())
}


pub(super) fn llvm_escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'"' => out.push_str("\\22"),
            b'\\' => out.push_str("\\5C"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("\\{b:02X}")),
        }
    }
    out
}


/// Every signed integer/bool/unit type maps to a fixed LLVM type name;
/// `Vector`/`Matrix` map to a flat array type (`[N x double]`, or
/// `[R*C x double]` row-major for a `Matrix` — matching
/// `interpreter.rs`'s `Value::Matrix` storage exactly); a non-affine
/// `struct`/`enum` instantiation (`Ty::Named`) maps to a real named LLVM
/// type (`declare_named_type`'s doc — this function only ever returns
/// its *name*, `%Point`/`%Result$i64$str`; the actual `%Name = type
/// {...}` declaration is a separate, `&mut self`-requiring step, see
/// `Codegen::llvm_ty`/`Codegen::declare_named_type`); everything else is
/// rejected — see module doc for exactly what and why. Returns an owned
/// `String`, not `&'static str`, because an aggregate's type string
/// depends on its compile-time-known but not statically-fixed length.
///
/// True iff every affine value reachable inside `ty` can be torn down
/// by the current codegen runtime. Only `box` (via `nir_free`) and
/// `tcp`/`tcp_listener` (via `nir_tcp_stop`) are supported today; any
/// other affine leaf (`thread`, `sandbox`, `file`, `db`, `mq`) or a
/// struct/enum containing one keeps the whole type rejected.
pub(super) fn affine_codegen_supported(registry: &TypeRegistry, ty: &Ty) -> bool {
    let mut visiting = Vec::new();
    affine_codegen_supported_visiting(registry, ty, &mut visiting)
}


/// `affine_codegen_supported`'s real recursion, guarded the same
/// "track names on the current path, a repeat is the cycle" way
/// `TypeRegistry::is_affine_visiting` is (`ast.rs`). Needed once
/// `check_supported`'s `has_cyclic_layout` guard started letting a
/// genuinely-finite `box`-indirected self-reference through (a
/// `struct Node { next: box Node }` cons-list shape, same as any real
/// language's) — that recursion crosses back into `Ty::Named` through
/// the `Ty::Box` arm below with no size limit of its own, so without a
/// `visiting` set it walks `Node -> box Node -> Node -> ...` forever,
/// a real stack overflow confirmed empirically the same way
/// `is_affine`'s own cycle bug was. A repeat on the path is sound to
/// treat as "supported" here (not "unsupported", unlike
/// `is_affine_visiting`'s `false`): the cycle is only reachable at all
/// because every step across it was a `box` — the one affine leaf this
/// function already tears down via `nir_free` — so a second pass over
/// the same name can only re-confirm what the first pass already
/// found, never turn up a new unsupported leaf.
pub(super) fn affine_codegen_supported_visiting(registry: &TypeRegistry, ty: &Ty, visiting: &mut Vec<String>) -> bool {
    if !registry.is_affine(ty) {
        return true;
    }
    match ty {
        Ty::Box(inner) => affine_codegen_supported_visiting(registry, inner, visiting),
        Ty::Tcp | Ty::TcpListener => true,
        // `db_connect`'s own `Result(db, str)` is the first affine handle
        // nested inside a prelude enum's payload (`File`/`Thread`/
        // `Channel` are all returned bare, never `Result`-wrapped, so
        // this arm was never needed until now) — a `db` handle is one
        // opaque `i64` either way, same as `Tcp`/`TcpListener` above.
        Ty::Db => true,
        // Same reason as `Ty::Db` just above — `mq_connect`'s own
        // `Result(mq, str)`.
        Ty::Mq => true,
        Ty::Named(name, args) => {
            if visiting.iter().any(|v| v == name.as_str()) {
                return true;
            }
            if let Some(fields) = registry.struct_fields(name) {
                let type_params = registry.struct_type_params(name).unwrap_or(&[]);
                let subst = zip_type_params(type_params, args);
                visiting.push(name.clone());
                let result = fields
                    .iter()
                    .all(|f| affine_codegen_supported_visiting(registry, &substitute_ty(&f.ty, &subst), visiting));
                visiting.pop();
                result
            } else if let Some(variants) = registry.enum_variants(name) {
                let type_params = registry.enum_type_params(name).unwrap_or(&[]);
                let subst = zip_type_params(type_params, args);
                visiting.push(name.clone());
                let result = variants.iter().all(|v| {
                    v.payload
                        .iter()
                        .all(|t| affine_codegen_supported_visiting(registry, &substitute_ty(t, &subst), visiting))
                });
                visiting.pop();
                result
            } else {
                false
            }
        }
        _ => false,
    }
}


/// Pure and stateless on purpose — no declaration emission, no mutable
/// state — so it's cheaply callable both from `check_supported`'s
/// pre-pass (which runs before any `Codegen` exists) and from every real
/// codegen call site (via `Codegen::llvm_ty`, the `&mut self` wrapper
/// that also ensures the real declaration gets emitted for a
/// `Ty::Named`).
pub(super) fn llvm_ty(ty: &Ty, registry: &TypeRegistry) -> Result<String, CodegenError> {
    match ty {
        Ty::I8 => Ok("i8".to_string()),
        Ty::I16 => Ok("i16".to_string()),
        Ty::I32 => Ok("i32".to_string()),
        Ty::I64 => Ok("i64".to_string()),
        Ty::Bool => Ok("i1".to_string()),
        Ty::Unit => Ok("void".to_string()),
        // Same bit widths as their signed counterparts -- LLVM has no
        // separate unsigned integer *type*, only separate *instructions*
        // for the operations that actually care (`icmp`/`div`/`rem`, and
        // widening a narrower value up to i64). See
        // `Codegen::widen_to_i64`'s doc comment for why that's the one
        // and only place this backend needs the signed-vs-unsigned
        // choice: every intermediate value is computed at `i64` width
        // (module doc), and `Ty::bounds()` already caps every unsigned
        // type's legal range at `[0, i64::MAX]` — so once a value is
        // correctly widened (`zext`, not `sext`), plain/signed `+`/`-`/
        // `*`/`icmp`/`div` at `i64` width give byte-identical results to
        // their unsigned counterparts (both interpretations agree on any
        // bit pattern whose sign bit is clear, which a validly-widened
        // unsigned value's always is) -- confirmed by actually compiling
        // and running comparison/division/boundary-value programs for
        // every one of `u8`/`u16`/`u32`/`u64`/`usize`, not just reasoned
        // about.
        Ty::U8 => Ok("i8".to_string()),
        Ty::U16 => Ok("i16".to_string()),
        Ty::U32 => Ok("i32".to_string()),
        Ty::U64 | Ty::Usize => Ok("i64".to_string()),
        // A single heap (`Box`) or borrowed (`Ref`) pointer — one word,
        // passed by value exactly like `Ty::Str`'s `{ptr, i64}` above,
        // just narrower. `Ref` needs no allocation of its own (it's
        // always just an existing binding's own storage address —
        // `Codegen::expr`'s `Expr::Ref` arm). `Box` allocates on the heap
        // at construction (`nir_alloc`, `Expr::Box`'s own codegen) and is
        // freed for real: `ownership.rs`'s `FreeMap` records each
        // binding's last use, and `emit_frees_for_names`/`emit_affine_free`
        // emit `nir_free` for it at every scope-closing point that data
        // lists — confirmed in generated IR (`nirdosha emit-llvm`) for a
        // simple `let b: box i64 = ...` case: the `nir_free` call is
        // really there, right after `b`'s last use.
        // `Ty::Froze` is exactly `Ty::Box`'s own representation, one
        // heap pointer — see `Ty::Froze`'s own doc comment for why
        // there's no extra runtime shape (leaked, not refcounted, for
        // now) to encode here.
        Ty::Box(_) | Ty::Froze(_) | Ty::Ref(_) => Ok("ptr".to_string()),
        // A spawned computation's handle — one opaque `i64`, exactly like
        // `Ty::Tcp`/`Ty::File` above: everything the handle needs (the
        // dedicated `Scope` a `spawn` call site created, the result word
        // a `join` reads back) lives in `runtime-kernels`' own
        // `HandleTable`, not in this value. `nir_thread_spawn`/
        // `nir_thread_join` (`runtime-kernels/src/lib.rs`) are the
        // backend `spawn`/`join` compile to — see `Codegen::expr`'s
        // `Expr::Spawn`/`Expr::Join` arms for the real codegen and their
        // doc comments for the disclosed narrower scope (word-sized
        // arguments/results only, for now).
        Ty::Thread(_) => Ok("i64".to_string()),
        // A channel handle — same "opaque `i64` into a kernel-owned
        // table" story as `Ty::Thread` just above. `nir_chan_new`/
        // `nir_chan_send`/`nir_chan_recv` are the backend `chan`/`send`/
        // `recv` compile to for a `Ty::Channel` operand (`Codegen::expr`'s
        // `Expr::Chan`/`Expr::Send`/`Expr::Recv` arms).
        Ty::Channel(_) => Ok("i64".to_string()),
        Ty::Sandbox => unsupported(
            "codegen doesn't support `sandbox` yet — sandbox/stop are interpreter-only for now",
        ),
        // A file descriptor/handle, exactly like `Ty::Tcp`/`Ty::TcpListener`
        // above — `nir_file_*` (`runtime-kernels/src/lib.rs`) is the linked-kernel
        // backend `open`/`send`/`recv`/`stop` on a `file` compile to.
        Ty::File => Ok("i64".to_string()),
        // A plain two-word value, matching `rust_decimal::Decimal::
        // serialize()`'s own 16-byte, little-endian round trip
        // (`runtime-kernels/src/lib.rs`'s `Dec128Bits`) split at the
        // midpoint. Not an aggregate (`Ty::is_aggregate()` deliberately
        // excludes `Dec128` — `transact_log.rs`'s own slot-eligibility
        // check already depends on that), so `dec128` values pass/return
        // through `expr()`/`call_args`/`call()`'s ordinary *value* paths
        // exactly the way `Ty::Str`'s own `{ptr, i64}` two-word value
        // already does, never through the pointer-based aggregate path
        // Vector/Matrix use.
        Ty::Dec128 => Ok("{i64, i64}".to_string()),
        // A `json` value compiles as the same `{ptr, i64}` two-word
        // value `Ty::Str` already is — the raw JSON text itself, not a
        // persisted parsed-tree handle. `json_get`/`json_array_get`/etc.
        // (`Codegen::call_ptr`'s dispatch) re-parse that text via
        // `nir_json_*` (`runtime-kernels/src/lib.rs`) on every call — the
        // simplest thing that reuses an existing representation with
        // zero new runtime value type, at the cost of re-parsing instead
        // of a persisted tree. Disclosed, not hidden — see `Ty::Json`'s
        // own doc comment and `nir_db_query`'s.
        Ty::Json => Ok("{ptr, i64}".to_string()),
        // A `db` connection handle — one opaque `i64` into
        // `runtime-kernels`' own `HandleTable<rusqlite::Connection>`
        // (`db_table()`, `lib.rs`'s "db kernels" section), same "the
        // handle itself is just a word, the real resource lives in a
        // kernel-owned table" shape `Ty::Thread`/`Ty::Channel` above
        // already use — a `rusqlite::Connection` isn't reconstructible
        // from a bare integer the way a `tcp`/`file` fd is, so a table
        // (not a raw fd) backs this one.
        Ty::Db => Ok("i64".to_string()),
        // rfcs/0008-native-plugin-abi-widening.md Phase 1: a plugin-held
        // resource id, exactly like `Ty::Thread`/`Ty::Channel`/`Ty::File`
        // just above — one opaque `i64` into a table this compiler never
        // looks inside (here, a table the *plugin's own* Rust code owns,
        // not a `runtime-kernels` one). All of its safety comes from
        // `ownership.rs`'s affine tracking at the type level (`Ty::
        // is_affine()` already lists `Handle(_)`, rfcs/0005 §1); codegen
        // itself just needs to pass the word through, identically to how
        // it already treats a spawn/channel/file handle.
        Ty::Handle(_) => Ok("i64".to_string()),
        // A `mq` (Redis) connection handle — same "opaque `i64` into a
        // `HandleTable`" shape as `Ty::Db` just above (`mq_table()`,
        // `lib.rs`'s "mq kernels" section).
        Ty::Mq => Ok("i64".to_string()),
        // A fixed-size, two-word value — pointer to the byte data plus an
        // explicit `i64` length, never NUL-terminated-only (a `str`'s
        // bytes are whatever the source literal's escapes resolved to,
        // and a future `tcp` payload needs to carry arbitrary bytes that
        // could contain an embedded NUL). Passed by value in registers,
        // like `f64`/`bool` — NOT `is_aggregate()` (that's the
        // sret/pointer convention Vector/Matrix need because their size
        // varies per-type; a `str` value is always these same two words,
        // so it needs no allocation of its own to pass around).
        Ty::Str => Ok("{ptr, i64}".to_string()),
        // Phase 4: `f64` maps directly to LLVM's `double`. No width
        // story the way integers have one (`is_integer()` is false for
        // `F64`, so `guard_in_range`/`narrow_from_i64`/`widen_to_i64`
        // all already treat it as a no-guard, no-narrow passthrough --
        // see each of their doc comments), and no range check either:
        // IEEE 754 saturates instead of trapping, the same semantics
        // the interpreter already committed to (`Value::Float`'s doc
        // comment).
        Ty::F64 => Ok("double".to_string()),
        // `Vector`/`Matrix` land as of this phase (Phase 0+1 of the
        // Vector/Matrix codegen plan): a flat array type, never a single
        // SSA register the way every other `Ty` here is -- see the
        // module doc and `Codegen::expr_ptr`'s doc comment for the
        // pointer-based codegen strategy this actually requires. Element
        // count is `n` for `Vector`, `r*c` (row-major) for `Matrix` --
        // `agg_elem_and_len` is the single place that flattening rule
        // lives, reused by every aggregate codegen path so it can never
        // silently disagree with `interpreter.rs`'s own flattening.
        Ty::Vector(_, _) | Ty::Matrix(_, _, _) => {
            let (elem, len) = agg_elem_and_len(ty);
            let elem_llty = llvm_ty(elem, registry)?;
            Ok(format!("[{len} x {elem_llty}]"))
        }
        // Phase B1 (str/tcp codegen plan): a `tcp`/`tcp_listener` handle
        // is just a raw OS file descriptor — the kernel already tracks
        // everything a "handle" needs, so no separate handle table is
        // needed the way `Value::Tcp`'s `Arc<Mutex<Option<..>>>` wrapper
        // gives the interpreter. Both lower to the same `i64` regardless
        // of which one they are (`nir_tcp_stop` closes either uniformly).
        Ty::Tcp | Ty::TcpListener => Ok("i64".to_string()),
        // Row 11: a `struct`/`enum` instantiation lowers to a real named
        // LLVM type — `%Point`, or `%Result$i64$str` for a generic
        // instantiation (`mangle_ty` gives every distinct concrete
        // instantiation its own distinct name, since their layouts
        // genuinely differ). An affine-containing instantiation is allowed
        // as long as every affine leaf inside it is one this backend can
        // actually tear down today (`box` via `nir_free`, `tcp`/
        // `tcp_listener` via `nir_tcp_stop`); if it contains an unsupported
        // affine leaf (`thread`, `sandbox`, `file`, `db`, `mq`, etc.), the
        // whole type is still rejected cleanly rather than mis-compiled.
        Ty::Named(name, _) => {
            if registry.is_affine(ty) && !affine_codegen_supported(registry, ty) {
                return unsupported(format!(
                    "codegen doesn't support `{name}` yet — it (transitively) contains an \
                     affine field of a type that has no native codegen yet (`thread`/`sandbox`/\
                     `file`/`db`/`mq`/etc.); a struct/enum whose only affine fields are `box` \
                     or `tcp` compiles now"
                ));
            }
            Ok(format!("%{}", mangle_ty(ty)))
        }
        // A first-class function value (ordinary or `acquire`d) is a
        // plain function-pointer word, freely copyable like any other
        // scalar (`Ty::is_affine`'s own doc comment on why `Ty::Fn` is
        // deliberately excluded there) — no separate handle table, no
        // refcounting, just `ptr`. Compiled for real, 2026-09: see
        // `Expr::Ident`'s and `Expr::Acquire`'s own codegen arms for how
        // a value of this type is actually produced, and `Codegen::call`/
        // `call_ptr`'s own local-variable check for how it's called.
        Ty::Fn(_, _) => Ok("ptr".to_string()),
        Ty::Error => unreachable!("a program with a type error is never handed to codegen"),
    }
}


pub(super) fn mangle_ty(ty: &Ty) -> String {
    match ty {
        Ty::I8 => "i8".to_string(),
        Ty::I16 => "i16".to_string(),
        Ty::I32 => "i32".to_string(),
        Ty::I64 => "i64".to_string(),
        Ty::U8 => "u8".to_string(),
        Ty::U16 => "u16".to_string(),
        Ty::U32 => "u32".to_string(),
        Ty::U64 => "u64".to_string(),
        Ty::Usize => "usize".to_string(),
        Ty::Bool => "bool".to_string(),
        Ty::Unit => "unit".to_string(),
        Ty::F64 => "f64".to_string(),
        Ty::Str => "str".to_string(),
        Ty::Vector(elem, n) => format!("vec{n}_{}", mangle_ty(elem)),
        Ty::Matrix(elem, r, c) => format!("mat{r}x{c}_{}", mangle_ty(elem)),
        Ty::Named(name, args) if args.is_empty() => name.clone(),
        Ty::Named(name, args) => {
            format!("{name}${}", args.iter().map(mangle_ty).collect::<Vec<_>>().join("$"))
        }
        // A real generic type argument since `acquire` started compiling
        // (2026-09): `acquire fn_name(proof)` evaluates to
        // `Result(Ty::Fn(params, ret), str)`, a genuinely new instantiation
        // per acquired signature. `Ty::name()`'s own `"fn(i64) -> i64"`
        // Display form contains `(`/`)`/`,`/` `/`->`, none of them legal
        // in an unquoted LLVM identifier — a dedicated arm instead of the
        // fallback below, which would leave the `-`/`>` from `->`
        // unescaped and produce unparseable IR.
        Ty::Fn(params, ret) => {
            format!("fn{}to{}", params.iter().map(|p| format!("_{}", mangle_ty(p))).collect::<String>(), mangle_ty(ret))
        }
        // Every other `Ty` (`Box`/`Ref`/`Thread`/`Channel`/`Sandbox`/
        // `Tcp`/`TcpListener`/`File`/`Json`/`Db`/`Mq`/`Error`) is
        // either affine (already rejected before this can run on one) or
        // otherwise never legally a struct/enum generic type argument —
        // this arm is a defensive fallback, not expected to actually run
        // for any program that reaches this point.
        _ => ty.name().replace(['(', ')', ',', ' '], "_"),
    }
}


/// A conservative, always-safe-to-over-allocate word count (8 bytes
/// each) for `ty` — used only to size an enum's raw `[N x i64]` payload
/// buffer (`declare_named_type`'s enum branch). Rounds every field up to
/// a whole 8-byte word rather than hand-replicating LLVM's exact
/// alignment rules, so the buffer is never undersized regardless of what
/// LLVM's real struct layout would have chosen for a synthesized
/// variant-payload type — a few wasted bytes in the rare case a variant
/// packs several sub-8-byte fields, in exchange for *never* under-sizing
/// the payload buffer, consistent with this codebase's repeatedly-stated
/// "correctness over cleverness" bias. Every field type this language
/// has needs at most 8-byte alignment, so this always produces a
/// correctly-aligned offset for every field (`declare_named_type`'s own
/// doc has the full reasoning).
pub(super) fn conservative_word_count(ty: &Ty, registry: &TypeRegistry) -> u64 {
    match ty {
        Ty::Str | Ty::Dec128 => 2,
        Ty::Vector(_, _) | Ty::Matrix(_, _, _) => {
            let (_, _, bytes) = agg_layout(ty);
            bytes.div_ceil(8)
        }
        Ty::Named(name, args) => {
            if let Some(fields) = registry.struct_fields(name) {
                let subst = zip_type_params(registry.struct_type_params(name).unwrap_or(&[]), args);
                fields.iter().map(|f| conservative_word_count(&substitute_ty(&f.ty, &subst), registry)).sum()
            } else if let Some(variants) = registry.enum_variants(name) {
                let subst = zip_type_params(registry.enum_type_params(name).unwrap_or(&[]), args);
                1 + variants
                    .iter()
                    .map(|v| v.payload.iter().map(|t| conservative_word_count(&substitute_ty(t, &subst), registry)).sum::<u64>())
                    .max()
                    .unwrap_or(0)
            } else {
                unreachable!("typeck.rs already proved this Ty::Named resolves to a struct or enum")
            }
        }
        // Every other `Ty` this fn can legally be reached with under
        // Phase 4a's non-affine scope (`llvm_ty`'s affine check already
        // rejected anything containing `Box`/`Tcp`/etc.) is one scalar
        // word or less (`Unit` needs zero, but rounding it up to one
        // costs nothing and keeps this fn's contract simple: "at most
        // this many words").
        _ => 1,
    }
}


/// Element type and total flat element count for an aggregate `Ty` --
/// `Matrix(T,R,C)` flattens to `R*C` elements, row-major, matching
/// `interpreter.rs`'s `Value::Matrix` storage exactly. Only ever called
/// where `Ty::is_aggregate()` is already known true.
pub(super) fn agg_elem_and_len(ty: &Ty) -> (&Ty, usize) {
    match ty {
        Ty::Vector(elem, n) => (elem, *n),
        Ty::Matrix(elem, r, c) => (elem, r * c),
        _ => unreachable!("only called on Ty::is_aggregate() types"),
    }
}


/// A scalar element type's in-memory size, for `llvm.memcpy` byte counts.
/// No literal or builtin in this language can ever *construct* a value
/// whose `Vector`/`Matrix` element type is itself `Vector`/`Matrix` --
/// `typeck::infer_array_lit` collapses any 2-level literal nesting
/// straight into `Ty::Matrix`, and 3+ levels is a static
/// `ArrayLiteralTooDeep` error — even though `Vector(Vector(f64,3), 2)`
/// is syntactically parseable as a bare type annotation
/// (`parser.rs::expect_type` recurses generically on the element type).
/// So this never actually runs on a nested-aggregate element for any
/// value that reaches codegen, even though the type system doesn't rule
/// the annotation itself out.
pub(super) fn elem_byte_size(ty: &Ty) -> u64 {
    match ty {
        Ty::I8 | Ty::U8 | Ty::Bool => 1,
        Ty::I16 | Ty::U16 => 2,
        Ty::I32 | Ty::U32 => 4,
        Ty::I64 | Ty::U64 | Ty::Usize | Ty::F64 => 8,
        _ => unreachable!(
            "Vector/Matrix element types are always plain scalars for any constructible \
             value -- see this fn's doc comment"
        ),
    }
}


/// The heap-allocation size (in bytes, as an LLVM `i64` operand — see
/// below) `box e` needs for a value of type `ty` — unlike
/// `elem_byte_size` (scalars only, for aggregate elements) or
/// `agg_layout` (`Vector`/`Matrix` only), this covers every `Ty` `box`
/// can legally wrap (ast.rs: "any type, recursively" — `box i64`, `box
/// box i64`, `box Vector(f64,3)`, `box Point{..}`, etc.), since `box`
/// itself has no such restriction. Matches `llvm_ty`'s own type-to-size
/// story exactly: `Vector`/`Matrix` use their flat byte layout, a
/// non-affine `Ty::Named` (Row 11) uses `agg_byte_size_operand`'s
/// sizeof-via-GEP constant expression, `Ty::Str` is the two-word `{ptr,
/// i64}` value (16 bytes), every pointer-shaped handle (`Box`, `Ref`,
/// and — once later phases compile them — `Thread`/`Channel`/`Sandbox`/
/// `Tcp`/`TcpListener`) is one pointer-word (8 bytes on every platform
/// this backend targets), and everything else is a plain scalar via
/// `elem_byte_size`.
///
/// Returns a `String` operand, not a plain `u64`, for the same reason
/// `agg_byte_size_operand` does: a `Ty::Named`'s real size isn't a
/// number this side can compute without risking disagreement with
/// LLVM's own struct-layout/alignment rules, so it's expressed as an
/// LLVM constant expression instead and left for LLVM itself to
/// evaluate. Every other arm still produces a plain integer literal
/// (unchanged from before), which is exactly as valid an `i64` operand
/// as the constant expression is.
pub(super) fn ty_byte_size(ty: &Ty, registry: &TypeRegistry) -> String {
    match ty {
        Ty::Vector(_, _) | Ty::Matrix(_, _, _) | Ty::Named(_, _) => agg_byte_size_operand(ty, registry),
        Ty::Str => "16".to_string(),
        Ty::Box(_) | Ty::Ref(_) | Ty::Thread(_) | Ty::Channel(_) | Ty::Sandbox | Ty::Tcp | Ty::TcpListener | Ty::File => {
            "8".to_string()
        }
        Ty::Unit => "0".to_string(),
        _ => elem_byte_size(ty).to_string(),
    }
}


/// `(element Ty, flat element count, total byte size)` for a `Vector`/
/// `Matrix` type — everything `llvm.memcpy`-based aggregate codegen
/// (function prologue copy-in, `let`/assignment copies, literal row
/// copies) needs in one call. `Ty::Named` (Row 11) deliberately doesn't
/// go through this — see `agg_byte_size_operand`.
pub(super) fn agg_layout(ty: &Ty) -> (&Ty, usize, u64) {
    let (elem, len) = agg_elem_and_len(ty);
    let bytes = len as u64 * elem_byte_size(elem);
    (elem, len, bytes)
}


/// The byte-size *operand text* for `ty` (`Ty::is_aggregate()` only) —
/// used everywhere an aggregate's whole-value byte count is needed as an
/// `llvm.memcpy`/`nir_alloc` operand. `Vector`/`Matrix` keep using
/// `agg_layout`'s plain integer literal exactly as before this function
/// existed. `Ty::Named` has no such literal available on the Rust side
/// without hand-replicating LLVM's own struct-layout/alignment rules (a
/// real under-sizing risk if that math ever drifted from LLVM's actual
/// choice) — instead this emits the standard LLVM "sizeof via
/// GEP-of-null-plus-one" constant expression, which LLVM itself computes
/// correctly from the real declared type: `ptrtoint (ptr getelementptr
/// (%Name, ptr null, i32 1) to i64)`. Constant-foldable, so it costs
/// nothing extra after `-O2` — the same "trust `clang -O2` to fold
/// constant-shaped IR" precedent already used to justify a plain
/// `llvm.memcpy` over hand-unrolled loads/stores (module doc).
pub(super) fn agg_byte_size_operand(ty: &Ty, registry: &TypeRegistry) -> String {
    match ty {
        Ty::Vector(_, _) | Ty::Matrix(_, _, _) => {
            let (_, _, bytes) = agg_layout(ty);
            bytes.to_string()
        }
        Ty::Named(_, _) => {
            let llty = llvm_ty(ty, registry).expect("check_supported already validated this type");
            format!("ptrtoint (ptr getelementptr ({llty}, ptr null, i32 1) to i64)")
        }
        _ => unreachable!("only called on Ty::is_aggregate() types"),
    }
}


/// Structural pre-check: walks the whole program and rejects, with a
/// specific reason, anything `llvm_ty` or the `print`-argument rule
/// would reject — run once, up front, so codegen itself can assume every
/// type/expression it encounters is in the supported subset.
///
/// Row 11 (`struct`/`enum`/`match`) is handled the same way every other
/// type-level restriction here is: `llvm_ty`, threaded a `TypeRegistry`
/// built fresh for this pre-pass (this runs before any real `Codegen`
/// exists, so it can't reuse one), rejects any *affine-containing*
/// `Ty::Named` reaching a param/`let`/return type — `program.enums` is
/// never actually empty (`Option`/`Result` are injected into every
/// program at parse time, `ast::prelude_enums`'s doc comment), but a
/// non-affine struct/enum is now genuinely supported, so there's nothing
/// left here that needs a bespoke ctor-name check the way there used to
/// be (a struct/variant construction is syntactically just `Expr::Call`,
/// walked exactly like any other call below).
/// True iff `ty`, expanded through direct (non-pointer) struct/enum field
/// containment, transitively contains itself — the shape LLVM itself
/// refuses at codegen time ("identified structure type 'X' is recursive":
/// an infinite-size aggregate). `Box`/`Ref`/`Thread`/`Channel`/`Sandbox`/
/// `Tcp`/`TcpListener`/`File`/`Fn` fields break the cycle on purpose —
/// they're a fixed-size handle regardless of what's behind them, the same
/// "pointer, not inline storage" reasoning `llvm_ty`'s `Ty::Box`/`Ty::Ref`
/// arm documents — so only a field inlined directly into the aggregate's
/// own byte layout can make it infinitely sized. `Vector`/`Matrix`
/// inline their element `n`/`r*c` times (`llvm_ty`'s own `[len x elem]`
/// lowering), so an element cycle is exactly as fatal as a struct-field
/// one and is walked the same way. Mirrors
/// `TypeRegistry::is_affine_visiting`'s "track names on the current path,
/// a repeat is the cycle" shape (`ast.rs`, same 2026-08-27 pass this
/// guards the codegen-only half of).
pub(super) fn has_cyclic_layout(ty: &Ty, registry: &TypeRegistry, visiting: &mut Vec<String>) -> bool {
    match ty {
        Ty::Named(name, args) => {
            if visiting.iter().any(|v| v == name.as_str()) {
                return true;
            }
            if let Some(fields) = registry.struct_fields(name) {
                let subst = zip_type_params(registry.struct_type_params(name).unwrap_or(&[]), args);
                visiting.push(name.clone());
                let result = fields.iter().any(|f| has_cyclic_layout(&substitute_ty(&f.ty, &subst), registry, visiting));
                visiting.pop();
                result
            } else if let Some(variants) = registry.enum_variants(name) {
                let subst = zip_type_params(registry.enum_type_params(name).unwrap_or(&[]), args);
                visiting.push(name.clone());
                let result = variants
                    .iter()
                    .any(|v| v.payload.iter().any(|t| has_cyclic_layout(&substitute_ty(t, &subst), registry, visiting)));
                visiting.pop();
                result
            } else {
                // Unknown name -- typeck.rs reports this separately; no
                // cycle to report from here.
                false
            }
        }
        Ty::Vector(elem, _) | Ty::Matrix(elem, _, _) => has_cyclic_layout(elem, registry, visiting),
        // Every other `Ty` is either a plain scalar (nothing to recurse
        // into) or a pointer-ish handle whose own size never depends on
        // what it points to, so it can't be part of an infinite-size
        // cycle regardless of what's behind it.
        _ => false,
    }
}


impl Codegen<'_> {
    /// `getelementptr` to one element of a flat aggregate buffer at a
    /// *compile-time-constant* flat index -- the workhorse every unrolled
    /// elementwise/product loop below uses (unlike `Expr::Index`'s own
    /// GEP, whose offset is a runtime SSA value; this one's offset is a
    /// literal, since every loop here unrolls over a compile-time-known
    /// shape).
    pub(super) fn agg_elem_ptr(&mut self, base_ptr: &str, elem_llty: &str, flat_idx: usize) -> String {
        let gep = self.fresh_reg("agg_elem.addr");
        writeln!(self.out, "  {gep} = getelementptr {elem_llty}, ptr {base_ptr}, i64 {flat_idx}").unwrap();
        gep
    }


    /// Load one element and widen it to this backend's internal i64/
    /// double convention (`widen_to_i64`) -- so the result composes
    /// directly with a plain scalar `expr()` value (e.g. the scalar
    /// operand of `scalar * Matrix`) and with `emit_mul`/`emit_add`.
    pub(super) fn agg_load_elem(&mut self, base_ptr: &str, elem_llty: &str, elem_ty: &Ty, flat_idx: usize) -> String {
        let gep = self.agg_elem_ptr(base_ptr, elem_llty, flat_idx);
        let loaded = self.fresh_reg("agg_elem.val");
        writeln!(self.out, "  {loaded} = load {elem_llty}, ptr {gep}").unwrap();
        self.widen_to_i64(&loaded, elem_ty)
    }


    /// Store one element, narrowing back down from the internal i64
    /// convention first if the element type is a narrower integer --
    /// the store side of `agg_load_elem`'s widen, mirroring how
    /// `array_lit` already narrows before storing a literal's elements.
    pub(super) fn agg_store_elem(&mut self, base_ptr: &str, elem_llty: &str, elem_ty: &Ty, flat_idx: usize, val: &str) -> Result<(), CodegenError> {
        let store_val = if elem_ty.is_integer() { self.narrow_from_i64(val, elem_ty)? } else { val.to_string() };
        let gep = self.agg_elem_ptr(base_ptr, elem_llty, flat_idx);
        writeln!(self.out, "  store {elem_llty} {store_val}, ptr {gep}").unwrap();
        Ok(())
    }


}
