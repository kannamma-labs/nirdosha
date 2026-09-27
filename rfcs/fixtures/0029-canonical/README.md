# RFC 0029 canonical influence/review vectors v1

This directory is the first executable encoding of the independently reviewed
Revision 1 semantics. It is a conformance artifact, not a production policy
evaluator.

## Contents

- `schema.json` is the closed top-level Draft 2020-12 fixture schema.
- `vectors/*.json` are readable, schema-valid representations.
- `jcs/*.jcs.json` are the exact RFC 8785 bytes hashed by `manifest.json`.
- `manifest.json` binds every fixture ID to both files, its byte count and its
  SHA-256 digest.
- `generate.py` expands the reviewed symbolic YAML deterministically and
  rejects duplicate YAML keys, floats, non-string JSON keys, naive timestamps
  and integers outside the exact I-JSON range.

Run:

```sh
python3 rfcs/fixtures/0029-canonical/generate.py --check
```

Schema validation, when the `jsonschema` Python package is available:

```sh
python3 - <<'PY'
import json, pathlib, jsonschema
root = pathlib.Path("rfcs/fixtures/0029-canonical")
schema = json.loads((root / "schema.json").read_text())
validator = jsonschema.Draft202012Validator(schema)
for path in sorted((root / "vectors").glob("*.json")):
    validator.validate(json.loads(path.read_text()))
PY
```

## Canonicalization profile

The JCS files use RFC 8785 object-key ordering and JSON string serialization.
Version 1 permits only integers in the exact interoperable range
`[-(2^53)+1, (2^53)-1]`; floating-point values are rejected. Timestamps are
explicit UTC RFC 3339 strings. The readable JSON is not hashed directly.

Every vector separates `input` from `expected`. The input never contains its
expected verdict. Four reviewed neutral-decomposition groups carry an
`asserted` normalized graph and its independent SHA-256 commitment. Other
cases honestly carry `normalized_graph.status = not_asserted`: their exact
verdict is executable, but a normalized-graph oracle must be emitted by the
reference normalizer rather than invented by this generator.

## Evolution

Unknown top-level, input and expected-result fields are rejected. A semantic
or encoding change requires a new `schema_version` and new directory; existing
vectors and hashes remain immutable. Attribute and profile-extension payloads
are intentionally typed by their referenced versioned contract during
admission. The production Rust types must narrow those payloads before this
candidate schema can be frozen as policy IR v1.
