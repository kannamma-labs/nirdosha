//! Narrow banking-transfer pilot: one authorization policy, one atomic
//! balance invariant, one capability-consuming in-process gateway over an
//! in-memory transactional fixture. This is a falsification harness against
//! the versioned candidate IR, not a production gateway — it has no store,
//! network, or effect-gateway dependency and authorizes nothing outside this
//! process. Scope is exactly `funds.reserve`; `funds.release` and external
//! settlement submission are named exclusions, not silently covered paths.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// A signed, versioned account fact as a PIP would supply it. "Signed" means
/// bound to a `signature` commitment over its own fields; this pilot does
/// not implement real cryptographic verification, only tamper detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFact {
    pub account_id: String,
    pub subject_id: String,
    pub balance_minor: u64,
    pub version: u64,
    pub valid_until: String,
    pub revoked: bool,
    /// Readiness matrix §7.1 "daily limit under concurrency/partition":
    /// the declared window, authority (the fact authority that signed this
    /// same record), and reservation ceiling for one calendar day
    /// (`now`'s leading `YYYY-MM-DD`). `None` means no daily limit is
    /// declared for this account — not "unlimited by omission" silently,
    /// but an explicit absence a caller can observe.
    pub daily_limit_minor: Option<u64>,
    pub signature: String,
}

impl AccountFact {
    pub fn signed(
        account_id: &str,
        subject_id: &str,
        balance_minor: u64,
        version: u64,
        valid_until: &str,
    ) -> Self {
        Self::signed_with_daily_limit(account_id, subject_id, balance_minor, version, valid_until, None)
    }

    pub fn signed_with_daily_limit(
        account_id: &str,
        subject_id: &str,
        balance_minor: u64,
        version: u64,
        valid_until: &str,
        daily_limit_minor: Option<u64>,
    ) -> Self {
        let mut fact = Self {
            account_id: account_id.into(),
            subject_id: subject_id.into(),
            balance_minor,
            version,
            valid_until: valid_until.into(),
            revoked: false,
            daily_limit_minor,
            signature: String::new(),
        };
        fact.signature = fact.commitment();
        fact
    }

    fn commitment(&self) -> String {
        let payload = format!(
            "{}:{}:{}:{}:{}:{}:{:?}",
            self.account_id,
            self.subject_id,
            self.balance_minor,
            self.version,
            self.valid_until,
            self.revoked,
            self.daily_limit_minor
        );
        format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
    }

    pub fn is_authentic(&self) -> bool {
        self.signature == self.commitment()
    }

