# nirdosha-speckit

A [GitHub Spec Kit](https://github.com/github/spec-kit) extension
(`nirdosha-master-plan.md` Part 3 Nov 2026's "Spec Kit extension") that
turns a feature's `spec.md` requirements into real Nirdosha `validate`
contracts and machine-checks that an implementation still satisfies
them, via `nirdosha certify`'s own JSON verdict — never a rollup based
on reading the code and judging it looks right.

## Commands

- **`/speckit.nirdosha-speckit.contracts`** — reads `spec.md`'s
  functional requirements, drafts/updates the matching `validate`
  block for each target function, and runs `nirdosha verify`
  immediately after each one — reporting `PROVED`/`DISPROVED`/
  `UNKNOWN` honestly, never assuming a contract that compiles is a
  contract that holds.
- **`/speckit.nirdosha-speckit.converge`** — for every `spec.md`
  requirement, checks whether a contract exists, whether its predicate
  still states the requirement (code drifts; contracts can silently
  stop matching what they were written for), and what `nirdosha
  certify` currently says about it. Reports a real fraction
  (`N PROVED and matching / total`), not a reassuring rollup.

See [`docs/examples/wallet-debit.md`](./docs/examples/wallet-debit.md)
for a full, real worked walkthrough — every JSON snippet in it is
copy-pasted from an actual `nirdosha verify`/`certify` run, not
illustrative output.

## Requirements

- [Spec Kit](https://github.com/github/spec-kit) `>=0.1.0`.
- A `nirdosha` binary reachable from wherever your agent runs shell
  commands (see the main repo's
  [`docs/STABILITY_AND_RELEASES.md`](https://github.com/kannamma-labs/nirdosha/blob/main/docs/STABILITY_AND_RELEASES.md)
  for the current install story — no tagged release binary exists yet
  as of this extension's `0.1.0`; build from source with
  `cargo build --release` in `crates/compiler`).

## Install (development)

```sh
specify extension add --dev /path/to/nirdosha/extensions/nirdosha-speckit
```

## Publishing status

**Not yet submitted to the Spec Kit community catalog.** Submission is
a manual step (a GitHub issue against `github/spec-kit` using their
"Extension Submission" template, per their own
`extensions/EXTENSION-PUBLISHING-GUIDE.md`) that names this project's
maintainer as the submitter on a third-party repository — left for a
human to do deliberately, not filed automatically. This extension is
otherwise complete and real: manifest validated against the published
schema, both commands written and worked through end to end against
the actual compiler (see the worked example above).

## What this extension does not do

It does not invent test cases, does not run the compiled binary, and
does not touch anything outside `validate` blocks and the specific
function bodies a `DISPROVED` result required fixing. It also can't
express every requirement as a `validate` predicate — non-functional
requirements (throughput, latency) belong in a Nirdosha `nfr(...)`
annotation instead, and a requirement Z3's Tier 1 can't model yet
(see the main repo's `docs/PUBLIC_ROADMAP.md` for current, disclosed
gaps — e.g. a predicate built on a division result) is reported
honestly as `UNKNOWN`, never silently dropped or claimed as proved.
