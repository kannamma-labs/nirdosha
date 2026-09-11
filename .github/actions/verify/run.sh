#!/bin/sh
# The composite action's real logic (action.yml just wires env vars to
# this) -- kept as a plain, portable `sh` script rather than inline YAML
# `run:` so it can be unit-tested directly (tests/run_test.sh) without a
# GitHub Actions runner, the same "test the logic, not the YAML" split
# nirdosha's own compiler tests apply everywhere else in this repo.
#
# Deliberately no JSON parsing dependency (no `jq`, no `python3 -c`) to
# decide pass/fail: `nirdosha verify`'s own exit code is already the
# three-valued signal (0/1/2 == PROVED/DISPROVED/UNKNOWN,
# docs/STABILITY_AND_RELEASES.md's own external-contract guarantee for
# this exact code), so this script reads `$?` directly rather than
# re-deriving the same fact from stdout -- one source of truth, and one
# fewer external tool a CI runner needs preinstalled.
set -u

BIN="${NIRDOSHA_BIN:-nirdosha}"
FAIL_ON_UNKNOWN="${NIRDOSHA_ACTION_FAIL_ON_UNKNOWN:-false}"

if ! command -v "$BIN" >/dev/null 2>&1; then
    echo "::error::nirdosha binary not found at '$BIN' -- install it first (cargo install --path crates/compiler, a container step, etc.) or set the nirdosha-path input"
    exit 1
fi

if [ -n "${NIRDOSHA_ACTION_FILES:-}" ]; then
    # shellcheck disable=SC2086 # deliberate word-splitting: a
    # space-separated file list is the documented input shape.
    set -- $NIRDOSHA_ACTION_FILES
else
    # `target/` is excluded so a workspace with build artifacts checked
    # out (or a submodule vendoring another nirdosha project) doesn't
    # get its own dependency's example/test fixtures re-verified.
    files=$(find . -name '*.nir' -not -path '*/target/*' | sort)
    # shellcheck disable=SC2086
    set -- $files
fi

if [ "$#" -eq 0 ]; then
    echo "no .nir files found to verify"
    {
        echo "verdict-summary<<NIRDOSHA_EOF"
        echo "[]"
        echo "NIRDOSHA_EOF"
    } >>"$GITHUB_OUTPUT"
    exit 0
fi

overall_exit=0
entries=""

for f in "$@"; do
    echo "::group::nirdosha verify $f"
    "$BIN" verify "$f"
    code=$?
    echo "::endgroup::"

    case $code in
        0) verdict=PROVED ;;
        1) verdict=DISPROVED ;;
        2) verdict=UNKNOWN ;;
        *) verdict=ERROR ;;
    esac

    if [ -n "$entries" ]; then
        entries="$entries,"
    fi
    entries="$entries{\"file\":\"$f\",\"verdict\":\"$verdict\"}"

    if [ "$verdict" = "DISPROVED" ] || [ "$verdict" = "ERROR" ]; then
        echo "::error file=$f::nirdosha verify reported $verdict (exit $code)"
        overall_exit=1
    elif [ "$verdict" = "UNKNOWN" ] && [ "$FAIL_ON_UNKNOWN" = "true" ]; then
        echo "::error file=$f::nirdosha verify reported UNKNOWN and fail-on-unknown is true"
        overall_exit=1
    fi
done

{
    echo "verdict-summary<<NIRDOSHA_EOF"
    echo "[$entries]"
    echo "NIRDOSHA_EOF"
} >>"$GITHUB_OUTPUT"

exit $overall_exit
