//! Governed policy bundle: `policy-bundle.toml` parsing, validation, and the
//! validity-window check every gateway's capability issuance is bound to.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize)]
struct RawBundleFile {
    bundle: RawBundle,
}

#[derive(Debug, Clone, Deserialize)]
struct RawBundle {
    authority_id: String,
    policy_owner: String,
    jurisdiction: String,
    approved_by: String,
    signed_by: String,
    effective_from: String,
    expires_at: String,
}

/// A domain authority's governed, versioned policy bundle -- the envelope a
/// service-catalog entry's admission is exercised under. `bundle_hash` is
/// always *computed* from the exact source bytes, never authored, so it
/// can't drift from the content it attests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub authority_id: String,
    pub policy_owner: String,
    pub jurisdiction: String,
    pub approved_by: String,
    pub signed_by: String,
    pub effective_from: String,
    pub expires_at: String,
    pub bundle_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    Parse(String),
    EmptyField(&'static str),
    InvertedWindow,
    UnparseableTimestamp(&'static str),
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::Parse(e) => write!(f, "policy bundle is invalid: {e}"),
            BundleError::EmptyField(name) => write!(f, "policy bundle field `{name}` must not be empty"),
            BundleError::InvertedWindow => {
                write!(f, "policy bundle effective_from must not be after expires_at")
            }
            BundleError::UnparseableTimestamp(name) => {
                write!(f, "policy bundle field `{name}` is not a valid `YYYY-MM-DDTHH:MM:SSZ` timestamp")
            }
        }
    }
}

impl PolicyBundle {
    /// Parse and validate `policy-bundle.toml`'s source text. Fails closed
    /// on any empty governance field, an inverted validity window, or a
    /// timestamp this crate's minimal RFC 3339 parser can't read.
    pub fn from_toml_str(src: &str) -> Result<Self, BundleError> {
        let raw: RawBundleFile = toml::from_str(src).map_err(|e| BundleError::Parse(e.to_string()))?;
        let b = raw.bundle;
        for (name, value) in [
            ("authority_id", &b.authority_id),
            ("policy_owner", &b.policy_owner),
            ("jurisdiction", &b.jurisdiction),
            ("approved_by", &b.approved_by),
            ("signed_by", &b.signed_by),
        ] {
            if value.trim().is_empty() {
                return Err(BundleError::EmptyField(name));
            }
        }
        let effective_from_secs = parse_rfc3339_utc_seconds(&b.effective_from)
            .ok_or(BundleError::UnparseableTimestamp("effective_from"))?;
        let expires_at_secs = parse_rfc3339_utc_seconds(&b.expires_at)
            .ok_or(BundleError::UnparseableTimestamp("expires_at"))?;
        if effective_from_secs > expires_at_secs {
            return Err(BundleError::InvertedWindow);
        }
        let bundle_hash = format!("sha256:{:x}", Sha256::digest(src.as_bytes()));
        Ok(PolicyBundle {
            authority_id: b.authority_id,
            policy_owner: b.policy_owner,
            jurisdiction: b.jurisdiction,
            approved_by: b.approved_by,
            signed_by: b.signed_by,
            effective_from: b.effective_from,
            expires_at: b.expires_at,
            bundle_hash,
        })
    }

    /// Whether this bundle's governance window covers `now_ms` (epoch
    /// milliseconds). An unparseable window (should not happen after
    /// `from_toml_str` validated it, but this is also reachable via
    /// `#[derive(Deserialize)]` on `PolicyBundle` itself from a trusted
    /// source) is treated as inactive, not active-by-default.
    pub fn is_active(&self, now_ms: u64) -> bool {
        let (Some(from), Some(until)) = (
            parse_rfc3339_utc_seconds(&self.effective_from),
            parse_rfc3339_utc_seconds(&self.expires_at),
        ) else {
            return false;
        };
        let now_secs = now_ms / 1000;
        now_secs >= from && now_secs < until
    }
}

/// Minimal `YYYY-MM-DDTHH:MM:SSZ` (UTC, whole seconds, no offset) parser --
/// `policy-bundle.toml`'s own fixed format. Deliberately not a datetime
/// dependency: capability TTL arithmetic needs a real epoch (unlike
/// `rfc0029-conformance`'s plain string comparisons of the same shape),
/// but nothing here needs calendars beyond this one shape.
pub(crate) fn parse_rfc3339_utc_seconds(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() {
        return None;
    }
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    if t.next().is_some() {
        return None;
    }

    // Howard Hinnant's `days_from_civil`.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01

    let secs = days * 86400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "banking-domain-authority"
policy_owner = "payments-platform-team"
jurisdiction = "IN"
approved_by = "banking-domain-authority"
signed_by = "banking-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    #[test]
    fn parses_known_epoch_anchors() {
        assert_eq!(parse_rfc3339_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
        let next_year = parse_rfc3339_utc_seconds("2027-01-01T00:00:00Z").unwrap();
        let this_year = parse_rfc3339_utc_seconds("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(next_year - this_year, 365 * 86400, "2026 is not a leap year");
    }

    #[test]
    fn rejects_empty_governance_field() {
        let src = VALID_BUNDLE.replace("approved_by = \"banking-domain-authority\"", "approved_by = \"\"");
        assert_eq!(PolicyBundle::from_toml_str(&src).unwrap_err(), BundleError::EmptyField("approved_by"));
    }

    #[test]
    fn rejects_inverted_window() {
        let src = VALID_BUNDLE.replace("expires_at = \"2027-01-01T00:00:00Z\"", "expires_at = \"2025-01-01T00:00:00Z\"");
        assert_eq!(PolicyBundle::from_toml_str(&src).unwrap_err(), BundleError::InvertedWindow);
    }
}
