//! `ListProvider` + `Matcher` for RFC 0025 §8.4 (screening) — Plan Phase 13.
//!
//! RFC 0025 §8.4 declares the real trait shape (`ListProvider::fetch`,
//! `Matcher::screen`) and the `matcher!` macro's declared defaults
//! (`algorithm = fuzzy_jaro_winkler; threshold = 0.92`), but neither trait
//! had an implementation anywhere in the workspace, and `matcher!` itself
//! isn't a callable macro yet (flagged separately in
//! `crates/nirdosha-rt/tests/rtm_policy_corpus.rs` as a grammar mismatch,
//! out of this phase's scope). This crate implements the driver side: a
//! real, honest `ListProvider` against a fixture list (no live OFAC/UN/EU
//! vendor feed is reachable from this workspace — the same reason
//! `nirdosha-guard-store-postgres`'s own doc describes its Postgres driver
//! as the "one honest implementation" of `StoreDriver`, with vendor slots
//! staying open for a real operator to supply), and a real Jaro-Winkler
//! `Matcher` (not a stub returning a fixed score).
//!
//! **"Signed" list integrity.** RFC 0025's `SignedList` implies vendor
//! cryptographic signing this workspace has no vendor keypair to honestly
//! produce. What *is* honestly implementable without inventing a PKI: the
//! same "pinned by digest, not a floating tag" integrity convention this
//! repo already uses for `docker-compose.dev.yml`'s Postgres image — a
//! list's content is checked against a pinned SHA-256 digest read from a
//! sidecar `<list_id>.sha256` file, and `fetch` fails closed
//! (`FetchError::IntegrityMismatch`) if the on-disk list has drifted from
//! that pin. A real vendor's cryptographic signature check is a strict
//! superset of this and can replace it without changing the trait.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use sha2::{Digest, Sha256};

pub type ListId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEntry {
	pub name: String,
	#[serde(default)]
	pub aliases: Vec<String>,
	pub program: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ListFile {
	list_id: String,
	version: String,
	entries: Vec<ListEntry>,
}

/// A fetched list plus the digest it was verified against — RFC 0025's
/// `SignedList`, honestly scoped to content-addressed integrity (see the
/// module doc's "Signed list integrity" section) rather than a vendor
/// signature this workspace has no key material to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedList {
	pub list_id: String,
	pub version: String,
	pub entries: Vec<ListEntry>,
	pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
	NotFound(String),
	Io(String),
	Malformed(String),
	/// The on-disk list's digest doesn't match its pinned `.sha256`
	/// sidecar — fails closed rather than serving a possibly-tampered or
	/// stale list.
	IntegrityMismatch { list_id: String, expected: String, actual: String },
}

pub trait ListProvider: Send + Sync {
	fn fetch(&self, list_id: &str) -> Result<SignedList, FetchError>;
}

/// `ListProvider` backed by `<root>/<list_id>.json` + a pinned
/// `<root>/<list_id>.json.sha256` sidecar (a single lowercase hex digest,
/// same format `sha256sum` prints).
pub struct FixtureListProvider {
	root: PathBuf,
}

impl FixtureListProvider {
	pub fn new(root: impl Into<PathBuf>) -> Self {
		Self { root: root.into() }
	}
}

impl ListProvider for FixtureListProvider {
	fn fetch(&self, list_id: &str) -> Result<SignedList, FetchError> {
		let list_path = self.root.join(format!("{list_id}.json"));
		let digest_path = self.root.join(format!("{list_id}.json.sha256"));
		let raw = std::fs::read(&list_path).map_err(|error| match error.kind() {
			std::io::ErrorKind::NotFound => FetchError::NotFound(list_id.to_string()),
			_ => FetchError::Io(error.to_string()),
		})?;

		let mut hasher = Sha256::new();
		hasher.update(&raw);
		let actual = hasher.finalize();
		let actual_hex = hex_encode(&actual);

		let pinned = std::fs::read_to_string(&digest_path)
			.map_err(|error| FetchError::Io(format!("reading pinned digest {digest_path:?}: {error}")))?;
		let expected_hex = pinned.trim().to_lowercase();
		if expected_hex != actual_hex {
			return Err(FetchError::IntegrityMismatch { list_id: list_id.to_string(), expected: expected_hex, actual: actual_hex });
		}

		let parsed: ListFile = serde_json::from_slice(&raw).map_err(|error| FetchError::Malformed(error.to_string()))?;
		if parsed.list_id != list_id {
			return Err(FetchError::Malformed(format!("list file's own list_id {:?} does not match requested {list_id:?}", parsed.list_id)));
		}
		let mut sha256 = [0u8; 32];
		sha256.copy_from_slice(&actual);
		Ok(SignedList { list_id: parsed.list_id, version: parsed.version, entries: parsed.entries, sha256 })
	}
}

