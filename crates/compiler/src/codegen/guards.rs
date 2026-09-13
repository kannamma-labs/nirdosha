use super::*;

impl Codegen<'_> {
    /// Tier 1 vs Tier 2, for real: if `span` is in the SMT report's
    /// proven-safe set, `val` is used exactly as computed — no runtime
    /// check, no cost, matching docs/goal.md §4's Tier 1. Otherwise, emits an
    /// actual compare-and-trap sequence: a value outside `ty`'s range
    /// calls `abort()` rather than silently wrapping or corrupting
    /// anything. Returns `val` unchanged either way — the check is
    /// side-effecting (branches to a trap block if it fails), not a
    /// transformation of the value itself.
    ///
    /// **Compares at `i64` width, always — this is load-bearing, not
    /// cosmetic.** An earlier draft compared at `ty`'s own (narrow)
    /// width, using LLVM's plain `add`/`sub`/`mul` computed *at that
    /// narrow width already* — which silently wraps on overflow, the
    /// same as any two's-complement machine addition. That meant the
    /// check was comparing an *already-wrapped* value against the very
    /// bounds it's supposed to detect escaping — a wrapped 8-bit value
    /// is, by construction, always representable in 8 bits, so the
    /// check could never fire. Found by actually running a deliberately
    /// overflowing test program through a compiled binary and watching
    /// it exit 0 instead of aborting — not caught by reading the code.
    /// The fix (this function, plus `widen_to_i64`/`narrow_from_i64`
    /// bracketing every arithmetic op) keeps every intermediate value at
    /// `i64` until *after* this check has run, exactly matching how
    /// `interpreter.rs`'s `Value::Int(i64)` already worked all along.
    pub(super) fn guard_in_range(&mut self, val: &str, ty: &Ty, span: Span) -> Result<String, CodegenError> {
        if self.audited || self.smt_report.proven_in_range.contains(&span) || !ty.is_integer() {
            return Ok(val.to_string());
        }
        let (lo, hi) = ty.bounds();
        let ok_lo = self.fresh_reg("ge_lo");
        writeln!(self.out, "  {ok_lo} = icmp sge i64 {val}, {lo}").unwrap();
        let ok_hi = self.fresh_reg("le_hi");
        writeln!(self.out, "  {ok_hi} = icmp sle i64 {val}, {hi}").unwrap();
        let ok = self.fresh_reg("in_range");
        writeln!(self.out, "  {ok} = and i1 {ok_lo}, {ok_hi}").unwrap();
        let pass = self.fresh_label("range_ok");
        let fail = self.fresh_label("range_trap");
        writeln!(self.out, "  br i1 {ok}, label %{pass}, label %{fail}").unwrap();
        writeln!(self.out, "{fail}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{pass}:").unwrap();
        Ok(val.to_string())
    }


    /// The array-bounds analog of `guard_in_range` — checks `0 <= idx <
    /// dim` (same two-sided AND-combine shape), elided when `span` is
    /// already covered by `SmtReport::proven_index_bounds`. That set was
    /// already populated by both `refine.rs` (interval analysis) and
    /// `smt.rs` (real Z3) before this phase — codegen simply didn't have
    /// a check to elide it against yet (this fn is that consumer, the
    /// same relationship `guard_in_range` already has with
    /// `proven_in_range`).
    pub(super) fn guard_index_in_bounds(&mut self, idx: &str, dim: usize, span: Span) -> Result<(), CodegenError> {
        if self.audited || self.smt_report.proven_index_bounds.contains(&span) {
            return Ok(());
        }
        let ok_lo = self.fresh_reg("idx_ge_lo");
        writeln!(self.out, "  {ok_lo} = icmp sge i64 {idx}, 0").unwrap();
        let ok_hi = self.fresh_reg("idx_lt_dim");
        writeln!(self.out, "  {ok_hi} = icmp slt i64 {idx}, {dim}").unwrap();
        let ok = self.fresh_reg("idx_in_bounds");
        writeln!(self.out, "  {ok} = and i1 {ok_lo}, {ok_hi}").unwrap();
        let pass = self.fresh_label("idx_ok");
        let fail = self.fresh_label("idx_trap");
        writeln!(self.out, "  br i1 {ok}, label %{pass}, label %{fail}").unwrap();
        writeln!(self.out, "{fail}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{pass}:").unwrap();
        Ok(())
    }


    /// Shared by scalar `/`/`./`'s int-path guard and `agg_elementwise`'s
    /// per-element int `./` guard -- `if !audited && not already proven
    /// nonzero: icmp eq 0 -> br -> trap(abort+unreachable) / ok`, the
    /// same shape every Tier-1/2 guard in this file follows.
    pub(super) fn guard_nonzero_divisor(&mut self, divisor: &str, span: Span) {
        if self.audited || self.smt_report.proven_nonzero_divisor.contains(&span) {
            return;
        }
        let is_zero = self.fresh_reg("div_zero");
        writeln!(self.out, "  {is_zero} = icmp eq i64 {divisor}, 0").unwrap();
        let trap = self.fresh_label("div_trap");
        let ok = self.fresh_label("div_ok");
        writeln!(self.out, "  br i1 {is_zero}, label %{trap}, label %{ok}").unwrap();
        writeln!(self.out, "{trap}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{ok}:").unwrap();
    }


    /// Phase 5's fallible-runtime-call analog of `guard_nonzero_divisor`:
    /// traps if `ok_i32` (a `nir_inv`/`nir_solve`/`nir_kf_update_*` call
    /// result) is `0` — the singular-matrix case, which `interpreter.rs`
    /// surfaces as `ErrorKind::SingularMatrix` (an ordinary runtime
    /// `Err`, not a panic there either). No `proven_*` elision set
    /// applies here — matrix singularity is a genuine runtime data fact,
    /// not something either bounds-prover can decide statically — so
    /// this always emits the check unless `audited`, same convention
    /// every other Tier-2 guard in this file already follows.
    pub(super) fn guard_call_ok(&mut self, ok_i32: &str) {
        if self.audited {
            return;
        }
        let is_fail = self.fresh_reg("call_failed");
        writeln!(self.out, "  {is_fail} = icmp eq i32 {ok_i32}, 0").unwrap();
        let trap = self.fresh_label("singular_trap");
        let ok = self.fresh_label("singular_ok");
        writeln!(self.out, "  br i1 {is_fail}, label %{trap}, label %{ok}").unwrap();
        writeln!(self.out, "{trap}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{ok}:").unwrap();
    }


    /// A `tcp` runtime kernel's `i64` result is "bytes/fd on success, `-1`
    /// on failure" — negative traps, matching the interpreter's
    /// `ChannelIoError` being fatal (there's no `try`/`catch` anywhere in
    /// the language to recover from it), same `abort()` trap idiom as
    /// every other guard in this file.
    pub(super) fn guard_io_ok(&mut self, result_i64: &str) {
        if self.audited {
            return;
        }
        let is_fail = self.fresh_reg("io_failed");
        writeln!(self.out, "  {is_fail} = icmp slt i64 {result_i64}, 0").unwrap();
        let trap = self.fresh_label("io_trap");
        let ok = self.fresh_label("io_ok");
        writeln!(self.out, "  br i1 {is_fail}, label %{trap}, label %{ok}").unwrap();
        writeln!(self.out, "{trap}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{ok}:").unwrap();
    }


    /// `recv`'s `0` (peer closed) is *also* an error — `interpreter.rs`'s
    /// `read_tcp` treats `n == 0` as `ChannelIoError`, not a valid empty
    /// read (module doc's "one chunk, not a message boundary" note) — so
    /// this traps on `<= 0`, not just `< 0` like `guard_io_ok`.
    pub(super) fn guard_recv_ok(&mut self, result_i64: &str) {
        if self.audited {
            return;
        }
        let is_fail = self.fresh_reg("recv_failed");
        writeln!(self.out, "  {is_fail} = icmp sle i64 {result_i64}, 0").unwrap();
        let trap = self.fresh_label("recv_trap");
        let ok = self.fresh_label("recv_ok");
        writeln!(self.out, "  br i1 {is_fail}, label %{trap}, label %{ok}").unwrap();
        writeln!(self.out, "{trap}:").unwrap();
        // The flight recorder (`runtime-kernels/src/kernel/mod.rs`'s own
        // doc comment) fires here too, not just on `emit_c_main`'s normal
        // `ret` paths -- `abort()` bypasses that entirely, and a
        // recorder that goes silent on exactly the failures worth
        // recording (a trap, an admission denial) would defeat the
        // point of having one.
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{ok}:").unwrap();
    }


    /// `str_slice(s, start, end)`'s bounds check — traps unless `0 <=
    /// start <= end <= len`, same `abort()`-after-flight-recorder-dump
    /// trap idiom as `guard_io_ok`/`guard_recv_ok`. A wrong bounds check
    /// here would either read out-of-bounds memory or spuriously abort on
    /// every valid slice, so this is deliberately three separate `icmp`s
    /// ANDed together rather than one clever combined comparison.
    pub(super) fn guard_str_bounds_ok(&mut self, start: &str, end: &str, len: &str) {
        if self.audited {
            return;
        }
        let start_neg = self.fresh_reg("slice_start_neg");
        writeln!(self.out, "  {start_neg} = icmp slt i64 {start}, 0").unwrap();
        let start_gt_end = self.fresh_reg("slice_start_gt_end");
        writeln!(self.out, "  {start_gt_end} = icmp sgt i64 {start}, {end}").unwrap();
        let end_gt_len = self.fresh_reg("slice_end_gt_len");
        writeln!(self.out, "  {end_gt_len} = icmp sgt i64 {end}, {len}").unwrap();
        let bad1 = self.fresh_reg("slice_bad1");
        writeln!(self.out, "  {bad1} = or i1 {start_neg}, {start_gt_end}").unwrap();
        let is_fail = self.fresh_reg("slice_out_of_bounds");
        writeln!(self.out, "  {is_fail} = or i1 {bad1}, {end_gt_len}").unwrap();
        let trap = self.fresh_label("slice_trap");
        let ok = self.fresh_label("slice_ok");
        writeln!(self.out, "  br i1 {is_fail}, label %{trap}, label %{ok}").unwrap();
        writeln!(self.out, "{trap}:").unwrap();
        writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
        writeln!(self.out, "  call void @abort()").unwrap();
        writeln!(self.out, "  unreachable").unwrap();
        writeln!(self.out, "{ok}:").unwrap();
    }


}
