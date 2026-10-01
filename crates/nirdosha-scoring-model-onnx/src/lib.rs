//! `ModelRuntime`/`Model` for RFC 0025 §8.3 (scoring) — Plan Phase 13.
//!
//! §8.3 declares the real trait shape (`ModelRuntime::load`,
//! `Model::score`) and names `OnnxRuntime` as a driver example wrapping
//! the `ort` crate, but no implementation existed anywhere in the
//! workspace — the RFC's own code block is illustrative Rust, not a real
//! type. This crate is that driver: a real `ort::Session` wrapped behind
//! the trait, scored against a small, checked-in reference ONNX model
//! (`tests/fixtures/rt_fraud_v1_test_fixture.onnx`) — a fixture proving
//! the driver contract (load, score, report an explanation), not a
//! production fraud model.
//!
//! Traits here are synchronous, unlike RFC 0025 §8.3's `async_trait`
//! illustration — this workspace's `StoreDriver`/`ListProvider` (Plan
//! Phases 7-13) are all synchronous too; an async MIC uplift is Plan
//! Phase 18's job, not this one's. `ort::Session::run` is itself
//! synchronous, so this isn't a capability loss, just consistency with
//! every other driver this phase and the ones before it added.

use ort::session::Session;
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
	pub name: String,
	pub version: String,
	pub inputs: Vec<String>,
	pub outputs: Vec<String>,
}

