#!/bin/sh
# End-to-end tests for run.sh, against a real, locally-built nirdosha
# binary -- no GitHub Actions runner needed (there's no `act` in this
# environment), but every assertion here is against the script's real
# behavior, not a mock: real files, the real binary, real exit codes,
# a real $GITHUB_OUTPUT file parsed back the same way the Actions
# runtime does.
set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")/.." && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../../.." && pwd)

NIRDOSHA_BIN="${NIRDOSHA_BIN:-}"
if [ -z "$NIRDOSHA_BIN" ]; then
    for profile in debug release; do
        candidate="$REPO_ROOT/target/$profile/nirdosha"
        if [ -x "$candidate" ]; then
            NIRDOSHA_BIN="$candidate"
            break
        fi
    done
fi
if [ -z "$NIRDOSHA_BIN" ] || [ ! -x "$NIRDOSHA_BIN" ]; then
    echo "SKIP: no built nirdosha binary found under target/debug or target/release -- run 'cargo build --bin nirdosha' in crates/compiler first"
    exit 0
fi
export NIRDOSHA_BIN

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

pass_count=0
fail_count=0

assert_eq() {
    label="$1"
    expected="$2"
    actual="$3"
    if [ "$expected" = "$actual" ]; then
        pass_count=$((pass_count + 1))
        echo "ok   - $label"
    else
        fail_count=$((fail_count + 1))
        echo "FAIL - $label: expected [$expected], got [$actual]"
    fi
}

assert_contains() {
    label="$1"
    haystack="$2"
    needle="$3"
    case "$haystack" in
        *"$needle"*)
            pass_count=$((pass_count + 1))
            echo "ok   - $label"
            ;;
        *)
            fail_count=$((fail_count + 1))
            echo "FAIL - $label: expected to find [$needle] in [$haystack]"
            ;;
    esac
}

# --- fixtures ---------------------------------------------------------

cat >"$WORKDIR/proved.nir" <<'EOF'
fn add(a: i64, b: i64) -> i64 {
    return a + b
}
EOF

cat >"$WORKDIR/disproved.nir" <<'EOF'
fn bad(a: i64) -> i64 {
    return a - 1
}

validate bad {
    post: result >= a
}
EOF

cat >"$WORKDIR/unknown.nir" <<'EOF'
fn flip(x: bool) -> bool {
    return x
}

validate flip {
    post: true
}
EOF

# --- test 1: a single PROVED file exits 0, output records PROVED ------

GITHUB_OUTPUT="$WORKDIR/gh_output_1"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/proved.nir"
unset NIRDOSHA_ACTION_FAIL_ON_UNKNOWN || true
set +e
"$SCRIPT_DIR/run.sh" >"$WORKDIR/stdout_1" 2>&1
code=$?
set -e
assert_eq "a PROVED-only run exits 0" "0" "$code"
assert_contains "output records PROVED for the proved file" "$(cat "$GITHUB_OUTPUT")" "\"verdict\":\"PROVED\""

# --- test 2: a DISPROVED file exits 1 and is annotated -----------------

GITHUB_OUTPUT="$WORKDIR/gh_output_2"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/disproved.nir"
set +e
out=$("$SCRIPT_DIR/run.sh" 2>&1)
code=$?
set -e
assert_eq "a DISPROVED file fails the job (exit 1)" "1" "$code"
assert_contains "a DISPROVED file gets an ::error annotation" "$out" "::error file=$WORKDIR/disproved.nir::nirdosha verify reported DISPROVED"
assert_contains "output records DISPROVED" "$(cat "$GITHUB_OUTPUT")" "\"verdict\":\"DISPROVED\""

# --- test 3: UNKNOWN does not fail the job by default ------------------

GITHUB_OUTPUT="$WORKDIR/gh_output_3"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/unknown.nir"
set +e
"$SCRIPT_DIR/run.sh" >/dev/null 2>&1
code=$?
set -e
assert_eq "an UNKNOWN file does not fail the job by default" "0" "$code"
assert_contains "output records UNKNOWN" "$(cat "$GITHUB_OUTPUT")" "\"verdict\":\"UNKNOWN\""

# --- test 4: fail-on-unknown=true makes UNKNOWN fail the job -----------

GITHUB_OUTPUT="$WORKDIR/gh_output_4"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/unknown.nir"
export NIRDOSHA_ACTION_FAIL_ON_UNKNOWN=true
set +e
"$SCRIPT_DIR/run.sh" >/dev/null 2>&1
code=$?
set -e
assert_eq "fail-on-unknown=true makes an UNKNOWN file fail the job" "1" "$code"
unset NIRDOSHA_ACTION_FAIL_ON_UNKNOWN

# --- test 5: multiple files, mixed verdicts, one summary ---------------

GITHUB_OUTPUT="$WORKDIR/gh_output_5"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/proved.nir $WORKDIR/disproved.nir"
set +e
"$SCRIPT_DIR/run.sh" >/dev/null 2>&1
code=$?
set -e
assert_eq "a mixed PROVED+DISPROVED batch fails the job" "1" "$code"
summary=$(cat "$GITHUB_OUTPUT")
assert_contains "batch summary records both files" "$summary" "proved.nir"
assert_contains "batch summary records both verdicts" "$summary" "\"verdict\":\"PROVED\""

# --- test 6: no files found is a clean no-op, not a failure ------------

EMPTY_DIR=$(mktemp -d)
GITHUB_OUTPUT="$WORKDIR/gh_output_6"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
unset NIRDOSHA_ACTION_FILES
(
    cd "$EMPTY_DIR"
    set +e
    "$SCRIPT_DIR/run.sh" >/dev/null 2>&1
    echo $? >"$WORKDIR/empty_dir_code"
)
code=$(cat "$WORKDIR/empty_dir_code")
rm -rf "$EMPTY_DIR"
assert_eq "no .nir files found exits 0" "0" "$code"
assert_eq "no .nir files found records an empty summary" "verdict-summary<<NIRDOSHA_EOF
[]
NIRDOSHA_EOF" "$(cat "$GITHUB_OUTPUT")"

# --- test 7: a missing binary is a clear failure, not a crash ----------

GITHUB_OUTPUT="$WORKDIR/gh_output_7"
: >"$GITHUB_OUTPUT"
export GITHUB_OUTPUT
export NIRDOSHA_ACTION_FILES="$WORKDIR/proved.nir"
set +e
out=$(NIRDOSHA_BIN=/nonexistent-nirdosha-binary-for-this-test "$SCRIPT_DIR/run.sh" 2>&1)
code=$?
set -e
assert_eq "a missing binary fails cleanly" "1" "$code"
assert_contains "a missing binary gets a clear error message" "$out" "nirdosha binary not found"

echo
echo "$pass_count passed, $fail_count failed"
[ "$fail_count" -eq 0 ]
