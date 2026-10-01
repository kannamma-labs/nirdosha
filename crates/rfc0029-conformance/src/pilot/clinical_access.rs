//! Healthcare access-and-medication pilot: readiness matrix §7.2's ten
//! rows, none of which map onto `funds_reserve`, `assertion_lifecycle`, or
//! any graph fixture — this domain needed its own stateful harness. Same
//! falsification-pilot boundary as the others: no store, network, or
//! effect-gateway dependency, no cross-process persistence claim.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Normal,
    BreakGlass,
}

/// §7.2 row 4: two authorities may assert conflicting allergy facts for the
/// same allergen. Each entry is one authority's assertion, not a merged
/// view — merging is exactly what must not happen silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllergyFact {
    pub allergen: String,
    pub authority: String,
    pub present: bool,
}

#[derive(Debug, Clone)]
pub struct PatientRecord {
    pub canonical_patient_id: String,
    pub allergies: Vec<AllergyFact>,
    /// §7.2 row 10: `Some(authority)` names a retention hold; its presence,
    /// not precedence-by-arrival-order, is what decides the conflict.
    pub retention_hold_authority: Option<String>,
    pub deletion_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionDecision {
    Deleted,
    Retained { authority: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessStatus {
    Granted,
    Denied,
    /// §7.2 row 7: evidence outage during emergency access. Mirrors
    /// `funds_reserve::ReservationStatus::AwaitingReconciliation` --
    /// access was clinically necessary and was allowed to proceed, but the
    /// durable evidence commitment for it is not yet settled.
    GrantedPendingEvidenceReconciliation,
    /// §7.2 row 4: neither silently merged nor silently denied -- named so
    /// a caller cannot mistake it for an ordinary grant/deny.
    AuthorityDisagreement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessResult {
    pub status: AccessStatus,
    pub capability_scope: Vec<String>,
    pub diagnostic: Option<String>,
}

fn deny(diagnostic: &str) -> AccessResult {
    AccessResult {
        status: AccessStatus::Denied,
        capability_scope: vec![],
        diagnostic: Some(diagnostic.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedicationOrder {
    pub order_id: String,
    pub patient_id: String,
    pub medication_id: String,
    pub version: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedResearchRecord {
    pub source_patient_id: String,
    origin_purpose: &'static str,
}

impl DerivedResearchRecord {
    /// §7.2 row 9: purpose inheritance. A record derived under one consent
    /// scope can only be used for the purpose it inherited -- not a purpose
    /// its own new caller would prefer.
    pub fn usable_for(&self, purpose: &str) -> bool {
        purpose == self.origin_purpose
    }
}

#[derive(Debug, Default)]
pub struct ClinicalAccessRegistry {
    patients: BTreeMap<String, PatientRecord>,
    /// §7.2 row 6: alias/merge/replica resolution to one canonical id --
    /// explicit, not inferred from field similarity.
    aliases: BTreeMap<String, String>,
    /// §7.2 row 2: an emergency access episode's declared binding
    /// (clinician -> the one patient+purpose it was opened for). A later
    /// request against the same clinician's break-glass session is checked
    /// against this, not re-authorized fresh each time.
    break_glass_bindings: BTreeMap<String, (String, String)>,
    pub consent_pdp_available: bool,
    pub evidence_plane_available: bool,
    medication_orders: BTreeMap<String, MedicationOrder>,
    administered: BTreeMap<String, String>, // order_id -> evidence commitment
}

impl ClinicalAccessRegistry {
    pub fn new() -> Self {
        Self {
            consent_pdp_available: true,
            evidence_plane_available: true,
            ..Self::default()
        }
    }

    pub fn put_patient(&mut self, record: PatientRecord) {
        self.patients.insert(record.canonical_patient_id.clone(), record);
    }

    /// §7.2 row 6.
    pub fn put_alias(&mut self, alias: &str, canonical_patient_id: &str) {
        self.aliases.insert(alias.into(), canonical_patient_id.into());
    }

    fn resolve(&self, patient_id: &str) -> Option<String> {
        if self.patients.contains_key(patient_id) {
            return Some(patient_id.to_string());
        }
        self.aliases.get(patient_id).cloned()
    }

    /// §7.2 row 2: opens one emergency episode, binding it to exactly one
    /// (patient, purpose). A later break-glass `access` call is checked
    /// against this binding, not re-derived from that call's own claims.
    pub fn declare_emergency(&mut self, clinician_id: &str, patient_id: &str, purpose: &str) {
        self.break_glass_bindings
            .insert(clinician_id.into(), (patient_id.into(), purpose.into()));
    }

    /// §7.2 rows 1, 2, 3, 4, 7.
    pub fn access(
        &self,
        clinician_id: &str,
        patient_id: &str,
        purpose: &str,
        mode: AccessMode,
    ) -> AccessResult {
        // Row 3: consent/PDP outage is an admitted safety posture's job to
        // override, never an implicit exception -- absent one, fail closed
        // regardless of mode, including BreakGlass.
        if !self.consent_pdp_available {
            return deny("ConsentPdpUnavailable");
        }
        let Some(canonical) = self.resolve(patient_id) else {
            return deny("UnknownPatient");
        };

        match mode {
            AccessMode::Normal => {
                if purpose != "treatment" {
                    return deny("PurposeNotAdmittedForNormalAccess");
                }
            }
            AccessMode::BreakGlass => {
                // Row 2: the binding was fixed at declare_emergency time;
                // a replay against a different patient or purpose is a
                // binding mismatch, not a fresh authorization decision.
                match self.break_glass_bindings.get(clinician_id) {
                    Some((bound_patient, bound_purpose)) => {
                        let bound_canonical =
                            self.resolve(bound_patient).unwrap_or_else(|| bound_patient.clone());
                        if bound_canonical != canonical || bound_purpose != purpose {
                            return deny("BreakGlassBindingMismatch");
                        }
                    }
                    None => return deny("NoDeclaredEmergencyBinding"),
                }
            }
        }

        // Row 4: two authorities disagreeing about the same allergen is a
        // declared disagreement, never silently resolved by last-write-wins.
        let patient = &self.patients[&canonical];
        let mut by_allergen: BTreeMap<&str, Vec<&AllergyFact>> = BTreeMap::new();
        for fact in &patient.allergies {
            by_allergen.entry(fact.allergen.as_str()).or_default().push(fact);
        }
        for facts in by_allergen.values() {
            let present_values: std::collections::BTreeSet<bool> =
                facts.iter().map(|f| f.present).collect();
            if present_values.len() > 1 {
                return AccessResult {
                    status: AccessStatus::AuthorityDisagreement,
                    capability_scope: vec![],
                    diagnostic: Some(format!(
                        "conflicting allergy authorities for {}",
                        facts[0].allergen
                    )),
                };
            }
        }

        let capability_scope = match mode {
            AccessMode::Normal => vec!["diagnosis".to_string(), "medications".to_string()],
            AccessMode::BreakGlass => vec![
                "diagnosis".to_string(),
                "medications".to_string(),
                "allergies".to_string(),
                "emergency_contacts".to_string(),
            ],
        };

        // Row 7: an emergency grant may proceed on clinical urgency even
        // if the evidence plane cannot durably record it yet -- but that
        // must be a declared, distinct outcome, never a plain Granted.
        if mode == AccessMode::BreakGlass && !self.evidence_plane_available {
            return AccessResult {
                status: AccessStatus::GrantedPendingEvidenceReconciliation,
                capability_scope,
                diagnostic: None,
            };
        }

        AccessResult {
            status: AccessStatus::Granted,
            capability_scope,
            diagnostic: None,
        }
    }

    /// §7.2 row 5.
    pub fn put_medication_order(&mut self, order: MedicationOrder) {
        self.medication_orders.insert(order.order_id.clone(), order);
    }

    /// §7.2 rows 5, 8: administration is irreversible. A stale/cancelled
    /// order (row 5, the same time-of-check/time-of-use property already
    /// proven in `funds_reserve`) is rejected; a second attempt against an
    /// already-administered order is rejected too -- there is no
    /// "compensating" mutation for medication already given, only a new,
    /// independent, separately-ordered clinical act.
    pub fn administer(&mut self, order_id: &str, expected_version: u64) -> Result<String, &'static str> {
        if self.administered.contains_key(order_id) {
            return Err("AlreadyAdministered");
        }
        let order = self.medication_orders.get(order_id).ok_or("UnknownOrder")?;
        if order.cancelled {
            return Err("OrderCancelled");
        }
        if order.version != expected_version {
            return Err("OrderVersionStale");
        }
        let commitment = format!(
            "sha256:{:x}",
            Sha256::digest(format!("{}:{}:{}", order.order_id, order.patient_id, order.medication_id).as_bytes())
        );
        self.administered.insert(order_id.to_string(), commitment.clone());
        Ok(commitment)
    }

    /// §7.2 row 9.
    pub fn derive_for_research(&self, patient_id: &str) -> Result<DerivedResearchRecord, &'static str> {
        let canonical = self.resolve(patient_id).ok_or("UnknownPatient")?;
        Ok(DerivedResearchRecord {
            source_patient_id: canonical.to_string(),
            origin_purpose: "research",
        })
    }

    /// §7.2 row 10: an explicit, named rule -- not arbitrary precedence by
    /// request order. A retention hold overrides a deletion request only
    /// when a hold authority is actually named.
    pub fn resolve_retention_conflict(&self, patient_id: &str) -> Result<RetentionDecision, &'static str> {
        let canonical = self.resolve(patient_id).ok_or("UnknownPatient")?;
        let patient = &self.patients[&canonical];
        if !patient.deletion_requested {
            return Ok(RetentionDecision::Deleted); // nothing conflicts
        }
        match &patient.retention_hold_authority {
            Some(authority) => Ok(RetentionDecision::Retained {
                authority: authority.clone(),
            }),
            None => Ok(RetentionDecision::Deleted),
        }
    }
}
