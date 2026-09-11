"""End-to-end tests for the `nirdosha_verify` thin client -- run
against the real, locally built `nirdosha` binary (never mocked), the
same "spawn the real binary" discipline the Rust
`crates/compiler/tests/*.rs` integration tests already use for the CLI
itself. `NIRDOSHA_BIN` is resolved once, from the repo's own Cargo
`target/` directory, so these tests don't depend on `nirdosha` being
separately installed on `PATH` in CI or a dev machine that just built
it.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

import nirdosha_verify


def _find_repo_root() -> Path:
    here = Path(__file__).resolve()
    for candidate in here.parents:
        if (candidate / "crates" / "compiler" / "Cargo.toml").exists():
            return candidate
    raise RuntimeError("could not locate the nirdosha repo root above this test file")


def _locate_or_build_binary() -> str:
    override = os.environ.get("NIRDOSHA_BIN")
    if override:
        return override
    repo_root = _find_repo_root()
    for profile in ("debug", "release"):
        candidate = repo_root / "target" / profile / "nirdosha"
        if candidate.exists():
            return str(candidate)
    on_path = shutil.which("nirdosha")
    if on_path:
        return on_path
    pytest.skip("no built `nirdosha` binary found under target/debug or target/release, and none on PATH -- run `cargo build --bin nirdosha` in crates/compiler first")


@pytest.fixture(autouse=True, scope="session")
def _nirdosha_bin_env():
    os.environ["NIRDOSHA_BIN"] = _locate_or_build_binary()


PROVED_SOURCE = "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n"

DISPROVED_SOURCE = "fn bad(a: i64) -> i64 {\n    return a - 1\n}\n\nvalidate bad {\n    post: result >= a\n}\n"

TYPO_SOURCE = "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n"


def test_verify_reports_proved():
    result = nirdosha_verify.verify(PROVED_SOURCE)
    assert result["verdict"] == "PROVED", result


def test_verify_reports_disproved_with_a_counterexample():
    result = nirdosha_verify.verify(DISPROVED_SOURCE)
    assert result["verdict"] == "DISPROVED", result
    assert result["contracts"]["failed"] == 1, result


def test_verify_does_not_raise_for_a_failing_verdict():
    # A DISPROVED verdict is data, not an exception -- this call must
    # not raise at all.
    result = nirdosha_verify.verify(DISPROVED_SOURCE)
    assert isinstance(result, dict)


def test_fix_reports_an_auto_patch_without_writing_anything_by_default():
    result = nirdosha_verify.fix(TYPO_SOURCE)
    fix_info = result["before"]["typecheck"]["errors"][0]["fix"]
    assert fix_info["applicability"] == "auto", result
    assert "patched_source" not in result, result


def test_fix_with_apply_returns_the_patched_source():
    result = nirdosha_verify.fix(TYPO_SOURCE, apply=True)
    assert result["patched_source"] == "fn main() {\n    let amount: i64 = 5\n    print(amount)\n}\n", result
    assert result["after"]["typecheck"]["status"] == "passed", result


def test_certify_returns_certificate_v0_with_a_real_evidence_tier():
    result = nirdosha_verify.certify(PROVED_SOURCE)
    assert result["certificate_version"] == "0", result
    assert result["evidence_tier"] == "proved", result
    assert result["source_hash"].startswith("sha256:"), result


def test_certify_is_deterministic_across_two_calls():
    first = nirdosha_verify.certify(PROVED_SOURCE)
    second = nirdosha_verify.certify(PROVED_SOURCE)
    assert first == second


def test_missing_binary_raises_a_clear_error(monkeypatch):
    monkeypatch.setenv("NIRDOSHA_BIN", "")
    monkeypatch.delenv("NIRDOSHA_BIN", raising=False)
    monkeypatch.setenv("PATH", "/nonexistent-path-for-this-test-only")
    with pytest.raises(nirdosha_verify.NirdoshaBinaryNotFound):
        nirdosha_verify.verify(PROVED_SOURCE)


def test_cli_entry_point_is_a_real_passthrough_to_nirdosha_verify():
    binary = os.environ["NIRDOSHA_BIN"]
    fd_path = Path(subprocess.run(
        ["python3", "-c", "import tempfile,sys; fd,p=tempfile.mkstemp(suffix='.nir'); open(p,'w').write(sys.argv[1]); print(p)", PROVED_SOURCE],
        capture_output=True, text=True, check=True,
    ).stdout.strip())
    try:
        direct = subprocess.run([binary, "verify", str(fd_path)], capture_output=True, text=True)
        via_cli = subprocess.run(["nirdosha-verify", str(fd_path)], capture_output=True, text=True, env={**os.environ})
        assert via_cli.stdout == direct.stdout
        assert via_cli.returncode == direct.returncode
    finally:
        fd_path.unlink(missing_ok=True)