fn hex_encode(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchAlgorithm {
	FuzzyJaroWinkler,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatcherCfg {
	pub algorithm: MatchAlgorithm,
	pub threshold: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
	pub list_id: String,
	pub entry_name: String,
	pub matched_against: String,
	pub score: f64,
}

pub trait Matcher: Send + Sync {
	fn screen(&self, name: &str, lists: &[SignedList], cfg: &MatcherCfg) -> Vec<Hit>;
}

/// Real Jaro-Winkler string similarity — RFC 0025's declared default
/// (`algorithm = fuzzy_jaro_winkler`). Screens `name` against every
/// entry's own name and every alias, keeping the entry's single best score
/// per list. Case-insensitive (sanctions-list matching is not
/// case-sensitive in practice — `"Test Entity Alpha"` must hit the same as
/// `"TEST ENTITY ALPHA"`).
pub struct FuzzyMatcher;

impl Matcher for FuzzyMatcher {
	fn screen(&self, name: &str, lists: &[SignedList], cfg: &MatcherCfg) -> Vec<Hit> {
		let MatchAlgorithm::FuzzyJaroWinkler = cfg.algorithm;
		let query = name.to_lowercase();
		let mut hits = Vec::new();
		for list in lists {
			for entry in &list.entries {
				let mut best: Option<(String, f64)> = None;
				for candidate in std::iter::once(&entry.name).chain(entry.aliases.iter()) {
					let score = jaro_winkler(&query, &candidate.to_lowercase());
					if best.as_ref().is_none_or(|(_, best_score)| score > *best_score) {
						best = Some((candidate.clone(), score));
					}
				}
				if let Some((matched_against, score)) = best {
					if score >= cfg.threshold {
						hits.push(Hit { list_id: list.list_id.clone(), entry_name: entry.name.clone(), matched_against, score });
					}
				}
			}
		}
		hits
	}
}

/// Standard Jaro-Winkler distance (Winkler's prefix-boosted variant of the
/// Jaro similarity), reference-checked against the well-known
/// `"martha"`/`"marhta"` pair (Jaro-Winkler ≈ 0.961). Implemented directly
/// rather than pulling in a string-distance crate — the algorithm is
/// small, fixed, and worth being able to read/audit inline for a control
/// this close to a compliance decision.
fn jaro_winkler(a: &str, b: &str) -> f64 {
	let jaro = jaro_similarity(a, b);
	if jaro <= 0.0 {
		return jaro;
	}
	let a_chars: Vec<char> = a.chars().collect();
	let b_chars: Vec<char> = b.chars().collect();
	let prefix_len = a_chars.iter().zip(b_chars.iter()).take(4).take_while(|(x, y)| x == y).count();
	const SCALING_FACTOR: f64 = 0.1;
	jaro + (prefix_len as f64) * SCALING_FACTOR * (1.0 - jaro)
}

fn jaro_similarity(a: &str, b: &str) -> f64 {
	let a_chars: Vec<char> = a.chars().collect();
	let b_chars: Vec<char> = b.chars().collect();
	let (a_len, b_len) = (a_chars.len(), b_chars.len());
	if a_len == 0 && b_len == 0 {
		return 1.0;
	}
	if a_len == 0 || b_len == 0 {
		return 0.0;
	}
	let match_distance = (a_len.max(b_len) / 2).saturating_sub(1);
	let mut a_matched = vec![false; a_len];
	let mut b_matched = vec![false; b_len];
	let mut matches = 0usize;

	for i in 0..a_len {
		let lo = i.saturating_sub(match_distance);
		let hi = (i + match_distance + 1).min(b_len);
		for j in lo..hi {
			if b_matched[j] || a_chars[i] != b_chars[j] {
				continue;
			}
			a_matched[i] = true;
			b_matched[j] = true;
			matches += 1;
			break;
		}
	}
	if matches == 0 {
		return 0.0;
	}

	let mut transpositions = 0usize;
	let mut b_index = 0usize;
	for i in 0..a_len {
		if !a_matched[i] {
			continue;
		}
		while !b_matched[b_index] {
			b_index += 1;
		}
		if a_chars[i] != b_chars[b_index] {
			transpositions += 1;
		}
		b_index += 1;
	}
	let transpositions = transpositions / 2;

	let m = matches as f64;
	(m / a_len as f64 + m / b_len as f64 + (m - transpositions as f64) / m) / 3.0
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	fn fixture_root() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
	}

	#[test]
	fn jaro_winkler_matches_the_well_known_martha_marhta_reference_value() {
		let score = jaro_winkler("martha", "marhta");
		assert!((score - 0.961).abs() < 0.001, "expected ~0.961, got {score}");
	}

	#[test]
	fn jaro_winkler_is_one_for_identical_strings() {
		assert_eq!(jaro_winkler("test entity alpha", "test entity alpha"), 1.0);
	}

	#[test]
	fn jaro_winkler_is_zero_for_completely_disjoint_strings() {
		assert_eq!(jaro_winkler("abc", "xyz"), 0.0);
	}

	#[test]
	fn fetch_loads_a_real_fixture_list_and_verifies_its_pinned_digest() {
		let provider = FixtureListProvider::new(fixture_root());
		let list = provider.fetch("test_sanctions_list").expect("fixture list must load");
		assert_eq!(list.list_id, "test_sanctions_list");
		assert_eq!(list.entries.len(), 3);
	}

	#[test]
	fn fetch_fails_closed_when_the_list_content_does_not_match_its_pinned_digest() {
		let temp_dir = std::env::temp_dir().join(format!("nirdosha-screening-tamper-{}", std::process::id()));
		std::fs::create_dir_all(&temp_dir).unwrap();
		std::fs::write(temp_dir.join("tampered.json"), br#"{"list_id":"tampered","version":"v1","entries":[]}"#).unwrap();
		// Pin a digest that does NOT match the content above.
		std::fs::write(temp_dir.join("tampered.json.sha256"), "0000000000000000000000000000000000000000000000000000000000000000\n").unwrap();
		let provider = FixtureListProvider::new(&temp_dir);
		let result = provider.fetch("tampered");
		assert!(matches!(result, Err(FetchError::IntegrityMismatch { .. })), "expected IntegrityMismatch, got {result:?}");
		let _ = std::fs::remove_dir_all(&temp_dir);
	}

	#[test]
	fn fetch_reports_not_found_for_a_missing_list() {
		let provider = FixtureListProvider::new(fixture_root());
		let result = provider.fetch("does_not_exist");
		assert!(matches!(result, Err(FetchError::NotFound(_))), "expected NotFound, got {result:?}");
	}

	#[test]
	fn screen_finds_a_hit_above_threshold_and_reports_the_matched_alias() {
		let provider = FixtureListProvider::new(fixture_root());
		let list = provider.fetch("test_sanctions_list").unwrap();
		let matcher = FuzzyMatcher;
		let cfg = MatcherCfg { algorithm: MatchAlgorithm::FuzzyJaroWinkler, threshold: 0.92 };
		let hits = matcher.screen("ALPHA TEST ENTITY", &[list], &cfg);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].entry_name, "TEST ENTITY ALPHA");
		assert_eq!(hits[0].matched_against, "ALPHA TEST ENTITY");
		assert!(hits[0].score >= 0.92);
	}

	#[test]
	fn screen_excludes_names_below_threshold() {
		let provider = FixtureListProvider::new(fixture_root());
		let list = provider.fetch("test_sanctions_list").unwrap();
		let matcher = FuzzyMatcher;
		let cfg = MatcherCfg { algorithm: MatchAlgorithm::FuzzyJaroWinkler, threshold: 0.92 };
		let hits = matcher.screen("A COMPLETELY UNRELATED NAME", &[list], &cfg);
		assert!(hits.is_empty(), "expected no hits for an unrelated name, got {hits:?}");
	}

	#[test]
	fn screen_is_case_insensitive() {
		let provider = FixtureListProvider::new(fixture_root());
		let list = provider.fetch("test_sanctions_list").unwrap();
		let matcher = FuzzyMatcher;
		let cfg = MatcherCfg { algorithm: MatchAlgorithm::FuzzyJaroWinkler, threshold: 0.92 };
		let hits = matcher.screen("test entity alpha", &[list], &cfg);
		assert_eq!(hits.len(), 1);
	}
}
