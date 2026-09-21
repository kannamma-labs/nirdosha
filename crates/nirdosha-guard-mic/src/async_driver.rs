//! Async `StoreDriver` bridge — Plan Phase 18.
//!
//! `GuardClient`/`StoreDriver` are synchronous throughout, and stay that
//! way — every other phase this session built (Phases 7-17) depends on
//! that. This module adds the async path *alongside* it, per the plan's
//! own scope note ("Add an async `StoreDriver` variant (**or an async
//! wrapper trait**) alongside the sync one — don't break the sync path
//! other phases build on"): `AsyncStoreDriverAdapter<D>` wraps any real,
//! existing `D: StoreDriver` and offloads each call to
//! `tokio::task::spawn_blocking` — genuine thread-pool offload of
//! synchronous (and, for `PostgresStoreDriver`, real blocking I/O) work,
//! not a fake `async fn` that never actually awaits anything. An async
//! caller (the networked policy-snapshot client in
//! `nirdosha-guard-snapshot-ws`, or any future async MIC surface) gets a
//! real non-blocking driver without either `StoreDriver` implementor
//! having to become async internally.

use std::sync::Arc;

use nirdosha_guard_core::{CapabilityManifest, LineageFacts};

use crate::{EntityBytes, PlanError, PlanIr, Prepared, QueryResult, ReadPlanIr, Receipt, StoreDriver};

#[async_trait::async_trait]
pub trait AsyncStoreDriver: Send + Sync {
	/// Owned, not `&CapabilityManifest` like the sync trait — a reference
	/// tied to `&self`'s lifetime can't cross a `spawn_blocking` boundary,
	/// which needs `'static` owned data to move onto the blocking pool.
	async fn manifest(&self) -> CapabilityManifest;
	async fn prepare(&self, plan: PlanIr) -> Result<Prepared, PlanError>;
	async fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError>;
	async fn query(&self, plan: ReadPlanIr) -> Result<QueryResult, PlanError>;
	async fn lineage(&self) -> LineageFacts;
}

/// Wraps a real `D: StoreDriver` behind `AsyncStoreDriver`, offloading
/// every call to `tokio::task::spawn_blocking`. `Arc<D>` (not `&D`)
/// because each call clones the handle to move it onto the blocking
/// task — the driver itself must already be `Send + Sync + 'static`
/// (every real `StoreDriver` in this workspace already is, to be usable
/// from `GuardClient` at all).
pub struct AsyncStoreDriverAdapter<D> {
	inner: Arc<D>,
}

impl<D: StoreDriver + 'static> AsyncStoreDriverAdapter<D> {
	pub fn new(inner: D) -> Self {
		Self { inner: Arc::new(inner) }
	}
}

fn blocking_panic_message(context: &str) -> String {
	format!("nirdosha-guard-mic: blocking task panicked inside AsyncStoreDriverAdapter::{context}")
}

#[async_trait::async_trait]
impl<D: StoreDriver + 'static> AsyncStoreDriver for AsyncStoreDriverAdapter<D> {
	async fn manifest(&self) -> CapabilityManifest {
		let inner = self.inner.clone();
		tokio::task::spawn_blocking(move || inner.manifest().clone()).await.unwrap_or_else(|_| panic!("{}", blocking_panic_message("manifest")))
	}

	async fn prepare(&self, plan: PlanIr) -> Result<Prepared, PlanError> {
		let inner = self.inner.clone();
		tokio::task::spawn_blocking(move || inner.prepare(&plan)).await.unwrap_or_else(|_| Err(PlanError::Store(blocking_panic_message("prepare"))))
	}

	async fn commit(&self, prepared: Prepared, entity: EntityBytes) -> Result<Receipt, PlanError> {
		let inner = self.inner.clone();
		tokio::task::spawn_blocking(move || inner.commit(prepared, entity)).await.unwrap_or_else(|_| Err(PlanError::Store(blocking_panic_message("commit"))))
	}

	async fn query(&self, plan: ReadPlanIr) -> Result<QueryResult, PlanError> {
		let inner = self.inner.clone();
		tokio::task::spawn_blocking(move || inner.query(&plan)).await.unwrap_or_else(|_| Err(PlanError::Store(blocking_panic_message("query"))))
	}

	async fn lineage(&self) -> LineageFacts {
		let inner = self.inner.clone();
		tokio::task::spawn_blocking(move || inner.lineage()).await.unwrap_or_else(|_| panic!("{}", blocking_panic_message("lineage")))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::MemStoreDriver;

	#[tokio::test]
	async fn async_adapter_really_offloads_to_the_blocking_pool_and_round_trips_a_write() {
		let adapter = AsyncStoreDriverAdapter::new(MemStoreDriver::new());
		let manifest = adapter.manifest().await;
		assert_eq!(manifest.driver_name, "memory");

		let plan = PlanIr {
			resource: "async-orders".into(),
			dataset: "memory".into(),
			filter: Some(nirdosha_guard_core::FilterExpr::TenantEq { value: nirdosha_guard_core::Value::Str("tenant-a".into()) }),
			row_scope: None,
			action: nirdosha_guard_core::WriteAction::Create,
			affected_row_cap: None,
			policy_version: "v1".into(),
		};
		let prepared = adapter.prepare(plan).await.expect("real prepare must succeed");
		let receipt = adapter.commit(prepared, EntityBytes(b"async row".to_vec())).await.expect("real commit must succeed");
		assert!(!receipt.store_commit_id.is_empty());

		let read_plan = ReadPlanIr {
			resource: "async-orders".into(),
			dataset: "memory".into(),
			filter: Some(nirdosha_guard_core::FilterExpr::TenantEq { value: nirdosha_guard_core::Value::Str("tenant-a".into()) }),
			caps: vec![],
			pagination: nirdosha_guard_core::PaginationMode::LimitOnly { limit: 10 },
			policy_version: "v1".into(),
		};
		let result = adapter.query(read_plan).await.expect("real query must succeed");
		assert_eq!(result.rows.len(), 1);
		assert_eq!(result.rows[0].0, b"async row");
	}

	#[tokio::test]
	async fn async_adapter_surfaces_a_real_driver_error_not_a_panic() {
		let adapter = AsyncStoreDriverAdapter::new(MemStoreDriver::new());
		let make_plan = || PlanIr {
			resource: "async-duplicate".into(),
			dataset: "memory".into(),
			filter: Some(nirdosha_guard_core::FilterExpr::TenantEq { value: nirdosha_guard_core::Value::Str("tenant-a".into()) }),
			row_scope: None,
			action: nirdosha_guard_core::WriteAction::Create,
			affected_row_cap: None,
			policy_version: "v1".into(),
		};
		let first = adapter.prepare(make_plan()).await.unwrap();
		adapter.commit(first, EntityBytes(b"row-1".to_vec())).await.expect("first create must succeed");

		let second = adapter.prepare(make_plan()).await.unwrap();
		let result = adapter.commit(second, EntityBytes(b"row-2".to_vec())).await;
		assert!(result.is_err(), "a real Create-on-an-existing-resource rejection must cross the async boundary as Err, not be swallowed: {result:?}");
	}
}
