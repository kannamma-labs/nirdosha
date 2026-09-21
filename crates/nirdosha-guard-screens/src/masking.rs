//! Applies a `guard_policy!`-granted `FieldMask` to a decoded row.
//!
//! `nirdosha-guard-mic`'s `ReadOutcome::masks` doc comment is explicit
//! that the guard/store layer can't do this itself: `EntityBytes` is
//! opaque there, so only the caller -- who deserializes into a concrete,
//! typed entity anyway -- can locate a `FieldMask.field` path inside it.
//! This module is that caller-side step, applied to the decoded
//! `serde_json::Value` *before* it's deserialized into the screen's own
//! `GuardedEntity` type (so a masked field must still parse as whatever
//! type that struct field declares -- see each `masked_value` arm for how
//! every transform stays string-shaped for that reason).

use nirdosha_guard_core::{FieldMask, MaskTransform};
use serde_json::Value;

pub fn apply_masks(value: &mut Value, masks: &[FieldMask]) {
    for mask in masks {
        apply_one(value, &mask.field, &mask.transform);
    }
}

fn apply_one(value: &mut Value, field: &[String], transform: &MaskTransform) {
    let Some((head, rest)) = field.split_first() else { return };
    let Value::Object(map) = value else { return };
    if rest.is_empty() {
        if let Some(slot) = map.get_mut(head) {
            *slot = masked_value(slot, transform);
        }
        return;
    }
    if let Some(nested) = map.get_mut(head) {
        apply_one(nested, rest, transform);
    }
}

/// Every arm returns a JSON string (never `Null`, never a bare number) --
/// a masked field must still deserialize into whatever concrete type the
/// screen's struct field declares (usually `String`), matching the same
/// "stay parseable" constraint `10_domains.nir`'s newtype wrappers
/// already carry. `Drop` is the one exception a real driver-level
/// projection would omit the key for; this in-process fallback strips to
/// a fixed placeholder string instead of `null` for the identical
/// deserialization reason.
fn masked_value(original: &Value, transform: &MaskTransform) -> Value {
    match transform {
        MaskTransform::Full => Value::String("[REDACTED]".into()),
        MaskTransform::PartialLast4 => {
            let s = original.as_str().map(str::to_string).unwrap_or_else(|| original.to_string());
            let tail: String = s.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            Value::String(format!("****{tail}"))
        }
        MaskTransform::Hash => {
            use std::hash::{Hash, Hasher};
            let s = original.as_str().map(str::to_string).unwrap_or_else(|| original.to_string());
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hasher);
            Value::String(format!("hash:{:x}", hasher.finish()))
        }
        MaskTransform::Drop => Value::String("[DROPPED]".into()),
        MaskTransform::Custom { name } => Value::String(format!("[{name}]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mask_redacts_a_top_level_field() {
        let mut row = serde_json::json!({ "card_token": "4111-1111-1111-1111", "amount": 12.5 });
        apply_masks(&mut row, &[FieldMask { field: vec!["card_token".into()], transform: MaskTransform::Full }]);
        assert_eq!(row["card_token"], "[REDACTED]");
        assert_eq!(row["amount"], 12.5);
    }

    #[test]
    fn partial_last4_keeps_only_the_tail() {
        let mut row = serde_json::json!({ "national_id": "990112345678" });
        apply_masks(&mut row, &[FieldMask { field: vec!["national_id".into()], transform: MaskTransform::PartialLast4 }]);
        assert_eq!(row["national_id"], "****5678");
    }
}
