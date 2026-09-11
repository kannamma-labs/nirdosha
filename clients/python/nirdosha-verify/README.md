# nirdosha-verify

A thin Python client for [`nirdosha verify`/`fix`/`certify`](https://github.com/kannamma-labs/nirdosha)
— shells out to a locally installed `nirdosha` compiler binary and
parses its JSON output. Not a reimplementation of the verifier: every
call here runs the exact same pipeline the CLI itself does.

## Requirements

This package does **not** bundle or download the `nirdosha` binary —
you need one already reachable:

- on `PATH` (e.g. `cargo install --path crates/compiler` from the
  [nirdosha repo](https://github.com/kannamma-labs/nirdosha)), or
- pointed to via the `NIRDOSHA_BIN` environment variable.

Auto-fetching a prebuilt platform binary is planned once nirdosha
publishes tagged release binaries — not yet, so this isn't faked here.

## Install

```sh
pip install nirdosha-verify
```

## Usage

```python
import nirdosha_verify

verdict = nirdosha_verify.verify("""
fn add(a: i64, b: i64) -> i64 {
    return a + b
}
""")
print(verdict["verdict"])  # "PROVED"

fixed = nirdosha_verify.fix(source_with_a_typo, apply=True)
print(fixed["patched_source"])

certificate = nirdosha_verify.certify(source)
print(certificate["evidence_tier"])  # "proved" | "unknown"
```

A `DISPROVED`/`UNKNOWN` verdict is returned as data, not raised as an
exception — asking "does this code pass" needs "no" to come back as an
answer, not a crash. `nirdosha_verify.NirdoshaError` is reserved for
the client itself failing to do its job (the binary produced no
parseable JSON at all).

### CLI

```sh
nirdosha-verify path/to/file.nir
```

A direct passthrough to `nirdosha verify path/to/file.nir` — same
stdout, same exit code.

## License

Apache-2.0
