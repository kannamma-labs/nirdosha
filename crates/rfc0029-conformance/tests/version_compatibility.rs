//! Executable compatibility/downgrade and version/extension policy tests —
//! two of the four freeze contracts named in the readiness matrix §6. The
//! policy itself, stated once here rather than only in prose:
//!
//! 1. **Compatibility/downgrade**: `schema_version` must match
//!    [`SCHEMA_VERSION`] *exactly*. There is no partial, fuzzy, older-minor,
//!    or newer-minor compatibility of any kind today — a fixture from any
//!    other version string, whether textually "older" or "newer" than the
//!    current one, is rejected outright with
//!    `Error::UnsupportedSchemaVersion`, never silently accepted or
//!    silently coerced. This is deliberately the whole policy until a real
//!    v2 is designed with an explicit migration rule; "no compatibility
//!    exists yet" is itself the tested, stable behavior.
//! 2. **Version/extension**: the *core* shape (`Fixture`, `Input`, `Node`,
//!    `Edge`, `Bundle`, `CatalogEntry`, `Expected` and their required
//!    fields) is closed — every one of those types is
//!    `#[serde(deny_unknown_fields)]`, and a genuinely new top-level or
//!    core-struct field requires a `schema_version` bump, not a silent
//!    accept. Domain/profile *extension* happens exclusively through the
//!    two fields deliberately left untyped for that purpose: a `Node`'s
//!    `attributes` map and `Input`'s `parameters` map. Both accept
//!    arbitrary new keys today, under the current version, with no schema
//!    change required — that is how `fact_provenance`/`fact_requirement`
//!    (this session) were added without touching `Node`'s Rust type at
//!    all. Extension is real and already exercised; it is bounded to
//!    exactly these two maps, not the core shape.

use rfc0029_conformance::{parse, Error, SCHEMA_VERSION};
use serde_json::json;
use std::fs;
use std::path::Path;

fn sample_bytes() -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rfcs/fixtures/0029-canonical");
    fs::read(root.join("vectors/EQ1A.json")).unwrap()
}

#[test]
fn exact_current_version_is_accepted() {
    assert!(parse(&sample_bytes()).is_ok());
}

#[test]
fn a_textually_older_version_string_is_rejected() {
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["schema_version"] = json!("rfc0029.influence-review.fixture.v0");
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::UnsupportedSchemaVersion(v)) if v == "rfc0029.influence-review.fixture.v0"
    ));
}

#[test]
fn a_textually_newer_version_string_is_equally_rejected() {
    // The policy is exact-match, not "accept anything >= current" or
    // "accept anything with a higher trailing digit" -- a hypothetical
    // future v2 must be equally rejected by today's parser, never silently
    // accepted as if forward-compatible. Symmetric with the downgrade case
    // above; this is the case that case alone does not prove.
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["schema_version"] = json!("rfc0029.influence-review.fixture.v2");
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::UnsupportedSchemaVersion(v)) if v == "rfc0029.influence-review.fixture.v2"
    ));
}

#[test]
fn an_unrelated_version_string_is_rejected_the_same_way() {
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["schema_version"] = json!("something-else-entirely");
    assert!(matches!(
        parse(&serde_json::to_vec(&value).unwrap()),
        Err(Error::UnsupportedSchemaVersion(_))
    ));
}

#[test]
fn current_schema_version_constant_has_not_silently_changed() {
    // Pins the literal string this whole file's policy is exact-matched
    // against, so a change to it is a visible, reviewed diff here too, not
    // just in lib.rs.
    assert_eq!(SCHEMA_VERSION, "rfc0029.influence-review.fixture.v1");
}

#[test]
fn a_new_node_attribute_key_is_accepted_without_any_schema_change() {
    // The extension point this session actually used for fact_provenance/
    // fact_requirement: Node.attributes is an open map under the current
    // version. A wholly invented key, never mentioned by any RFC or schema
    // doc, must still parse -- proving extension doesn't require the
    // author to have anticipated the exact key name in advance.
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["input"]["nodes"][0]["attributes"]["x_future_profile_extension_field"] =
        json!("anything");
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_ok());
}

#[test]
fn a_new_parameter_key_is_accepted_without_any_schema_change() {
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["input"]["parameters"]["x_future_profile_extension_param"] = json!({"nested": 1});
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_ok());
}

#[test]
fn a_new_top_level_field_is_still_rejected_under_the_current_version() {
    // The other half of the extension policy: `attributes`/`parameters`
    // are open, but the core envelope around them is not. A new top-level
    // field is not an "extension" under v1 -- it would need a version
    // bump, and today it is simply rejected. (A companion to the existing
    // `rejects_unknown_top_level_field` in tests/vectors.rs; kept here too
    // since this file is the one place the whole version/extension policy
    // is stated together.)
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["a_wholly_new_top_level_field"] = json!(true);
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn a_new_field_on_a_closed_core_struct_is_still_rejected() {
    // Same point, but on a nested core struct (Bundle) rather than the
    // fixture envelope itself -- proves `deny_unknown_fields` closure is
    // not only enforced at the top level.
    let mut value: serde_json::Value = serde_json::from_slice(&sample_bytes()).unwrap();
    value["input"]["bundle"]["extra_field"] = json!(1);
    assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
}
