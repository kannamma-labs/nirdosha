//! What [`crate::GuardedTable`] needs to serialize a screen's Rust struct
//! into an `EntityBytes` payload and back, and to know which `resource`
//! name (a `guard_policy!` corpus's `resource ==` string) it stands for.

/// One screen-mounted, guard-enforced dataset. `RESOURCE` must equal the
/// literal `resource` string the corpus's `#[dataset(entity = "...")]`/
/// `guard_policy!` blocks declare (e.g. `"transaction"`) -- policy
/// matching (`evaluator::matches_context`'s `context.entity ==
/// policy.resource`) is plain string equality, so a typo here silently
/// matches nothing rather than failing to compile.
pub trait GuardedEntity: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync + 'static {
    const RESOURCE: &'static str;

    /// This row's own identity within `RESOURCE` (e.g. a `TxnId`'s
    /// string form) -- becomes the suffix of the driver-level storage
    /// key `"<RESOURCE>:<row_id>"` (see `GuardedTable`'s module doc
    /// comment for why storage identity and policy-matching identity
    /// have to be kept separate).
    fn row_id(&self) -> String;

    /// Struct fields that exist only for this screen-row projection (a
    /// synthetic `id: i64` every `nirdosha_rt::screens` render helper
    /// needs, a row-identity field the corpus's own dataset has no
    /// self-referencing column for, ...) and were never part of *any*
    /// corpus `field_policy` at all, on any action. `GuardedTable::
    /// guarded_insert`/`guarded_update`'s G1 check skips these -- every
    /// other field is checked for real, so a genuine drift between the
    /// dataset and a policy's `field_policy` (like `20_ingestion.nir`'s
    /// `ingest-create-txn` missing `account_id` until Phase A's own
    /// first real write caught it) still fails closed. Empty by default;
    /// override only for fields that are provably plumbing-only on every
    /// write action.
    fn field_policy_exempt() -> &'static [&'static str] {
        &[]
    }

    /// Like [`Self::field_policy_exempt`], but for `guarded_insert`
    /// only -- fields a policy-checked *other* action genuinely governs
    /// (so [`Self::field_policy_exempt`] must not swallow them) but that
    /// create-time simply never sets. `TransactionRow`'s `analyst_flag`/
    /// `analyst_flag_reason` are the motivating case: real fields
    /// `analyst-flag-transaction`'s own `field_policy` names for
    /// *update*, always false/empty at ingest time, and never mentioned
    /// by `ingest-create-txn`'s own `field_policy` at all -- exempting
    /// them here (not from [`Self::field_policy_exempt`]) keeps create's
    /// check honest without also blinding update's check to the one
    /// thing it exists to enforce. Defaults to [`Self::field_policy_exempt`]
    /// itself, so a dataset with no such split-personality fields never
    /// needs to think about this at all.
    fn create_field_policy_exempt() -> &'static [&'static str] {
        Self::field_policy_exempt()
    }

    /// Real, per-dataset dispatch for a `requires invariant(name)`
    /// condition -- the only way `nirdosha-guard-screens` can call e.g.
    /// `00_core.nir`'s real `amount_positive(&Transaction) -> bool`
    /// without depending on the app crate that declares it (the
    /// dependency runs the other way: RTM depends on this crate, not the
    /// reverse). `None` means "this dataset hasn't wired that name" --
    /// [`crate::field_policy::check_custom_conditions`] fails closed on
    /// that, exactly as it did before any invariant was wired at all.
    /// Checked against the row as it would exist *after* the write
    /// commits (matching each real invariant fn's own contract, e.g.
    /// `amount_positive` validates the transaction being created).
    fn check_invariant(&self, _name: &str) -> Option<bool> {
        None
    }

    /// Real dispatch for a `requires field(status).transition_allowed()`
    /// condition: can `self` (the row's state *before* this write) reach
    /// `requested_status` (the submitted update's own proposed value for
    /// that field) via *some* legal event in the real `workflow!`-
    /// generated state machine? `requested_status` is the raw decoded
    /// JSON value (not a `&str`) so each dataset's own enum can decide
    /// its own serde representation rather than this generic trait
    /// guessing at one. `None` means "this dataset hasn't wired a
    /// transition check" -- fails closed, same posture as
    /// `check_invariant`.
    fn transition_allowed(&self, _requested_status: &serde_json::Value) -> Option<bool> {
        None
    }

    /// The field identifying "the owning subject" for `filter
    /// subject_scope()` (RTM's `self-read-qa`/`self-update-profile`
    /// policies) -- different per dataset (`user_profile`'s own row is
    /// keyed by `user_id`; nothing generic here should guess which field
    /// an arbitrary dataset uses -- `QaReview`'s current schema, for
    /// example, has no such field at all, a real corpus gap this default
    /// surfaces honestly rather than papering over). `None` (the
    /// default) fails closed, same posture as an unresolved `filter_ref`.
    fn subject_scope_field() -> Option<&'static str> {
        None
    }
}
