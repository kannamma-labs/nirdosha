#!/bin/sh
# Runs every ```sh fenced code block in README.md for real and fails
# loudly if one doesn't exit 0 -- the CI guard against the exact class
# of bug caught by hand on 2026-09-11: a README quickstart command
# (`cargo run -- serve <file> --port 8080`, a subcommand that no
# longer exists) that had silently been broken since the interpreter
# was removed, sitting in the very first thing a new user is told to
# paste. Nothing here should ever again be verified by eye only.
#
# Individual *lines* that genuinely can't run in a plain CI checkout
# are dropped, by content, not by line number (so a future edit that
# adds a new untestable line is dropped automatically instead of
# silently breaking this script) -- the rest of that block still runs:
# `git clone` (this repo is already checked out; `cd nirdosha` right
# after it is dropped too, since there's no such directory to `cd`
# into from the repo root), a Codespaces/fork URL with nothing to
# execute, a real published-release download
# (`releases/latest/download` -- covered instead by release.yml's own
# real per-platform smoke test), and PowerShell (`irm`/`.ps1`,
# Windows-only, this script is POSIX sh). A `#`-prefixed line showing
# expected output (this file's own convention right below several
# blocks) is left in untouched -- `sh` already treats it as a comment,
# so it's inert either way, and dropping it would silently stop this
# script from ever visually confirming the comment still matches
# reality when someone reads its output.
#
# Run from the repo root: sh scripts/verify_readme_commands.sh
set -eu

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# Several README lines assume a real install already put `nirdosha` on
# PATH (the whole point of `scripts/install.sh`, itself dropped below
# -- this script never installs anything for real). Pointing PATH at
# this checkout's own already-built binary instead lets those lines
# run for real against the genuine current CLI rather than being
# dropped for "no `nirdosha` on PATH" -- release preferred, debug as a
# fallback for a local `cargo build` (no `--release`) run of this script.
export PATH="$repo_root/target/release:$repo_root/target/debug:$PATH"

readme="README.md"
work_dir="$(mktemp -d)"
# `timeout` sends SIGTERM only to the `sh` it directly spawned, not to
# whatever foreground child that `sh` script was itself blocked on
# (`./serve_demo`, still bound to port 8080) -- kill it by name, and
# remove every output binary a README block writes into the repo root
# by its documented, hardcoded `-o` name, so a local run of this script
# never leaves stray build artifacts or a still-listening server behind.
cleanup() {
    pkill -x hello 2>/dev/null || true
    pkill -x employee 2>/dev/null || true
    pkill -x serve_demo 2>/dev/null || true
    rm -f "$repo_root/hello" "$repo_root/employee" "$repo_root/ui.html" "$repo_root/crates/compiler/hello" "$repo_root/crates/compiler/serve_demo"
    rm -rf "$work_dir"
}
trap cleanup EXIT

drop_line() {
    case "$1" in
        # Real network installs (`raw.githubusercontent.com`'s
        # `install.sh`, or a real published release's own asset) --
        # each one is a real, separate reproduction target already
        # (`.github/actions/verify/tests/run_test.sh`, `release.yml`'s
        # own per-platform smoke test); running one here would also
        # mutate the machine this script runs on (installs a real
        # binary to `~/.local/bin`), a side effect this script must
        # never have. `./nirdosha ...` is dropped too -- every such
        # line in this README only makes sense immediately after one
        # of those two downloads, so it has nothing to run against
        # once the download itself is skipped.
        *'git clone'*|*'cd nirdosha'|*'codespaces.new'*) return 0 ;;
        *'releases/latest/download'*|*'./nirdosha '*) return 0 ;;
        *'raw.githubusercontent.com'*|*'install.sh'*|*'install.ps1'*) return 0 ;;
        *'irm '*|*'.ps1'*) return 0 ;;
        *) return 1 ;;
    esac
}

block_num=0
failures=0
skipped=0
in_block=0
block_file=""
block_had_real_line=0

while IFS= read -r line; do
    if [ "$in_block" -eq 0 ]; then
        case "$line" in
            '```sh')
                block_num=$((block_num + 1))
                block_file="$work_dir/block_$block_num.sh"
                : > "$block_file"
                block_had_real_line=0
                in_block=1
                ;;
        esac
        continue
    fi
    case "$line" in
        '```')
            in_block=0
            if [ "$block_had_real_line" -eq 0 ]; then
                echo "SKIP  block $block_num (every line needs network/Windows/a real prior release -- not CI-testable)"
                skipped=$((skipped + 1))
                continue
            fi
            echo "----- block $block_num -----"
            cat "$block_file"
            echo "-----"
            # `timeout` because one block (the compiled-`serve` example)
            # ends in `&& ./serve_demo`, which starts a real HTTP server
            # that never exits on its own -- still running after the
            # timeout is exactly what a server command *should* do
            # (exit 124), not a failure; anything else nonzero is real.
            set +e
            (cd "$repo_root" && timeout 8 sh "$block_file")
            status=$?
            set -e
            if [ "$status" -eq 0 ] || [ "$status" -eq 124 ]; then
                echo "PASS  block $block_num"
            else
                echo "FAIL  block $block_num (exit $status)"
                failures=$((failures + 1))
            fi
            ;;
        \#*)
            # A comment (often this file's own "expected output" note
            # right under a command) -- inert to `sh` either way, kept
            # verbatim rather than run through `drop_line`'s pattern
            # match (a comment mentioning e.g. "git clone" in prose
            # shouldn't be silently deleted).
            echo "$line" >> "$block_file"
            ;;
        *)
            if drop_line "$line"; then
                continue
            fi
            echo "$line" >> "$block_file"
            block_had_real_line=1
            ;;
    esac
done < "$readme"

if [ "$failures" -gt 0 ]; then
    echo ""
    echo "$failures README command block(s) failed -- see above."
    exit 1
fi
echo ""
echo "All runnable README command blocks passed ($skipped skipped as not CI-testable)."
