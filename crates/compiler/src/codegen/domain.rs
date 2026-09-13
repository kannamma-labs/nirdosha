use super::*;
use super::layout::*;

impl Codegen<'_> {
    /// `check_role(identity, role) -> Result(RoleView, str)` — the real
    /// compiled implementation (`IDENTITY_BUILTINS`'s own doc comment
    /// has the full scope/disclosure). Reads `identity.claims_json` as a
    /// comma-separated role list (`nir_check_role`,
    /// `runtime-kernels/src/lib.rs`) and constructs a real `Ok(RoleView(role))`
    /// or `Err("...")` by hand — the same tag-then-payload shape
    /// `construct_variant`'s generic path already uses, just written
    /// directly rather than through it (no `Expr` exists for "the string
    /// this kernel call already computed" the generic path could recurse
    /// on).
    pub(super) fn emit_check_role(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let role_view_ty = Ty::Named("RoleView".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![role_view_ty.clone(), Ty::Str]);

        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let (claims_idx, _) = self.field_index_and_ty(&identity_ty, "claims_json").expect("VerifiedIdentity always has claims_json, ast::prelude_structs");
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let claims_field_ptr = self.fresh_reg("check_role_claims_ptr");
        writeln!(self.out, "  {claims_field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {claims_idx}").unwrap();
        let claims_val = self.fresh_reg("check_role_claims_val");
        writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {claims_field_ptr}").unwrap();
        let claims_ptr = self.fresh_reg("check_role_claims_data_ptr");
        writeln!(self.out, "  {claims_ptr} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
        let claims_len = self.fresh_reg("check_role_claims_len");
        writeln!(self.out, "  {claims_len} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();

        let role_val = self.expr(&args[1], scopes)?;
        let role_ptr = self.fresh_reg("check_role_role_ptr");
        writeln!(self.out, "  {role_ptr} = extractvalue {{ptr, i64}} {role_val}, 0").unwrap();
        let role_len = self.fresh_reg("check_role_role_len");
        writeln!(self.out, "  {role_len} = extractvalue {{ptr, i64}} {role_val}, 1").unwrap();

        let found = self.fresh_reg("check_role_found");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_check_role(ptr {claims_ptr}, i64 {claims_len}, ptr {role_ptr}, i64 {role_len})"
        )
        .unwrap();
        let is_found = self.fresh_reg("check_role_is_found");
        writeln!(self.out, "  {is_found} = icmp ne i32 {found}, 0").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("check_role_result.addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("check_role_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("check_role_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("check_role_ok");
        let err_label = self.fresh_label("check_role_err");
        let merge_label = self.fresh_label("check_role_merge");
        writeln!(self.out, "  br i1 {is_found}, label %{ok_label}, label %{err_label}").unwrap();

        // `Ok(RoleView(role))` — variant 0 (`ast::prelude_enums`'
        // `Result` declaration order). `RoleView`'s own sole field is
        // `role: str`, so its whole value *is* the same `{ptr, i64}`
        // word pair already computed above — stored straight into the
        // payload's first two words, no separate temp/memcpy needed.
        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {role_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        // `Err("...")` — variant 1. Same "the payload's first two words
        // are directly a `str` value" shape as `Ok` above.
        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let msg_global = self.fresh_global("check_role_err_msg");
        const MSG: &str = "role not present in identity's claims";
        writeln!(self.string_globals, "{msg_global} = private unnamed_addr constant [{} x i8] c\"{}\"", MSG.len(), llvm_escape_bytes(MSG.as_bytes()))
            .unwrap();
        let msg_partial = self.fresh_reg("check_role_err_msg_partial");
        writeln!(self.out, "  {msg_partial} = insertvalue {{ptr, i64}} undef, ptr {msg_global}, 0").unwrap();
        let msg_full = self.fresh_reg("check_role_err_msg_full");
        writeln!(self.out, "  {msg_full} = insertvalue {{ptr, i64}} {msg_partial}, i64 {}, 1", MSG.len()).unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {msg_full}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `oidc_validate_token(token, expected_issuer, expected_audience,
    /// jwks_json) -> Result(VerifiedIdentity, str)` — real JWT/JWKS
    /// signature verification (`nir_oidc_validate_token`,
    /// `IDENTITY_BUILTINS`'s own doc comment has the full scope). Unlike
    /// `emit_check_role`'s single-`str`-field `RoleView` payload,
    /// `VerifiedIdentity` has six fields, so this writes the kernel's
    /// out-params **directly into a scratch `VerifiedIdentity`'s own
    /// field pointers** (via `field_index_and_ty`, same helper
    /// `emit_check_role` uses to *read* `claims_json`) rather than
    /// double-buffering through intermediate locals, then `memcpy`s that
    /// whole struct into the `Result`'s payload on success — the same
    /// "GEP to the field, no intermediate copy" discipline
    /// `construct_variant`'s generic path already uses for aggregate
    /// payloads. Same tag-then-payload/br/merge shape as
    /// `emit_check_role` otherwise; the one real difference is the `Err`
    /// message is a genuine runtime `str` value from the kernel's own
    /// `out_err`, not a compile-time literal (a malformed token/JWKS
    /// produces a different message than a bad issuer/audience/
    /// signature, and callers of a real relying-party check deserve to
    /// know which).
    pub(super) fn emit_oidc_validate_token(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![identity_ty.clone(), Ty::Str]);

        let (token_ptr, token_len) = self.str_parts(&args[0], scopes)?;
        let (issuer_ptr, issuer_len) = self.str_parts(&args[1], scopes)?;
        let (audience_ptr, audience_len) = self.str_parts(&args[2], scopes)?;
        let (jwks_ptr, jwks_len) = self.str_parts(&args[3], scopes)?;

        // A scratch `VerifiedIdentity`, written to directly by the
        // kernel call below on success — its own field pointers *are*
        // the kernel's `out_subject`/`out_issuer`/etc. arguments, no
        // separate out-param locals to copy from afterward.
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let identity_scratch = self.fresh_reg("oidc_identity_scratch");
        self.emit_alloca(&identity_scratch, &identity_llty);
        let field_ptr = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("oidc_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {identity_llty}, ptr {identity_scratch}, i32 0, i32 {idx}").unwrap();
            ptr
        };
        let subject_ptr = field_ptr(self, "subject");
        let issuer_ptr_out = field_ptr(self, "issuer");
        let audience_ptr_out = field_ptr(self, "audience");
        let expires_at_ptr = field_ptr(self, "expires_at");
        let issued_at_ptr = field_ptr(self, "issued_at");
        let claims_json_ptr = field_ptr(self, "claims_json");

        let err_scratch = self.fresh_reg("oidc_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let found = self.fresh_reg("oidc_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_oidc_validate_token(ptr {token_ptr}, i64 {token_len}, ptr {issuer_ptr}, i64 {issuer_len}, \
             ptr {audience_ptr}, i64 {audience_len}, ptr {jwks_ptr}, i64 {jwks_len}, ptr {subject_ptr}, ptr {issuer_ptr_out}, \
             ptr {audience_ptr_out}, ptr {expires_at_ptr}, ptr {issued_at_ptr}, ptr {claims_json_ptr}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.fresh_reg("oidc_is_ok");
        writeln!(self.out, "  {is_ok} = icmp ne i32 {found}, 0").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("oidc_result.addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("oidc_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("oidc_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("oidc_ok");
        let err_label = self.fresh_label("oidc_err");
        let merge_label = self.fresh_label("oidc_merge");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let identity_bytes = agg_byte_size_operand(&identity_ty, &self.registry);
        writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {identity_scratch}, i64 {identity_bytes}, i1 false)").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let err_val = self.fresh_reg("oidc_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {err_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `mock_issue_token(subject, issuer, audience, issued_at, ttl_secs,
    /// claims_json, jwks_json) -> Result(str, str)` — the inverse of
    /// `emit_oidc_validate_token` just above: a single `str` payload, not
    /// a six-field struct, so this follows `emit_json_get_str`'s simpler
    /// shape instead (`nir_mock_issue_token`'s own doc comment has the
    /// real HS256-only scope).
    pub(super) fn emit_mock_issue_token(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]);
        let (subject_ptr, subject_len) = self.str_parts(&args[0], scopes)?;
        let (issuer_ptr, issuer_len) = self.str_parts(&args[1], scopes)?;
        let (audience_ptr, audience_len) = self.str_parts(&args[2], scopes)?;
        let issued_at = self.expr(&args[3], scopes)?;
        let ttl_secs = self.expr(&args[4], scopes)?;
        let (claims_ptr, claims_len) = self.str_parts(&args[5], scopes)?;
        let (jwks_ptr, jwks_len) = self.str_parts(&args[6], scopes)?;

        let token_scratch = self.fresh_reg("mock_issue_token_value_scratch");
        self.emit_alloca(&token_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("mock_issue_token_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("mock_issue_token_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_mock_issue_token(ptr {subject_ptr}, i64 {subject_len}, ptr {issuer_ptr}, i64 {issuer_len}, \
             ptr {audience_ptr}, i64 {audience_len}, i64 {issued_at}, i64 {ttl_secs}, ptr {claims_ptr}, i64 {claims_len}, \
             ptr {jwks_ptr}, i64 {jwks_len}, ptr {token_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let token_val = self.fresh_reg("mock_issue_token_value");
        writeln!(self.out, "  {token_val} = load {{ptr, i64}}, ptr {token_scratch}").unwrap();
        let err_val = self.fresh_reg("mock_issue_token_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &token_val, &err_val, "mock_issue_token")
    }


    /// `extract_claim(identity, name) -> Result(ClaimView, str)` — real
    /// JSON claim extraction (`nir_extract_claim`). `ClaimView`'s sole
    /// field (`value: str`) makes this structurally identical to
    /// `emit_check_role`: one kernel call producing a bool-shaped status
    /// plus (on success) a single `str` out-param that *is* the whole
    /// `Ok` payload, same tag-then-payload/br/merge shape, same
    /// compile-time `Err` literal (unlike `oidc_validate_token`, a
    /// missing/non-string claim has exactly one reason, so no dynamic
    /// message is needed from the kernel).
    pub(super) fn emit_extract_claim(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let claim_view_ty = Ty::Named("ClaimView".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![claim_view_ty.clone(), Ty::Str]);

        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let (claims_idx, _) = self.field_index_and_ty(&identity_ty, "claims_json").expect("VerifiedIdentity always has claims_json, ast::prelude_structs");
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let claims_field_ptr = self.fresh_reg("extract_claim_claims_ptr");
        writeln!(self.out, "  {claims_field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {claims_idx}").unwrap();
        let claims_val = self.fresh_reg("extract_claim_claims_val");
        writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {claims_field_ptr}").unwrap();
        let claims_ptr = self.fresh_reg("extract_claim_claims_data_ptr");
        writeln!(self.out, "  {claims_ptr} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
        let claims_len = self.fresh_reg("extract_claim_claims_len");
        writeln!(self.out, "  {claims_len} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();

        let (name_ptr, name_len) = self.str_parts(&args[1], scopes)?;

        let out_scratch = self.fresh_reg("extract_claim_out_scratch");
        self.emit_alloca(&out_scratch, "{ptr, i64}");
        let found = self.fresh_reg("extract_claim_found");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_extract_claim(ptr {claims_ptr}, i64 {claims_len}, ptr {name_ptr}, i64 {name_len}, ptr {out_scratch})"
        )
        .unwrap();
        let is_found = self.fresh_reg("extract_claim_is_found");
        writeln!(self.out, "  {is_found} = icmp ne i32 {found}, 0").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("extract_claim_result.addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("extract_claim_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("extract_claim_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("extract_claim_ok");
        let err_label = self.fresh_label("extract_claim_err");
        let merge_label = self.fresh_label("extract_claim_merge");
        writeln!(self.out, "  br i1 {is_found}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let value_val = self.fresh_reg("extract_claim_value");
        writeln!(self.out, "  {value_val} = load {{ptr, i64}}, ptr {out_scratch}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {value_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let msg_global = self.fresh_global("extract_claim_err_msg");
        const MSG: &str = "claim not present in identity's claims";
        writeln!(self.string_globals, "{msg_global} = private unnamed_addr constant [{} x i8] c\"{}\"", MSG.len(), llvm_escape_bytes(MSG.as_bytes()))
            .unwrap();
        let msg_partial = self.fresh_reg("extract_claim_err_msg_partial");
        writeln!(self.out, "  {msg_partial} = insertvalue {{ptr, i64}} undef, ptr {msg_global}, 0").unwrap();
        let msg_full = self.fresh_reg("extract_claim_err_msg_full");
        writeln!(self.out, "  {msg_full} = insertvalue {{ptr, i64}} {msg_partial}, i64 {}, 1", MSG.len()).unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {msg_full}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `check_role_path(identity, path, role) -> Result(RoleView, str)` —
    /// `emit_check_role`'s own twin, with a dotted `path` argument threaded
    /// through to `nir_check_role_path` instead of assuming a top-level
    /// `"roles"` key.
    pub(super) fn emit_check_role_path(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let role_view_ty = Ty::Named("RoleView".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![role_view_ty, Ty::Str]);

        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let (claims_idx, _) = self.field_index_and_ty(&identity_ty, "claims_json").expect("VerifiedIdentity always has claims_json, ast::prelude_structs");
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let claims_field_ptr = self.fresh_reg("check_role_path_claims_ptr");
        writeln!(self.out, "  {claims_field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {claims_idx}").unwrap();
        let claims_val = self.fresh_reg("check_role_path_claims_val");
        writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {claims_field_ptr}").unwrap();
        let claims_ptr = self.fresh_reg("check_role_path_claims_data_ptr");
        writeln!(self.out, "  {claims_ptr} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
        let claims_len = self.fresh_reg("check_role_path_claims_len");
        writeln!(self.out, "  {claims_len} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();

        let (path_ptr, path_len) = self.str_parts(&args[1], scopes)?;
        let role_val = self.expr(&args[2], scopes)?;
        let role_ptr = self.fresh_reg("check_role_path_role_ptr");
        writeln!(self.out, "  {role_ptr} = extractvalue {{ptr, i64}} {role_val}, 0").unwrap();
        let role_len = self.fresh_reg("check_role_path_role_len");
        writeln!(self.out, "  {role_len} = extractvalue {{ptr, i64}} {role_val}, 1").unwrap();

        let found = self.fresh_reg("check_role_path_found");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_check_role_path(ptr {claims_ptr}, i64 {claims_len}, ptr {path_ptr}, i64 {path_len}, ptr {role_ptr}, i64 {role_len})"
        )
        .unwrap();
        let is_found = self.fresh_reg("check_role_path_is_found");
        writeln!(self.out, "  {is_found} = icmp ne i32 {found}, 0").unwrap();

        let err_msg = self.const_str_value("check_role_path_err_msg", "role not present at the given claims path");
        self.emit_result_merge(&result_ty, &is_found, "{ptr, i64}", &role_val, &err_msg, "check_role_path")
    }


    /// `extract_claim_path(identity, path) -> Result(ClaimView, str)` —
    /// `emit_extract_claim`'s own twin with a dotted path.
    pub(super) fn emit_extract_claim_path(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let claim_view_ty = Ty::Named("ClaimView".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![claim_view_ty, Ty::Str]);

        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let (claims_idx, _) = self.field_index_and_ty(&identity_ty, "claims_json").expect("VerifiedIdentity always has claims_json, ast::prelude_structs");
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let claims_field_ptr = self.fresh_reg("extract_claim_path_claims_ptr");
        writeln!(self.out, "  {claims_field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {claims_idx}").unwrap();
        let claims_val = self.fresh_reg("extract_claim_path_claims_val");
        writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {claims_field_ptr}").unwrap();
        let claims_ptr = self.fresh_reg("extract_claim_path_claims_data_ptr");
        writeln!(self.out, "  {claims_ptr} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
        let claims_len = self.fresh_reg("extract_claim_path_claims_len");
        writeln!(self.out, "  {claims_len} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();

        let (path_ptr, path_len) = self.str_parts(&args[1], scopes)?;

        let out_scratch = self.fresh_reg("extract_claim_path_out_scratch");
        self.emit_alloca(&out_scratch, "{ptr, i64}");
        let found = self.fresh_reg("extract_claim_path_found");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_extract_claim_path(ptr {claims_ptr}, i64 {claims_len}, ptr {path_ptr}, i64 {path_len}, ptr {out_scratch})"
        )
        .unwrap();
        let is_found = self.fresh_reg("extract_claim_path_is_found");
        writeln!(self.out, "  {is_found} = icmp ne i32 {found}, 0").unwrap();
        let value_val = self.fresh_reg("extract_claim_path_value");
        writeln!(self.out, "  {value_val} = load {{ptr, i64}}, ptr {out_scratch}").unwrap();

        let err_msg = self.const_str_value("extract_claim_path_err_msg", "claim not present at the given path");
        self.emit_result_merge(&result_ty, &is_found, "{ptr, i64}", &value_val, &err_msg, "extract_claim_path")
    }


    /// `check_revocation(identity) -> bool` — infallible, a plain GEP +
    /// linked call + `icmp`, same "no `Result` wrap" shape
    /// `identity_expired` already has (unlike that one, this needs a real
    /// JSON-parsing kernel call, not just a memory read, since `"revoked"`
    /// lives inside `claims_json`, not a dedicated struct field).
    pub(super) fn emit_check_revocation(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let (claims_idx, _) = self.field_index_and_ty(&identity_ty, "claims_json").expect("VerifiedIdentity always has claims_json, ast::prelude_structs");
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let claims_field_ptr = self.fresh_reg("check_revocation_claims_ptr");
        writeln!(self.out, "  {claims_field_ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {claims_idx}").unwrap();
        let claims_val = self.fresh_reg("check_revocation_claims_val");
        writeln!(self.out, "  {claims_val} = load {{ptr, i64}}, ptr {claims_field_ptr}").unwrap();
        let claims_ptr = self.fresh_reg("check_revocation_claims_data_ptr");
        writeln!(self.out, "  {claims_ptr} = extractvalue {{ptr, i64}} {claims_val}, 0").unwrap();
        let claims_len = self.fresh_reg("check_revocation_claims_len");
        writeln!(self.out, "  {claims_len} = extractvalue {{ptr, i64}} {claims_val}, 1").unwrap();
        let revoked = self.fresh_reg("check_revocation_revoked");
        writeln!(self.out, "  {revoked} = call i32 @nir_check_revocation(ptr {claims_ptr}, i64 {claims_len})").unwrap();
        self.icmp("ne", "i32", &revoked, "0")
    }


    /// `create_application_session(identity) -> ApplicationSession` —
    /// infallible. `identity_subject`/`identity_issuer` are plain copies
    /// of the input identity's own `subject`/`issuer` fields (no kernel
    /// call needed for those two); `session_id`/`created_at`/`expires_at`/
    /// `last_accessed_at` are written directly into the destination
    /// struct's own field pointers by `nir_create_application_session`,
    /// same "the out-params *are* the field pointers" discipline
    /// `emit_oidc_validate_token` already established.
    pub(super) fn emit_create_application_session(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let session_ty = Ty::Named("ApplicationSession".to_string(), vec![]);
        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let identity_llty = self.llvm_ty(&identity_ty)?;

        let read_str_field = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("create_session_identity_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {idx}").unwrap();
            let val = cg.fresh_reg(&format!("create_session_identity_{field}_val"));
            writeln!(cg.out, "  {val} = load {{ptr, i64}}, ptr {ptr}").unwrap();
            val
        };
        // `(ptr, i64)`-split versions of the same fields, additionally —
        // needed as *scalar* call args to `nir_create_application_session`
        // below (red-team report A2: previously this function's own
        // identity argument was read here only to populate
        // `ApplicationSession`'s own `identity_subject`/`identity_issuer`
        // fields, never actually passed to the kernel — nothing durable
        // backed a session id. Now it's also stored into a real
        // server-side session record `verify_session` looks up against).
        let split_str_field = |cg: &mut Self, field: &str| -> (String, String) {
            let val = read_str_field(cg, field);
            let ptr = cg.fresh_reg(&format!("create_session_identity_{field}_split_ptr"));
            writeln!(cg.out, "  {ptr} = extractvalue {{ptr, i64}} {val}, 0").unwrap();
            let len = cg.fresh_reg(&format!("create_session_identity_{field}_split_len"));
            writeln!(cg.out, "  {len} = extractvalue {{ptr, i64}} {val}, 1").unwrap();
            (ptr, len)
        };
        let subject_val = read_str_field(self, "subject");
        let issuer_val = read_str_field(self, "issuer");
        let (subject_ptr, subject_len) = split_str_field(self, "subject");
        let (issuer_ptr, issuer_len) = split_str_field(self, "issuer");
        let (audience_ptr, audience_len) = split_str_field(self, "audience");
        let (claims_json_ptr, claims_json_len) = split_str_field(self, "claims_json");

        let session_llty = self.llvm_ty(&session_ty)?;
        let dest = self.fresh_reg("create_session_dest");
        self.emit_alloca(&dest, &session_llty);
        let field_ptr = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&session_ty, field).expect("ApplicationSession always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("create_session_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {session_llty}, ptr {dest}, i32 0, i32 {idx}").unwrap();
            ptr
        };
        let session_id_ptr = field_ptr(self, "session_id");
        let identity_subject_ptr = field_ptr(self, "identity_subject");
        let identity_issuer_ptr = field_ptr(self, "identity_issuer");
        let created_at_ptr = field_ptr(self, "created_at");
        let expires_at_ptr = field_ptr(self, "expires_at");
        let last_accessed_at_ptr = field_ptr(self, "last_accessed_at");

        writeln!(self.out, "  store {{ptr, i64}} {subject_val}, ptr {identity_subject_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {issuer_val}, ptr {identity_issuer_ptr}").unwrap();
        writeln!(
            self.out,
            "  call void @nir_create_application_session(ptr {subject_ptr}, i64 {subject_len}, ptr {issuer_ptr}, i64 {issuer_len}, \
             ptr {audience_ptr}, i64 {audience_len}, ptr {claims_json_ptr}, i64 {claims_json_len}, ptr {session_id_ptr}, \
             ptr {created_at_ptr}, ptr {expires_at_ptr}, ptr {last_accessed_at_ptr})"
        )
        .unwrap();
        Ok(dest)
    }


    /// `verify_session(session_id) -> Result(VerifiedIdentity, str)`
    /// (red-team report A2) — the real server-side lookup
    /// `nir_create_application_session`'s own doc comment above promises
    /// exists now. Structurally identical to [`Codegen::emit_validate_api_key`]
    /// (same `Result(VerifiedIdentity, str)` shape, same aggregate-merge
    /// pattern), just one `str` input (a session id) instead of two (a
    /// key + expected hash).
    pub(super) fn emit_verify_session(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![identity_ty.clone(), Ty::Str]);

        let (session_id_ptr, session_id_len) = self.str_parts(&args[0], scopes)?;

        let identity_llty = self.llvm_ty(&identity_ty)?;
        let identity_scratch = self.fresh_reg("verify_session_identity_scratch");
        self.emit_alloca(&identity_scratch, &identity_llty);
        let out_field_ptr = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("verify_session_out_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {identity_llty}, ptr {identity_scratch}, i32 0, i32 {idx}").unwrap();
            ptr
        };
        let out_subject_ptr = out_field_ptr(self, "subject");
        let out_issuer_ptr = out_field_ptr(self, "issuer");
        let out_audience_ptr = out_field_ptr(self, "audience");
        let out_expires_at_ptr = out_field_ptr(self, "expires_at");
        let out_issued_at_ptr = out_field_ptr(self, "issued_at");
        let out_claims_json_ptr = out_field_ptr(self, "claims_json");

        let err_scratch = self.fresh_reg("verify_session_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let ok = self.fresh_reg("verify_session_ok");
        writeln!(
            self.out,
            "  {ok} = call i32 @nir_verify_session(ptr {session_id_ptr}, i64 {session_id_len}, ptr {out_subject_ptr}, ptr {out_issuer_ptr}, \
             ptr {out_audience_ptr}, ptr {out_expires_at_ptr}, ptr {out_issued_at_ptr}, ptr {out_claims_json_ptr}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.fresh_reg("verify_session_is_ok");
        writeln!(self.out, "  {is_ok} = icmp ne i32 {ok}, 0").unwrap();

        self.emit_result_merge_agg(&result_ty, &is_ok, &identity_ty, &identity_scratch, &err_scratch, "verify_session")
    }


    /// `session_cookie(session) -> str` — infallible, a plain formatted
    /// string built from the session's own real `session_id`/
    /// `created_at`/`expires_at` fields.
    pub(super) fn emit_session_cookie(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let session_ty = Ty::Named("ApplicationSession".to_string(), vec![]);
        let session_ptr = self.expr_ptr_expected(&args[0], &session_ty, scopes)?;
        let session_llty = self.llvm_ty(&session_ty)?;

        let (sid_idx, _) = self.field_index_and_ty(&session_ty, "session_id").expect("ApplicationSession always has session_id, ast::prelude_structs");
        let sid_field_ptr = self.fresh_reg("session_cookie_sid_field_ptr");
        writeln!(self.out, "  {sid_field_ptr} = getelementptr inbounds {session_llty}, ptr {session_ptr}, i32 0, i32 {sid_idx}").unwrap();
        let sid_val = self.fresh_reg("session_cookie_sid_val");
        writeln!(self.out, "  {sid_val} = load {{ptr, i64}}, ptr {sid_field_ptr}").unwrap();
        let sid_ptr = self.fresh_reg("session_cookie_sid_ptr");
        writeln!(self.out, "  {sid_ptr} = extractvalue {{ptr, i64}} {sid_val}, 0").unwrap();
        let sid_len = self.fresh_reg("session_cookie_sid_len");
        writeln!(self.out, "  {sid_len} = extractvalue {{ptr, i64}} {sid_val}, 1").unwrap();

        let read_i64_field = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&session_ty, field).expect("ApplicationSession always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("session_cookie_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {session_llty}, ptr {session_ptr}, i32 0, i32 {idx}").unwrap();
            let val = cg.fresh_reg(&format!("session_cookie_{field}_val"));
            writeln!(cg.out, "  {val} = load i64, ptr {ptr}").unwrap();
            val
        };
        let created_at = read_i64_field(self, "created_at");
        let expires_at = read_i64_field(self, "expires_at");

        let out_scratch = self.fresh_reg("session_cookie_out_scratch");
        self.emit_alloca(&out_scratch, "{ptr, i64}");
        writeln!(
            self.out,
            "  call void @nir_session_cookie(ptr {sid_ptr}, i64 {sid_len}, i64 {created_at}, i64 {expires_at}, ptr {out_scratch})"
        )
        .unwrap();
        let cookie_val = self.fresh_reg("session_cookie_val");
        writeln!(self.out, "  {cookie_val} = load {{ptr, i64}}, ptr {out_scratch}").unwrap();
        Ok(cookie_val)
    }


    /// `new_refresh_token(expires_at) -> RefreshTokenHandle` — infallible.
    /// The `handle: box i64` field's own heap slot (a real `nir_alloc(8)`,
    /// the same allocator `Expr::Box` construction already uses) is
    /// passed *directly* as the kernel's `out_handle_id` — the box's
    /// storage and the out-param are the same memory, no intermediate
    /// scratch/copy needed, the same "out-param is the real field
    /// pointer" discipline this file's other identity emitters already
    /// use for aggregate fields.
    pub(super) fn emit_new_refresh_token(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let handle_ty = Ty::Named("RefreshTokenHandle".to_string(), vec![]);
        let expires_at = self.expr(&args[0], scopes)?;

        let heap_ptr = self.fresh_reg("new_refresh_token_heap");
        writeln!(self.out, "  {heap_ptr} = call ptr @nir_alloc(i64 8)").unwrap();
        writeln!(self.out, "  call void @nir_new_refresh_token(i64 {expires_at}, ptr {heap_ptr})").unwrap();

        let handle_llty = self.llvm_ty(&handle_ty)?;
        let dest = self.fresh_reg("new_refresh_token_dest");
        self.emit_alloca(&dest, &handle_llty);
        let (handle_idx, _) = self.field_index_and_ty(&handle_ty, "handle").expect("RefreshTokenHandle always has handle, ast::prelude_structs");
        let handle_field_ptr = self.fresh_reg("new_refresh_token_handle_field_ptr");
        writeln!(self.out, "  {handle_field_ptr} = getelementptr inbounds {handle_llty}, ptr {dest}, i32 0, i32 {handle_idx}").unwrap();
        writeln!(self.out, "  store ptr {heap_ptr}, ptr {handle_field_ptr}").unwrap();
        let (expires_idx, _) = self.field_index_and_ty(&handle_ty, "expires_at").expect("RefreshTokenHandle always has expires_at, ast::prelude_structs");
        let expires_field_ptr = self.fresh_reg("new_refresh_token_expires_field_ptr");
        writeln!(self.out, "  {expires_field_ptr} = getelementptr inbounds {handle_llty}, ptr {dest}, i32 0, i32 {expires_idx}").unwrap();
        writeln!(self.out, "  store i64 {expires_at}, ptr {expires_field_ptr}").unwrap();
        Ok(dest)
    }


    /// `exchange_refresh_token(identity, handle, new_issued_at) ->
    /// Result(VerifiedIdentity, str)` — redeems `handle`'s own boxed id
    /// (dereferenced here — two loads, `ptr` then the `i64` it points
    /// at, same shape `Expr::Deref` already uses for any `box i64`) and,
    /// on success, reissues `identity` with a fresh `issued_at`. Same
    /// out-param-is-the-real-field-pointer + `Result`-merge shape
    /// `emit_oidc_validate_token` already established for a
    /// `VerifiedIdentity` payload.
    pub(super) fn emit_exchange_refresh_token(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let handle_ty = Ty::Named("RefreshTokenHandle".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![identity_ty.clone(), Ty::Str]);

        let identity_ptr = self.expr_ptr_expected(&args[0], &identity_ty, scopes)?;
        let identity_llty = self.llvm_ty(&identity_ty)?;
        let read_str_field = |cg: &mut Self, field: &str| -> (String, String) {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let fptr = cg.fresh_reg(&format!("exchange_refresh_identity_{field}_fptr"));
            writeln!(cg.out, "  {fptr} = getelementptr inbounds {identity_llty}, ptr {identity_ptr}, i32 0, i32 {idx}").unwrap();
            let val = cg.fresh_reg(&format!("exchange_refresh_identity_{field}_val"));
            writeln!(cg.out, "  {val} = load {{ptr, i64}}, ptr {fptr}").unwrap();
            let p = cg.fresh_reg(&format!("exchange_refresh_identity_{field}_ptr"));
            writeln!(cg.out, "  {p} = extractvalue {{ptr, i64}} {val}, 0").unwrap();
            let l = cg.fresh_reg(&format!("exchange_refresh_identity_{field}_len"));
            writeln!(cg.out, "  {l} = extractvalue {{ptr, i64}} {val}, 1").unwrap();
            (p, l)
        };
        let (subject_ptr, subject_len) = read_str_field(self, "subject");
        let (issuer_ptr, issuer_len) = read_str_field(self, "issuer");
        let (audience_ptr, audience_len) = read_str_field(self, "audience");
        let (claims_ptr, claims_len) = read_str_field(self, "claims_json");

        let handle_ptr = self.expr_ptr_expected(&args[1], &handle_ty, scopes)?;
        let handle_llty = self.llvm_ty(&handle_ty)?;
        let (handle_idx, _) = self.field_index_and_ty(&handle_ty, "handle").expect("RefreshTokenHandle always has handle, ast::prelude_structs");
        let handle_field_ptr = self.fresh_reg("exchange_refresh_handle_field_ptr");
        writeln!(self.out, "  {handle_field_ptr} = getelementptr inbounds {handle_llty}, ptr {handle_ptr}, i32 0, i32 {handle_idx}").unwrap();
        let box_ptr = self.fresh_reg("exchange_refresh_box_ptr");
        writeln!(self.out, "  {box_ptr} = load ptr, ptr {handle_field_ptr}").unwrap();
        let handle_id = self.fresh_reg("exchange_refresh_handle_id");
        writeln!(self.out, "  {handle_id} = load i64, ptr {box_ptr}").unwrap();

        let new_issued_at = self.expr(&args[2], scopes)?;

        let identity_scratch = self.fresh_reg("exchange_refresh_identity_scratch");
        self.emit_alloca(&identity_scratch, &identity_llty);
        let out_field_ptr = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("exchange_refresh_out_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {identity_llty}, ptr {identity_scratch}, i32 0, i32 {idx}").unwrap();
            ptr
        };
        let out_subject_ptr = out_field_ptr(self, "subject");
        let out_issuer_ptr = out_field_ptr(self, "issuer");
        let out_audience_ptr = out_field_ptr(self, "audience");
        let out_expires_at_ptr = out_field_ptr(self, "expires_at");
        let out_issued_at_ptr = out_field_ptr(self, "issued_at");
        let out_claims_json_ptr = out_field_ptr(self, "claims_json");

        let err_scratch = self.fresh_reg("exchange_refresh_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let ok = self.fresh_reg("exchange_refresh_ok");
        writeln!(
            self.out,
            "  {ok} = call i32 @nir_exchange_refresh_token(i64 {handle_id}, i64 {new_issued_at}, ptr {subject_ptr}, i64 {subject_len}, \
             ptr {issuer_ptr}, i64 {issuer_len}, ptr {audience_ptr}, i64 {audience_len}, ptr {claims_ptr}, i64 {claims_len}, \
             ptr {out_subject_ptr}, ptr {out_issuer_ptr}, ptr {out_audience_ptr}, ptr {out_expires_at_ptr}, ptr {out_issued_at_ptr}, \
             ptr {out_claims_json_ptr}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.fresh_reg("exchange_refresh_is_ok");
        writeln!(self.out, "  {is_ok} = icmp ne i32 {ok}, 0").unwrap();

        self.emit_result_merge_agg(&result_ty, &is_ok, &identity_ty, &identity_scratch, &err_scratch, "exchange_refresh")
    }


    /// `validate_api_key(key, expected_hash) -> Result(VerifiedIdentity, str)`
    /// — same out-param-is-the-real-field-pointer + `Result`-merge shape
    /// as `exchange_refresh_token`/`oidc_validate_token`, a constant-time
    /// hash compare decides success instead of a JWT/JWKS check.
    pub(super) fn emit_validate_api_key(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let identity_ty = Ty::Named("VerifiedIdentity".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![identity_ty.clone(), Ty::Str]);

        let (key_ptr, key_len) = self.str_parts(&args[0], scopes)?;
        let (hash_ptr, hash_len) = self.str_parts(&args[1], scopes)?;

        let identity_llty = self.llvm_ty(&identity_ty)?;
        let identity_scratch = self.fresh_reg("validate_api_key_identity_scratch");
        self.emit_alloca(&identity_scratch, &identity_llty);
        let out_field_ptr = |cg: &mut Self, field: &str| -> String {
            let (idx, _) = cg.field_index_and_ty(&identity_ty, field).expect("VerifiedIdentity always has this field, ast::prelude_structs");
            let ptr = cg.fresh_reg(&format!("validate_api_key_out_{field}_ptr"));
            writeln!(cg.out, "  {ptr} = getelementptr inbounds {identity_llty}, ptr {identity_scratch}, i32 0, i32 {idx}").unwrap();
            ptr
        };
        let out_subject_ptr = out_field_ptr(self, "subject");
        let out_issuer_ptr = out_field_ptr(self, "issuer");
        let out_audience_ptr = out_field_ptr(self, "audience");
        let out_expires_at_ptr = out_field_ptr(self, "expires_at");
        let out_issued_at_ptr = out_field_ptr(self, "issued_at");
        let out_claims_json_ptr = out_field_ptr(self, "claims_json");

        let err_scratch = self.fresh_reg("validate_api_key_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let ok = self.fresh_reg("validate_api_key_ok");
        writeln!(
            self.out,
            "  {ok} = call i32 @nir_validate_api_key(ptr {key_ptr}, i64 {key_len}, ptr {hash_ptr}, i64 {hash_len}, ptr {out_subject_ptr}, \
             ptr {out_issuer_ptr}, ptr {out_audience_ptr}, ptr {out_expires_at_ptr}, ptr {out_issued_at_ptr}, ptr {out_claims_json_ptr}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.fresh_reg("validate_api_key_is_ok");
        writeln!(self.out, "  {is_ok} = icmp ne i32 {ok}, 0").unwrap();

        self.emit_result_merge_agg(&result_ty, &is_ok, &identity_ty, &identity_scratch, &err_scratch, "validate_api_key")
    }


    /// Builds the `[N x NIR_BIND_VALUE_LLTY]` array `db_execute`/
    /// `db_query`'s trailing `?`-placeholder arguments become
    /// (`runtime-kernels/src/lib.rs`'s `NirBindValue`'s own doc comment
    /// has the exact field layout this mirrors). Returns `("null", "0")`
    /// for the zero-bind case (the bare 2-arg form) — the same "no real
    /// buffer needed when the length says not to dereference it"
    /// convention `sha256_hex`'s 1-arg form already established for
    /// `b_ptr`/`b_len`.
    pub(super) fn emit_db_binds(&mut self, binds: &[Expr], scopes: &mut Scopes) -> Result<(String, String), CodegenError> {
        if binds.is_empty() {
            return Ok(("null".to_string(), "0".to_string()));
        }
        let n = binds.len();
        let arr_llty = format!("[{n} x {NIR_BIND_VALUE_LLTY}]");
        let arr_ptr = self.fresh_reg("db_binds_arr");
        self.emit_alloca(&arr_ptr, &arr_llty);
        for (i, arg) in binds.iter().enumerate() {
            let elem_ptr = self.fresh_reg("db_bind_elem_ptr");
            writeln!(self.out, "  {elem_ptr} = getelementptr inbounds {arr_llty}, ptr {arr_ptr}, i32 0, i32 {i}").unwrap();
            let tag_ptr = self.fresh_reg("db_bind_tag_ptr");
            writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 0").unwrap();
            let i_ptr = self.fresh_reg("db_bind_i_ptr");
            writeln!(self.out, "  {i_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 1").unwrap();
            let f_ptr = self.fresh_reg("db_bind_f_ptr");
            writeln!(self.out, "  {f_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 2").unwrap();
            let sptr_ptr = self.fresh_reg("db_bind_sptr_ptr");
            writeln!(self.out, "  {sptr_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 3").unwrap();
            let slen_ptr = self.fresh_reg("db_bind_slen_ptr");
            writeln!(self.out, "  {slen_ptr} = getelementptr inbounds {NIR_BIND_VALUE_LLTY}, ptr {elem_ptr}, i32 0, i32 4").unwrap();

            // Zero every field first, so the kernel never reads a stale/
            // uninitialized value out of the field(s) this arg's own
            // tag doesn't select.
            writeln!(self.out, "  store i64 0, ptr {i_ptr}").unwrap();
            writeln!(self.out, "  store double 0.0, ptr {f_ptr}").unwrap();
            writeln!(self.out, "  store ptr null, ptr {sptr_ptr}").unwrap();
            writeln!(self.out, "  store i64 0, ptr {slen_ptr}").unwrap();

            let arg_ty = self.local_ty_of(arg, scopes);
            if arg_ty == Ty::Str {
                writeln!(self.out, "  store i32 2, ptr {tag_ptr}").unwrap();
                let (ptr, len) = self.str_parts(arg, scopes)?;
                writeln!(self.out, "  store ptr {ptr}, ptr {sptr_ptr}").unwrap();
                writeln!(self.out, "  store i64 {len}, ptr {slen_ptr}").unwrap();
            } else if arg_ty == Ty::F64 {
                writeln!(self.out, "  store i32 1, ptr {tag_ptr}").unwrap();
                let v = self.expr(arg, scopes)?;
                writeln!(self.out, "  store double {v}, ptr {f_ptr}").unwrap();
            } else if arg_ty == Ty::Bool {
                writeln!(self.out, "  store i32 3, ptr {tag_ptr}").unwrap();
                let v = self.expr(arg, scopes)?;
                let widened = self.fresh_reg("db_bind_bool_widened");
                writeln!(self.out, "  {widened} = zext i1 {v} to i64").unwrap();
                writeln!(self.out, "  store i64 {widened}, ptr {i_ptr}").unwrap();
            } else if let Ty::Named(_, _) = &arg_ty {
                // A zero-payload `enum` bind (`typeck.rs::check_db_bind_ty`
                // already proved every variant of this enum carries no
                // payload — anything else was a clean compile-time
                // rejection, never reaches codegen) — binds as its plain
                // `i64` discriminant, the same tag-0 "integer" slot the
                // fallback arm below uses for a real integer. `expr_ptr`,
                // not `expr`: an enum is aggregate-valued (`is_aggregate()`),
                // so its value lives behind a pointer, same as
                // `Expr::Match`'s own scrutinee-tag extraction.
                writeln!(self.out, "  store i32 0, ptr {tag_ptr}").unwrap();
                let value_ptr = self.expr_ptr(arg, scopes)?;
                let enum_llty = self.llvm_ty(&arg_ty)?;
                let enum_tag_ptr = self.fresh_reg("db_bind_enum_tag_ptr");
                writeln!(self.out, "  {enum_tag_ptr} = getelementptr inbounds {enum_llty}, ptr {value_ptr}, i32 0, i32 0").unwrap();
                let tag_val = self.fresh_reg("db_bind_enum_tag");
                writeln!(self.out, "  {tag_val} = load i64, ptr {enum_tag_ptr}").unwrap();
                writeln!(self.out, "  store i64 {tag_val}, ptr {i_ptr}").unwrap();
            } else {
                // Every other bind-value type `typeck.rs`'s `check_db_bind_ty`
                // allows through is some integer width — widen to i64 the
                // same way every other integer-typed value already does on
                // its way into a linked kernel call.
                writeln!(self.out, "  store i32 0, ptr {tag_ptr}").unwrap();
                let v = self.expr(arg, scopes)?;
                let widened = self.widen_to_i64(&v, &arg_ty);
                writeln!(self.out, "  store i64 {widened}, ptr {i_ptr}").unwrap();
            }
        }
        Ok((arr_ptr, n.to_string()))
    }


    /// `db_connect(path) -> Result(db, str)`.
    /// `env(name) -> Result(str, str)` (RFC 0011 §1) — `Ok(value)` when
    /// the process environment variable is set, `Err(_)` when unset. Not
    /// resource-gated: no `Domain`, no handle, unlike `db_connect` right
    /// below — same shape as `emit_json_get_str`, just one `str` arg
    /// instead of two.
    pub(super) fn emit_env(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]);
        let (name_ptr, name_len) = self.str_parts(&args[0], scopes)?;
        let value_scratch = self.fresh_reg("env_value_scratch");
        self.emit_alloca(&value_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("env_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        // Named `env_found`, not `env_ok` -- `emit_result_merge`'s own
        // `label_prefix` below is `"env"`, which mints labels literally
        // named `env_ok`/`env_err`/`env_merge`. `fresh_reg`/`fresh_label`
        // keep separate counters, so a register also named `env_ok` can
        // land on the same numeric suffix as the label on a second call
        // to `env(...)` in the same function -- `%env_ok.9` bound once as
        // an `i32` value and again as a branch target is a genuine LLVM
        // parse error ("not a basic block"), caught by compiling a
        // program with two `env(...)` calls, not by the single-call case.
        let found = self.fresh_reg("env_found");
        writeln!(self.out, "  {found} = call i32 @nir_env_get(ptr {name_ptr}, i64 {name_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let value_val = self.fresh_reg("env_value");
        writeln!(self.out, "  {value_val} = load {{ptr, i64}}, ptr {value_scratch}").unwrap();
        let err_val = self.fresh_reg("env_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &value_val, &err_val, "env")
    }


    pub(super) fn emit_db_connect(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Db, Ty::Str]);
        let (path_ptr, path_len) = self.str_parts(&args[0], scopes)?;
        let handle_scratch = self.fresh_reg("db_connect_handle_scratch");
        self.emit_alloca(&handle_scratch, "i64");
        let err_scratch = self.fresh_reg("db_connect_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("db_connect_ok");
        writeln!(self.out, "  {found} = call i32 @nir_db_connect(ptr {path_ptr}, i64 {path_len}, ptr {handle_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let handle_val = self.fresh_reg("db_connect_handle");
        writeln!(self.out, "  {handle_val} = load i64, ptr {handle_scratch}").unwrap();
        let err_val = self.fresh_reg("db_connect_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i64", &handle_val, &err_val, "db_connect")
    }


    /// `db_execute(conn, sql, ...binds) -> Result(i64, str)` — everything
    /// except `SELECT`; the affected-row count.
    pub(super) fn emit_db_execute(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str]);
        let conn = self.expr(&args[0], scopes)?; // Ty::Db is scalar (i64), not aggregate
        let (sql_ptr, sql_len) = self.str_parts(&args[1], scopes)?;
        let (binds_ptr, binds_len) = self.emit_db_binds(&args[2..], scopes)?;
        let affected_scratch = self.fresh_reg("db_execute_affected_scratch");
        self.emit_alloca(&affected_scratch, "i64");
        let err_scratch = self.fresh_reg("db_execute_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("db_execute_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_db_execute(i64 {conn}, ptr {sql_ptr}, i64 {sql_len}, ptr {binds_ptr}, i64 {binds_len}, ptr {affected_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let affected_val = self.fresh_reg("db_execute_affected");
        writeln!(self.out, "  {affected_val} = load i64, ptr {affected_scratch}").unwrap();
        let err_val = self.fresh_reg("db_execute_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i64", &affected_val, &err_val, "db_execute")
    }


    /// `db_query(conn, sql, ...binds) -> Result(json, str)` — `SELECT`
    /// statements; every row comes back as one JSON object, the whole
    /// result set a JSON array (`nir_db_query`'s own doc comment).
    pub(super) fn emit_db_query(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]);
        let conn = self.expr(&args[0], scopes)?;
        let (sql_ptr, sql_len) = self.str_parts(&args[1], scopes)?;
        let (binds_ptr, binds_len) = self.emit_db_binds(&args[2..], scopes)?;
        let json_scratch = self.fresh_reg("db_query_json_scratch");
        self.emit_alloca(&json_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("db_query_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("db_query_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_db_query(i64 {conn}, ptr {sql_ptr}, i64 {sql_len}, ptr {binds_ptr}, i64 {binds_len}, ptr {json_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let json_val = self.fresh_reg("db_query_json");
        writeln!(self.out, "  {json_val} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
        let err_val = self.fresh_reg("db_query_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &json_val, &err_val, "db_query")
    }


    /// `json_parse(s) -> Result(json, str)` — an identity function on
    /// success under this representation (`Ty::Json` compiles as `s`'s
    /// own raw text, `Ty::Json`'s `llvm_ty` arm), so only validity needs
    /// checking; the `Ok` payload reuses `s`'s own already-computed
    /// `{ptr, i64}` value directly rather than re-evaluating `s`.
    pub(super) fn emit_json_parse(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]);
        let json_val = self.expr(&args[0], scopes)?;
        let json_ptr = self.fresh_reg("json_parse_ptr");
        writeln!(self.out, "  {json_ptr} = extractvalue {{ptr, i64}} {json_val}, 0").unwrap();
        let json_len = self.fresh_reg("json_parse_len");
        writeln!(self.out, "  {json_len} = extractvalue {{ptr, i64}} {json_val}, 1").unwrap();
        let err_scratch = self.fresh_reg("json_parse_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_parse_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_validate(ptr {json_ptr}, i64 {json_len}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let err_val = self.fresh_reg("json_parse_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &json_val, &err_val, "json_parse")
    }


    /// `json_get(doc, key) -> Result(json, str)`.
    pub(super) fn emit_json_get(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let json_scratch = self.fresh_reg("json_get_json_scratch");
        self.emit_alloca(&json_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("json_get_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_get_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_get(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {json_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let json_val = self.fresh_reg("json_get_json_val");
        writeln!(self.out, "  {json_val} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
        let err_val = self.fresh_reg("json_get_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &json_val, &err_val, "json_get")
    }


    /// `json_array_get(doc, idx) -> Result(json, str)` — same shape as
    /// `json_get`, indexed by position instead of key.
    pub(super) fn emit_json_array_get(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let idx = self.expr(&args[1], scopes)?;
        let json_scratch = self.fresh_reg("json_array_get_json_scratch");
        self.emit_alloca(&json_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("json_array_get_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_array_get_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_array_get(ptr {doc_ptr}, i64 {doc_len}, i64 {idx}, ptr {json_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let json_val = self.fresh_reg("json_array_get_json_val");
        writeln!(self.out, "  {json_val} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
        let err_val = self.fresh_reg("json_array_get_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &json_val, &err_val, "json_array_get")
    }


    /// `json_array_len(doc) -> Result(i64, str)`.
    pub(super) fn emit_json_array_len(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let value_scratch = self.fresh_reg("json_array_len_value_scratch");
        self.emit_alloca(&value_scratch, "i64");
        let err_scratch = self.fresh_reg("json_array_len_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_array_len_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_array_len(ptr {doc_ptr}, i64 {doc_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let value_val = self.fresh_reg("json_array_len_value");
        writeln!(self.out, "  {value_val} = load i64, ptr {value_scratch}").unwrap();
        let err_val = self.fresh_reg("json_array_len_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i64", &value_val, &err_val, "json_array_len")
    }


    /// `json_get_str(doc, key) -> Result(str, str)`.
    pub(super) fn emit_json_get_str(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let value_scratch = self.fresh_reg("json_get_str_value_scratch");
        self.emit_alloca(&value_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("json_get_str_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_get_str_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_get_str(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let value_val = self.fresh_reg("json_get_str_value");
        writeln!(self.out, "  {value_val} = load {{ptr, i64}}, ptr {value_scratch}").unwrap();
        let err_val = self.fresh_reg("json_get_str_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &value_val, &err_val, "json_get_str")
    }


    /// `json_get_i64(doc, key) -> Result(i64, str)`.
    pub(super) fn emit_json_get_i64(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::I64, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let value_scratch = self.fresh_reg("json_get_i64_value_scratch");
        self.emit_alloca(&value_scratch, "i64");
        let err_scratch = self.fresh_reg("json_get_i64_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_get_i64_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_get_i64(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let value_val = self.fresh_reg("json_get_i64_value");
        writeln!(self.out, "  {value_val} = load i64, ptr {value_scratch}").unwrap();
        let err_val = self.fresh_reg("json_get_i64_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i64", &value_val, &err_val, "json_get_i64")
    }


    /// `json_get_f64(doc, key) -> Result(f64, str)`.
    pub(super) fn emit_json_get_f64(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::F64, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let value_scratch = self.fresh_reg("json_get_f64_value_scratch");
        self.emit_alloca(&value_scratch, "double");
        let err_scratch = self.fresh_reg("json_get_f64_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_get_f64_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_get_f64(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let value_val = self.fresh_reg("json_get_f64_value");
        writeln!(self.out, "  {value_val} = load double, ptr {value_scratch}").unwrap();
        let err_val = self.fresh_reg("json_get_f64_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "double", &value_val, &err_val, "json_get_f64")
    }


    /// `json_get_bool(doc, key) -> Result(bool, str)`.
    pub(super) fn emit_json_get_bool(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Bool, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let value_scratch = self.fresh_reg("json_get_bool_value_scratch");
        self.emit_alloca(&value_scratch, "i32");
        let err_scratch = self.fresh_reg("json_get_bool_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_get_bool_ok");
        writeln!(self.out, "  {found} = call i32 @nir_json_get_bool(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {value_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let raw = self.fresh_reg("json_get_bool_raw");
        writeln!(self.out, "  {raw} = load i32, ptr {value_scratch}").unwrap();
        let value_val = self.icmp("ne", "i32", &raw, "0")?;
        let err_val = self.fresh_reg("json_get_bool_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i1", &value_val, &err_val, "json_get_bool")
    }


    /// `json_set_str(doc, key, value) -> Result(json, str)`.
    pub(super) fn emit_json_set_str(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Json, Ty::Str]);
        let (doc_ptr, doc_len) = self.str_parts(&args[0], scopes)?;
        let (key_ptr, key_len) = self.str_parts(&args[1], scopes)?;
        let (value_ptr, value_len) = self.str_parts(&args[2], scopes)?;
        let json_scratch = self.fresh_reg("json_set_str_json_scratch");
        self.emit_alloca(&json_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("json_set_str_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("json_set_str_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_json_set_str(ptr {doc_ptr}, i64 {doc_len}, ptr {key_ptr}, i64 {key_len}, ptr {value_ptr}, i64 {value_len}, ptr {json_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let json_val = self.fresh_reg("json_set_str_json_val");
        writeln!(self.out, "  {json_val} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
        let err_val = self.fresh_reg("json_set_str_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &json_val, &err_val, "json_set_str")
    }


    /// `mq_connect(host, port) -> Result(mq, str)`.
    pub(super) fn emit_mq_connect(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Mq, Ty::Str]);
        let (host_ptr, host_len) = self.str_parts(&args[0], scopes)?;
        let port = self.expr(&args[1], scopes)?;
        let handle_scratch = self.fresh_reg("mq_connect_handle_scratch");
        self.emit_alloca(&handle_scratch, "i64");
        let err_scratch = self.fresh_reg("mq_connect_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("mq_connect_ok");
        writeln!(self.out, "  {found} = call i32 @nir_mq_connect(ptr {host_ptr}, i64 {host_len}, i64 {port}, ptr {handle_scratch}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let handle_val = self.fresh_reg("mq_connect_handle");
        writeln!(self.out, "  {handle_val} = load i64, ptr {handle_scratch}").unwrap();
        let err_val = self.fresh_reg("mq_connect_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "i64", &handle_val, &err_val, "mq_connect")
    }


    /// `mq_publish(conn, queue, message) -> Result(unit, str)`.
    pub(super) fn emit_mq_publish(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Unit, Ty::Str]);
        let conn = self.expr(&args[0], scopes)?;
        let (queue_ptr, queue_len) = self.str_parts(&args[1], scopes)?;
        let (msg_ptr, msg_len) = self.str_parts(&args[2], scopes)?;
        let err_scratch = self.fresh_reg("mq_publish_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("mq_publish_ok");
        writeln!(self.out, "  {found} = call i32 @nir_mq_publish(i64 {conn}, ptr {queue_ptr}, i64 {queue_len}, ptr {msg_ptr}, i64 {msg_len}, ptr {err_scratch})").unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let err_val = self.fresh_reg("mq_publish_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        // `unit`'s own value is always the same placeholder `i64 0`
        // every other unit-returning site in this file already uses.
        self.emit_result_merge(&result_ty, &is_ok, "i64", "0", &err_val, "mq_publish")
    }


    /// `mq_consume(conn, queue, timeout_secs) -> Result(str, str)`.
    pub(super) fn emit_mq_consume(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = Ty::Named("Result".to_string(), vec![Ty::Str, Ty::Str]);
        let conn = self.expr(&args[0], scopes)?;
        let (queue_ptr, queue_len) = self.str_parts(&args[1], scopes)?;
        let timeout_secs = self.expr(&args[2], scopes)?;
        let msg_scratch = self.fresh_reg("mq_consume_msg_scratch");
        self.emit_alloca(&msg_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("mq_consume_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("mq_consume_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_mq_consume(i64 {conn}, ptr {queue_ptr}, i64 {queue_len}, i64 {timeout_secs}, ptr {msg_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let msg_val = self.fresh_reg("mq_consume_msg");
        writeln!(self.out, "  {msg_val} = load {{ptr, i64}}, ptr {msg_scratch}").unwrap();
        let err_val = self.fresh_reg("mq_consume_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        self.emit_result_merge(&result_ty, &is_ok, "{ptr, i64}", &msg_val, &err_val, "mq_consume")
    }


    /// `http_get`/`http_post`/`https_get`/`https_post` — shared shape,
    /// differing only in which kernel is linked and whether a request
    /// `body` argument exists. `HttpResponse { status: i64, body: str }`
    /// is a real two-field struct payload (unlike every other builtin in
    /// this file's own `emit_result_merge` uses, which all have exactly
    /// one payload value) — built in a scratch alloca via `field_index_
    /// and_ty`/GEP (the same shape `emit_oidc_validate_token` already
    /// uses for its own multi-field `VerifiedIdentity` payload), then
    /// `memcpy`'d into the `Result`'s payload directly rather than going
    /// through `emit_result_merge` at all.
    pub(super) fn emit_http_call(&mut self, kernel_name: &str, args: &[Expr], has_body: bool, scopes: &mut Scopes) -> Result<String, CodegenError> {
        let http_response_ty = Ty::Named("HttpResponse".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![http_response_ty.clone(), Ty::Str]);

        let (host_ptr, host_len) = self.str_parts(&args[0], scopes)?;
        let port = self.expr(&args[1], scopes)?;
        let (path_ptr, path_len) = self.str_parts(&args[2], scopes)?;

        let status_scratch = self.fresh_reg("http_status_scratch");
        self.emit_alloca(&status_scratch, "i64");
        let body_scratch = self.fresh_reg("http_body_scratch");
        self.emit_alloca(&body_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("http_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let found = self.fresh_reg("http_ok");
        if has_body {
            let (req_body_ptr, req_body_len) = self.str_parts(&args[3], scopes)?;
            writeln!(
                self.out,
                "  {found} = call i32 @{kernel_name}(ptr {host_ptr}, i64 {host_len}, i64 {port}, ptr {path_ptr}, i64 {path_len}, \
                 ptr {req_body_ptr}, i64 {req_body_len}, ptr {status_scratch}, ptr {body_scratch}, ptr {err_scratch})"
            )
            .unwrap();
        } else {
            writeln!(
                self.out,
                "  {found} = call i32 @{kernel_name}(ptr {host_ptr}, i64 {host_len}, i64 {port}, ptr {path_ptr}, i64 {path_len}, \
                 ptr {status_scratch}, ptr {body_scratch}, ptr {err_scratch})"
            )
            .unwrap();
        }
        let is_ok = self.icmp("ne", "i32", &found, "0")?;

        let http_llty = self.llvm_ty(&http_response_ty)?;
        let http_scratch = self.fresh_reg("http_response_scratch");
        self.emit_alloca(&http_scratch, &http_llty);
        let (status_idx, _) =
            self.field_index_and_ty(&http_response_ty, "status").expect("HttpResponse always has status, ast::prelude_structs");
        let (body_idx, _) = self.field_index_and_ty(&http_response_ty, "body").expect("HttpResponse always has body, ast::prelude_structs");
        let status_field_ptr = self.fresh_reg("http_status_field_ptr");
        writeln!(self.out, "  {status_field_ptr} = getelementptr inbounds {http_llty}, ptr {http_scratch}, i32 0, i32 {status_idx}").unwrap();
        let status_val = self.fresh_reg("http_status_val");
        writeln!(self.out, "  {status_val} = load i64, ptr {status_scratch}").unwrap();
        writeln!(self.out, "  store i64 {status_val}, ptr {status_field_ptr}").unwrap();
        let body_field_ptr = self.fresh_reg("http_body_field_ptr");
        writeln!(self.out, "  {body_field_ptr} = getelementptr inbounds {http_llty}, ptr {http_scratch}, i32 0, i32 {body_idx}").unwrap();
        let body_val = self.fresh_reg("http_body_val");
        writeln!(self.out, "  {body_val} = load {{ptr, i64}}, ptr {body_scratch}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {body_val}, ptr {body_field_ptr}").unwrap();

        let err_val = self.fresh_reg("http_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("http_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("http_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("http_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("http_ok");
        let err_label = self.fresh_label("http_err");
        let merge_label = self.fresh_label("http_merge");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let http_bytes = agg_byte_size_operand(&http_response_ty, &self.registry);
        writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {http_scratch}, i64 {http_bytes}, i1 false)").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {err_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `call_via(url, path, body) -> Result(HttpResponse, str)` —
    /// rfcs/0011-uniform-service-provider-model.md §1/§2's `call`-shape
    /// dispatch entrypoint. Structurally the same result-merge shape as
    /// [`Codegen::emit_http_call`] (same `HttpResponse`/`Result` layout,
    /// same ok/err/merge label pattern) — kept as its own function
    /// rather than folded into `emit_http_call` because the call-site
    /// argument shape genuinely differs (`(url, path, body)`, no
    /// separate `host`/`port`, and always exactly 3 args — `call_via`
    /// has no bodyless-GET-shaped sibling the way `http_get`/`http_post`
    /// do).
    pub(super) fn emit_call_via(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let http_response_ty = Ty::Named("HttpResponse".to_string(), vec![]);
        let result_ty = Ty::Named("Result".to_string(), vec![http_response_ty.clone(), Ty::Str]);

        let (url_ptr, url_len) = self.str_parts(&args[0], scopes)?;
        let (path_ptr, path_len) = self.str_parts(&args[1], scopes)?;
        let (body_ptr, body_len) = self.str_parts(&args[2], scopes)?;

        let status_scratch = self.fresh_reg("call_via_status_scratch");
        self.emit_alloca(&status_scratch, "i64");
        let body_scratch = self.fresh_reg("call_via_body_scratch");
        self.emit_alloca(&body_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("call_via_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");

        let found = self.fresh_reg("call_via_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_call_via(ptr {url_ptr}, i64 {url_len}, ptr {path_ptr}, i64 {path_len}, \
             ptr {body_ptr}, i64 {body_len}, ptr {status_scratch}, ptr {body_scratch}, ptr {err_scratch})"
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;

        let http_llty = self.llvm_ty(&http_response_ty)?;
        let http_scratch = self.fresh_reg("call_via_response_scratch");
        self.emit_alloca(&http_scratch, &http_llty);
        let (status_idx, _) =
            self.field_index_and_ty(&http_response_ty, "status").expect("HttpResponse always has status, ast::prelude_structs");
        let (body_idx, _) = self.field_index_and_ty(&http_response_ty, "body").expect("HttpResponse always has body, ast::prelude_structs");
        let status_field_ptr = self.fresh_reg("call_via_status_field_ptr");
        writeln!(self.out, "  {status_field_ptr} = getelementptr inbounds {http_llty}, ptr {http_scratch}, i32 0, i32 {status_idx}").unwrap();
        let status_val = self.fresh_reg("call_via_status_val");
        writeln!(self.out, "  {status_val} = load i64, ptr {status_scratch}").unwrap();
        writeln!(self.out, "  store i64 {status_val}, ptr {status_field_ptr}").unwrap();
        let body_field_ptr = self.fresh_reg("call_via_body_field_ptr");
        writeln!(self.out, "  {body_field_ptr} = getelementptr inbounds {http_llty}, ptr {http_scratch}, i32 0, i32 {body_idx}").unwrap();
        let body_val = self.fresh_reg("call_via_body_val");
        writeln!(self.out, "  {body_val} = load {{ptr, i64}}, ptr {body_scratch}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {body_val}, ptr {body_field_ptr}").unwrap();

        let err_val = self.fresh_reg("call_via_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("call_via_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("call_via_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("call_via_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("call_via_ok");
        let err_label = self.fresh_label("call_via_err");
        let merge_label = self.fresh_label("call_via_merge");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let http_bytes = agg_byte_size_operand(&http_response_ty, &self.registry);
        writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {http_scratch}, i64 {http_bytes}, i1 false)").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {err_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// Resolves a `workflow_lower.rs`-synthesized call's compile-time-
    /// literal `workflow_name` argument back to the original
    /// `WorkflowDecl` it names (`Codegen.workflows`, populated from
    /// `program.workflows` at construction — the lowering pass keeps the
    /// source `WorkflowDecl` around for exactly this) — and rejects,
    /// with a specific reason, the one Layer 1 restriction not already
    /// caught by `check_expr`'s pre-pass: a workflow whose `data { ... }`
    /// block is non-empty. `nir_workflow_create_instance` (`runtime-
    /// kernels`) takes no `data` parameter at all — an instance's
    /// `WorkflowInstance` row has no `data_json` field to persist one
    /// into — so a `data.<field>` reference inside any state's
    /// `on_entry`/`on_exit` *other* than the very first state's (whose
    /// actions run inline, inside `start_<workflow>`'s own generated
    /// function body, sharing its still-live `data` parameter — see
    /// `emit_workflow_start`) could never resolve once an `advance_*`
    /// call has moved the instance on. Rather than track which
    /// individual action bodies happen to reference `data` (fragile,
    /// easy to silently miscompile a workflow edited later to add one),
    /// this rejects the whole workflow up front — the same "reject
    /// early and specifically, not partially and silently" posture this
    /// file takes throughout.
    pub(super) fn resolve_workflow_layer1(&self, name_expr: &Expr) -> Result<&WorkflowDecl, CodegenError> {
        let Expr::Str(name, _) = name_expr else {
            return unsupported(
                "workflow codegen expects a compile-time string literal workflow name (always \
                 true for workflow_lower.rs-synthesized calls)"
                    .to_string(),
            );
        };
        let wf = self
            .workflows
            .iter()
            .find(|w| &w.name == name)
            .ok_or_else(|| CodegenError { message: format!("no `workflow {name}` declaration found for this compiled program") })?;
        if !wf.data.is_empty() {
            return unsupported(format!(
                "codegen doesn't support `workflow {name}` yet — its `data {{ ... }}` block is \
                 non-empty, and this compiled backend's Layer 1 workflow runtime has nowhere \
                 durable to persist a `data` value past the initial `start_*` call \
                 (`nir_workflow_create_instance` takes no `data` parameter at all — an \
                 instance's `WorkflowInstance` row has no `data_json` field); a workflow with an \
                 empty `data {{ }}` block compiles now"
            ));
        }
        Ok(wf)
    }


    /// Runs one `on_entry`/`on_exit` action slot (`TransactSlot`, same
    /// shape `transact`'s own `precheck`/`verify`/... slots use) for its
    /// side effect only, discarding its return value —
    /// `docs/WORKFLOW.md`'s grammar restricts an action call to a bare
    /// `name(args)`, so nothing here would use the value even if kept.
    /// Deliberately **not** `emit_transact_call` (whose
    /// `self.sigs.get(&slot.name).expect(...)` only resolves user-
    /// defined functions): a workflow action's callee may just as often
    /// be a builtin (`send_email` etc. are exactly the point) —
    /// reusing `Stmt::Expr`'s own `is_aggregate()`-routed dispatch
    /// instead naturally supports both, through the same `call()`/
    /// `call_ptr()` machinery every other call in this file already
    /// goes through.
    pub(super) fn emit_workflow_action(&mut self, slot: &TransactSlot, scopes: &mut Scopes) -> Result<(), CodegenError> {
        let call_expr = Expr::Call(slot.name.clone(), slot.args.clone(), slot.span);
        if self.local_ty_of(&call_expr, scopes).is_aggregate() {
            self.expr_ptr(&call_expr, scopes)?;
        } else {
            self.expr(&call_expr, scopes)?;
        }
        Ok(())
    }


    /// Builds a scratch `WorkflowActionError` value with the given
    /// variant's tag (`ast::prelude_enums`' own declaration order —
    /// `NoRecipientsForRole`=0 through `NotStateOwner`=8) and, for the
    /// one payload-carrying case a caller needs, its `str` payload —
    /// returns a pointer to it, ready for the caller's own
    /// `llvm.memcpy` into a `Result(_, WorkflowActionError)`'s payload
    /// field (this file's "build the aggregate fully, then one memcpy"
    /// convention — `emit_http_call`'s doc comment). A zero-payload
    /// variant's payload buffer is left exactly as `emit_alloca` leaves
    /// it (uninitialized, never read — `construct_variant`'s own
    /// zero-payload path already leaves the very same buffer untouched
    /// for a hand-written `Err(SomeZeroPayloadVariant)` anywhere else in
    /// this language, so this isn't a new risk).
    pub(super) fn emit_workflow_error(&mut self, variant_tag: i64, str_payload: Option<&str>) -> Result<String, CodegenError> {
        let err_ty = Ty::Named("WorkflowActionError".to_string(), vec![]);
        let err_llty = self.llvm_ty(&err_ty)?;
        let scratch = self.fresh_reg("workflow_err_scratch");
        self.emit_alloca(&scratch, &err_llty);
        let tag_ptr = self.fresh_reg("workflow_err_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {err_llty}, ptr {scratch}, i32 0, i32 0").unwrap();
        writeln!(self.out, "  store i64 {variant_tag}, ptr {tag_ptr}").unwrap();
        if let Some(v) = str_payload {
            let payload_ptr = self.fresh_reg("workflow_err_payload_ptr");
            writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {err_llty}, ptr {scratch}, i32 0, i32 1").unwrap();
            writeln!(self.out, "  store {{ptr, i64}} {v}, ptr {payload_ptr}").unwrap();
        }
        Ok(scratch)
    }


    /// Stores tag `1` (`Err`, `ast::prelude_enums`' `Result` declaration
    /// order) into an already-GEP'd `Result(_, WorkflowActionError)`'s
    /// `tag_ptr`, then `memcpy`s a `WorkflowActionError` scratch value
    /// (built by `emit_workflow_error`) into its `payload_ptr` — shared
    /// tail every `emit_workflow_*`/`emit_send_notification`/
    /// `emit_notify` error path uses.
    pub(super) fn emit_workflow_err_into_result(&mut self, tag_ptr: &str, payload_ptr: &str, err_scratch: &str) {
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let err_ty = Ty::Named("WorkflowActionError".to_string(), vec![]);
        let bytes = agg_byte_size_operand(&err_ty, &self.registry);
        writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {err_scratch}, i64 {bytes}, i1 false)").unwrap();
    }


    /// `__workflow_start(workflow_name, identity, data) ->
    /// Result(i64, WorkflowActionError)` — Layer 1 (`WORKFLOW_BUILTINS`'
    /// own doc comment). Creates a fresh, in-process instance
    /// (`nir_workflow_create_instance`) in the workflow's first-declared
    /// state, then runs that state's `on_entry` actions — `instance_id`/
    /// `data.<field>` (`docs/WORKFLOW.md`'s implicit-binding rules) both
    /// resolve for free here: this codegen runs *inside*
    /// `start_<workflow>`'s own generated function body, which already
    /// has `data` as a real parameter (`workflow_lower.rs`), and a fresh
    /// `instance_id` local is bound into `scopes` just for the duration
    /// of these actions. `identity` (`args[1]`) is accepted and
    /// typechecked but not yet durably recorded as `started_by_subject`
    /// — no `WorkflowInstance` field for it exists yet (Layer 1's
    /// disclosed narrower cut, same posture as everything else this
    /// file names explicitly rather than silently drops).
    pub(super) fn emit_workflow_start(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let wf = self.resolve_workflow_layer1(&args[0])?.clone();
        let result_ty = workflow_result_of(Ty::I64);

        let (name_ptr, name_len) = self.str_parts(&args[0], scopes)?;
        let first_state = wf.states.first().expect("parser rejects a workflow with zero states");
        let initial_global = self.fresh_global("workflow_start_initial_state");
        writeln!(
            self.string_globals,
            "{initial_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
            first_state.name.len(),
            llvm_escape_bytes(first_state.name.as_bytes())
        )
        .unwrap();

        let instance_scratch = self.fresh_reg("workflow_start_instance_scratch");
        self.emit_alloca(&instance_scratch, "i64");
        let found = self.fresh_reg("workflow_start_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_workflow_create_instance(ptr {name_ptr}, i64 {name_len}, ptr {initial_global}, i64 {}, ptr {instance_scratch})",
            first_state.name.len()
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("workflow_start_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("workflow_start_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("workflow_start_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("workflow_start_ok");
        let err_label = self.fresh_label("workflow_start_err");
        let merge_label = self.fresh_label("workflow_start_merge");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        let instance_val = self.fresh_reg("workflow_start_instance_id");
        writeln!(self.out, "  {instance_val} = load i64, ptr {instance_scratch}").unwrap();
        scopes.push();
        let instance_slot = self.fresh_reg("workflow_start_instance_slot");
        self.emit_alloca(&instance_slot, "i64");
        writeln!(self.out, "  store i64 {instance_val}, ptr {instance_slot}").unwrap();
        scopes.define("instance_id", Ty::I64, instance_slot);
        for action in &first_state.on_entry {
            self.emit_workflow_action(action, scopes)?;
        }
        scopes.pop();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store i64 {instance_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        // Can't-really-happen in practice: `nir_workflow_create_instance`
        // only returns `0` for an invalid-UTF8 workflow/initial-state
        // name, and both are always compile-time source identifiers here
        // — but a well-typed `Err` arm is still required. `InstanceNotFound`
        // is the closest available "the operation didn't succeed" signal;
        // `WorkflowActionError` has no dedicated catch-all variant.
        let err_scratch = self.emit_workflow_error(4, None)?; // InstanceNotFound
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &err_scratch);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `__workflow_advance(workflow_name, identity, instance_id, event,
    /// payload) -> Result(bool, WorkflowActionError)` — Layer 1. Looks up
    /// the instance's current state (`Err(InstanceNotFound)` if it
    /// doesn't exist for this workflow), then a sequential per-state
    /// `nir_str_eq` comparison chain — the same shape `match_literal`'s
    /// own `Ty::Str` arm uses — finds the matching state, and within it,
    /// a sequential comparison of the runtime `event` argument's enum
    /// tag against each of that state's own transitions' compile-time-
    /// known variant index in the desugared `<Workflow>Event` enum
    /// (`self.registry.enum_variants`, so this never has to re-derive
    /// `workflow_lower.rs`'s own first-appearance event-numbering by
    /// hand). On a match: runs the *old* state's `on_exit` actions, then
    /// `nir_workflow_set_state`, then the *new* state's `on_entry`
    /// actions — `instance_id` (`docs/WORKFLOW.md`'s implicit binding)
    /// resolves for free in every one of these actions, since this
    /// codegen runs *inside* `advance_<workflow>`'s own generated
    /// function body, which already has `instance_id: i64` as a real
    /// parameter; no separate binding needed the way `emit_workflow_start`
    /// needs one for its own fresh instance id. No matching state name
    /// or no matching transition both fall through to a shared
    /// `Err(NoSuchTransition)` — `payload` (`args[4]`) is accepted and
    /// typechecked but not yet threaded into any binding
    /// (`docs/WORKFLOW.md`'s own disclosed gap, unchanged here). State
    /// ownership (`owner: role(...)`/`owner: claim(...)`) is accepted
    /// syntactically (`typeck.rs::check_workflow_decl`) but not enforced
    /// by this Layer 1 runtime — no `NotStateOwner` is ever produced.
    pub(super) fn emit_workflow_advance(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let wf = self.resolve_workflow_layer1(&args[0])?.clone();
        let result_ty = workflow_result_of(Ty::Bool);
        let event_ty = Ty::Named(format!("{}Event", wf.name), vec![]);
        let event_variants: Vec<Variant> = self
            .registry
            .enum_variants(&format!("{}Event", wf.name))
            .expect("workflow_lower.rs always synthesizes <Workflow>Event")
            .to_vec();

        let (name_ptr, name_len) = self.str_parts(&args[0], scopes)?;
        let instance_id_val = self.expr(&args[2], scopes)?;

        let state_scratch = self.fresh_reg("workflow_advance_state_scratch");
        self.emit_alloca(&state_scratch, "{ptr, i64}");
        let found_instance = self.fresh_reg("workflow_advance_found_instance");
        writeln!(
            self.out,
            "  {found_instance} = call i32 @nir_workflow_get_state(ptr {name_ptr}, i64 {name_len}, i64 {instance_id_val}, ptr {state_scratch})"
        )
        .unwrap();
        let instance_found = self.icmp("ne", "i32", &found_instance, "0")?;

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("workflow_advance_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("workflow_advance_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("workflow_advance_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let found_label = self.fresh_label("workflow_advance_instance_found");
        let not_found_label = self.fresh_label("workflow_advance_instance_not_found");
        let no_such_transition_label = self.fresh_label("workflow_advance_no_such_transition");
        let merge_label = self.fresh_label("workflow_advance_merge");
        writeln!(self.out, "  br i1 {instance_found}, label %{found_label}, label %{not_found_label}").unwrap();

        writeln!(self.out, "{not_found_label}:").unwrap();
        let err_scratch_nf = self.emit_workflow_error(4, None)?; // InstanceNotFound
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &err_scratch_nf);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{found_label}:").unwrap();
        let current_state = self.fresh_reg("workflow_advance_current_state");
        writeln!(self.out, "  {current_state} = load {{ptr, i64}}, ptr {state_scratch}").unwrap();
        let cur_ptr = self.fresh_reg("workflow_advance_cur_ptr");
        writeln!(self.out, "  {cur_ptr} = extractvalue {{ptr, i64}} {current_state}, 0").unwrap();
        let cur_len = self.fresh_reg("workflow_advance_cur_len");
        writeln!(self.out, "  {cur_len} = extractvalue {{ptr, i64}} {current_state}, 1").unwrap();

        let event_ptr = self.expr_ptr_expected(&args[3], &event_ty, scopes)?;
        let event_llty = self.llvm_ty(&event_ty)?;
        let event_tag_ptr = self.fresh_reg("workflow_advance_event_tag_ptr");
        writeln!(self.out, "  {event_tag_ptr} = getelementptr inbounds {event_llty}, ptr {event_ptr}, i32 0, i32 0").unwrap();
        let event_tag = self.fresh_reg("workflow_advance_event_tag");
        writeln!(self.out, "  {event_tag} = load i64, ptr {event_tag_ptr}").unwrap();

        let state_cmp_labels: Vec<String> = wf.states.iter().map(|_| self.fresh_label("workflow_advance_state_cmp")).collect();
        writeln!(self.out, "  br label %{}", state_cmp_labels[0]).unwrap();

        for (i, s) in wf.states.iter().enumerate() {
            let cmp_label = &state_cmp_labels[i];
            let next_state_label = state_cmp_labels.get(i + 1).unwrap_or(&no_such_transition_label);
            writeln!(self.out, "{cmp_label}:").unwrap();
            let state_global = self.fresh_global("workflow_advance_state_lit");
            writeln!(
                self.string_globals,
                "{state_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
                s.name.len(),
                llvm_escape_bytes(s.name.as_bytes())
            )
            .unwrap();
            let state_eq_raw = self.fresh_reg("workflow_advance_state_eq");
            writeln!(
                self.out,
                "  {state_eq_raw} = call i32 @nir_str_eq(ptr {cur_ptr}, i64 {cur_len}, ptr {state_global}, i64 {})",
                s.name.len()
            )
            .unwrap();
            let state_eq = self.icmp("ne", "i32", &state_eq_raw, "0")?;
            let state_body_label = self.fresh_label("workflow_advance_state_body");
            writeln!(self.out, "  br i1 {state_eq}, label %{state_body_label}, label %{next_state_label}").unwrap();

            writeln!(self.out, "{state_body_label}:").unwrap();
            let transition_cmp_labels: Vec<String> =
                s.transitions.iter().map(|_| self.fresh_label("workflow_advance_event_cmp")).collect();
            if transition_cmp_labels.is_empty() {
                writeln!(self.out, "  br label %{no_such_transition_label}").unwrap();
            } else {
                writeln!(self.out, "  br label %{}", transition_cmp_labels[0]).unwrap();
            }
            for (j, t) in s.transitions.iter().enumerate() {
                let event_idx = event_variants
                    .iter()
                    .position(|v| v.name == t.event)
                    .expect("typeck.rs already validated every transition event exists in <Workflow>Event");
                let cmp_label = &transition_cmp_labels[j];
                let next_event_label = transition_cmp_labels.get(j + 1).unwrap_or(&no_such_transition_label);
                writeln!(self.out, "{cmp_label}:").unwrap();
                let event_eq = self.icmp("eq", "i64", &event_tag, &event_idx.to_string())?;
                let action_label = self.fresh_label("workflow_advance_action");
                writeln!(self.out, "  br i1 {event_eq}, label %{action_label}, label %{next_event_label}").unwrap();

                writeln!(self.out, "{action_label}:").unwrap();
                for action in &s.on_exit {
                    self.emit_workflow_action(action, scopes)?;
                }
                let target_state = wf.states.iter().find(|st| st.name == t.target).expect("workflow_lower.rs already validated target state exists");
                let target_global = self.fresh_global("workflow_advance_target_state");
                writeln!(
                    self.string_globals,
                    "{target_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
                    target_state.name.len(),
                    llvm_escape_bytes(target_state.name.as_bytes())
                )
                .unwrap();
                let set_ok = self.fresh_reg("workflow_advance_set_ok");
                writeln!(
                    self.out,
                    "  {set_ok} = call i32 @nir_workflow_set_state(ptr {name_ptr}, i64 {name_len}, i64 {instance_id_val}, ptr {target_global}, i64 {})",
                    target_state.name.len()
                )
                .unwrap();
                // `set_ok`'s own failure is the same can't-really-happen
                // case `emit_workflow_start`'s own `err_label` doc comment
                // already names (this instance and workflow name were
                // just proven to match above) — not checked separately.
                for action in &target_state.on_entry {
                    self.emit_workflow_action(action, scopes)?;
                }
                writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
                writeln!(self.out, "  store i1 true, ptr {payload_ptr}").unwrap();
                writeln!(self.out, "  br label %{merge_label}").unwrap();
            }
        }

        writeln!(self.out, "{no_such_transition_label}:").unwrap();
        let err_scratch_nt = self.emit_workflow_error(3, None)?; // NoSuchTransition
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &err_scratch_nt);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `__workflow_overdue(workflow_name) -> Result(json,
    /// WorkflowActionError)` — `docs/ROADMAP.md` A15's SLA/escalation
    /// design, only reachable for a workflow that declares at least one
    /// `sla_seconds` entry (`workflow_lower.rs` only synthesizes
    /// `list_<workflow>_overdue` in that case). Builds a compile-time
    /// JSON literal `{"State":seconds,...}` from every
    /// `state { sla_seconds: N }` entry (`typeck.rs::check_workflow_decl`
    /// already proved every such value a non-negative literal `i64`) and
    /// hands it to `nir_workflow_list_overdue`, which does the actual
    /// now-vs-`entered_at` comparison against every live
    /// `WorkflowInstance` row for this workflow.
    pub(super) fn emit_workflow_overdue(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let wf = self.resolve_workflow_layer1(&args[0])?.clone();
        let result_ty = workflow_result_of(Ty::Json);
        let (name_ptr, name_len) = self.str_parts(&args[0], scopes)?;

        let mut sla_json = String::from("{");
        let mut first = true;
        for s in &wf.states {
            if let Some((_, value)) = s.entries.iter().find(|(k, _)| k == "sla_seconds") {
                let Expr::Int(n, _) = value else {
                    unreachable!("typeck.rs::check_workflow_decl only accepts a literal i64 for sla_seconds")
                };
                if !first {
                    sla_json.push(',');
                }
                first = false;
                sla_json.push_str(&format!("{:?}:{}", s.name, n));
            }
        }
        sla_json.push('}');

        let sla_global = self.fresh_global("workflow_overdue_sla_json");
        writeln!(
            self.string_globals,
            "{sla_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
            sla_json.len(),
            llvm_escape_bytes(sla_json.as_bytes())
        )
        .unwrap();

        let json_scratch = self.fresh_reg("workflow_overdue_json_scratch");
        self.emit_alloca(&json_scratch, "{ptr, i64}");
        let err_scratch = self.fresh_reg("workflow_overdue_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let found = self.fresh_reg("workflow_overdue_ok");
        writeln!(
            self.out,
            "  {found} = call i32 @nir_workflow_list_overdue(ptr {name_ptr}, i64 {name_len}, ptr {sla_global}, i64 {}, ptr {json_scratch}, ptr {err_scratch})",
            sla_json.len()
        )
        .unwrap();
        let is_ok = self.icmp("ne", "i32", &found, "0")?;
        let json_val = self.fresh_reg("workflow_overdue_json_val");
        writeln!(self.out, "  {json_val} = load {{ptr, i64}}, ptr {json_scratch}").unwrap();
        let err_val = self.fresh_reg("workflow_overdue_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("workflow_overdue_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("workflow_overdue_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("workflow_overdue_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("workflow_overdue_ok");
        let err_label = self.fresh_label("workflow_overdue_err");
        let merge_label = self.fresh_label("workflow_overdue_merge");
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {json_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        // `nir_workflow_list_overdue`'s own failure mode is a plain `str`
        // message (bad UTF-8 — the SLA JSON itself can't be malformed,
        // it's a compile-time-built literal), not a `WorkflowActionError`
        // — wrapped here as `ProviderRequestFailed(msg)`, reusing the one
        // variant with a `str` payload as the generic "something went
        // wrong, here's why" carrier (closest fit, not a perfect name
        // match: this is an ops/admin query, not a notification send —
        // `WorkflowActionError` has no dedicated catch-all variant).
        let err_scratch2 = self.emit_workflow_error(2, Some(&err_val))?; // ProviderRequestFailed(msg)
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &err_scratch2);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `__workflow_pending_for_me`/`__workflow_submitted_by_me`/
    /// `__workflow_history` — real, compiled, always-`Err` (never a
    /// trap) implementations. `workflow_lower.rs` synthesizes
    /// `list_<w>_pending_for_me`/`list_<w>_submitted_by_me`/
    /// `get_<w>_history` **unconditionally** for every `workflow` block,
    /// so unlike `__workflow_link_advance` these three can't be rejected
    /// at compile time without breaking every workflow, including ones
    /// that never call them (`WORKFLOW_BUILTINS`'s own doc comment).
    /// State ownership (`owner: role(...)`), "who submitted this," and
    /// the audit trail all need data this Layer 1 runtime genuinely
    /// doesn't keep — `WorkflowInstance` (`runtime-kernels/src/lib.rs`)
    /// has no `owner`/`started_by_subject`/history fields at all — so
    /// each of these three real, compiled functions always returns a
    /// real `Err(ProviderRequestFailed(msg))`, naming exactly which
    /// tracking is missing, rather than a misleading `Ok("[]")` that a
    /// caller could read as "you truly have zero pending items" instead
    /// of "this isn't tracked." `ProviderRequestFailed` is reused as the
    /// generic "something went wrong, here's why" carrier (the same
    /// "closest fit, not a perfect name match" reuse `emit_workflow_overdue`
    /// already makes) since `WorkflowActionError` has no dedicated
    /// catch-all variant of its own.
    pub(super) fn emit_workflow_unsupported_query(&mut self, name: &str) -> Result<String, CodegenError> {
        let result_ty = workflow_result_of(Ty::Json);
        let reason = format!(
            "`{name}` needs real durable, restart-surviving storage (state ownership/started-by/\
             audit-trail tracking) that this compiled backend's Layer 1 workflow runtime doesn't \
             have — `WorkflowInstance` (runtime-kernels) tracks only the current state"
        );
        let msg_global = self.fresh_global("workflow_unsupported_query_msg");
        writeln!(
            self.string_globals,
            "{msg_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
            reason.len(),
            llvm_escape_bytes(reason.as_bytes())
        )
        .unwrap();
        let msg_partial = self.fresh_reg("workflow_unsupported_query_msg_partial");
        writeln!(self.out, "  {msg_partial} = insertvalue {{ptr, i64}} undef, ptr {msg_global}, 0").unwrap();
        let msg_full = self.fresh_reg("workflow_unsupported_query_msg_full");
        writeln!(self.out, "  {msg_full} = insertvalue {{ptr, i64}} {msg_partial}, i64 {}, 1", reason.len()).unwrap();

        let result_llty = self.llvm_ty(&result_ty)?;
        let dest = self.fresh_reg("workflow_unsupported_query_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("workflow_unsupported_query_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("workflow_unsupported_query_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();
        let err_scratch = self.emit_workflow_error(2, Some(&msg_full))?; // ProviderRequestFailed(msg)
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &err_scratch);
        Ok(dest)
    }


    /// Shared 3-way branch every `send_email`/`send_sms`/`send_push`/
    /// `notify` builtin ends with, once its own kernel call has already
    /// produced `status` (`nir_workflow_send`/`nir_notify`'s shared
    /// status convention: `0` ok, `1` provider not configured, anything
    /// else a request failure whose message is in `err_scratch`) — an
    /// LLVM `switch` rather than a linear `icmp` chain, so "any value
    /// that isn't 0 or 1" (not just literally `2`) safely falls to the
    /// request-failed case as the `default`.
    pub(super) fn emit_notify_result(&mut self, result_ty: &Ty, status: &str, err_scratch: &str) -> Result<String, CodegenError> {
        let result_llty = self.llvm_ty(result_ty)?;
        let dest = self.fresh_reg("notify_result_addr");
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg("notify_tag_ptr");
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg("notify_payload_ptr");
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label("notify_ok");
        let not_configured_label = self.fresh_label("notify_not_configured");
        let request_failed_label = self.fresh_label("notify_request_failed");
        let merge_label = self.fresh_label("notify_merge");
        writeln!(
            self.out,
            "  switch i32 {status}, label %{request_failed_label} [ i32 0, label %{ok_label}  i32 1, label %{not_configured_label} ]"
        )
        .unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store i1 true, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{not_configured_label}:").unwrap();
        let not_configured_scratch = self.emit_workflow_error(1, None)?; // ProviderNotConfigured
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &not_configured_scratch);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{request_failed_label}:").unwrap();
        let err_val = self.fresh_reg("notify_err_val");
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        let request_failed_scratch = self.emit_workflow_error(2, Some(&err_val))?; // ProviderRequestFailed(msg)
        self.emit_workflow_err_into_result(&tag_ptr, &payload_ptr, &request_failed_scratch);
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }


    /// `send_email`/`send_sms`/`send_push(conn, to, template, vars) ->
    /// Result(bool, WorkflowActionError)` — shared shape, differing only
    /// in which literal `channel` string (`"email"`/`"sms"`/`"push"`) is
    /// passed to `nir_workflow_send` (`runtime-kernels`'s "send/notify
    /// section" — a real, generic authenticated HTTPS POST against an
    /// admin-configured provider row, `docs/WORKFLOW.md`'s own design).
    /// `to: Recipient`'s `str` payload sits at the same word offset (1)
    /// for both `BySubject`/`ByRole` (`ast::prelude_enums`) — extracted
    /// without branching on the tag, same "both variants agree, so skip
    /// the dispatch" shape `RoleView`/`ClaimView`'s own single-field
    /// payloads already get.
    pub(super) fn emit_send_notification(&mut self, channel: &str, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = workflow_result_of(Ty::Bool);
        let recipient_ty = Ty::Named("Recipient".to_string(), vec![]);

        let conn = self.expr(&args[0], scopes)?; // Ty::Db is scalar (i64)
        let recipient_ptr = self.expr_ptr_expected(&args[1], &recipient_ty, scopes)?;
        let recipient_llty = self.llvm_ty(&recipient_ty)?;
        let recipient_payload_ptr = self.fresh_reg("send_notification_recipient_payload_ptr");
        writeln!(self.out, "  {recipient_payload_ptr} = getelementptr inbounds {recipient_llty}, ptr {recipient_ptr}, i32 0, i32 1").unwrap();
        let to_val = self.fresh_reg("send_notification_to_val");
        writeln!(self.out, "  {to_val} = load {{ptr, i64}}, ptr {recipient_payload_ptr}").unwrap();
        let to_ptr = self.fresh_reg("send_notification_to_ptr");
        writeln!(self.out, "  {to_ptr} = extractvalue {{ptr, i64}} {to_val}, 0").unwrap();
        let to_len = self.fresh_reg("send_notification_to_len");
        writeln!(self.out, "  {to_len} = extractvalue {{ptr, i64}} {to_val}, 1").unwrap();

        let (template_ptr, template_len) = self.str_parts(&args[2], scopes)?;
        let (vars_ptr, vars_len) = self.str_parts(&args[3], scopes)?;

        let channel_global = self.fresh_global("send_notification_channel");
        writeln!(
            self.string_globals,
            "{channel_global} = private unnamed_addr constant [{} x i8] c\"{}\"",
            channel.len(),
            llvm_escape_bytes(channel.as_bytes())
        )
        .unwrap();

        let err_scratch = self.fresh_reg("send_notification_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let status = self.fresh_reg("send_notification_status");
        writeln!(
            self.out,
            "  {status} = call i32 @nir_workflow_send(ptr {channel_global}, i64 {}, i64 {conn}, ptr {to_ptr}, i64 {to_len}, \
             ptr {template_ptr}, i64 {template_len}, ptr {vars_ptr}, i64 {vars_len}, ptr {err_scratch})",
            channel.len()
        )
        .unwrap();

        self.emit_notify_result(&result_ty, &status, &err_scratch)
    }


    /// `notify(conn, mq, to, template, vars) -> Result(bool,
    /// WorkflowActionError)` — same shape as `emit_send_notification`,
    /// but the callee kernel is `nir_notify` (no channel string — it
    /// always takes the documented *offline* path, delegating to the
    /// email channel; see that kernel's own doc comment for why the
    /// presence-bridge online path isn't reachable this round) and `mq`
    /// (`args[1]`) is accepted/typechecked but otherwise unused here.
    pub(super) fn emit_notify(&mut self, args: &[Expr], scopes: &mut Scopes) -> Result<String, CodegenError> {
        let result_ty = workflow_result_of(Ty::Bool);
        let recipient_ty = Ty::Named("Recipient".to_string(), vec![]);

        let conn = self.expr(&args[0], scopes)?;
        let mq = self.expr(&args[1], scopes)?; // Ty::Mq is scalar (i64); unused by nir_notify itself
        let recipient_ptr = self.expr_ptr_expected(&args[2], &recipient_ty, scopes)?;
        let recipient_llty = self.llvm_ty(&recipient_ty)?;
        let recipient_payload_ptr = self.fresh_reg("notify_recipient_payload_ptr");
        writeln!(self.out, "  {recipient_payload_ptr} = getelementptr inbounds {recipient_llty}, ptr {recipient_ptr}, i32 0, i32 1").unwrap();
        let to_val = self.fresh_reg("notify_to_val");
        writeln!(self.out, "  {to_val} = load {{ptr, i64}}, ptr {recipient_payload_ptr}").unwrap();
        let to_ptr = self.fresh_reg("notify_to_ptr");
        writeln!(self.out, "  {to_ptr} = extractvalue {{ptr, i64}} {to_val}, 0").unwrap();
        let to_len = self.fresh_reg("notify_to_len");
        writeln!(self.out, "  {to_len} = extractvalue {{ptr, i64}} {to_val}, 1").unwrap();

        let (template_ptr, template_len) = self.str_parts(&args[3], scopes)?;
        let (vars_ptr, vars_len) = self.str_parts(&args[4], scopes)?;

        let err_scratch = self.fresh_reg("notify_err_scratch");
        self.emit_alloca(&err_scratch, "{ptr, i64}");
        let status = self.fresh_reg("notify_status");
        writeln!(
            self.out,
            "  {status} = call i32 @nir_notify(i64 {conn}, i64 {mq}, ptr {to_ptr}, i64 {to_len}, ptr {template_ptr}, i64 {template_len}, \
             ptr {vars_ptr}, i64 {vars_len}, ptr {err_scratch})"
        )
        .unwrap();

        self.emit_notify_result(&result_ty, &status, &err_scratch)
    }


}
