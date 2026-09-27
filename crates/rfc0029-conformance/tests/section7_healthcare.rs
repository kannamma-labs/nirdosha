//! Executable coverage for readiness matrix §7.2 "Mandatory non-AI rerun
//! matrices: healthcare access and medication" -- one test per row,
//! against `pilot::clinical_access`.

use rfc0029_conformance::pilot::clinical_access::{
    AccessMode, AccessStatus, AllergyFact, ClinicalAccessRegistry, MedicationOrder, PatientRecord,
    RetentionDecision,
};

fn registry_with_patient(patient_id: &str) -> ClinicalAccessRegistry {
    let mut reg = ClinicalAccessRegistry::new();
    reg.put_patient(PatientRecord {
        canonical_patient_id: patient_id.into(),
        allergies: vec![],
        retention_hold_authority: None,
        deletion_requested: false,
    });
    reg
}

/// Row 1: normal versus emergency record access -- distinct scope/capability
/// and field projection.
#[test]
fn normal_and_break_glass_access_grant_distinct_field_projections() {
    let mut reg = registry_with_patient("patient:1");
    reg.declare_emergency("clinician:1", "patient:1", "emergency-treatment");

    let normal = reg.access("clinician:1", "patient:1", "treatment", AccessMode::Normal);
    assert_eq!(normal.status, AccessStatus::Granted);

    let emergency = reg.access("clinician:1", "patient:1", "emergency-treatment", AccessMode::BreakGlass);
    assert_eq!(emergency.status, AccessStatus::Granted);

    assert_ne!(normal.capability_scope, emergency.capability_scope);
    assert!(emergency.capability_scope.len() > normal.capability_scope.len());
    assert!(emergency.capability_scope.contains(&"allergies".to_string()));
    assert!(!normal.capability_scope.contains(&"allergies".to_string()));
}

/// Row 2: break-glass replay against another patient or purpose -- binding
/// mismatch and rejection.
#[test]
fn break_glass_replay_against_a_different_patient_or_purpose_is_a_binding_mismatch() {
    let mut reg = registry_with_patient("patient:1");
    reg.put_patient(PatientRecord {
        canonical_patient_id: "patient:2".into(),
        allergies: vec![],
        retention_hold_authority: None,
        deletion_requested: false,
    });
    reg.declare_emergency("clinician:1", "patient:1", "emergency-treatment");

    let wrong_patient = reg.access("clinician:1", "patient:2", "emergency-treatment", AccessMode::BreakGlass);
    assert_eq!(wrong_patient.status, AccessStatus::Denied);
    assert_eq!(wrong_patient.diagnostic.as_deref(), Some("BreakGlassBindingMismatch"));

    let wrong_purpose = reg.access("clinician:1", "patient:1", "curiosity", AccessMode::BreakGlass);
    assert_eq!(wrong_purpose.status, AccessStatus::Denied);
    assert_eq!(wrong_purpose.diagnostic.as_deref(), Some("BreakGlassBindingMismatch"));

    let no_binding_at_all = reg.access("clinician:2", "patient:1", "emergency-treatment", AccessMode::BreakGlass);
    assert_eq!(no_binding_at_all.diagnostic.as_deref(), Some("NoDeclaredEmergencyBinding"));
}

/// Row 3: consent/PDP outage -- admitted safety posture, never an implicit
/// exception. Absent a named posture (none is modeled here), outage denies
/// even a break-glass request.
#[test]
fn consent_pdp_outage_denies_even_break_glass_access() {
    let mut reg = registry_with_patient("patient:1");
    reg.declare_emergency("clinician:1", "patient:1", "emergency-treatment");
    reg.consent_pdp_available = false;

    let result = reg.access("clinician:1", "patient:1", "emergency-treatment", AccessMode::BreakGlass);
    assert_eq!(result.status, AccessStatus::Denied);
    assert_eq!(result.diagnostic.as_deref(), Some("ConsentPdpUnavailable"));
}

/// Row 4: conflicting allergy authorities -- declared disagreement result
/// and evidence, never silently merged.
#[test]
fn conflicting_allergy_authorities_produce_a_declared_disagreement_not_a_silent_merge() {
    let mut reg = ClinicalAccessRegistry::new();
    reg.put_patient(PatientRecord {
        canonical_patient_id: "patient:1".into(),
        allergies: vec![
            AllergyFact { allergen: "penicillin".into(), authority: "lab:a".into(), present: true },
            AllergyFact { allergen: "penicillin".into(), authority: "lab:b".into(), present: false },
        ],
        retention_hold_authority: None,
        deletion_requested: false,
    });

    let result = reg.access("clinician:1", "patient:1", "treatment", AccessMode::Normal);
    assert_eq!(result.status, AccessStatus::AuthorityDisagreement);
    assert!(result.diagnostic.unwrap().contains("penicillin"));
}

