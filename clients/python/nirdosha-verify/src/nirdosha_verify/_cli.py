"""``nirdosha-verify`` console-script entry point.

A pure passthrough to ``nirdosha verify <args...>`` on the located
binary (see :func:`nirdosha_verify._binary_path`) -- ``os.execvp``
replaces this process's image entirely rather than spawning a child
and re-raising its exit code, so stdout/stderr/exit code/signal
handling are all exactly the real binary's own, not a re-derived copy
that could drift. This mirrors the whole package's own "thin client"
framing: the CLI entry point is thin even relative to the rest of the
package, since :func:`nirdosha_verify.verify` at least parses JSON --
this doesn't even do that.
"""

from __future__ import annotations

import os
import sys

from . import _binary_path, NirdoshaBinaryNotFound


def main() -> None:
    try:
        binary = _binary_path()
    except NirdoshaBinaryNotFound as e:
        print(f"nirdosha-verify: {e}", file=sys.stderr)
        raise SystemExit(1) from None
    os.execvp(binary, [binary, "verify", *sys.argv[1:]])


if __name__ == "__main__":
    main()
