use super::*;
use super::layout::*;

impl Codegen<'_> {
    /// Generic `Ty`<->JSON for one compiled value — the foundation a
    /// compiled `serve` route wrapper (`rfcs/0010-landing-and-serve-
    /// exposure.md`'s exposure set, not yet wired to codegen — this is
    /// standalone plumbing, not called from anywhere real yet) will
    /// decode a request body and encode a response body with. Deliberately
    /// *not* a generalization of `nir_transact_decode_args`/
    /// `emit_replay_decode_operands` just above: that path's flat
    /// `NirBindValue` array is a real, separate design for durability-log
    /// compactness (`is_transact_scalar`'s own doc comment), scoped to
    /// exactly 4 scalar shapes on purpose — conflating it with a struct-
    /// capable, real-JSON-text wire format would misrepresent both. This
    /// reuses the *pattern* (walk a static `Ty`, GEP one field pointer at
    /// a time) against the real `nir_json_*` kernels instead.
    ///
    /// **Wire shape is not invented here — it's recovered.** The deleted
    /// tree-walking interpreter's `nirdosha serve` had exactly this
    /// (`decode_value`/`encode_value`, `git show
    /// 05a747c~1:crates/compiler/src/serve.rs`), and `ui_gen_template.html`'s
    /// generated client-side `callFn` already hard-codes the result: a
    /// struct is a plain `{"field": ...}` object keyed by declared field
    /// name, and `Ty::Named("Result", [_, _])` is `{"ok": ...}`/
    /// `{"err": ...}` — matched exactly, not re-derived, so a future
    /// served app's generated frontend needs no changes to talk to
    /// whatever calls these.
    ///
    /// **Scope for this pass**: `bool`/every integer width/`f64`/`str`
    /// (the same leaf shapes `emit_replay_decode_operands` already
    /// handles), plus structs (every field, including a nested struct
    /// field) and `Result(T, E)`. `Option`/enum/`Vector`/`Matrix`/`json`/
    /// `dec128` are real cases the old `decode_value`/`encode_value` also
    /// had — disclosed, separate follow-up for whichever later stage's
    /// route shapes actually need them, not silently assumed covered.
    ///
    /// Pointer-based for *every* `ty`, including scalars — a struct/
    /// `Result` field naturally produces a pointer via `getelementptr`,
    /// and one uniform calling convention here means the recursive walk
    /// never has to branch on "do I have a value or a pointer" on top of
    /// already branching on `ty`'s own shape. `ptr` must already point at
    /// a live slot holding one value of `ty` (a scalar's own `alloca`d
    /// temp is fine — see the round-trip tests in `tests/codegen.rs` for
    /// the exact pattern a caller uses).
    pub(super) fn emit_encode_value_json(&mut self, ty: &Ty, ptr: &str) -> Result<String, CodegenError> {
        match ty {
            Ty::Bool => {
                let v = self.fresh_reg("encode_json_bool_val");
                writeln!(self.out, "  {v} = load i1, ptr {ptr}").unwrap();
                let v32 = self.fresh_reg("encode_json_bool_i32");
                writeln!(self.out, "  {v32} = zext i1 {v} to i32").unwrap();
                Ok(self.emit_infallible_json_encode("nir_json_encode_bool", &[format!("i32 {v32}")], "encode_json_bool"))
            }
            other if other.is_integer() => {
                let llty = self.llvm_ty(other)?;
                let v = self.fresh_reg("encode_json_int_val");
                writeln!(self.out, "  {v} = load {llty}, ptr {ptr}").unwrap();
                let widened = self.widen_to_i64(&v, other);
                Ok(self.emit_infallible_json_encode("nir_json_encode_i64", &[format!("i64 {widened}")], "encode_json_int"))
            }
            Ty::F64 => {
                let v = self.fresh_reg("encode_json_f64_val");
                writeln!(self.out, "  {v} = load double, ptr {ptr}").unwrap();
                Ok(self.emit_infallible_json_encode("nir_json_encode_f64", &[format!("double {v}")], "encode_json_f64"))
            }
            Ty::Str => {
                let v = self.fresh_reg("encode_json_str_val");
                writeln!(self.out, "  {v} = load {{ptr, i64}}, ptr {ptr}").unwrap();
                let sptr = self.fresh_reg("encode_json_str_ptr");
                writeln!(self.out, "  {sptr} = extractvalue {{ptr, i64}} {v}, 0").unwrap();
                let slen = self.fresh_reg("encode_json_str_len");
                writeln!(self.out, "  {slen} = extractvalue {{ptr, i64}} {v}, 1").unwrap();
                Ok(self.emit_infallible_json_encode("nir_json_encode_str", &[format!("ptr {sptr}"), format!("i64 {slen}")], "encode_json_str"))
            }
            Ty::Json => {
                // A `json` value *is* already raw JSON text (`{ptr,
                // i64}`). The route wrapper just returns it as the body.
                let v = self.fresh_reg("encode_json_json_val");
                writeln!(self.out, "  {v} = load {{ptr, i64}}, ptr {ptr}").unwrap();
                Ok(v)
            }
            Ty::Named(name, args) if name == "Result" && args.len() == 2 => self.emit_encode_result_json(&args[0], &args[1], ptr),
            Ty::Named(name, _) if self.registry.is_struct(name) => self.emit_encode_struct_json(ty, ptr),
            Ty::Named(name, _) if self.registry.is_enum(name) => self.emit_encode_enum_json(ty, ptr),
            other => unsupported(format!("encoding a `{}` to JSON isn't supported yet (compiled `serve` Stage 1's scope: scalars, structs, enums, `Result`)", other.name())),
        }
    }


    /// The infallible leaf call every scalar encoder above shares: call
    /// `fn_name(<operands>, ptr out)`, load the `{ptr, i64}` JSON text it
    /// wrote, return it. No status/error branch — see
    /// `nir_json_encode_i64`'s own doc comment (`runtime-kernels/src/
    /// lib.rs`) for why these four specifically can't fail.
    pub(super) fn emit_infallible_json_encode(&mut self, fn_name: &str, operands: &[String], label_prefix: &str) -> String {
        let out = self.fresh_reg(&format!("{label_prefix}_out_scratch"));
        self.emit_alloca(&out, "{ptr, i64}");
        let joined = operands.join(", ");
        writeln!(self.out, "  call void @{fn_name}({joined}, ptr {out})").unwrap();
        let val = self.fresh_reg(&format!("{label_prefix}_out"));
        writeln!(self.out, "  {val} = load {{ptr, i64}}, ptr {out}").unwrap();
        val
    }


    /// The struct half of `emit_encode_value_json`: fold `nir_json_set_raw`
    /// over every declared field in order, starting from the empty object
    /// `"{}"` — each field's own value is encoded first (recursing through
    /// `emit_encode_value_json`, so a nested struct field just works,
    /// built bottom-up the same way `construct_struct` builds a struct
    /// top-down), then spliced in under its declared name. Field
    /// *order* doesn't matter to the wire format (a JSON object is
    /// unordered) but iterating `struct_fields`' own declaration order
    /// keeps the emitted IR deterministic run to run.
    pub(super) fn emit_encode_struct_json(&mut self, ty: &Ty, ptr: &str) -> Result<String, CodegenError> {
        let Ty::Named(name, _) = ty else { unreachable!("caller already matched Ty::Named") };
        let struct_llty = self.llvm_ty(ty)?;
        let fields = self.registry.struct_fields(name).expect("caller already confirmed this is a struct").to_vec();
        let empty_obj = self.fresh_global("encode_json_struct_empty");
        writeln!(self.string_globals, "{empty_obj} = private unnamed_addr constant [2 x i8] c\"{{}}\"").unwrap();
        let mut doc_ptr = self.fresh_reg("encode_json_struct_doc_ptr0");
        writeln!(self.out, "  {doc_ptr} = insertvalue {{ptr, i64}} undef, ptr {empty_obj}, 0").unwrap();
        let doc0 = self.fresh_reg("encode_json_struct_doc0");
        writeln!(self.out, "  {doc0} = insertvalue {{ptr, i64}} {doc_ptr}, i64 2, 1").unwrap();
        doc_ptr = doc0;
        for (i, field) in fields.iter().enumerate() {
            let (idx, field_ty) = self.field_index_and_ty(ty, &field.name).expect("field came from this same struct's own field list");
            let field_ptr = self.fresh_reg(&format!("encode_json_struct_field{i}_ptr"));
            writeln!(self.out, "  {field_ptr} = getelementptr inbounds {struct_llty}, ptr {ptr}, i32 0, i32 {idx}").unwrap();
            let field_json = self.emit_encode_value_json(&field_ty, &field_ptr)?;
            let field_json_ptr = self.fresh_reg(&format!("encode_json_struct_field{i}_jptr"));
            writeln!(self.out, "  {field_json_ptr} = extractvalue {{ptr, i64}} {field_json}, 0").unwrap();
            let field_json_len = self.fresh_reg(&format!("encode_json_struct_field{i}_jlen"));
            writeln!(self.out, "  {field_json_len} = extractvalue {{ptr, i64}} {field_json}, 1").unwrap();
            let doc_ptr_word = self.fresh_reg(&format!("encode_json_struct_doc{}_ptr", i + 1));
            writeln!(self.out, "  {doc_ptr_word} = extractvalue {{ptr, i64}} {doc_ptr}, 0").unwrap();
            let doc_len_word = self.fresh_reg(&format!("encode_json_struct_doc{}_len", i + 1));
            writeln!(self.out, "  {doc_len_word} = extractvalue {{ptr, i64}} {doc_ptr}, 1").unwrap();
            let key_global = self.fresh_global(&format!("encode_json_struct_key{i}"));
            writeln!(self.string_globals, "{key_global} = private unnamed_addr constant [{} x i8] c\"{}\"", field.name.len(), llvm_escape_bytes(field.name.as_bytes())).unwrap();
            let out_scratch = self.fresh_reg(&format!("encode_json_struct_out{}_scratch", i + 1));
            self.emit_alloca(&out_scratch, "{ptr, i64}");
            let err_scratch = self.fresh_reg(&format!("encode_json_struct_err{}_scratch", i + 1));
            self.emit_alloca(&err_scratch, "{ptr, i64}");
            writeln!(
                self.out,
                "  call i32 @nir_json_set_raw(ptr {doc_ptr_word}, i64 {doc_len_word}, ptr {key_global}, i64 {}, ptr {field_json_ptr}, i64 {field_json_len}, ptr {out_scratch}, ptr {err_scratch})",
                field.name.len()
            )
            .unwrap();
            // `set_raw` only fails on a malformed `doc`/`raw` — both are
            // this same walk's own always-well-formed output (`"{}"` the
            // first time, a previous `set_raw`'s own JSON text after),
            // so there's nothing for a caller to meaningfully recover
            // from here; asserting via the loaded value itself (never
            // branching on the status) keeps this fold a straight line,
            // matching `construct_struct`'s own unconditional field-store
            // loop.
            let next_doc = self.fresh_reg(&format!("encode_json_struct_doc{}", i + 1));
            writeln!(self.out, "  {next_doc} = load {{ptr, i64}}, ptr {out_scratch}").unwrap();
            doc_ptr = next_doc;
        }
        Ok(doc_ptr)
    }


    /// The `Result(T, E)` half: reads the already-computed value's real
    /// tag (`0` = `Ok`, `1` = `Err`, `ast::prelude_enums`' own `Result`
    /// declaration order — the same convention `emit_check_role`/
    /// `emit_result_merge` construct *into*, read back out here), encodes
    /// whichever payload is live, and wraps it as `{"ok": ...}`/
    /// `{"err": ...}` via the same `nir_json_set_raw` fold
    /// `emit_encode_struct_json` uses for its own single-field case —
    /// there's no flatter representation for a two-variant enum to fall
    /// back to.
    pub(super) fn emit_encode_result_json(&mut self, ok_ty: &Ty, err_ty: &Ty, ptr: &str) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![ok_ty.clone(), err_ty.clone()]);
        let result_llty = self.llvm_ty(&result_ty)?;
        let tag_ptr = self.fresh_reg("encode_json_result_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {ptr}, i32 0, i32 0").unwrap();
        let tag = self.fresh_reg("encode_json_result_tag");
        writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
        let payload_ptr = self.fresh_reg("encode_json_result_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {ptr}, i32 0, i32 1").unwrap();
        let is_ok = self.icmp("eq", "i64", &tag, "0")?;

        let ok_label = self.fresh_label("encode_json_result_ok");
        let err_label = self.fresh_label("encode_json_result_err");
        let merge_label = self.fresh_label("encode_json_result_merge");
        let dest = self.fresh_reg("encode_json_result_dest");
        self.emit_alloca(&dest, "{ptr, i64}");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        let ok_json = self.emit_encode_value_json(ok_ty, &payload_ptr)?;
        let ok_wrapped = self.emit_wrap_json_field("ok", &ok_json, "encode_json_result_ok_wrap")?;
        writeln!(self.out, "  store {{ptr, i64}} {ok_wrapped}, ptr {dest}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        let err_json = self.emit_encode_value_json(err_ty, &payload_ptr)?;
        let err_wrapped = self.emit_wrap_json_field("err", &err_json, "encode_json_result_err_wrap")?;
        writeln!(self.out, "  store {{ptr, i64}} {err_wrapped}, ptr {dest}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let result = self.fresh_reg("encode_json_result_val");
        writeln!(self.out, "  {result} = load {{ptr, i64}}, ptr {dest}").unwrap();
        Ok(result)
    }


    /// Encode a user-defined enum as a JSON string for zero-payload
    /// variants. Payload-carrying enums are still rejected -- the wire
    /// format for those wants an object shape that Stage 1 doesn't yet
    /// emit, and most route enums (PaymentChannel, status enums, etc.)
    /// are zero-payload anyway.
    pub(super) fn emit_encode_enum_json(
        &mut self,
        ty: &Ty,
        ptr: &str,
    ) -> Result<String, CodegenError> {
        let Ty::Named(name, _args) = ty else { unreachable!("caller already matched Ty::Named") };
        let variants = self
            .registry
            .enum_variants(name)
            .expect("caller already confirmed this is an enum")
            .to_vec();
        if variants.iter().any(|v| !v.payload.is_empty()) {
            return unsupported(format!(
                "encoding enum `{name}` to JSON: payload-carrying enum variants are not supported yet by compiled `serve` Stage 1"
            ));
        }

        let enum_llty = self.llvm_ty(ty)?;
        let tag_ptr = self.fresh_reg("encode_enum_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {enum_llty}, ptr {ptr}, i32 0, i32 0").unwrap();
        let tag = self.fresh_reg("encode_enum_tag");
        writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();

        let merge_label = self.fresh_label("encode_enum_merge");
        let mut arm_labels = Vec::new();
        for i in 0..variants.len() {
            arm_labels.push(self.fresh_label(&format!("encode_enum_variant_{i}")));
        }
        let default_label = self.fresh_label("encode_enum_default");
        writeln!(self.out, "  switch i64 {tag}, label %{default_label} [").unwrap();
        for (i, label) in arm_labels.iter().enumerate() {
            writeln!(self.out, "    i64 {i}, label %{label}").unwrap();
        }
        writeln!(self.out, "  ]").unwrap();

        let mut phi_pairs = Vec::new();
        for (i, v) in variants.iter().enumerate() {
            writeln!(self.out, "{}:", arm_labels[i]).unwrap();
            let variant_name = &v.name;
            let name_global = self.fresh_global(&format!("encode_enum_name_{i}"));
            let escaped = llvm_escape_bytes(variant_name.as_bytes());
            writeln!(
                self.string_globals,
                "{name_global} = private unnamed_addr constant [{} x i8] c\"{escaped}\"",
                variant_name.len()
            )
            .unwrap();
            let str_val = self.fresh_reg(&format!("encode_enum_str_{i}"));
            writeln!(self.out, "  {str_val} = insertvalue {{ptr, i64}} undef, ptr {name_global}, 0").unwrap();
            let str_val_full = self.fresh_reg(&format!("encode_enum_str_full_{i}"));
            writeln!(
                self.out,
                "  {str_val_full} = insertvalue {{ptr, i64}} {str_val}, i64 {}, 1",
                variant_name.len()
            )
            .unwrap();
            let str_ptr = self.fresh_reg(&format!("encode_enum_str_ptr_{i}"));
            writeln!(self.out, "  {str_ptr} = extractvalue {{ptr, i64}} {str_val_full}, 0").unwrap();
            let str_len = self.fresh_reg(&format!("encode_enum_str_len_{i}"));
            writeln!(self.out, "  {str_len} = extractvalue {{ptr, i64}} {str_val_full}, 1").unwrap();
            let json = self.emit_infallible_json_encode("nir_json_encode_str", &[format!("ptr {str_ptr}"), format!("i64 {str_len}")], &format!("encode_enum_{i}"));
            phi_pairs.push((json.clone(), arm_labels[i].clone()));
            writeln!(self.out, "  br label %{merge_label}").unwrap();
        }

        writeln!(self.out, "{default_label}:").unwrap();
        writeln!(self.out, "  unreachable").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let result = self.fresh_reg("encode_enum_result");
        let phi_in = phi_pairs
            .iter()
            .map(|(v, l)| format!("[ {v}, %{l} ]"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(self.out, "  {result} = phi {{ptr, i64}} {phi_in}").unwrap();
        Ok(result)
    }


    /// `nir_json_set_raw("{}", key, value_json)` — wraps one already-
    /// encoded JSON value as a single-field object, `{"<key>": <value>}`.
    /// Shared by `emit_encode_result_json`'s `ok`/`err` branches.
    pub(super) fn emit_wrap_json_field(&mut self, key: &str, value_json: &str, label_prefix: &str) -> Result<String, CodegenError> {
        let empty_obj = self.fresh_global(&format!("{label_prefix}_empty"));
        writeln!(self.string_globals, "{empty_obj} = private unnamed_addr constant [2 x i8] c\"{{}}\"").unwrap();
        let key_global = self.fresh_global(&format!("{label_prefix}_key"));
        writeln!(self.string_globals, "{key_global} = private unnamed_addr constant [{} x i8] c\"{}\"", key.len(), llvm_escape_bytes(key.as_bytes())).unwrap();
        let value_ptr = self.fresh_reg(&format!("{label_prefix}_value_ptr"));
        writeln!(self.out, "  {value_ptr} = extractvalue {{ptr, i64}} {value_json}, 0").unwrap();
        let value_len = self.fresh_reg(&format!("{label_prefix}_value_len"));
        writeln!(self.out, "  {value_len} = extractvalue {{ptr, i64}} {value_json}, 1").unwrap();
        let out_scratch = self.fresh_reg(&format!("{label_prefix}_out_scratch"));
        self.emit_alloca(&out_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg(&format!("{label_prefix}_err_scratch"));
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        writeln!(
            self.out,
            "  call i32 @nir_json_set_raw(ptr {empty_obj}, i64 2, ptr {key_global}, i64 {}, ptr {value_ptr}, i64 {value_len}, ptr {out_scratch}, ptr {err_scratch})",
            key.len()
        )
        .unwrap();
        let wrapped = self.fresh_reg(&format!("{label_prefix}_wrapped"));
        writeln!(self.out, "  {wrapped} = load {{ptr, i64}}, ptr {out_scratch}").unwrap();
        Ok(wrapped)
    }


    /// The decode direction: parses `json` (a `{ptr, i64}` *value* — the
    /// raw JSON text for one whole value, e.g. `nir_json_array_get`'s own
    /// output for one request-body argument) as `ty`, returning a
    /// pointer to a real `Result(ty, str)` — malformed/missing/wrong-
    /// shaped input is a real `Err` with a human-readable message, never
    /// a trap, the same contract every `nir_json_get_*` builtin already
    /// gives `.nir` source. Struct decode short-circuits on the first
    /// field failure (sequential checks, not a fan-in `phi`) — simpler to
    /// emit correctly than merging N independent failure messages, and a
    /// request body with more than one malformed field is going to be
    /// re-read by a human either way.
    pub(super) fn emit_decode_value_json(&mut self, ty: &Ty, json: &str) -> Result<String, CodegenError> {
        match ty {
            Ty::Bool => {
                let (json_ptr, json_len) = self.split_str_word(json);
                let value_scratch = self.fresh_reg("decode_json_bool_value_scratch");
                self.emit_alloca(&value_scratch, "i32");
                let err_scratch = self.fresh_reg("decode_json_bool_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("decode_json_bool_found");
                writeln!(self.out, "  {found} = call i32 @nir_json_decode_bool(ptr {json_ptr}, i64 {json_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
                let is_ok = self.icmp("ne", "i32", &found, "0")?;
                let raw = self.fresh_reg("decode_json_bool_raw");
                writeln!(self.out, "  {raw} = load i32, ptr {value_scratch}").unwrap();
                let value = self.icmp("ne", "i32", &raw, "0")?;
                let err_val = self.fresh_reg("decode_json_bool_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.emit_result_merge(&Ty::Named("Result".to_string(), vec![Ty::Bool, Ty::Str]), &is_ok, "i1", &value, &err_val, "decode_json_bool")
            }
            other if other.is_integer() => {
                let (json_ptr, json_len) = self.split_str_word(json);
                let value_scratch = self.fresh_reg("decode_json_int_value_scratch");
                self.emit_alloca(&value_scratch, "i64");
                let err_scratch = self.fresh_reg("decode_json_int_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("decode_json_int_found");
                writeln!(self.out, "  {found} = call i32 @nir_json_decode_i64(ptr {json_ptr}, i64 {json_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
                let is_ok = self.icmp("ne", "i32", &found, "0")?;
                let wide = self.fresh_reg("decode_json_int_wide");
                writeln!(self.out, "  {wide} = load i64, ptr {value_scratch}").unwrap();
                let narrowed = self.narrow_from_i64(&wide, other)?;
                let llty = self.llvm_ty(other)?;
                let err_val = self.fresh_reg("decode_json_int_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.emit_result_merge(&Ty::Named("Result".to_string(), vec![other.clone(), Ty::Str]), &is_ok, &llty, &narrowed, &err_val, "decode_json_int")
            }
            Ty::F64 => {
                let (json_ptr, json_len) = self.split_str_word(json);
                let value_scratch = self.fresh_reg("decode_json_f64_value_scratch");
                self.emit_alloca(&value_scratch, "double");
                let err_scratch = self.fresh_reg("decode_json_f64_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("decode_json_f64_found");
                writeln!(self.out, "  {found} = call i32 @nir_json_decode_f64(ptr {json_ptr}, i64 {json_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
                let is_ok = self.icmp("ne", "i32", &found, "0")?;
                let value = self.fresh_reg("decode_json_f64_value");
                writeln!(self.out, "  {value} = load double, ptr {value_scratch}").unwrap();
                let err_val = self.fresh_reg("decode_json_f64_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.emit_result_merge(&Ty::Named("Result".to_string(), vec![Ty::F64, Ty::Str]), &is_ok, "double", &value, &err_val, "decode_json_f64")
            }
            Ty::Str => {
                let (json_ptr, json_len) = self.split_str_word(json);
                let value_scratch = self.fresh_reg("decode_json_str_value_scratch");
                self.emit_alloca(&value_scratch, "{ptr, i64}");
                let err_scratch = self.fresh_reg("decode_json_str_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("decode_json_str_found");
                writeln!(self.out, "  {found} = call i32 @nir_json_decode_str(ptr {json_ptr}, i64 {json_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
                let is_ok = self.icmp("ne", "i32", &found, "0")?;
                let value = self.fresh_reg("decode_json_str_value");
                writeln!(self.out, "  {value} = load {{ptr, i64}}, ptr {value_scratch}").unwrap();
                let err_val = self.fresh_reg("decode_json_str_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.emit_result_merge(&Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]), &is_ok, "{ptr, i64}", &value, &err_val, "decode_json_str")
            }
            Ty::Json => {
                // A `json` value *is* raw JSON text (`Ty::Json`'s
                // `llvm_ty` arm), so decoding it is the same identity-
                // on-success shape as `emit_json_parse`: validate with
                // `nir_json_validate` (which accepts *any* JSON value --
                // object, array, number, bool, string, null -- unlike
                // `nir_json_decode_str`, which only accepts a JSON
                // string literal and would wrongly reject an object or
                // array `json` payload), and reuse `json`'s own
                // already-split `{ptr, i64}` as the `Ok` payload rather
                // than re-decoding it.
                let (json_ptr, json_len) = self.split_str_word(json);
                let err_scratch = self.fresh_reg("decode_json_json_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("decode_json_json_found");
                writeln!(self.out, "  {found} = call i32 @nir_json_validate(ptr {json_ptr}, i64 {json_len}, ptr {err_scratch})").unwrap();
                let is_ok = self.icmp("ne", "i32", &found, "0")?;
                let err_val = self.fresh_reg("decode_json_json_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.emit_result_merge(&Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]),
                    &is_ok, "{ptr, i64}", json, &err_val, "decode_json_json",
                )
            }
            Ty::Named(name, args) if name == "Result" && args.len() == 2 => self.emit_decode_result_json(&args[0], &args[1], json),
            Ty::Named(name, _) if self.registry.is_struct(name) => self.emit_decode_struct_json(ty, json),
            Ty::Named(name, args) if self.registry.is_enum(name) => self.emit_decode_enum_json(name, args, json),
            other => unsupported(format!("decoding a `{}` from JSON isn't supported yet (compiled `serve` Stage 1's scope: scalars, structs, enums, `Result`)", other.name())),
        }
    }


    /// Splits an already-computed `{ptr, i64}` SSA value into its two
    /// words — the same `extractvalue` pair every decode leaf above
    /// needs on its `json` argument, factored out since none of them can
    /// use `str_parts` (that helper re-evaluates an `Expr`; every caller
    /// here already has the value, often itself the output of a previous
    /// kernel call with no corresponding `Expr` to re-evaluate).
    pub(super) fn split_str_word(&mut self, value: &str) -> (String, String) {
        let ptr = self.fresh_reg("str_word_ptr");
        writeln!(self.out, "  {ptr} = extractvalue {{ptr, i64}} {value}, 0").unwrap();
        let len = self.fresh_reg("str_word_len");
        writeln!(self.out, "  {len} = extractvalue {{ptr, i64}} {value}, 1").unwrap();
        (ptr, len)
    }


    /// The struct half of `emit_decode_value_json`: looks up each
    /// declared field by name (`nir_json_get`, the same keyed-lookup
    /// builtin `.nir` source itself uses via `json_get`), recursing
    /// through `emit_decode_value_json` for the field's own `Ty` — a
    /// nested struct field just works, since `nir_json_get`'s own output
    /// is itself a bare JSON value ready to hand straight back into this
    /// same function. Stores each successfully-decoded field directly
    /// into its slot in a fresh struct-shaped scratch alloca
    /// (`construct_struct`'s own field-by-field style), then loads the
    /// whole thing as one aggregate value to hand to `emit_result_merge`
    /// once every field has succeeded.
    pub(super) fn emit_decode_struct_json(&mut self, ty: &Ty, json: &str) -> Result<String, CodegenError> {
        let Ty::Named(name, _) = ty else { unreachable!("caller already matched Ty::Named") };
        let struct_llty = self.llvm_ty(ty)?;
        let fields = self.registry.struct_fields(name).expect("caller already confirmed this is a struct").to_vec();
        let result_ty = Ty::Named("Result".to_string(), vec![ty.clone(), Ty::Str]);
        let (json_ptr, json_len) = self.split_str_word(json);

        let scratch = self.fresh_reg("decode_json_struct_scratch");
        self.emit_alloca(&scratch, &struct_llty);
        let fail_label = self.fresh_label("decode_json_struct_fail");
        let err_scratch = self.fresh_reg("decode_json_struct_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        for (i, field) in fields.iter().enumerate() {
            let (idx, field_ty) = self.field_index_and_ty(ty, &field.name).expect("field came from this same struct's own field list");
            let key_global = self.fresh_global(&format!("decode_json_struct_key{i}"));
            writeln!(self.string_globals, "{key_global} = private unnamed_addr constant [{} x i8] c\"{}\"", field.name.len(), llvm_escape_bytes(field.name.as_bytes())).unwrap();
            let field_json_scratch = self.fresh_reg(&format!("decode_json_struct_field{i}_json_scratch"));
            self.emit_alloca(&field_json_scratch, "{ptr, i64}");
            let field_get_err = self.fresh_reg(&format!("decode_json_struct_field{i}_get_err"));
            self.emit_alloca(&field_get_err, "{ptr, i64}");
            let field_found = self.fresh_reg(&format!("decode_json_struct_field{i}_found"));
            writeln!(
                self.out,
                "  {field_found} = call i32 @nir_json_get(ptr {json_ptr}, i64 {json_len}, ptr {key_global}, i64 {}, ptr {field_json_scratch}, ptr {field_get_err})",
                field.name.len()
            )
            .unwrap();
            let field_is_found = self.icmp("ne", "i32", &field_found, "0")?;
            let has_field_label = self.fresh_label(&format!("decode_json_struct_field{i}_has"));
            let missing_field_label = self.fresh_label(&format!("decode_json_struct_field{i}_missing"));
            writeln!(self.out, "  br i1 {field_is_found}, label %{has_field_label}, label %{missing_field_label}").unwrap();

            writeln!(self.out, "{missing_field_label}:").unwrap();
            let missing_err = self.fresh_reg(&format!("decode_json_struct_field{i}_missing_err"));
            writeln!(self.out, "  {missing_err} = load {{ptr, i64}}, ptr {field_get_err}").unwrap();
            writeln!(self.out, "  store {{ptr, i64}} {missing_err}, ptr {err_scratch}").unwrap();
            writeln!(self.out, "  br label %{fail_label}").unwrap();

            writeln!(self.out, "{has_field_label}:").unwrap();
            let field_json = self.fresh_reg(&format!("decode_json_struct_field{i}_json"));
            writeln!(self.out, "  {field_json} = load {{ptr, i64}}, ptr {field_json_scratch}").unwrap();
            let field_result = self.emit_decode_value_json(&field_ty, &field_json)?;
            let field_result_llty = self.llvm_ty(&Ty::Named("Result".to_string(), vec![field_ty.clone(), Ty::Str]))?;
            let field_tag_ptr = self.fresh_reg(&format!("decode_json_struct_field{i}_tag_ptr"));
            writeln!(self.out, "  {field_tag_ptr} = getelementptr inbounds {field_result_llty}, ptr {field_result}, i32 0, i32 0").unwrap();
            let field_tag = self.fresh_reg(&format!("decode_json_struct_field{i}_tag"));
            writeln!(self.out, "  {field_tag} = load i64, ptr {field_tag_ptr}").unwrap();
            let field_payload_ptr = self.fresh_reg(&format!("decode_json_struct_field{i}_payload_ptr"));
            writeln!(self.out, "  {field_payload_ptr} = getelementptr inbounds {field_result_llty}, ptr {field_result}, i32 0, i32 1").unwrap();
            let field_is_ok = self.icmp("eq", "i64", &field_tag, "0")?;
            let field_ok_label = self.fresh_label(&format!("decode_json_struct_field{i}_ok"));
            let field_err_label = self.fresh_label(&format!("decode_json_struct_field{i}_err"));
            writeln!(self.out, "  br i1 {field_is_ok}, label %{field_ok_label}, label %{field_err_label}").unwrap();

            writeln!(self.out, "{field_err_label}:").unwrap();
            let field_err_val = self.fresh_reg(&format!("decode_json_struct_field{i}_err_val"));
            writeln!(self.out, "  {field_err_val} = load {{ptr, i64}}, ptr {field_payload_ptr}").unwrap();
            writeln!(self.out, "  store {{ptr, i64}} {field_err_val}, ptr {err_scratch}").unwrap();
            writeln!(self.out, "  br label %{fail_label}").unwrap();

            writeln!(self.out, "{field_ok_label}:").unwrap();
            let field_llty = self.llvm_ty(&field_ty)?;
            let field_val = self.fresh_reg(&format!("decode_json_struct_field{i}_val"));
            writeln!(self.out, "  {field_val} = load {field_llty}, ptr {field_payload_ptr}").unwrap();
            let dest_field_ptr = self.fresh_reg(&format!("decode_json_struct_field{i}_dest_ptr"));
            writeln!(self.out, "  {dest_field_ptr} = getelementptr inbounds {struct_llty}, ptr {scratch}, i32 0, i32 {idx}").unwrap();
            writeln!(self.out, "  store {field_llty} {field_val}, ptr {dest_field_ptr}").unwrap();
        }
        // Every field succeeded (no `br` to `fail_label` was taken) —
        // falls straight through from the last field's own `_ok` block.
        let loaded_struct = self.fresh_reg("decode_json_struct_val");
        writeln!(self.out, "  {loaded_struct} = load {struct_llty}, ptr {scratch}").unwrap();
        let success_label = self.fresh_label("decode_json_struct_success");
        let merge_label = self.fresh_label("decode_json_struct_merge");
        writeln!(self.out, "  br label %{success_label}").unwrap();

        writeln!(self.out, "{success_label}:").unwrap();
        let dest = self.fresh_reg("decode_json_struct_dest");
        let result_llty = self.llvm_ty(&result_ty)?;
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("decode_json_struct_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let payload_ptr = self.fresh_reg("decode_json_struct_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();
        writeln!(self.out, "  store {struct_llty} {loaded_struct}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{fail_label}:").unwrap();
        let fail_err = self.fresh_reg("decode_json_struct_fail_err");
        writeln!(self.out, "  {fail_err} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        let fail_dest = self.fresh_reg("decode_json_struct_fail_dest");
        self.emit_alloca(&fail_dest, &result_llty);
        let fail_tag_ptr = self.fresh_reg("decode_json_struct_fail_tag_ptr");
        writeln!(self.out, "  {fail_tag_ptr} = getelementptr inbounds {result_llty}, ptr {fail_dest}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 1, ptr {fail_tag_ptr}").unwrap();
        let fail_payload_ptr = self.fresh_reg("decode_json_struct_fail_payload_ptr");
        writeln!(self.out, "  {fail_payload_ptr} = getelementptr inbounds {result_llty}, ptr {fail_dest}, i32 0, i32 1").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {fail_err}, ptr {fail_payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let final_dest = self.fresh_reg("decode_json_struct_final_dest");
        writeln!(self.out, "  {final_dest} = phi ptr [ {dest}, %{success_label} ], [ {fail_dest}, %{fail_label} ]").unwrap();
        Ok(final_dest)
    }


    /// The `Result(T, E)` half of `emit_decode_value_json`: `{"ok": v}`
    /// decodes as `Ok(v: T)`, `{"err": v}` as `Err(v: E)` — the exact
    /// inverse of `emit_encode_result_json`'s own wrapping, checked via
    /// `nir_json_get`'s own presence check (`"ok"` present is enough to
    /// commit to the `Ok` branch, matching the old interpreter's own
    /// `decode_value` having no third shape for this type). Anything
    /// else (neither key present, `T`/`E`'s own decode failing, or a
    /// whole-value shape that isn't even an object) is a real top-level
    /// `Err`, not a trap. Returns a pointer to `Result(Result(T,E), str)`
    /// — one more layer of `Result` than `emit_decode_struct_json`
    /// because *this* function's own decode can fail for two independent
    /// reasons (malformed wrapper, or a bad inner payload) that both
    /// still need to report as "decoding the whole thing failed",
    /// distinct from the inner `Result(T,E)` value itself succeeding
    /// with a real `Err(e)` payload once decoded.
    pub(super) fn emit_decode_result_json(&mut self, ok_ty: &Ty, err_ty: &Ty, json: &str) -> Result<String, CodegenError> {
        let inner_result_ty = Ty::Named("Result".to_string(), vec![ok_ty.clone(), err_ty.clone()]);
        let inner_result_llty = self.llvm_ty(&inner_result_ty)?;
        let outer_result_ty = Ty::Named("Result".to_string(), vec![inner_result_ty.clone(), Ty::Str]);
        let outer_result_llty = self.llvm_ty(&outer_result_ty)?;
        let (json_ptr, json_len) = self.split_str_word(json);

        // Builds one `Ok(<inner Result(T,E) tagged as `variant_tag`,
        // payload `payload_llty`/`payload_val`>)` outcome for the outer
        // `Result` — shared by the "`ok` key decoded fine" and "`err`
        // key decoded fine" cases below, which differ only in which
        // inner tag/payload type they carry.
        let emit_outer_ok = |cg: &mut Self, variant_tag: i64, payload_llty: &str, payload_val: &str, label_prefix: &str| -> String {
            let inner_dest = cg.fresh_reg(&format!("{label_prefix}_inner_dest"));
            cg.emit_alloca(&inner_dest, &inner_result_llty);
            let inner_tag_ptr = cg.fresh_reg(&format!("{label_prefix}_inner_tag_ptr"));
            writeln!(cg.out, "  {inner_tag_ptr} = getelementptr inbounds {inner_result_llty}, ptr {inner_dest}, i32 0, i32 0").unwrap();
            writeln!(cg.out, "  store i64 {variant_tag}, ptr {inner_tag_ptr}").unwrap();
            let inner_payload_ptr = cg.fresh_reg(&format!("{label_prefix}_inner_payload_ptr"));
            writeln!(cg.out, "  {inner_payload_ptr} = getelementptr inbounds {inner_result_llty}, ptr {inner_dest}, i32 0, i32 1").unwrap();
            writeln!(cg.out, "  store {payload_llty} {payload_val}, ptr {inner_payload_ptr}").unwrap();
            let loaded_inner = cg.fresh_reg(&format!("{label_prefix}_inner_loaded"));
            writeln!(cg.out, "  {loaded_inner} = load {inner_result_llty}, ptr {inner_dest}").unwrap();
            let outer_dest = cg.fresh_reg(&format!("{label_prefix}_outer_dest"));
            cg.emit_alloca(&outer_dest, &outer_result_llty);
            let outer_tag_ptr = cg.fresh_reg(&format!("{label_prefix}_outer_tag_ptr"));
            writeln!(cg.out, "  {outer_tag_ptr} = getelementptr inbounds {outer_result_llty}, ptr {outer_dest}, i32 0, i32 0").unwrap();
            writeln!(cg.out, "  store i64 0, ptr {outer_tag_ptr}").unwrap();
            let outer_payload_ptr = cg.fresh_reg(&format!("{label_prefix}_outer_payload_ptr"));
            writeln!(cg.out, "  {outer_payload_ptr} = getelementptr inbounds {outer_result_llty}, ptr {outer_dest}, i32 0, i32 1").unwrap();
            writeln!(cg.out, "  store {inner_result_llty} {loaded_inner}, ptr {outer_payload_ptr}").unwrap();
            outer_dest
        };
        // Builds the outer `Err(message)` outcome — shared by every
        // failure path (malformed wrapper, bad `ok`/`err` payload).
        let emit_outer_err = |cg: &mut Self, message: &str, label_prefix: &str| -> String {
            let outer_dest = cg.fresh_reg(&format!("{label_prefix}_outer_dest"));
            cg.emit_alloca(&outer_dest, &outer_result_llty);
            let outer_tag_ptr = cg.fresh_reg(&format!("{label_prefix}_outer_tag_ptr"));
            writeln!(cg.out, "  {outer_tag_ptr} = getelementptr inbounds {outer_result_llty}, ptr {outer_dest}, i32 0, i32 0").unwrap();
            writeln!(cg.out, "  store i64 1, ptr {outer_tag_ptr}").unwrap();
            let outer_payload_ptr = cg.fresh_reg(&format!("{label_prefix}_outer_payload_ptr"));
            writeln!(cg.out, "  {outer_payload_ptr} = getelementptr inbounds {outer_result_llty}, ptr {outer_dest}, i32 0, i32 1").unwrap();
            writeln!(cg.out, "  store {{ptr, i64}} {message}, ptr {outer_payload_ptr}").unwrap();
            outer_dest
        };

        let ok_key = self.fresh_global("decode_json_result_ok_key");
        writeln!(self.string_globals, "{ok_key} = private unnamed_addr constant [2 x i8] c\"ok\"").unwrap();
        let ok_scratch = self.fresh_reg("decode_json_result_ok_scratch");
        self.emit_alloca(&ok_scratch, "{ptr, i64}");
        let ok_get_err = self.fresh_reg("decode_json_result_ok_get_err");
        self.emit_alloca(&ok_get_err, "{ptr, i64}");
        let has_ok = self.fresh_reg("decode_json_result_has_ok");
        writeln!(self.out, "  {has_ok} = call i32 @nir_json_get(ptr {json_ptr}, i64 {json_len}, ptr {ok_key}, i64 2, ptr {ok_scratch}, ptr {ok_get_err})").unwrap();
        let has_ok_bool = self.icmp("ne", "i32", &has_ok, "0")?;
        let is_ok_branch_label = self.fresh_label("decode_json_result_is_ok_branch");
        let check_err_label = self.fresh_label("decode_json_result_check_err");
        writeln!(self.out, "  br i1 {has_ok_bool}, label %{is_ok_branch_label}, label %{check_err_label}").unwrap();

        // `{"ok": v}` — decode `v` as `T`; a decode failure here is a
        // real top-level failure (bubbled up as the outer `Err`), not a
        // successfully-decoded `Err(...)` payload — the wire never says
        // "the `ok` field itself failed to parse" any other way.
        writeln!(self.out, "{is_ok_branch_label}:").unwrap();
        let ok_json = self.fresh_reg("decode_json_result_ok_json");
        writeln!(self.out, "  {ok_json} = load {{ptr, i64}}, ptr {ok_scratch}").unwrap();
        let ok_field_result = self.emit_decode_value_json(ok_ty, &ok_json)?;
        let ok_ty_result_llty = self.llvm_ty(&Ty::Named("Result".to_string(), vec![ok_ty.clone(), Ty::Str]))?;
        let ok_field_tag_ptr = self.fresh_reg("decode_json_result_ok_field_tag_ptr");
        writeln!(self.out, "  {ok_field_tag_ptr} = getelementptr inbounds {ok_ty_result_llty}, ptr {ok_field_result}, i32 0, i32 0").unwrap();
        let ok_field_tag = self.fresh_reg("decode_json_result_ok_field_tag");
        writeln!(self.out, "  {ok_field_tag} = load i64, ptr {ok_field_tag_ptr}").unwrap();
        let ok_field_payload_ptr = self.fresh_reg("decode_json_result_ok_field_payload_ptr");
        writeln!(self.out, "  {ok_field_payload_ptr} = getelementptr inbounds {ok_ty_result_llty}, ptr {ok_field_result}, i32 0, i32 1").unwrap();
        let ok_field_is_ok = self.icmp("eq", "i64", &ok_field_tag, "0")?;
        let ok_payload_good_label = self.fresh_label("decode_json_result_ok_payload_good");
        let ok_payload_bad_label = self.fresh_label("decode_json_result_ok_payload_bad");
        writeln!(self.out, "  br i1 {ok_field_is_ok}, label %{ok_payload_good_label}, label %{ok_payload_bad_label}").unwrap();

        writeln!(self.out, "{ok_payload_good_label}:").unwrap();
        let ok_llty = self.llvm_ty(ok_ty)?;
        let ok_payload_val = self.fresh_reg("decode_json_result_ok_payload_val");
        writeln!(self.out, "  {ok_payload_val} = load {ok_llty}, ptr {ok_field_payload_ptr}").unwrap();
        let dest_ok_good = emit_outer_ok(self, 0, &ok_llty, &ok_payload_val, "decode_json_result_ok_good");
        let merge_label = self.fresh_label("decode_json_result_merge");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{ok_payload_bad_label}:").unwrap();
        let ok_payload_err = self.fresh_reg("decode_json_result_ok_payload_err");
        writeln!(self.out, "  {ok_payload_err} = load {{ptr, i64}}, ptr {ok_field_payload_ptr}").unwrap();
        let dest_ok_bad = emit_outer_err(self, &ok_payload_err, "decode_json_result_ok_bad");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        // `{"err": v}`, or neither key — the `err` half mirrors `ok`
        // exactly (variant tag `1` instead of `0`); neither key present
        // is its own distinct top-level failure message.
        writeln!(self.out, "{check_err_label}:").unwrap();
        let err_key = self.fresh_global("decode_json_result_err_key");
        writeln!(self.string_globals, "{err_key} = private unnamed_addr constant [3 x i8] c\"err\"").unwrap();
        let err_scratch = self.fresh_reg("decode_json_result_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let err_get_err = self.fresh_reg("decode_json_result_err_get_err");
        self.emit_alloca(&err_get_err, "{ptr, i64}");
        let has_err = self.fresh_reg("decode_json_result_has_err");
        writeln!(self.out, "  {has_err} = call i32 @nir_json_get(ptr {json_ptr}, i64 {json_len}, ptr {err_key}, i64 3, ptr {err_scratch}, ptr {err_get_err})").unwrap();
        let has_err_bool = self.icmp("ne", "i32", &has_err, "0")?;
        let is_err_branch_label = self.fresh_label("decode_json_result_is_err_branch");
        let neither_label = self.fresh_label("decode_json_result_neither");
        writeln!(self.out, "  br i1 {has_err_bool}, label %{is_err_branch_label}, label %{neither_label}").unwrap();

        writeln!(self.out, "{is_err_branch_label}:").unwrap();
        let err_json = self.fresh_reg("decode_json_result_err_json");
        writeln!(self.out, "  {err_json} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        let err_field_result = self.emit_decode_value_json(err_ty, &err_json)?;
        let err_ty_result_llty = self.llvm_ty(&Ty::Named("Result".to_string(), vec![err_ty.clone(), Ty::Str]))?;
        let err_field_tag_ptr = self.fresh_reg("decode_json_result_err_field_tag_ptr");
        writeln!(self.out, "  {err_field_tag_ptr} = getelementptr inbounds {err_ty_result_llty}, ptr {err_field_result}, i32 0, i32 0").unwrap();
        let err_field_tag = self.fresh_reg("decode_json_result_err_field_tag");
        writeln!(self.out, "  {err_field_tag} = load i64, ptr {err_field_tag_ptr}").unwrap();
        let err_field_payload_ptr = self.fresh_reg("decode_json_result_err_field_payload_ptr");
        writeln!(self.out, "  {err_field_payload_ptr} = getelementptr inbounds {err_ty_result_llty}, ptr {err_field_result}, i32 0, i32 1").unwrap();
        let err_field_is_ok = self.icmp("eq", "i64", &err_field_tag, "0")?;
        let err_payload_good_label = self.fresh_label("decode_json_result_err_payload_good");
        let err_payload_bad_label = self.fresh_label("decode_json_result_err_payload_bad");
        writeln!(self.out, "  br i1 {err_field_is_ok}, label %{err_payload_good_label}, label %{err_payload_bad_label}").unwrap();

        writeln!(self.out, "{err_payload_good_label}:").unwrap();
        let err_llty = self.llvm_ty(err_ty)?;
        let err_payload_val = self.fresh_reg("decode_json_result_err_payload_val");
        writeln!(self.out, "  {err_payload_val} = load {err_llty}, ptr {err_field_payload_ptr}").unwrap();
        let dest_err_good = emit_outer_ok(self, 1, &err_llty, &err_payload_val, "decode_json_result_err_good");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_payload_bad_label}:").unwrap();
        let err_payload_err = self.fresh_reg("decode_json_result_err_payload_err");
        writeln!(self.out, "  {err_payload_err} = load {{ptr, i64}}, ptr {err_field_payload_ptr}").unwrap();
        let dest_err_bad = emit_outer_err(self, &err_payload_err, "decode_json_result_err_bad");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{neither_label}:").unwrap();
        let neither_msg = self.fresh_global("decode_json_result_neither_msg");
        const NEITHER_MSG: &str = "expected an object with an `ok` or `err` field";
        writeln!(self.string_globals, "{neither_msg} = private unnamed_addr constant [{} x i8] c\"{}\"", NEITHER_MSG.len(), llvm_escape_bytes(NEITHER_MSG.as_bytes())).unwrap();
        let neither_msg_partial = self.fresh_reg("decode_json_result_neither_msg_partial");
        writeln!(self.out, "  {neither_msg_partial} = insertvalue {{ptr, i64}} undef, ptr {neither_msg}, 0").unwrap();
        let neither_msg_full = self.fresh_reg("decode_json_result_neither_msg_full");
        writeln!(self.out, "  {neither_msg_full} = insertvalue {{ptr, i64}} {neither_msg_partial}, i64 {}, 1", NEITHER_MSG.len()).unwrap();
        let dest_neither = emit_outer_err(self, &neither_msg_full, "decode_json_result_neither");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let final_dest = self.fresh_reg("decode_json_result_final_dest");
        writeln!(
            self.out,
            "  {final_dest} = phi ptr [ {dest_ok_good}, %{ok_payload_good_label} ], [ {dest_ok_bad}, %{ok_payload_bad_label} ], [ {dest_err_good}, %{err_payload_good_label} ], [ {dest_err_bad}, %{err_payload_bad_label} ], [ {dest_neither}, %{neither_label} ]"
        )
        .unwrap();
        Ok(final_dest)
    }


    /// Decode a user-defined enum from a JSON string for zero-payload
    /// variants. The JSON text must be one of the variant names (e.g.
    /// `"Card"`) -- no surrounding object. Returns a pointer to a
    /// `Result(enum_ty, str)`.
    pub(super) fn emit_decode_enum_json(
        &mut self,
        enum_name: &str,
        type_args: &[Ty],
        json: &str,
    ) -> Result<String, CodegenError> {
        let variants = self
            .registry
            .enum_variants(enum_name)
            .expect("caller already confirmed this is an enum")
            .to_vec();
        let type_params = self.registry.enum_type_params(enum_name).unwrap_or(&[]);
        let subst = zip_type_params(type_params, type_args);

        if variants.iter().any(|v| !v.payload.is_empty()) {
            return unsupported(format!(
                "decoding enum `{enum_name}` from JSON: payload-carrying enum variants are not supported yet by compiled `serve` Stage 1"
            ));
        }

        let enum_ty = Ty::Named(enum_name.to_string(), type_args.to_vec());
        let enum_llty = self.llvm_ty(&enum_ty)?;
        let result_ty = Ty::Named("Result".to_string(), vec![enum_ty.clone(), Ty::Str]);
        let result_llty = self.llvm_ty(&result_ty)?;

        let (json_ptr, json_len) = self.split_str_word(json);

        let value_scratch = self.fresh_reg("decode_enum_str_value_scratch");
        self.emit_alloca(&value_scratch, "{ptr, i64}");
        let str_err_scratch = self.fresh_reg("decode_enum_str_err_scratch");
        self.emit_alloca(&str_err_scratch, "{ptr, i64}");
        let str_found = self.fresh_reg("decode_enum_str_found");
        writeln!(
            self.out,
            "  {str_found} = call i32 @nir_json_decode_str(ptr {json_ptr}, i64 {json_len}, ptr {value_scratch}, ptr {str_err_scratch})"
        )
        .unwrap();
        let str_is_ok = self.icmp("ne", "i32", &str_found, "0")?;
        let str_ok_label = self.fresh_label("decode_enum_str_ok");
        let str_fail_label = self.fresh_label("decode_enum_str_fail");
        writeln!(self.out, "  br i1 {str_is_ok}, label %{str_ok_label}, label %{str_fail_label}").unwrap();

        writeln!(self.out, "{str_fail_label}:").unwrap();
        let str_err_val = self.fresh_reg("decode_enum_str_err_val");
        writeln!(self.out, "  {str_err_val} = load {{ptr, i64}}, ptr {str_err_scratch}").unwrap();
        let dest_str_fail = self.fresh_reg("decode_enum_dest_str_fail");
        self.emit_alloca(&dest_str_fail, &result_llty);
        let tag_ptr = self.fresh_reg("decode_enum_str_fail_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest_str_fail}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let payload_ptr = self.fresh_reg("decode_enum_str_fail_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest_str_fail}, i32 0, i32 1").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {str_err_val}, ptr {payload_ptr}").unwrap();
        let merge_label = self.fresh_label("decode_enum_merge");
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{str_ok_label}:").unwrap();
        let str_val = self.fresh_reg("decode_enum_str_val");
        writeln!(self.out, "  {str_val} = load {{ptr, i64}}, ptr {value_scratch}").unwrap();
        let actual_ptr = self.fresh_reg("decode_enum_actual_ptr");
        writeln!(self.out, "  {actual_ptr} = extractvalue {{ptr, i64}} {str_val}, 0").unwrap();
        let actual_len = self.fresh_reg("decode_enum_actual_len");
        writeln!(self.out, "  {actual_len} = extractvalue {{ptr, i64}} {str_val}, 1").unwrap();

        let mut cmp_labels = Vec::new();
        for i in 0..variants.len() {
            cmp_labels.push(self.fresh_label(&format!("decode_enum_cmp_{i}")));
        }
        let no_match_label = self.fresh_label("decode_enum_no_match");
        writeln!(self.out, "  br label %{}", cmp_labels[0]).unwrap();

        let mut ok_dests = Vec::new();
        let mut ok_labels = Vec::new();
        for (i, v) in variants.iter().enumerate() {
            let cmp_label = &cmp_labels[i];
            let next_label = cmp_labels.get(i + 1).unwrap_or(&no_match_label);
            writeln!(self.out, "{cmp_label}:").unwrap();
            let expected_global = self.fresh_global(&format!("decode_enum_expected_{i}"));
            let escaped = llvm_escape_bytes(v.name.as_bytes());
            writeln!(
                self.string_globals,
                "{expected_global} = private unnamed_addr constant [{} x i8] c\"{escaped}\"",
                v.name.len()
            )
            .unwrap();
            let eq = self.fresh_reg(&format!("decode_enum_eq_{i}"));
            writeln!(
                self.out,
                "  {eq} = call i32 @nir_str_eq(ptr {actual_ptr}, i64 {actual_len}, ptr {expected_global}, i64 {})",
                v.name.len()
            )
            .unwrap();
            let is_match = self.icmp("ne", "i32", &eq, "0")?;
            let match_label = self.fresh_label(&format!("decode_enum_match_{i}"));
            writeln!(self.out, "  br i1 {is_match}, label %{match_label}, label %{next_label}").unwrap();

            writeln!(self.out, "{match_label}:").unwrap();
            let dest = self.fresh_reg(&format!("decode_enum_dest_{i}"));
            self.emit_alloca(&dest, &result_llty);
            let tag_ptr = self.fresh_reg(&format!("decode_enum_tag_ptr_{i}"));
            writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
            writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
            let enum_dest_ptr = self.fresh_reg(&format!("decode_enum_val_ptr_{i}"));
            self.emit_alloca(&enum_dest_ptr, &enum_llty);
            let enum_tag_ptr = self.fresh_reg(&format!("decode_enum_enum_tag_ptr_{i}"));
            writeln!(self.out, "  {enum_tag_ptr} = getelementptr inbounds {enum_llty}, ptr {enum_dest_ptr}, i32 0, i32 0").unwrap();
            writeln!(self.out, "  store i64 {i}, ptr {enum_tag_ptr}").unwrap();
            let payload_ptr = self.fresh_reg(&format!("decode_enum_payload_ptr_{i}"));
            writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {enum_llty}, ptr {enum_dest_ptr}, i32 0, i32 1").unwrap();
            let payload_words = variants[i]
                .payload
                .iter()
                .map(|t| conservative_word_count(&substitute_ty(t, &subst), &self.registry))
                .sum::<u64>();
            let words = std::cmp::max(1, payload_words);
            writeln!(self.out, "  call void @llvm.memset.p0.i64(ptr {payload_ptr}, i8 0, i64 {}, i1 false)", words * 8).unwrap();
            let enum_val = self.fresh_reg(&format!("decode_enum_val_{i}"));
            writeln!(self.out, "  {enum_val} = load {enum_llty}, ptr {enum_dest_ptr}").unwrap();
            let result_payload_ptr = self.fresh_reg(&format!("decode_enum_result_payload_ptr_{i}"));
            writeln!(self.out, "  {result_payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();
            writeln!(self.out, "  store {enum_llty} {enum_val}, ptr {result_payload_ptr}").unwrap();
            ok_dests.push(dest.clone());
            ok_labels.push(match_label.clone());
            writeln!(self.out, "  br label %{merge_label}").unwrap();
        }

        writeln!(self.out, "{no_match_label}:").unwrap();
        let no_match_msg = self.fresh_global("decode_enum_no_match_msg");
        const NO_MATCH_MSG: &str = "value is not a known enum variant";
        writeln!(
            self.string_globals,
            "{no_match_msg} = private unnamed_addr constant [{} x i8] c\"{}\"",
            NO_MATCH_MSG.len(),
            llvm_escape_bytes(NO_MATCH_MSG.as_bytes())
        )
        .unwrap();
        let no_match_partial = self.fresh_reg("decode_enum_no_match_partial");
        writeln!(self.out, "  {no_match_partial} = insertvalue {{ptr, i64}} undef, ptr {no_match_msg}, 0").unwrap();
        let no_match_full = self.fresh_reg("decode_enum_no_match_full");
        writeln!(self.out, "  {no_match_full} = insertvalue {{ptr, i64}} {no_match_partial}, i64 {}, 1", NO_MATCH_MSG.len()).unwrap();
        let dest_no_match = self.fresh_reg("decode_enum_dest_no_match");
        self.emit_alloca(&dest_no_match, &result_llty);
        let tag_ptr = self.fresh_reg("decode_enum_no_match_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest_no_match}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let payload_ptr = self.fresh_reg("decode_enum_no_match_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest_no_match}, i32 0, i32 1").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {no_match_full}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        let final_dest = self.fresh_reg("decode_enum_final_dest");
        let mut phi_pairs: Vec<String> = ok_dests
            .iter()
            .zip(ok_labels.iter())
            .map(|(d, l)| format!("[ {d}, %{l} ]"))
            .collect();
        phi_pairs.push(format!("[ {dest_str_fail}, %{str_fail_label} ]"));
        phi_pairs.push(format!("[ {dest_no_match}, %{no_match_label} ]"));
        writeln!(self.out, "  {final_dest} = phi ptr {}", phi_pairs.join(", ")).unwrap();
        Ok(final_dest)
    }

    // ==== Reviving compiled `serve` (rfcs/0010), Stage 3: per-route ====
    // ==== wrapper codegen, dispatch table, `requires` enforcement.  ====


    /// Given a value's own `Ty` and a pointer to it, produces the
    /// operand string `emit_call_known_fn` needs for that argument —
    /// `call_args`'s exact scalar-vs-pointer convention
    /// (`Ty::is_aggregate()`), just driven by an already-materialized
    /// pointer instead of an `Expr` to evaluate (a `--serve` route
    /// wrapper's arguments come from JSON-decoding, not source syntax).
    pub(super) fn serve_call_operand(&mut self, ty: &Ty, ptr: &str) -> Result<String, CodegenError> {
        if ty.is_aggregate() {
            return Ok(format!("ptr {ptr}"));
        }
        let llty = self.llvm_ty(ty)?;
        let v = self.fresh_reg("serve_call_arg");
        writeln!(self.out, "  {v} = load {llty}, ptr {ptr}").unwrap();
        Ok(format!("{llty} {v}"))
    }


    /// Calls a statically-known top-level `fn` (resolved via `self.sigs`
    /// the same way `call`/`call_ptr` already do) given already-
    /// formatted operand strings — the one piece `call`/`call_ptr`
    /// can't be reused for directly, since both evaluate their own
    /// arguments from `&[Expr]`. Mirrors their exact aggregate-vs-scalar
    /// return convention (sret out-pointer vs. a plain typed return
    /// value) — see `call_ptr`'s own generic fallback (`codegen.rs`
    /// ~5862-5872) and `call`'s (~4759-4767), which this is a byte-for-
    /// byte match of, minus the `&[Expr]` evaluation neither needs here.
    pub(super) fn emit_call_known_fn(&mut self, fn_name: &str, arg_operands: &[String], sig_ret: &Ty) -> Result<String, CodegenError> {
        if sig_ret.is_aggregate() {
            let agg_llty = self.llvm_ty(sig_ret)?;
            let dest = self.fresh_reg("serve_call_result_addr");
            self.emit_alloca(&dest, &agg_llty);
            let mut all_args = vec![format!("ptr {dest}")];
            all_args.extend(arg_operands.iter().cloned());
            writeln!(self.out, "  call void @{fn_name}({})", all_args.join(", ")).unwrap();
            Ok(dest)
        } else {
            let ret_llty = self.llvm_ty(sig_ret)?;
            if ret_llty == "void" {
                writeln!(self.out, "  call void @{fn_name}({})", arg_operands.join(", ")).unwrap();
                Ok("0".to_string())
            } else {
                let r = self.fresh_reg("serve_call_result");
                writeln!(self.out, "  {r} = call {ret_llty} @{fn_name}({})", arg_operands.join(", ")).unwrap();
                Ok(r)
            }
        }
    }


    /// Writes a real `Option(VerifiedIdentity)` value into the
    /// already-allocated slot `dest` — `Some` (tag `0`, the payload's
    /// first word holding a copy of the already-decoded
    /// `VerifiedIdentity` at `some_ptr`) or `None` (tag `1`, payload
    /// left unwritten), matching `ast::prelude_enums`' own `Option`
    /// variant order and `construct_variant`'s tag/payload-buffer
    /// layout. Writes into a *given* slot rather than allocating and
    /// returning a fresh one — the caller's two branches (identity
    /// present/absent) both funnel into one shared `dest` and jump
    /// unconditionally to a merge block that just reads it back,
    /// deliberately avoiding a `phi` whose predecessor-block name would
    /// otherwise have to track whichever *inner* label a nested decode
    /// call last opened, not the literal branch-arm label — the same
    /// "output slot instead of phi" shape used throughout this section
    /// for exactly that reason.
    pub(super) fn emit_store_option_verified_identity(&mut self, dest: &str, some_ptr: Option<&str>) -> Result<(), CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let option_ty = Ty::Named("Option".to_string(), vec![identity_ty.clone()]);
        let option_llty = self.llvm_ty(&option_ty)?;
        let tag_ptr = self.fresh_reg("serve_option_identity_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {option_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        match some_ptr {
            Some(identity_ptr) => {
                writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
                let payload_ptr = self.fresh_reg("serve_option_identity_payload_ptr");
                writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {option_llty}, ptr {dest}, i32 0, i32 1").unwrap();
                let bytes = agg_byte_size_operand(&identity_ty, &self.registry);
                writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {identity_ptr}, i64 {bytes}, i1 false)").unwrap();
            }
            None => {
                writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
            }
        }
        Ok(())
    }


    /// `out_body_ptr`/`out_body_len` = `message_json` (a `{ptr,i64}` SSA
    /// value already holding the *whole* JSON body to write — usually
    /// `emit_wrap_json_field("err", ...)`'s own output), then
    /// `ret i32 1` — `RouteHandler`'s own documented "business-level
    /// error" shape: a malformed/missing request argument and an
    /// ordinary `Err(...)` returned by `f` itself both end up here, by
    /// design (from the HTTP caller's point of view, both are just "the
    /// request didn't succeed, here's why").
    pub(super) fn emit_serve_return_business_error(&mut self, message_json: &str) {
        let ptr_reg = self.fresh_reg("serve_err_body_ptr");
        writeln!(self.out, "  {ptr_reg} = extractvalue {{ptr, i64}} {message_json}, 0").unwrap();
        let len_reg = self.fresh_reg("serve_err_body_len");
        writeln!(self.out, "  {len_reg} = extractvalue {{ptr, i64}} {message_json}, 1").unwrap();
        writeln!(self.out, "  store ptr {ptr_reg}, ptr %out_body_ptr").unwrap();
        writeln!(self.out, "  store i64 {len_reg}, ptr %out_body_len").unwrap();
        writeln!(self.out, "  ret i32 1").unwrap();
    }


    /// `RouteHandler`'s documented `2` (unauthorized/forbidden) —
    /// `out_body` ignored per that same doc comment, so this writes a
    /// null/zero body rather than a real error message (the crate
    /// answering the HTTP request writes a bare `401`/`403` itself,
    /// `compiled_serve::call_route`'s own status mapping).
    pub(super) fn emit_serve_return_unauthorized(&mut self) {
        writeln!(self.out, "  store ptr null, ptr %out_body_ptr").unwrap();
        writeln!(self.out, "  store i64 0, ptr %out_body_len").unwrap();
        writeln!(self.out, "  ret i32 2").unwrap();
    }


    /// Branches on `found_i32 != 0`; on failure, JSON-wraps
    /// `err_message` (a `{ptr,i64}` SSA value) as `{"err": ...}` and
    /// returns business-error code `1` (`emit_serve_return_business_error`)
    /// — a real `ret`, so nothing after this call in the *fail* arm
    /// ever executes. On success, this simply opens a fresh block and
    /// returns — the caller keeps emitting there, exactly the "terminate
    /// the fail arm, keep building on the pass arm" shape `emit_c_main`'s
    /// own transact-log-init failure check already uses.
    pub(super) fn serve_require_ok_or_business_error(&mut self, found_i32: &str, err_message: &str, label_prefix: &str) -> Result<(), CodegenError> {
        let ok = self.icmp("ne", "i32", found_i32, "0")?;
        let pass_label = self.fresh_label(&format!("{label_prefix}_pass"));
        let fail_label = self.fresh_label(&format!("{label_prefix}_fail"));
        writeln!(self.out, "  br i1 {ok}, label %{pass_label}, label %{fail_label}").unwrap();
        writeln!(self.out, "{fail_label}:").unwrap();
        let wrapped = self.emit_wrap_json_field("err", err_message, &format!("{label_prefix}_wrap"))?;
        self.emit_serve_return_business_error(&wrapped);
        writeln!(self.out, "{pass_label}:").unwrap();
        Ok(())
    }


    /// Unwraps a `Result(ty, str)` pointer (`emit_decode_value_json`'s
    /// own return shape): on `Err`, JSON-wraps the message and returns
    /// business-error `1` (same helper as above); on `Ok`, returns a
    /// pointer to the payload *in place* (a GEP into `result_ptr`'s own
    /// field 1, not a copy) — valid to hand straight to
    /// `serve_call_operand`/`emit_encode_value_json` the same way any
    /// other addressable storage would be, since the callee (or the
    /// encoder) only ever reads through it or copies out of it.
    pub(super) fn emit_decode_or_business_error(&mut self, ty: &Ty, result_ptr: &str, label_prefix: &str) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![ty.clone(), Ty::Str]);
        let result_llty = self.llvm_ty(&result_ty)?;
        let tag_ptr = self.fresh_reg(&format!("{label_prefix}_tag_ptr"));
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {result_ptr}, i32 0, i32 0").unwrap();
        let tag = self.fresh_reg(&format!("{label_prefix}_tag"));
        writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
        let payload_ptr = self.fresh_reg(&format!("{label_prefix}_payload_ptr"));
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {result_ptr}, i32 0, i32 1").unwrap();
        let is_ok = self.icmp("eq", "i64", &tag, "0")?;
        let pass_label = self.fresh_label(&format!("{label_prefix}_pass"));
        let fail_label = self.fresh_label(&format!("{label_prefix}_fail"));
        writeln!(self.out, "  br i1 {is_ok}, label %{pass_label}, label %{fail_label}").unwrap();
        writeln!(self.out, "{fail_label}:").unwrap();
        let err_val = self.fresh_reg(&format!("{label_prefix}_err_val"));
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {payload_ptr}").unwrap();
        let wrapped = self.emit_wrap_json_field("err", &err_val, &format!("{label_prefix}_wrap"))?;
        self.emit_serve_return_business_error(&wrapped);
        writeln!(self.out, "{pass_label}:").unwrap();
        Ok(payload_ptr)
    }


    /// Builds the `{ptr, i64}` SSA value for `%identity_json_ptr`/
    /// `%identity_json_len` (the wrapper's own function parameters) —
    /// shared by every place that needs to decode the whole identity
    /// blob as a `str`-shaped value (`emit_decode_value_json` takes a
    /// `{ptr,i64}` value, never split pointer/length parameters).
    pub(super) fn serve_identity_json_value(&mut self) -> String {
        let v0 = self.fresh_reg("serve_identity_json_v0");
        writeln!(self.out, "  {v0} = insertvalue {{ptr, i64}} undef, ptr %identity_json_ptr, 0").unwrap();
        let v1 = self.fresh_reg("serve_identity_json_v1");
        writeln!(self.out, "  {v1} = insertvalue {{ptr, i64}} {v0}, i64 %identity_json_len, 1").unwrap();
        v1
    }


    /// Emits one route wrapper for `f` (already confirmed to be in the
    /// program's HTTP exposure set — `typeck::exposed_fn_names`,
    /// `rfcs/0010-landing-and-serve-exposure.md`) matching
    /// `compiled_serve::RouteHandler`'s exact ABI. Three things happen
    /// here that never happen inside `f`'s own compiled body:
    ///
    /// - `f.requires` is enforced *at this call boundary* — the
    ///   deleted interpreted `serve.rs`'s own module doc explains why a
    ///   `requires`-gated function's own body can't do this itself
    ///   (the gate is structural, checked wherever `acquire` happens to
    ///   be written in `.nir` source, not tied to how the function was
    ///   *reached*) — so this wrapper does the equivalent check itself,
    ///   independently, reusing the exact same compiled
    ///   `nir_check_role`/`nir_extract_claim` kernels `acquire`'s own
    ///   codegen (`emit_check_role`/`emit_extract_claim`) already calls.
    /// - `args_json`'s positional elements are decoded into `f`'s real
    ///   parameter types (Stage 1's `emit_decode_value_json`) —
    ///   *skipping* any `VerifiedIdentity`/`Option(VerifiedIdentity)`
    ///   parameter, which is filled from `identity_json` instead, never
    ///   from the request body (`typeck::is_verified_identity`/
    ///   `is_optional_verified_identity`, the same predicates
    ///   `is_reachable_with_no_token` already uses for this exact
    ///   distinction) -- and, as of a real red-team finding
    ///   (2026-09-11), *also* skipping any `RoleView`/`ClaimView`
    ///   parameter, filled instead from whichever `f.requires` check
    ///   above already verified against the real identity
    ///   (`verified_role_view_val`/`verified_claim_view_val`). Before
    ///   this fix, `RoleView`/`ClaimView` fell through to the same
    ///   generic decode path as any other struct parameter, meaning a
    ///   client could supply `[{"role":"admin"}]` in the request body
    ///   and construct an **arbitrary, self-asserted `RoleView`** for a
    ///   route that only proved the caller's *real* identity held
    ///   `hr_staff` -- a full bypass of field-level masking
    ///   (`requires(role: "admin")` on a field) despite `RoleView`
    ///   being, everywhere else in this language, unforgeable by
    ///   construction. `typeck::check_serve_exposure`'s
    ///   `ExposedFnRoleViewParamUnverifiable` refuses to compile a
    ///   program that exposes a `RoleView`/`ClaimView` parameter with no
    ///   matching `requires` to anchor it, so the `.expect()`s below are
    ///   sound, not merely hopeful.
    /// - `f`'s return value is JSON-encoded back out (Stage 1's
    ///   `emit_encode_value_json`), `Result(_, _)`'s own tag driving
    ///   the `0`/`1` status `compiled_serve::call_route` already
    ///   expects — a plain (non-`Result`) return is always `0`.
    pub(super) fn emit_serve_route_wrapper(&mut self, f: &FnDecl) -> Result<String, CodegenError> {
        let wrapper_name = format!("__serve_route_{}", f.name);
        writeln!(
            self.out,
            "define i32 @{wrapper_name}(ptr %args_json_ptr, i64 %args_json_len, ptr %identity_json_ptr, i64 %identity_json_len, ptr %out_body_ptr, ptr %out_body_len, ptr %out_cookie_ptr, ptr %out_cookie_len) {{"
        )
        .unwrap();
        writeln!(self.out, "entry:").unwrap();
        let alloca_splice_pos = self.out.len();
        self.entry_allocas.clear();
        self.terminated = false;

        // No route wrapper in this pass ever sets a session cookie
        // (`ApplicationSession`/`session_cookie` is a separate, real
        // follow-up feature) — always null/zero, matching
        // `RouteHandler`'s own "null/zero-length for no Set-Cookie"
        // contract.
        writeln!(self.out, "  store ptr null, ptr %out_cookie_ptr").unwrap();
        writeln!(self.out, "  store i64 0, ptr %out_cookie_len").unwrap();

        let identity_present = self.icmp("sgt", "i64", "%identity_json_len", "0")?;

        // Set inside the matching `Requirement::Role`/`Requirement::Claim`
        // arm below, once that check has actually passed -- the *only*
        // place a `RoleView`/`ClaimView` value is ever allowed to come
        // from in this wrapper. `typeck::check_serve_exposure`'s
        // `ExposedFnRoleViewParamUnverifiable` already guarantees any
        // `RoleView`/`ClaimView`-typed parameter has a matching
        // `requires` of the right kind, so the parameter loop below can
        // `.expect()` these unconditionally rather than silently falling
        // back to decoding a caller-suppliable value from the request
        // body -- exactly the red-team-confirmed bypass
        // (`[{"role":"admin"}]` in the JSON body previously overrode a
        // real `hr_staff`-only identity's masking) this whole mechanism
        // exists to close.
        let mut verified_role_view_val: Option<String> = None;
        let mut verified_claim_view_val: Option<String> = None;

        // ---- `f.requires` enforcement, independent of whether `f`
        // itself declares a `VerifiedIdentity`/`Option(VerifiedIdentity)`
        // parameter at all. ----
        if let Some(req) = &f.requires {
            let has_token_label = self.fresh_label("serve_requires_has_token");
            let no_token_label = self.fresh_label("serve_requires_no_token");
            writeln!(self.out, "  br i1 {identity_present}, label %{has_token_label}, label %{no_token_label}").unwrap();
            writeln!(self.out, "{no_token_label}:").unwrap();
            self.emit_serve_return_unauthorized();
            writeln!(self.out, "{has_token_label}:").unwrap();

            // `identity_json`'s own `claims_json` field (a JSON *string*
            // — `compiled_serve::identity::identity_json`'s own doc
            // comment) is exactly the raw claims text
            // `nir_check_role`/`nir_extract_claim` already expect —
            // `nir_json_get_str` reads it out directly, no full struct
            // decode needed just for this one field.
            let key = "claims_json";
            let key_global = self.fresh_global("serve_requires_claims_key");
            writeln!(self.string_globals, "{key_global} = private unnamed_addr constant [{} x i8] c\"{key}\"", key.len()).unwrap();
            let value_scratch = self.fresh_reg("serve_requires_claims_scratch");
            self.emit_alloca(&value_scratch, "{ptr, i64}");
            let err_scratch = self.fresh_reg("serve_requires_claims_err_scratch");
            self.emit_alloca(&err_scratch, "{ptr, i64}");
            writeln!(
                self.out,
                "  call i32 @nir_json_get_str(ptr %identity_json_ptr, i64 %identity_json_len, ptr {key_global}, i64 {}, ptr {value_scratch}, ptr {err_scratch})",
                key.len()
            )
            .unwrap();
            let claims_val = self.fresh_reg("serve_requires_claims_val");
            writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {value_scratch}").unwrap();
            let claims_ptr_reg = self.fresh_reg("serve_requires_claims_ptr");
            writeln!(self.out, "  {claims_ptr_reg} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
            let claims_len_reg = self.fresh_reg("serve_requires_claims_len");
            writeln!(self.out, "  {claims_len_reg} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();

            match req {
                Requirement::Role(role) => {
                    let role_global = self.fresh_global("serve_requires_role");
                    writeln!(self.string_globals, "{role_global} = private unnamed_addr constant [{} x i8] c\"{}\"", role.len(), llvm_escape_bytes(role.as_bytes())).unwrap();
                    let found = self.fresh_reg("serve_requires_role_found");
                    writeln!(
                        self.out,
                        "  {found} = call i32 @nir_check_role(ptr {claims_ptr_reg}, i64 {claims_len_reg}, ptr {role_global}, i64 {})",
                        role.len()
                    )
                    .unwrap();
                    let ok = self.icmp("ne", "i32", &found, "0")?;
                    let pass_label = self.fresh_label("serve_requires_role_pass");
                    let fail_label = self.fresh_label("serve_requires_role_fail");
                    writeln!(self.out, "  br i1 {ok}, label %{pass_label}, label %{fail_label}").unwrap();
                    writeln!(self.out, "{fail_label}:").unwrap();
                    self.emit_serve_return_unauthorized();
                    writeln!(self.out, "{pass_label}:").unwrap();

                    // The real `RoleView` this wrapper is ever allowed to
                    // hand to `f` -- built from `role_global`/`role.len()`
                    // (the *literal role text this exact check just
                    // verified against the caller's real identity*),
                    // never from anything client-suppliable. Same
                    // `{ptr, i64}`-via-`insertvalue` shape a plain `str`
                    // literal expression already uses elsewhere in this
                    // module (`RoleView`'s sole field is `role: str`, so
                    // its value representation *is* this pair, per
                    // `emit_check_role`'s own doc comment).
                    let role_partial = self.fresh_reg("serve_verified_role_view_partial");
                    writeln!(self.out, "  {role_partial} = insertvalue {{ptr, i64}} undef, ptr {role_global}, 0").unwrap();
                    let role_full = self.fresh_reg("serve_verified_role_view");
                    writeln!(self.out, "  {role_full} = insertvalue {{ptr, i64}} {role_partial}, i64 {}, 1", role.len()).unwrap();
                    verified_role_view_val = Some(role_full);
                }
                Requirement::Claim(key, expected_value) => {
                    let key_global = self.fresh_global("serve_requires_claim_key");
                    writeln!(self.string_globals, "{key_global} = private unnamed_addr constant [{} x i8] c\"{}\"", key.len(), llvm_escape_bytes(key.as_bytes())).unwrap();
                    let value_scratch2 = self.fresh_reg("serve_requires_claim_value_scratch");
                    self.emit_alloca(&value_scratch2, "{ptr, i64}");
                    let found = self.fresh_reg("serve_requires_claim_found");
                    writeln!(
                        self.out,
                        "  {found} = call i32 @nir_extract_claim(ptr {claims_ptr_reg}, i64 {claims_len_reg}, ptr {key_global}, i64 {}, ptr {value_scratch2})",
                        key.len()
                    )
                    .unwrap();
                    let has_claim = self.icmp("ne", "i32", &found, "0")?;
                    let check_value_label = self.fresh_label("serve_requires_claim_check_value");
                    let fail_label = self.fresh_label("serve_requires_claim_fail");
                    writeln!(self.out, "  br i1 {has_claim}, label %{check_value_label}, label %{fail_label}").unwrap();

                    writeln!(self.out, "{check_value_label}:").unwrap();
                    let actual_val = self.fresh_reg("serve_requires_claim_actual");
                    writeln!(self.out, "  {actual_val} = load {{ptr, i64}}, ptr {value_scratch2}").unwrap();
                    let actual_ptr = self.fresh_reg("serve_requires_claim_actual_ptr");
                    writeln!(self.out, "  {actual_ptr} = extractvalue {{ptr, i64}} {actual_val}, 0").unwrap();
                    let actual_len = self.fresh_reg("serve_requires_claim_actual_len");
                    writeln!(self.out, "  {actual_len} = extractvalue {{ptr, i64}} {actual_val}, 1").unwrap();
                    let expected_global = self.fresh_global("serve_requires_claim_expected");
                    writeln!(
                        self.string_globals,
                        "{expected_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
                        expected_value.len(),
                        llvm_escape_bytes(expected_value.as_bytes())
                    )
                    .unwrap();
                    let eq = self.fresh_reg("serve_requires_claim_eq");
                    writeln!(
                        self.out,
                        "  {eq} = call i32 @nir_str_eq(ptr {actual_ptr}, i64 {actual_len}, ptr {expected_global}, i64 {})",
                        expected_value.len()
                    )
                    .unwrap();
                    let matches = self.icmp("ne", "i32", &eq, "0")?;
                    let pass_label = self.fresh_label("serve_requires_claim_pass");
                    writeln!(self.out, "  br i1 {matches}, label %{pass_label}, label %{fail_label}").unwrap();
                    writeln!(self.out, "{fail_label}:").unwrap();
                    self.emit_serve_return_unauthorized();
                    writeln!(self.out, "{pass_label}:").unwrap();

                    // The real `ClaimView` this wrapper is ever allowed
                    // to hand to `f` -- built from `actual_ptr`/
                    // `actual_len`, the claim value *this exact check
                    // just extracted from the caller's real identity and
                    // confirmed equals `expected_value`* (not
                    // `expected_global`/`expected_value` directly, though
                    // they're equal by this point -- using the extracted
                    // value keeps this honestly "what the identity
                    // actually said," not "what the source code expected
                    // it to say"). Never from anything client-suppliable.
                    let claim_partial = self.fresh_reg("serve_verified_claim_view_partial");
                    writeln!(self.out, "  {claim_partial} = insertvalue {{ptr, i64}} undef, ptr {actual_ptr}, 0").unwrap();
                    let claim_full = self.fresh_reg("serve_verified_claim_view");
                    writeln!(self.out, "  {claim_full} = insertvalue {{ptr, i64}} {claim_partial}, i64 {actual_len}, 1").unwrap();
                    verified_claim_view_val = Some(claim_full);
                }
            }
        }

        // ---- decode `f`'s real parameters ----
        let mut call_operands: Vec<String> = Vec::new();
        let mut json_idx: i64 = 0;
        for p in &f.params {
            let is_role_view = matches!(&p.ty, Ty::Named(n, args) if n == "RoleView" && args.is_empty());
            let is_claim_view = matches!(&p.ty, Ty::Named(n, args) if n == "ClaimView" && args.is_empty());
            if is_role_view || is_claim_view {
                // Never decoded from `args_json` -- see this function's
                // own doc comment and `verified_role_view_val`/
                // `verified_claim_view_val`'s. `typeck::check_serve_exposure`
                // already refused to compile this program at all if `f`
                // has one of these parameters without the matching
                // `requires`, so exactly one of the two `.expect()`s
                // below is live for any program that reaches codegen.
                let (verified_val, ty_name) = if is_role_view {
                    (verified_role_view_val.as_deref().expect("typeck guarantees a RoleView param has a matching requires(role: ...)"), "RoleView")
                } else {
                    (verified_claim_view_val.as_deref().expect("typeck guarantees a ClaimView param has a matching requires(claim: ...)"), "ClaimView")
                };
                let slot = self.fresh_reg("serve_verified_proof_slot");
                let proof_ty = Ty::Named(ty_name.to_string(), vec![]);
                let proof_llty = self.llvm_ty(&proof_ty)?;
                self.emit_alloca(&slot, &proof_llty);
                writeln!(self.out, "  store {{ptr, i64}} {verified_val}, ptr {slot}").unwrap();
                call_operands.push(format!("ptr {slot}"));
                continue;
            }
            if is_verified_identity(&p.ty) {
                let fail_label = self.fresh_label("serve_identity_required_fail");
                let ok_label = self.fresh_label("serve_identity_required_ok");
                writeln!(self.out, "  br i1 {identity_present}, label %{ok_label}, label %{fail_label}").unwrap();
                writeln!(self.out, "{fail_label}:").unwrap();
                self.emit_serve_return_unauthorized();
                writeln!(self.out, "{ok_label}:").unwrap();
                let identity_json_val = self.serve_identity_json_value();
                let decoded = self.emit_decode_value_json(&p.ty, &identity_json_val)?;
                let payload_ptr = self.emit_decode_or_business_error(&p.ty, &decoded, &format!("serve_identity_req{json_idx}"))?;
                call_operands.push(self.serve_call_operand(&p.ty, &payload_ptr)?);
            } else if is_optional_verified_identity(&p.ty) {
                let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
                let option_ty = Ty::Named("Option".to_string(), vec![identity_ty.clone()]);
                let option_llty = self.llvm_ty(&option_ty)?;
                let out_slot = self.fresh_reg("serve_identity_opt_slot");
                self.emit_alloca(&out_slot, &option_llty);
                let some_label = self.fresh_label("serve_identity_opt_some");
                let none_label = self.fresh_label("serve_identity_opt_none");
                let merge_label = self.fresh_label("serve_identity_opt_merge");
                writeln!(self.out, "  br i1 {identity_present}, label %{some_label}, label %{none_label}").unwrap();

                writeln!(self.out, "{some_label}:").unwrap();
                let identity_json_val = self.serve_identity_json_value();
                let decoded = self.emit_decode_value_json(&identity_ty, &identity_json_val)?;
                let identity_payload_ptr = self.emit_decode_or_business_error(&identity_ty, &decoded, &format!("serve_identity_opt{json_idx}"))?;
                self.emit_store_option_verified_identity(&out_slot, Some(&identity_payload_ptr))?;
                writeln!(self.out, "  br label %{merge_label}").unwrap();

                writeln!(self.out, "{none_label}:").unwrap();
                self.emit_store_option_verified_identity(&out_slot, None)?;
                writeln!(self.out, "  br label %{merge_label}").unwrap();

                writeln!(self.out, "{merge_label}:").unwrap();
                call_operands.push(format!("ptr {out_slot}"));
            } else {
                let json_scratch = self.fresh_reg("serve_arg_json_scratch");
                self.emit_alloca(&json_scratch, "{ptr, i64}");
                let err_scratch = self.fresh_reg("serve_arg_err_scratch");
                self.emit_alloca(&err_scratch, "{ptr, i64}");
                let found = self.fresh_reg("serve_arg_found");
                writeln!(
                    self.out,
                    "  {found} = call i32 @nir_json_array_get(ptr %args_json_ptr, i64 %args_json_len, i64 {json_idx}, ptr {json_scratch}, ptr {err_scratch})"
                )
                .unwrap();
                let err_val = self.fresh_reg("serve_arg_err_val");
                writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
                self.serve_require_ok_or_business_error(&found, &err_val, &format!("serve_arg{json_idx}"))?;
                let element_json = self.fresh_reg("serve_arg_element_json");
                writeln!(self.out, "  {element_json} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
                let decoded = self.emit_decode_value_json(&p.ty, &element_json)?;
                let payload_ptr = self.emit_decode_or_business_error(&p.ty, &decoded, &format!("serve_arg{json_idx}_decode"))?;
                call_operands.push(self.serve_call_operand(&p.ty, &payload_ptr)?);
                json_idx += 1;
            }
        }

        // ---- call `f`, encode its result back out ----
        let result = self.emit_call_known_fn(&f.name, &call_operands, &f.ret)?;
        if let Ty::Named(name, args) = &f.ret
            && name == "Result"
            && args.len() == 2
        {
            let ok_ty = &args[0];
            let err_ty = &args[1];
            let result_llty = self.llvm_ty(&f.ret)?;
            let tag_ptr = self.fresh_reg("serve_ret_tag_ptr");
            writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {result}, i32 0, i32 0").unwrap();
            let tag = self.fresh_reg("serve_ret_tag");
            writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
            let payload_ptr = self.fresh_reg("serve_ret_payload_ptr");
            writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {result}, i32 0, i32 1").unwrap();
            let is_ok = self.icmp("eq", "i64", &tag, "0")?;
            let ok_label = self.fresh_label("serve_ret_ok");
            let err_label = self.fresh_label("serve_ret_err");
            writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

            writeln!(self.out, "{ok_label}:").unwrap();
            let ok_json = self.emit_encode_value_json(ok_ty, &payload_ptr)?;
            let ok_wrapped = self.emit_wrap_json_field("ok", &ok_json, "serve_ret_ok_wrap")?;
            let ok_ptr = self.fresh_reg("serve_ret_ok_body_ptr");
            writeln!(self.out, "  {ok_ptr} = extractvalue {{ptr, i64}} {ok_wrapped}, 0").unwrap();
            let ok_len = self.fresh_reg("serve_ret_ok_body_len");
            writeln!(self.out, "  {ok_len} = extractvalue {{ptr, i64}} {ok_wrapped}, 1").unwrap();
            writeln!(self.out, "  store ptr {ok_ptr}, ptr %out_body_ptr").unwrap();
            writeln!(self.out, "  store i64 {ok_len}, ptr %out_body_len").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();

            writeln!(self.out, "{err_label}:").unwrap();
            let err_json = self.emit_encode_value_json(err_ty, &payload_ptr)?;
            let err_wrapped = self.emit_wrap_json_field("err", &err_json, "serve_ret_err_wrap")?;
            self.emit_serve_return_business_error(&err_wrapped);
        } else if f.ret == Ty::Unit {
            let null_body = self.fresh_global("serve_ret_unit_body");
            writeln!(self.string_globals, "{null_body} = private unnamed_addr constant [4 x i8] c\"null\"").unwrap();
            writeln!(self.out, "  store ptr {null_body}, ptr %out_body_ptr").unwrap();
            writeln!(self.out, "  store i64 4, ptr %out_body_len").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();
        } else {
            // A plain (non-`Result`, non-`unit`) return: `emit_call_known_fn`
            // handed back a pointer for an aggregate return, or a bare
            // value for a scalar one — the encoder always wants a
            // pointer, so a scalar gets one temp slot of its own first.
            let value_ptr = if f.ret.is_aggregate() {
                result
            } else {
                let llty = self.llvm_ty(&f.ret)?;
                let slot = self.fresh_reg("serve_ret_scalar_slot");
                self.emit_alloca(&slot, &llty);
                writeln!(self.out, "  store {llty} {result}, ptr {slot}").unwrap();
                slot
            };
            let body_json = self.emit_encode_value_json(&f.ret, &value_ptr)?;
            let body_ptr = self.fresh_reg("serve_ret_body_ptr");
            writeln!(self.out, "  {body_ptr} = extractvalue {{ptr, i64}} {body_json}, 0").unwrap();
            let body_len = self.fresh_reg("serve_ret_body_len");
            writeln!(self.out, "  {body_len} = extractvalue {{ptr, i64}} {body_json}, 1").unwrap();
            writeln!(self.out, "  store ptr {body_ptr}, ptr %out_body_ptr").unwrap();
            writeln!(self.out, "  store i64 {body_len}, ptr %out_body_len").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();
        }

        self.out.insert_str(alloca_splice_pos, &self.entry_allocas.clone());
        writeln!(self.out, "}}").unwrap();
        Ok(wrapper_name)
    }


}
