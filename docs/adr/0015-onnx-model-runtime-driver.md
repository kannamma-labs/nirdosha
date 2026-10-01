# 0015: `ModelRuntime`/`Model` for scoring — real `ort` (ONNX Runtime) driver, digest-pinned artifact, synchronous trait

Date: 2026-09-21
Status: accepted

## Context

`rfcs/0025-nirdosha-rtm-ecosystem.md` §8.3 declares real trait shapes for
scoring (`ModelRuntime::load(&SignedArtifact) -> Box<dyn Model>`,
`Model::score(&FeatureRow) -> ModelOutput`) and names `OnnxRuntime`
wrapping the `ort` crate as a driver example, but no implementation existed
anywhere in the workspace — the RFC's own code block (`async_trait`,
`Box<dyn Model>`, `FeatureRow`) is illustrative, not a real, compiling type.

## Decision

**New crate `crates/nirdosha-scoring-model-onnx`**, same naming convention
as `docs/adr/0013`/`docs/adr/0014`.

**Synchronous traits, not `async_trait`.** Every other driver this phase
and the phases before it added (`StoreDriver`, `ListProvider`) is
synchronous — `ort::Session::run` itself is a synchronous call, so an
`async fn` wrapper here would add nothing but a mismatch with the rest of
the codebase's driver traits. Plan Phase 18 is the workspace's declared
place for an async MIC uplift; this driver doesn't pre-empt that decision
by picking async on its own.

**Real `ort` 2.0.0-rc.13** (pinned exact — no stable 2.x `ort` release
exists yet on crates.io as of this writing), `download-binaries` +
`tls-rustls` features (the crate's build script refuses to compile without
an explicit TLS feature selection for its binary downloader). The
`ndarray` interop feature was tried first and dropped: `ort`'s own
`ndarray` feature pulled in `ndarray 0.17`, whose `NdFloat` trait sits
behind the `std` feature that `ort`'s dependency spec does not enable,
making the combination not compile. `Tensor::from_array` with a plain
`(shape, boxed_slice)` tuple needs no `ndarray` interop at all for this
driver's one-tensor-in/one-tensor-out shape, so the feature (and the
`ndarray` dependency itself) was removed rather than chased further.

**Fixture model IR-version pin.** The onnx Python package used to generate
the checked-in reference model (`tests/fixtures/rt_fraud_v1_test_fixture.onnx`,
a tiny `MatMul` → `Add` → `Sigmoid` graph, weights/bias hardcoded, not a
production fraud model) defaults to a newer ONNX IR version than the
ONNX Runtime binary `ort-sys` downloads supports (`ir_version: 14` vs. "max
supported IR version: 13" — a real, confirmed mismatch this phase hit and
fixed, not a theoretical concern). Regenerated with `model.ir_version = 9`
explicitly set (still opset 17, well within IR 9's supported range) rather
than pinning to an older/less-current `onnx` package.

**"Signed" artifact integrity, same honest scoping as `docs/adr/0014`.**
`SignedArtifact.sha256` is checked before `Session::builder().commit_from_memory`
ever runs (`LoadError::IntegrityMismatch` on mismatch, fail-closed) — a
pinned content digest, not a vendor cryptographic signature this workspace
has no key material to verify.

## Consequences

**What this makes possible.** `cargo test -p nirdosha-scoring-model-onnx`
proves, with no network access needed after the one-time `ort-sys` binary
download: a real ONNX graph loads through `ort::Session`, a tampered
artifact is rejected before it ever reaches the ONNX Runtime, a
shape-mismatched feature vector is rejected before it ever reaches
`Session::run`, and a real inference result matches an independently
computed (plain NumPy, not ONNX Runtime) reference value to within
floating-point tolerance — proof this is genuine inference, not a stub
returning a fixed number.

**What this does *not* make possible, stated so it's never misread later.**
`model_artifact!` is still not a callable macro — a `guard_policy!`-declared
model reference has nothing to bind this driver to yet, the same kind of
grammar-mismatch gap `docs/adr/0014` documents for `matcher!`. No
production fraud model ships here; the fixture proves the driver contract
(load, score, report an explanation), same posture RFC 0025 §8.3 itself
describes for `RulesInterpreter`/`TritonClient` satisfying the identical
`ModelRuntime`/`Model` contract as a distinct, swappable driver.
