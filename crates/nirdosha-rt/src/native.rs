//! Shared native-runtime boundaries. Requires the `native` feature.
//!
//! `Auth::login` remains a fixture API. Production authorization uses a
//! configured Authority and signature-verified Identity from this module.
pub use nirdosha_runtime_kernels::kernel::transact::durable::{DurableSaga, Outcome, Saga};
pub use nirdosha_runtime_kernels::process::WorkerProcess;
pub use nirdosha_runtime_kernels::verified_identity::{Authority, Grant, Identity};

/// Request boundary: verifies the token and role before creating any durable
/// transaction row or invoking an external operation. Recover at startup.
pub struct AuthorizedSaga<S: Saga> {
    authority: Authority,
    role: String,
    saga: DurableSaga<S>,
}

impl<S: Saga> AuthorizedSaga<S> {
    pub fn new(
        authority: Authority,
        role: String,
        mut saga: DurableSaga<S>,
    ) -> Result<Self, String> {
        if role.is_empty() {
            return Err("required role cannot be empty".into());
        }
        saga.recover()?;
        Ok(Self {
            authority,
            role,
            saga,
        })
    }

    pub fn execute(
        &mut self,
        token: &str,
        now: i64,
        txn_id: &str,
        input: &serde_json::Value,
    ) -> Result<Outcome, String> {
        let identity = self.authority.verify(token, now)?;
        let _grant = self.authority.authorize(&identity, &self.role, now)?;
        self.saga.execute(txn_id, input)
    }
}
