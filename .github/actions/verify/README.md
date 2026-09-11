# `nirdosha/verify` — CI/PR gating action

`nirdosha-master-plan.md` Part 3's "GitHub Action for CI/PR gating"
(parity target: Predictable, Sequent). Runs `nirdosha verify` against
your repo's `.nir` files and fails the job on any `DISPROVED` verdict —
a real, three-valued gate (`PROVED`/`DISPROVED`/`UNKNOWN`), never
collapsed to a plain pass/fail, matching `nirdosha verify`'s own
documented exit-code contract
(`docs/STABILITY_AND_RELEASES.md`).

## This action does not install `nirdosha`

There is no published release binary for this project yet
(`docs/STABILITY_AND_RELEASES.md`'s monthly cadence starts
2026-10-01) — a step before this one in your workflow must produce a
`nirdosha` binary, e.g.:

```yaml
- uses: actions/checkout@v4
  with:
    repository: kannamma-labs/nirdosha
    path: nirdosha-src
- run: cargo build --release
  working-directory: nirdosha-src/crates/compiler
- uses: kannamma-labs/nirdosha/.github/actions/verify@main
  with:
    nirdosha-path: ${{ github.workspace }}/nirdosha-src/target/release/nirdosha
```

Once a release binary or container image is published, a lighter setup
step (or a Docker-based variant of this action) becomes possible
without a full `cargo build` — tracked as a follow-up, not implemented
here yet.

## Usage (once a `nirdosha` binary is available)

```yaml
- uses: kannamma-labs/nirdosha/.github/actions/verify@main
  with:
    # Optional. Defaults to "nirdosha" (must resolve on PATH).
    nirdosha-path: /path/to/nirdosha
    # Optional. Defaults to every .nir file in the repo (excluding target/).
    files: "path/to/a.nir path/to/b.nir"
    # Optional. Defaults to "false" -- UNKNOWN (Z3 couldn't decide)
    # doesn't fail the job by default, only a real DISPROVED
    # counterexample does.
    fail-on-unknown: "false"
```

**Output:** `verdict-summary`, a JSON array of
`{"file": ..., "verdict": "PROVED"|"DISPROVED"|"UNKNOWN"|"ERROR"}`, one
entry per file checked — usable in a later step, e.g. to post a PR
comment summarizing results.

## How it decides pass/fail

`run.sh` reads `nirdosha verify`'s own exit code directly (`0`/`1`/`2`
== `PROVED`/`DISPROVED`/`UNKNOWN`) rather than parsing its JSON output —
one source of truth, and no `jq`/`python3` dependency required on the
runner. See `run.sh`'s own comment for why.

## Testing this action

`tests/run_test.sh` is a real, no-mocking end-to-end test suite —
fixture `.nir` files with known PROVED/DISPROVED/UNKNOWN verdicts, run
against a real, locally-built `nirdosha` binary (`target/debug/nirdosha`
or `target/release/nirdosha` in this repo, or `$NIRDOSHA_BIN`), checking
real exit codes and the real `$GITHUB_OUTPUT` file contents. Run it
directly:

```sh
cd crates/compiler && cargo build --bin nirdosha
.github/actions/verify/tests/run_test.sh
```

`.github/workflows/build.yml` also self-tests the action for real
inside an actual GitHub Actions run, against
`examples/features/01_scalar_types_and_literals.nir`.
