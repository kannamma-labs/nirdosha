#!/usr/bin/env python3
"""A tiny, real HTTP/1.1 listener for `NIRDOSHA_OBSERVABILITY_URL` --
the exact same escalation channel `nfr.rs`'s own annotations use
(`docs/LANGUAGE.md`, `examples/features/49_nfr.nir`), here receiving
`isolation_check.rs`'s `escalate()` POSTs instead. Nothing about this
script is Nirdosha-specific machinery -- it exists purely so this
demo's own claim ("the checker really escalated N anomalies") is
independently checkable, not asserted: one JSON line logged per POST
actually received, on the wire, from the real compiled binary's own
background escalation thread.

`post_json_fire_and_forget` (`nfr.rs`) never reads a response and
closes the connection itself (`Connection: close`), so this handler
does the minimum: parse `Content-Length`, read exactly that many body
bytes, append them as one line to the given log file, and reply `200`
(harmless, ignored by the client either way).

Usage:
    python3 listener.py <port> <log_file>
Runs until killed (SIGTERM/SIGINT) -- `run.sh` starts it in the
background and stops it after the probe binary exits.
"""
import http.server
import socketserver
import sys
import threading


def make_handler(log_path):
    lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass  # keep run.sh's own output clean; the log file is the record

        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length).decode("utf-8", errors="replace")
            with lock:
                with open(log_path, "a") as f:
                    f.write(body + "\n")
            self.send_response(200)
            self.end_headers()

    return Handler


def main():
    port = int(sys.argv[1])
    log_path = sys.argv[2]
    open(log_path, "w").close()  # truncate/create fresh for this run
    with socketserver.ThreadingTCPServer(("127.0.0.1", port), make_handler(log_path)) as httpd:
        httpd.serve_forever()


if __name__ == "__main__":
    main()