    /// A fact authority revoking a fact re-issues it signed with
    /// `revoked = true`; it does not merely flip a field on an
    /// already-issued signed blob (that would be indistinguishable from
    /// tampering, which is exactly what `is_authentic` must catch).
    pub fn revoke(&mut self) {
        self.revoked = true;
        self.signature = self.commitment();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationRequest {
    pub request_id: String,
    pub subject_id: String,
    pub account_id: String,
    pub amount_minor: u64,
    pub account_version: u64,
    pub now: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    Reserved,
    Replayed,
    Denied,
    /// The local commit succeeded but the paired external submission (e.g.
    /// a settlement-rail confirmation) timed out: readiness matrix §7.1
    /// "local ledger commit succeeds, external submit times out." This is
    /// a distinct terminal state from `Reserved` specifically so no caller
    /// can read `Reserved` as "fully settled" — it requires
    /// `Ledger::reconcile` before the request is considered closed, and a
    /// retry of the same request_id never re-executes the local mutation
    /// (idempotency already covers that) or double-submits externally.
    AwaitingReconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationReceipt {
    pub request: ReservationRequest,
    pub status: ReservationStatus,
    pub diagnostic: Option<String>,
    pub resulting_version: Option<u64>,
    pub evidence_hash: String,
}

/// One evidence log entry. Every decision — permitted or denied — is logged;
/// only the `Reserved` path shares the balance mutation's atomic boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub request_id: String,
    pub account_id: String,
    pub status: ReservationStatus,
    pub diagnostic: Option<String>,
    pub resulting_version: Option<u64>,
    pub evidence_hash: String,
}

#[derive(Debug, Default)]
pub struct Ledger {
    pub accounts: BTreeMap<String, AccountFact>,
    pub consumed: BTreeMap<String, ReservationReceipt>,
    /// Simulates an atomic-commit failure (e.g. a storage fault) after every
    /// precondition has already passed. No mutation must occur when this is
    /// set; the pilot's own report checks that invariant, it does not just
    /// assert it.
    pub commit_should_fail: bool,
    pub evidence_log: Vec<EvidenceRecord>,
    /// Cumulative reserved amount per (account_id, calendar day), checked
    /// and updated inside `reserve`'s own atomic boundary — readiness
    /// matrix §7.1 "daily limit under concurrency/partition." Keying by
    /// day, not a rolling window, matches `AccountFact::daily_limit_minor`
    /// being a per-calendar-day declared ceiling.
    daily_reserved: BTreeMap<(String, String), u64>,
    /// Readiness matrix §7.1 "evidence plane unavailable": when false, a
    /// `Reserved` outcome cannot be committed at all, because its evidence
    /// commitment could not share the same atomic boundary as the balance
    /// mutation (§8.3's Numeric-invariant proof obligation) — fail-closed,
    /// not a silently-accepted gap in the evidence trail. Defaults to
    /// available; the pilot's own tests flip it explicitly.
    pub evidence_plane_available: bool,
}

impl Ledger {
    pub fn new() -> Self {
        Self {
            evidence_plane_available: true,
            ..Self::default()
        }
    }

    /// Readiness matrix §7.1 "authority later compromised": affected-decision
    /// discovery. Returns every receipt (permitted or denied) this account
    /// was ever party to, in request order — the reverse index a real
    /// containment response would query after a compromise is detected.
    /// Discovery does not itself retract or compensate a prior `Reserved`
    /// effect; §36.7's containment/compensation obligations are a separate,
    /// explicitly-triggered step this pilot does not model.
    pub fn decisions_for_account<'a>(&'a self, account_id: &str) -> Vec<&'a EvidenceRecord> {
        self.evidence_log
            .iter()
            .filter(|r| r.account_id == account_id)
            .collect()
    }

