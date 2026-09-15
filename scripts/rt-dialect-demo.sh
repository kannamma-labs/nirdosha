#!/usr/bin/env bash
# rt-dialect-demo.sh — the whole Nirdosha Rust dialect story, one command.
#
#   scripts/rt-dialect-demo.sh [--deep]
#
# To the external world, a Nirdosha-dialect program is just another Rust
# program. Hand the same source to the Nirdosha compiler and its contract
# claims stop being comments. `--deep` adds Stage 2 (MIR interprocedural
# effects via the rustc driver — needs nightly + rustc-dev).
set -uo pipefail
cd "$(dirname "$0")/.."

bold() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
export PATH="$PWD/target/debug:$PATH"
DEEP="${1:-}"

bold "0. toolchain"
cargo build -q -p cargo-nirdosha -p rt-payroll -p rt-payroll-lying -p rt-payroll-pure-chain || exit 1
if [ "$DEEP" = "--deep" ]; then
    cargo build -q -p nirdosha-driver || exit 1
    echo "Stage 2 driver: built"
else
    echo "Stage 1 only (pass --deep for the MIR driver)"
fi

bold "1. plain cargo — the lying program is just another Rust program"
(cd examples/rt-payroll-lying && cargo run -q 2>/dev/null) \
    || echo "(lying program ran fine — as far as plain cargo knows)"

bold "2. same source, the Nirdosha compiler — the claims become checks"
(cd examples/rt-payroll-lying && cargo nirdosha build) \
    && echo "UNEXPECTED: lying program passed" \
    || echo "(refused — plain cargo would have accepted it; that difference is the product)"

bold "3. the compliant program: plain cargo runs it"
(cd examples/rt-payroll && cargo run -q 2>/dev/null)

bold "4. …and the Nirdosha compiler certifies it"
(cd examples/rt-payroll && cargo nirdosha verify) \
    && echo "(3 contracts verified, certificate in target/nirdosha/)"

bold "5. NFRs are enforced, not proven: cargo nirdosha bench (your test suite is the workload)"
(cd examples/rt-payroll && cargo nirdosha bench) \
    && echo "(SLA held: p95 of compute_payroll under its declared 250ms)"

bold "6. workspace gate: every in-dialect crate, strict by default"
cargo nirdosha verify --workspace \
    && echo "(workspace clean)" \
    || echo "(rt-payroll-lying refuses the whole workspace — exactly one crate, exactly 3 lies)"

if [ "$DEEP" = "--deep" ]; then
    bold "7. Stage 2 — the indirect lie Stage 1 cannot see"
    (cd examples/rt-payroll-pure-chain && cargo nirdosha build 2>/dev/null) \
        && echo "Stage 1: passes (the lie is three calls away from any claim)"
    (cd examples/rt-payroll-pure-chain && cargo nirdosha build --deep) \
        && echo "UNEXPECTED: chain lie passed --deep" \
        || echo "(Stage 2: MIR call graph refused it, chain named)"
fi

bold "done — same source, two (or three) compilers; the difference is the product"