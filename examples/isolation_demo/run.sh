#!/usr/bin/env bash
# Drives the full adversarial demo end to end: builds
# race_probe_transact_checked.nir, starts a real local HTTP listener on
# NIRDOSHA_OBSERVABILITY_URL (listener.py -- see its own doc comment),
# runs the compiled binary against a fresh ledger and a fresh transact
# durability log, then reports both halves of the result: the ledger
# drift (the corruption itself) and the anomaly escalations the
# listener actually received on the wire (the checker catching it).
#
# Must be run from the repo root (the .nir file's own db_connect/
# log paths are relative to it, matching every other example in this
# repo). Needs `clang`/LLVM on PATH (same as any `nirdosha build`) and
# python3.
set -euo pipefail
cd "$(dirname "$0")/../.."

NIR_FILE="examples/isolation_demo/race_probe_transact_checked.nir"
BIN="/tmp/nirdosha_isolation_demo_bin"
PORT=8073
ANOMALY_LOG="/tmp/nirdosha_isolation_demo_anomalies.jsonl"
TRANSACT_LOG="/tmp/nirdosha_isolation_demo_transact_log.sqlite"
LEDGER_DB="examples/isolation_demo/isolation_probe.db"

echo "== building =="
./target/release/nirdosha build "$NIR_FILE" -o "$BIN"

echo "== starting listener on 127.0.0.1:$PORT =="
python3 examples/isolation_demo/listener.py "$PORT" "$ANOMALY_LOG" &
LISTENER_PID=$!
trap 'kill "$LISTENER_PID" 2>/dev/null || true' EXIT
sleep 0.3 # let the listener actually bind before the probe can race it

rm -f "$LEDGER_DB" "$TRANSACT_LOG" "$TRANSACT_LOG-wal" "$TRANSACT_LOG-shm"

echo "== running (fresh ledger, fresh transact log, real escalation channel) =="
NIRDOSHA_OBSERVABILITY_URL="http://127.0.0.1:$PORT/ingest" \
NIRDOSHA_TRANSACT_LOG_PATH="$TRANSACT_LOG" \
  "$BIN"

sleep 0.5 # escalate() is async (its own std::thread::spawn) -- give the last few a moment to land
kill "$LISTENER_PID" 2>/dev/null || true
trap - EXIT

echo
echo "== anomalies the listener actually received =="
if [ -s "$ANOMALY_LOG" ]; then
  wc -l < "$ANOMALY_LOG" | xargs echo "count:"
  cat "$ANOMALY_LOG"
else
  echo "count: 0"
fi