    /// Readiness matrix §7.1 "local ledger commit succeeds, external submit
    /// times out." Call after a `Reserved` receipt to record the paired
    /// external submission's outcome. `None` means the external system's
    /// result is genuinely unknown (a timeout, not a negative
    /// acknowledgement) — the receipt moves to `AwaitingReconciliation`,
    /// never silently back to a plain `Reserved` "as if" it had settled.
    pub fn confirm_external_submission(
        &mut self,
        request_id: &str,
        external_confirmed: Option<bool>,
    ) -> Result<ReservationStatus, &'static str> {
        let Some(receipt) = self.consumed.get_mut(request_id) else {
            return Err("UnknownRequest");
        };
        if receipt.status != ReservationStatus::Reserved {
            return Err("NotAwaitingExternalSubmission");
        }
        let new_status = match external_confirmed {
            Some(true) => ReservationStatus::Reserved,
            Some(false) | None => ReservationStatus::AwaitingReconciliation,
        };
        receipt.status = new_status;
        Ok(new_status)
    }

    /// Resolves a receipt sitting in `AwaitingReconciliation` once the true
    /// external outcome is later learned. Never issues a second local
    /// mutation or a second external submission by construction — this
    /// only ever changes the *status* of the one receipt already recorded
    /// against `request_id`, matching "no duplicate effect."
    pub fn reconcile(
        &mut self,
        request_id: &str,
        externally_confirmed: bool,
    ) -> Result<ReservationStatus, &'static str> {
        let Some(receipt) = self.consumed.get_mut(request_id) else {
            return Err("UnknownRequest");
        };
        if receipt.status != ReservationStatus::AwaitingReconciliation {
            return Err("NotAwaitingReconciliation");
        }
        receipt.status = if externally_confirmed {
            ReservationStatus::Reserved
        } else {
            ReservationStatus::Denied
        };
        Ok(receipt.status)
    }

    pub fn put_fact(&mut self, fact: AccountFact) {
        self.accounts.insert(fact.account_id.clone(), fact);
    }

    /// Runs the full decision/enforcement protocol for one `funds.reserve`
    /// request: idempotency check, fact provenance/freshness/revocation,
    /// subject binding, resource-version match (time-of-check/time-of-use),
    /// sufficient funds, then an atomic commit that either mutates the
    /// balance and writes the receipt together, or does neither.
    pub fn reserve(&mut self, request: &ReservationRequest) -> ReservationReceipt {
        if let Some(existing) = self.consumed.get(&request.request_id).cloned() {
            if existing.request == *request {
                let mut replay = existing;
                // AwaitingReconciliation is not a settled outcome; relabeling
                // a retry of it to Replayed would misreport a still-pending
                // decision as though it had a known, cached result. Every
                // other status (Reserved, Denied) is settled, so the retry
                // reports the ordinary Replayed label.
                if replay.status != ReservationStatus::AwaitingReconciliation {
                    replay.status = ReservationStatus::Replayed;
                }
                self.log(&replay);
                return replay;
            }
            return self.deny(request, "IdempotencyKeyReuseMismatch");
        }

        let Some(fact) = self.accounts.get(&request.account_id).cloned() else {
            return self.deny(request, "UnknownAccount");
        };
        if !fact.is_authentic() {
            return self.deny(request, "FactSignatureInvalid");
        }
        if fact.revoked {
            return self.deny(request, "RevokedFact");
        }
        if fact.valid_until.as_str() < request.now.as_str() {
            return self.deny(request, "StaleFact");
        }
        if fact.subject_id != request.subject_id {
            return self.deny(request, "SubjectMismatch");
        }
        if fact.version != request.account_version {
            return self.deny(request, "ResourceVersionMismatch");
        }
        if fact.balance_minor < request.amount_minor {
            return self.deny(request, "InsufficientFunds");
        }
        let day = request.now.get(..10).unwrap_or(request.now.as_str()).to_string();
        let daily_key = (request.account_id.clone(), day.clone());
        if let Some(limit) = fact.daily_limit_minor {
            let already_today = self.daily_reserved.get(&daily_key).copied().unwrap_or(0);
            if already_today + request.amount_minor > limit {
                return self.deny(request, "DailyLimitExceeded");
            }
        }
        if !self.evidence_plane_available {
            // Fail-closed, before any mutation: a Reserved outcome's
            // evidence commitment cannot share this call's atomic boundary
            // if the evidence plane cannot durably accept it right now.
            return self.deny(request, "EvidencePlaneUnavailable");
        }
        if self.commit_should_fail {
            return self.deny(request, "CommitFailed");
        }

        // Atomic boundary: balance mutation, daily-window counter, and
        // receipt/evidence commit happen together in this scope, or (on any
        // path above) none of them did.
        let mut updated = fact;
        updated.balance_minor -= request.amount_minor;
        updated.version += 1;
        updated.signature = updated.commitment();
        let resulting_version = updated.version;
        self.accounts.insert(updated.account_id.clone(), updated);
        *self.daily_reserved.entry(daily_key).or_insert(0) += request.amount_minor;

        let receipt = ReservationReceipt {
            request: request.clone(),
            status: ReservationStatus::Reserved,
            diagnostic: None,
            resulting_version: Some(resulting_version),
            evidence_hash: receipt_hash(request, "Reserved", Some(resulting_version)),
        };
        self.consumed.insert(request.request_id.clone(), receipt.clone());
        self.log(&receipt);
        receipt
    }

    fn deny(&mut self, request: &ReservationRequest, diagnostic: &str) -> ReservationReceipt {
        let receipt = ReservationReceipt {
            request: request.clone(),
            status: ReservationStatus::Denied,
            diagnostic: Some(diagnostic.into()),
            resulting_version: None,
            evidence_hash: receipt_hash(request, diagnostic, None),
        };
        self.log(&receipt);
        receipt
    }

    fn log(&mut self, receipt: &ReservationReceipt) {
        self.evidence_log.push(EvidenceRecord {
            request_id: receipt.request.request_id.clone(),
            account_id: receipt.request.account_id.clone(),
            status: receipt.status,
            diagnostic: receipt.diagnostic.clone(),
            resulting_version: receipt.resulting_version,
            evidence_hash: receipt.evidence_hash.clone(),
        });
    }
}

fn receipt_hash(
    request: &ReservationRequest,
    outcome: &str,
    resulting_version: Option<u64>,
) -> String {
    let payload = format!(
        "{}:{}:{}:{}:{}:{}:{:?}",
        request.request_id,
        request.subject_id,
        request.account_id,
        request.amount_minor,
        request.account_version,
        outcome,
        resulting_version
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}