/// Row 5: fact changes before dispense/administration -- transition
/// dependency invalidation. Same time-of-check/time-of-use property as
/// `funds_reserve`'s resource-version check.
#[test]
fn a_cancelled_or_changed_order_is_re_evaluated_not_administered_from_a_stale_read() {
    let mut reg = registry_with_patient("patient:1");
    reg.put_medication_order(MedicationOrder {
        order_id: "order:1".into(),
        patient_id: "patient:1".into(),
        medication_id: "med:amoxicillin".into(),
        version: 1,
        cancelled: false,
    });
    // Physician cancels/amends the order before it is administered.
    reg.put_medication_order(MedicationOrder {
        order_id: "order:1".into(),
        patient_id: "patient:1".into(),
        medication_id: "med:amoxicillin".into(),
        version: 2,
        cancelled: true,
    });
    let attempt = reg.administer("order:1", 1);
    assert_eq!(attempt, Err("OrderCancelled"));

    reg.put_medication_order(MedicationOrder {
        order_id: "order:2".into(),
        patient_id: "patient:1".into(),
        medication_id: "med:amoxicillin".into(),
        version: 1,
        cancelled: false,
    });
    let stale_version_attempt = reg.administer("order:2", 99);
    assert_eq!(stale_version_attempt, Err("OrderVersionStale"));
}

/// Row 6: patient alias, merge, replica or search projection -- explicit
/// identity/policy inheritance, not per-alias re-derivation of policy.
#[test]
fn an_alias_resolves_to_the_same_canonical_identity_and_policy() {
    let mut reg = registry_with_patient("patient:canonical-1");
    reg.put_alias("patient:mrn-old-system-42", "patient:canonical-1");
    reg.declare_emergency("clinician:1", "patient:canonical-1", "emergency-treatment");

    let via_alias = reg.access("clinician:1", "patient:mrn-old-system-42", "treatment", AccessMode::Normal);
    let via_canonical = reg.access("clinician:1", "patient:canonical-1", "treatment", AccessMode::Normal);
    assert_eq!(via_alias.status, AccessStatus::Granted);
    assert_eq!(via_alias.capability_scope, via_canonical.capability_scope);

    let unknown = reg.access("clinician:1", "patient:never-registered", "treatment", AccessMode::Normal);
    assert_eq!(unknown.diagnostic.as_deref(), Some("UnknownPatient"));
}

/// Row 7: evidence outage during emergency access -- declared finality
/// mode and reconciliation, not a plain grant.
#[test]
fn evidence_outage_during_emergency_access_is_a_distinct_pending_state() {
    let mut reg = registry_with_patient("patient:1");
    reg.declare_emergency("clinician:1", "patient:1", "emergency-treatment");
    reg.evidence_plane_available = false;

    let result = reg.access("clinician:1", "patient:1", "emergency-treatment", AccessMode::BreakGlass);
    assert_eq!(result.status, AccessStatus::GrantedPendingEvidenceReconciliation);
    assert!(!result.capability_scope.is_empty(), "clinically urgent access still proceeds");
}

/// Row 8: medication administered -- irreversible effect, never rewritten
/// as compensatable atomic work.
#[test]
fn administering_medication_twice_is_rejected_not_compensated() {
    let mut reg = registry_with_patient("patient:1");
    reg.put_medication_order(MedicationOrder {
        order_id: "order:once".into(),
        patient_id: "patient:1".into(),
        medication_id: "med:x".into(),
        version: 1,
        cancelled: false,
    });
    let first = reg.administer("order:once", 1);
    assert!(first.is_ok());
    let second = reg.administer("order:once", 1);
    assert_eq!(second, Err("AlreadyAdministered"), "no compensating 'un-administer' path exists, by design");
}

/// Row 9: research derivation from treatment data -- explicit derivation
/// and purpose inheritance.
#[test]
fn a_research_derived_record_is_only_usable_for_its_inherited_purpose() {
    let mut reg = registry_with_patient("patient:1");
    let derived = reg.derive_for_research("patient:1").unwrap();
    assert!(derived.usable_for("research"));
    assert!(!derived.usable_for("marketing"), "purpose does not silently expand beyond what was derived");

    reg.put_alias("patient:alias-1", "patient:1");
    // suppress unused-field warning while keeping the source id documented
    let _ = &derived.source_patient_id;
}

/// Row 10: retention conflicts with deletion request -- applicability/
/// conflict resolution, not arbitrary precedence.
#[test]
fn retention_hold_overrides_deletion_only_when_an_authority_is_actually_named() {
    let mut reg = ClinicalAccessRegistry::new();
    reg.put_patient(PatientRecord {
        canonical_patient_id: "patient:held".into(),
        allergies: vec![],
        retention_hold_authority: Some("authority:legal-hold".into()),
        deletion_requested: true,
    });
    reg.put_patient(PatientRecord {
        canonical_patient_id: "patient:free".into(),
        allergies: vec![],
        retention_hold_authority: None,
        deletion_requested: true,
    });
    reg.put_patient(PatientRecord {
        canonical_patient_id: "patient:no-request".into(),
        allergies: vec![],
        retention_hold_authority: None,
        deletion_requested: false,
    });

    assert_eq!(
        reg.resolve_retention_conflict("patient:held").unwrap(),
        RetentionDecision::Retained { authority: "authority:legal-hold".into() }
    );
    assert_eq!(reg.resolve_retention_conflict("patient:free").unwrap(), RetentionDecision::Deleted);
    assert_eq!(reg.resolve_retention_conflict("patient:no-request").unwrap(), RetentionDecision::Deleted);
}
