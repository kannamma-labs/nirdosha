"""Thin Python client for ``nirdosha verify``/``fix``/``certify``.

``nirdosha-master-plan.md`` Part 3 Sprint 1's PyPI thin client (parity
target: dottxt/Outlines' own distribution model -- a pip-installable
package that makes a Rust-backed tool reachable from Python without
reimplementing it). Every function here shells out to a locally
available ``nirdosha`` executable and parses its JSON output --
*exactly* the same ``run_verify_pipeline`` result the CLI itself prints
(``crates/compiler/src/main.rs``), over a subprocess boundary, never a
second implementation of the verification logic.

**v0 scope, disclosed honestly:** this package does not bundle or
download a prebuilt ``nirdosha`` binary. It expects one already
reachable -- on ``PATH``, or pointed to via the ``NIRDOSHA_BIN``
environment variable. Auto-fetching a platform-specific binary from a
real GitHub release is a real, later item, gated on nirdosha actually
publishing tagged release binaries
(``docs/STABILITY_AND_RELEASES.md``'s monthly cadence, starting
2026-10-01) -- faking that today would mean silently downloading
nothing, or a placeholder that isn't really there. See
:class:`NirdoshaBinaryNotFound`.

A ``DISPROVED``/``UNKNOWN`` verdict is not an exception here: a caller
asking "does this code pass" needs the answer "no" to come back as
data (the same dict shape the JSON always has), not as a raised
exception -- exceptions are reserved for this *client* failing to do
its job (:class:`NirdoshaBinaryNotFound`, :class:`NirdoshaError`), the
same distinction ``nirdosha mcp``'s own ``isError`` field draws between
a protocol/tool failure and a normal, informative "no."
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

__all__ = [
    "NirdoshaBinaryNotFound",
    "NirdoshaError",
    "verify",
    "fix",
    "certify",
]


class NirdoshaBinaryNotFound(RuntimeError):
    """No ``nirdosha`` executable could be located.

    Checked in order: the ``NIRDOSHA_BIN`` environment variable (an
    explicit override, for CI or a non-``PATH`` install), then
    ``PATH`` (:func:`shutil.which`) -- never a bundled binary in v0
    (see the module docstring for why).
    """


class NirdoshaError(RuntimeError):
    """The ``nirdosha`` binary ran but produced something this client
    can't interpret -- e.g. no parseable JSON on stdout at all
    (a real crash, or an unexpected CLI usage error). Distinct from a
    normal ``DISPROVED``/``UNKNOWN`` verdict, which is not an error.
    """


def _binary_path() -> str:
    override = os.environ.get("NIRDOSHA_BIN")
    if override:
        return override
    found = shutil.which("nirdosha")
    if found is None:
        raise NirdoshaBinaryNotFound(
            "no `nirdosha` executable found on PATH and NIRDOSHA_BIN is not set -- "
            "install the nirdosha compiler (https://github.com/kannamma-labs/nirdosha) "
            "or point NIRDOSHA_BIN at its binary"
        )
    return found


def _run_on_source(subcommand: list[str], source: str) -> dict[str, Any]:
    """Writes ``source`` to a temp ``.nir`` file, runs
    ``nirdosha <subcommand...> <tempfile>``, parses stdout as JSON, and
    always removes the temp file afterward, even on an exception --
    the same "materialize inline source to a real file for a
    path-only pipeline" bridge ``main.rs``'s own MCP handlers use
    (``TempNirFile``), for the identical reason: every nirdosha CLI
    command still only takes a file path, never inline source.
    """
    binary = _binary_path()
    fd, path = tempfile.mkstemp(suffix=".nir")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(source)
        completed = subprocess.run([binary, *subcommand, path], capture_output=True, text=True)
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as e:
            raise NirdoshaError(
                f"`nirdosha {' '.join(subcommand)}` did not print valid JSON on stdout "
                f"(exit {completed.returncode}): {completed.stderr.strip()}"
            ) from e
    finally:
        os.remove(path)


def verify(source: str) -> dict[str, Any]:
    """Runs ``nirdosha verify`` against ``source`` and returns the
    parsed JSON verdict unchanged: ``"verdict"`` is one of
    ``"PROVED"``/``"DISPROVED"``/``"UNKNOWN"``, alongside every stage's
    own diagnostics -- see the nirdosha repo's ``docs/LANGUAGE.md`` for
    the exact schema.
    """
    return _run_on_source(["verify"], source)


def fix(source: str, *, apply: bool = False) -> dict[str, Any]:
    """Runs ``nirdosha fix`` against ``source``.

    ``apply=True`` mirrors the CLI's own ``--apply`` in spirit, but
    since there is no file on this side of the subprocess boundary for
    the CLI to rewrite and hand back a path to, the patched text is
    read back from the temp file into ``result["patched_source"]``
    before it's removed -- the same "return the patched source
    directly instead of a file path" shape the ``nirdosha mcp`` server's
    own ``fix`` tool already uses, for an identical reason (an
    in-memory caller, not a file already on disk).
    """
    binary = _binary_path()
    fd, path = tempfile.mkstemp(suffix=".nir")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(source)
        args = [binary, "fix", path]
        if apply:
            args.append("--apply")
        completed = subprocess.run(args, capture_output=True, text=True)
        try:
            result = json.loads(completed.stdout)
        except json.JSONDecodeError as e:
            raise NirdoshaError(
                f"`nirdosha fix` did not print valid JSON on stdout (exit {completed.returncode}): {completed.stderr.strip()}"
            ) from e
        if apply:
            result["patched_source"] = Path(path).read_text(encoding="utf-8")
        return result
    finally:
        os.remove(path)


def certify(source: str) -> dict[str, Any]:
    """Runs ``nirdosha certify`` against ``source`` and returns the
    parsed Certificate v0 JSON unchanged.
    """
    return _run_on_source(["certify"], source)
