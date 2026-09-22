# Nirdosha Recipe Format v1 integration

The Recipe is a signed, reproducible record of a verification run. It records
the specification, tool, solver, inputs, command, environment, per-invariant
results, and signatures. It is a run record, not a universal correctness
certificate.

The shared deterministic primitives live in
`nirdosha-contract-core::recipe`:

- RFC 8785 JCS bytes;
- byte-preserving DSSE PAE for
  `application/vnd.nirdosha.recipe.v1+json`;
- self-nulled `recipe_id` computation;
- `result_hash` computation;
- normative aggregate flattening (`sat` > `unknown` > `unsat`);
- rejection of empty or unsorted invariant results.

The remaining integration work is the Recipe runner and publisher: invoke the
real browser adapter and Z3, include runtime trace hashes and solver metadata,
then sign the exact recipe with the existing audit/signing infrastructure.
Sigstore identity and Rekor policy remain consumer/deployment concerns.

## Required interpretation

`unsat` means only that no counterexample exists within the declared finite
model, domains, and solver configuration. It does not prove model fidelity,
tool correctness, solver correctness, or deployment correctness. `unknown`,
missing evidence, a changed build, a hash mismatch, or a reproduction failure
must never be promoted to a passing assurance result.

## Spec-hash correction

The specification must not hash a document after replacing its own hash
placeholder: that is self-referential. Use a fixed sentinel (for example
`<NIRDOSHA-RECIPE-SPEC-HASH>`) and define the hash as the SHA-256 of the bytes
containing that sentinel, or publish the specification at an immutable
content-addressed URL and hash those bytes.

The recipe should also distinguish artifact hash kinds (binary, source archive,
OCI image, or manifest), bind the exact JCS/PAE implementation versions, and
carry a Sigstore/Rekor bundle reference rather than treating a bare key ID as
proof of transparency-log inclusion.

## UI assurance contents

For UI runs, `inputs` must include the UI contract/model and invariants. The
result must additionally bind:

- the tested application build;
- browser and adapter versions;
- DOM/accessibility/network evidence hashes;
- independent API/database/audit evidence;
- the Z3 formula hash, solver version, and deterministic `rlimit`;
- the `RuntimeTrace` hash produced by the browser adapter.

The consumer decides whether this bounded evidence is sufficient for release;
the Recipe itself does not invent a stronger claim.
