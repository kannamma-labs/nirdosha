# MIR numeric proofs (GitHub #67)

`nirdosha-driver` now checks numeric assertions in rustc's pre-optimization
runtime MIR. The native `.nir` compiler and MIR frontend share the integer
encoder in `crates/nirdosha-smt-core`; neither frontend depends on the other.

For example, Rust `if x > y { 100 / (x - y) } else { 0 }` with `u32`
arguments proves both subtraction without overflow and division by a nonzero
value. Independent intervals cannot recover this relation. The driver uses
these proofs when checking `effects(pure)`, so guarded division and array
indexing no longer fail merely because MIR contains an assertion.

## Research and semantics

The implementation was checked against the installed rustc source
(`rustc 1.100.0-nightly (8fa1c96cf 2026-08-17)`) and these primary sources:

- [Rust operator reference](https://doc.rust-lang.org/reference/expressions/operator-expr.html):
  signed division truncates toward zero, remainder follows the dividend,
  and signed `MIN / -1` and `MIN % -1` overflow even with checks disabled.
- [MIR binary operations](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_middle/mir/enum.BinOp.html):
  checked arithmetic returns a wrapped integer and a separate overflow flag;
  shifts use the offset modulo the left operand's width.
- [MIR terminators](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_middle/mir/enum.TerminatorKind.html):
  `Assert` compares its condition with `expected`. Disabled add/sub/mul,
  shift, and negation overflow assertions act as `goto`, so their conditions
  must not be assumed on the continuation.
- [Z3 arithmetic guide](https://microsoft.github.io/z3guide/docs/theories/Arithmetic/):
  SMT arithmetic does not itself supply Rust's bounded integer and partial
  division semantics. Nonlinear arithmetic can return unknown.
- [Kani loop contracts](https://model-checking.github.io/kani/reference/experimental/loop-contracts.html):
  general loop proofs need inductive invariants. Merely checking a finite
  prefix does not establish the property for arbitrary iterations.

The shared `div_rem` relation is extracted unchanged from the native
compiler. It describes normal continuation after a nonzero-divisor guard;
introducing it before checking that guard would incorrectly exclude zero.
Integer ranges include all signed/unsigned widths through 128 bits, with
`isize`/`usize` taken from the compilation target. Arithmetic wraps to its
Rust width while overflow flags compare the mathematical result with the
range. Integer casts truncate/sign-extend; bit operations use bitvectors.

The driver explores each normal control-flow path separately. A check is
proven only when every visit proves its condition. Unknown expression
results are fresh typed values. Calls and indirect writes forget local
facts, so stale values cannot survive an unknown mutation. Cycles,
unsupported control flow, or exceeding 2048 visits/256 nesting depth discard
all proofs for that body. Cleanup paths, loop invariants, numeric call
summaries, and slice-length relationships are not modeled yet.

Each Z3 query is limited to 200ms and 100000 resource units. Only `unsat`
proves an obligation. Satisfiable, unknown, or timeout retains the guard. Bounded solver outcomes
can vary across machines; deterministic serialization does not eliminate
resource-dependent unknown results.
Previously enabled guards may be assumed on their normal continuation;
these records establish per-assertion partial correctness, not whole-program
panic freedom or termination. No function preconditions are invented:
`requires(expr)` remains the companion grammar issue.

## Run the examples

From the repository root (nightly plus `rustc-dev`, system Z3):

```sh
cargo build -p nirdosha-driver
mkdir -p /tmp/rt-numeric
target/debug/nirdosha-driver examples/rt-numeric-proofs.rs --crate-type lib --out-dir /tmp/rt-numeric -C overflow-checks=yes
cargo run -p nirdosha -- build examples/features/58_numeric_smt_proofs.nir -o /tmp/native-numeric
/tmp/native-numeric
```

The native example prints `25`, `0`, and `-3`. It demonstrates correlated
nonzero-divisor reasoning and signed truncation using the `.nir` frontend;
the Rust example exercises the new MIR frontend directly.

`cargo nirdosha build --deep` also runs the numeric pass. Per-invocation MIR
certificates are written beneath rustc's output directory in `nirdosha/mir/`
(the exact prefix follows Cargo's artifact layout and is printed by the driver). Set
`NIRDOSHA_MIR_CERT_DIR` to choose a directory explicitly. The filename
includes rustc's crate name and extra filename, plus a test suffix where
applicable, so Cargo targets do not overwrite each other's evidence.

The existing `nirdosha.certificate/v1` envelope contains `tool.mode =
"mir_driver"`, exact driver build toolchain, compilation arguments, backend,
per-function solver query counts, per-check outcomes, source hashes, and
nonempty `proofs` for successful discharges. Attaching proofs recomputes the
binding; changing a proof breaks integrity verification. Reports are
published only after the rustc invocation succeeds. Source-scan reports
remain separate. `elidable: true` records proof eligibility; this change
does **not** rewrite MIR or remove runtime assertions (the follow-on in #65).

These are unsigned discharge records, not independently replayable solver
proof objects. They do not certify executable bytes or dependency closure.
Source files outside the package are listed as excluded. The existing
`check-certificate` guarantee policy continues to reject this new analysis
mode until it gains an explicit numeric-proof policy.

## Z3-independent fallback and tests

```sh
cargo test -p nirdosha-smt-core -p nirdosha-driver
cargo test -p nirdosha-smt-core -p nirdosha-driver --no-default-features
cargo test -p nirdosha --test smt --test contract_check
```

The driver and shared crate default to `smt`. `--no-default-features`
substitutes interval analysis with explicit `backend: "interval"` evidence
and zero solver queries. It can prove constants and bounded arithmetic,
but has no branch narrowing or relational reasoning; unsupported/wrapping
ranges become unknown. This build has no Z3 dependency. `--features dist`
vendors Z3 as in the native compiler. Build the fallback packages separately
from the native compiler, whose Z3 feature would otherwise unify with them.

`z3_is_invoked_for_a_relational_mir_proof` compiles the relational fixture at
O0 and O3, checks the counter incremented at `Solver::check`, and requires
Z3-tagged overflow and divisor proof records. The same test under the
interval build requires rejection. Other regressions cover unsafe paths,
joins, loops, mutation, narrowing casts, array bounds, signed division and
remainder overflow, shifts, u128 bounds, disabled overflow guards, and
certificate tampering/determinism. The native example is also tested against
both native Z3 and interval analyses.