/// RFC 0025 §8.3's `SignedArtifact`. As with `nirdosha-screening-list-fixture`'s
/// `SignedList` (`docs/adr/0014`), "signed" here is honestly scoped to a
/// pinned content digest — this workspace has no model-signing key
/// material to check a real vendor signature against.
#[derive(Debug, Clone)]
pub struct SignedArtifact {
	pub spec: ModelSpec,
	pub bytes: Vec<u8>,
	pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelOutput {
	pub score: f64,
	pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCaps {
	pub format: &'static str,
	pub batching: bool,
	pub gpu: bool,
}

#[derive(Debug)]
pub enum LoadError {
	IntegrityMismatch { expected: String, actual: String },
	Onnx(String),
}

#[derive(Debug)]
pub enum ScoreError {
	ShapeMismatch { expected: usize, actual: usize },
	Onnx(String),
}

pub trait ModelRuntime: Send + Sync {
	fn capabilities(&self) -> ModelCaps;
	fn load(&self, artifact: &SignedArtifact) -> Result<Box<dyn Model>, LoadError>;
}

pub trait Model: Send + Sync {
	fn score(&self, features: &[f64]) -> Result<ModelOutput, ScoreError>;
}

fn hex_encode(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies `artifact.bytes` against `artifact.sha256` and computes a
/// fresh digest for a rejecting `LoadError`'s message — shared by
/// `OnnxRuntime::load` so the integrity check reads the same way
/// `nirdosha-screening-list-fixture::FixtureListProvider::fetch` does.
fn verify_digest(bytes: &[u8], expected: &[u8; 32]) -> Result<(), LoadError> {
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	let actual = hasher.finalize();
	if actual.as_slice() != expected.as_slice() {
		return Err(LoadError::IntegrityMismatch { expected: hex_encode(expected), actual: hex_encode(&actual) });
	}
	Ok(())
}

pub struct OnnxRuntime;

impl ModelRuntime for OnnxRuntime {
	fn capabilities(&self) -> ModelCaps {
		ModelCaps { format: "onnx", batching: false, gpu: false }
	}

	fn load(&self, artifact: &SignedArtifact) -> Result<Box<dyn Model>, LoadError> {
		verify_digest(&artifact.bytes, &artifact.sha256)?;
		let session = Session::builder()
			.map_err(|error| LoadError::Onnx(error.to_string()))?
			.commit_from_memory(&artifact.bytes)
			.map_err(|error| LoadError::Onnx(error.to_string()))?;
		Ok(Box::new(OnnxModel { session: Mutex::new(session), spec: artifact.spec.clone() }))
	}
}

pub struct OnnxModel {
	session: Mutex<Session>,
	spec: ModelSpec,
}

impl Model for OnnxModel {
	fn score(&self, features: &[f64]) -> Result<ModelOutput, ScoreError> {
		if features.len() != self.spec.inputs.len() {
			return Err(ScoreError::ShapeMismatch { expected: self.spec.inputs.len(), actual: features.len() });
		}
		// The graph exposes one input tensor, "features", packing every
		// declared spec.inputs feature positionally in the artifact's own
		// declared order — matching the fixture generator's own graph
		// shape (a single [1, N] MatMul input), not a per-feature tensor.
		let input: Vec<f32> = features.iter().map(|v| *v as f32).collect();
		let tensor = Tensor::from_array(([1usize, input.len()], input.into_boxed_slice())).map_err(|error| ScoreError::Onnx(error.to_string()))?;

		let mut session = self.session.lock().map_err(|_| ScoreError::Onnx("session lock poisoned".into()))?;
		let outputs = session.run(ort::inputs!["features" => tensor]).map_err(|error| ScoreError::Onnx(error.to_string()))?;

		let output_name = self.spec.outputs.first().map(String::as_str).unwrap_or("score");
		let (_, score_data) = outputs[output_name]
			.try_extract_tensor::<f32>()
			.map_err(|error| ScoreError::Onnx(error.to_string()))?;
		let score = *score_data.first().ok_or_else(|| ScoreError::Onnx("model produced an empty output tensor".into()))? as f64;

		Ok(ModelOutput { score, explanation: vec![format!("{}: sigmoid(features . weights + bias) = {score:.6}", self.spec.name)] })
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	fn decode_hex_digest(hex: &str) -> [u8; 32] {
		let mut out = [0u8; 32];
		for (i, byte) in out.iter_mut().enumerate() {
			*byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("valid hex digest");
		}
		out
	}

	fn load_fixture() -> SignedArtifact {
		let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
		let bytes = std::fs::read(fixture_dir.join("rt_fraud_v1_test_fixture.onnx")).expect("fixture model must exist");
		let pinned = std::fs::read_to_string(fixture_dir.join("rt_fraud_v1_test_fixture.onnx.sha256")).expect("pinned digest must exist");
		let sha256 = decode_hex_digest(pinned.trim());
		SignedArtifact {
			spec: ModelSpec {
				name: "rt_fraud_v1_test_fixture".into(),
				version: "test-1".into(),
				inputs: vec!["velocity_1h".into(), "distinct_payees_7d".into(), "amount_dev_30d".into(), "impossible_travel".into()],
				outputs: vec!["score".into()],
			},
			bytes,
			sha256,
		}
	}

	#[test]
	fn load_rejects_an_artifact_whose_digest_does_not_match() {
		let mut artifact = load_fixture();
		artifact.sha256 = [0u8; 32];
		let result = OnnxRuntime.load(&artifact);
		match result {
			Err(LoadError::IntegrityMismatch { .. }) => {}
			Err(other) => panic!("expected IntegrityMismatch, got a different LoadError: {other:?}"),
			Ok(_) => panic!("expected IntegrityMismatch, but load succeeded"),
		}
	}

	#[test]
	fn load_and_score_a_real_onnx_model_matches_the_independently_computed_reference_value() {
		let artifact = load_fixture();
		let model = OnnxRuntime.load(&artifact).expect("real fixture model must load");
		// Independently computed in Python (numpy, not onnxruntime) from
		// the same weights/bias the fixture generator embedded:
		// sigmoid([1,1,1,1] . [0.8,0.5,0.6,1.2] + (-1.0)) = 0.8909032
		let output = model.score(&[1.0, 1.0, 1.0, 1.0]).expect("scoring must succeed");
		assert!((output.score - 0.8909032).abs() < 0.0005, "expected ~0.8909032, got {}", output.score);
		assert!(!output.explanation.is_empty());
	}

	#[test]
	fn score_rejects_a_feature_vector_of_the_wrong_length() {
		let artifact = load_fixture();
		let model = OnnxRuntime.load(&artifact).expect("real fixture model must load");
		let result = model.score(&[1.0, 2.0]);
		assert!(matches!(result, Err(ScoreError::ShapeMismatch { expected: 4, actual: 2 })), "expected ShapeMismatch, got {result:?}");
	}
}
