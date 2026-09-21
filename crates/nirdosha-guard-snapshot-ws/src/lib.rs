//! Networked policy-snapshot distribution — Plan Phase 18.
//!
//! `nirdosha_guard_core::snapshot::{PolicyStore, PolicySnapshot}` are
//! genuinely `[DONE]` (replay/commit-time revalidation types, a real,
//! tested `InMemoryPolicyStore`), but had no network transport — a
//! policy change required a redeploy, not a hot reload. This crate is
//! that transport: a real `SnapshotServer` (WebSocket, matching
//! `crates/presence-gateway`'s existing transport choice — this
//! workspace's only other real-time push service — for consistency
//! rather than a second, independent transport decision) that pushes a
//! `PolicySnapshot` to every connected client on `publish`, and a real
//! `SnapshotClient` that connects, receives updates, and calls the
//! existing `PolicyStore::replace` change-callback hook.
//!
//! **`PolicySnapshot` carries a version/hash fingerprint, not the policy
//! set itself** — that's this type's own pre-existing, established
//! design (`nirdosha-guard-core/src/snapshot.rs`, unchanged by this
//! phase), not something this transport invents. A real deployment's
//! `on_change` callback reacts to a new fingerprint by re-fetching the
//! corresponding policy content (a registry dump keyed by the new
//! version) from wherever it's authoritative — this crate's job is
//! getting that fingerprint to a running `GuardClient` promptly and
//! reliably, not owning how policy content itself is distributed.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use nirdosha_guard_core::snapshot::PolicySnapshot;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
pub enum SnapshotTransportError {
	Io(std::io::Error),
	WebSocket(tokio_tungstenite::tungstenite::Error),
	Json(serde_json::Error),
}

impl std::fmt::Display for SnapshotTransportError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SnapshotTransportError::Io(e) => write!(f, "io error: {e}"),
			SnapshotTransportError::WebSocket(e) => write!(f, "websocket error: {e}"),
			SnapshotTransportError::Json(e) => write!(f, "json error: {e}"),
		}
	}
}
impl std::error::Error for SnapshotTransportError {}
impl From<std::io::Error> for SnapshotTransportError {
	fn from(e: std::io::Error) -> Self {
		SnapshotTransportError::Io(e)
	}
}
impl From<tokio_tungstenite::tungstenite::Error> for SnapshotTransportError {
	fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
		SnapshotTransportError::WebSocket(e)
	}
}

/// A real WebSocket server broadcasting `PolicySnapshot` updates. Every
/// newly-connected client is sent the current snapshot immediately (so a
/// client that connects between publishes still learns the real current
/// state, not just future updates), then every subsequent `publish` is
/// forwarded live.
pub struct SnapshotServer {
	listener: TcpListener,
	current: Arc<Mutex<PolicySnapshot>>,
	tx: broadcast::Sender<PolicySnapshot>,
}

impl SnapshotServer {
	pub async fn bind(addr: &str, initial: PolicySnapshot) -> Result<Self, SnapshotTransportError> {
		let listener = TcpListener::bind(addr).await?;
		let (tx, _rx) = broadcast::channel(16);
		Ok(Self { listener, current: Arc::new(Mutex::new(initial)), tx })
	}

	pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
		self.listener.local_addr()
	}

	/// Publishes a real update to every currently-connected client and
	/// updates the snapshot any newly-connecting client is sent.
	pub fn publish(&self, snapshot: PolicySnapshot) {
		*self.current.lock().expect("snapshot lock poisoned") = snapshot.clone();
		// No receivers connected yet is not an error — a client that
		// connects later still gets the now-current snapshot via the
		// connect-time send below.
		let _ = self.tx.send(snapshot);
	}

	/// Runs the accept loop forever — the caller spawns this on a real
	/// tokio task (`tokio::spawn(server.serve())`) and keeps the returned
	/// handle (or a shutdown channel of its own) to stop it.
	pub async fn serve(self: Arc<Self>) {
		loop {
			let Ok((stream, _peer)) = self.listener.accept().await else { continue };
			let server = self.clone();
			tokio::spawn(async move {
				let _ = server.handle_connection(stream).await;
			});
		}
	}

	async fn handle_connection(&self, stream: TcpStream) -> Result<(), SnapshotTransportError> {
		let ws = tokio_tungstenite::accept_async(stream).await?;
		let (mut write, _read) = ws.split();
		let initial = self.current.lock().expect("snapshot lock poisoned").clone();
		write.send(Message::Text(serde_json::to_string(&initial)?.into())).await?;

		let mut rx = self.tx.subscribe();
		loop {
			match rx.recv().await {
				Ok(snapshot) => {
					write.send(Message::Text(serde_json::to_string(&snapshot)?.into())).await?;
				}
				Err(broadcast::error::RecvError::Lagged(_)) => continue,
				Err(broadcast::error::RecvError::Closed) => break,
			}
		}
		Ok(())
	}
}

impl From<serde_json::Error> for SnapshotTransportError {
	fn from(e: serde_json::Error) -> Self {
		SnapshotTransportError::Json(e)
	}
}

/// Connects to a real `SnapshotServer` and calls `on_snapshot` for every
/// real update received (including the connect-time current snapshot) —
/// a caller wires this to `PolicyStore::replace` (and, transitively, to
/// `GuardClient::apply_policy_snapshot` via that store's own `on_change`
/// callback) to get a live-updating `GuardClient` with no restart.
pub async fn connect_and_apply<F>(url: &str, mut on_snapshot: F) -> Result<(), SnapshotTransportError>
where
	F: FnMut(PolicySnapshot) + Send,
{
	let (ws, _response) = tokio_tungstenite::connect_async(url).await?;
	let (_write, mut read) = ws.split();
	while let Some(message) = read.next().await {
		let message = message?;
		if let Message::Text(text) = message {
			let snapshot: PolicySnapshot = serde_json::from_str(&text)?;
			on_snapshot(snapshot);
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn snapshot(version: &str) -> PolicySnapshot {
		PolicySnapshot { version: version.into(), records_hash: format!("hash-{version}"), issued_at: 1 }
	}

	#[tokio::test]
	async fn a_connecting_client_receives_the_real_current_snapshot_immediately() {
		let server = Arc::new(SnapshotServer::bind("127.0.0.1:0", snapshot("v1")).await.unwrap());
		let addr = server.local_addr().unwrap();
		tokio::spawn(server.clone().serve());

		let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
		let url = format!("ws://{addr}");
		tokio::spawn(async move {
			let _ = connect_and_apply(&url, move |snapshot| {
				let _ = tx.send(snapshot);
			})
			.await;
		});

		let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.expect("must receive within timeout").expect("channel must not close");
		assert_eq!(received.version, "v1");
	}

	#[tokio::test]
	async fn a_real_publish_reaches_an_already_connected_client() {
		let server = Arc::new(SnapshotServer::bind("127.0.0.1:0", snapshot("v1")).await.unwrap());
		let addr = server.local_addr().unwrap();
		tokio::spawn(server.clone().serve());

		let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
		let url = format!("ws://{addr}");
		tokio::spawn(async move {
			let _ = connect_and_apply(&url, move |snapshot| {
				let _ = tx.send(snapshot);
			})
			.await;
		});

		// Drain the connect-time initial snapshot first.
		let first = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
		assert_eq!(first.version, "v1");

		server.publish(snapshot("v2"));
		let second = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.expect("must receive the real published update").unwrap();
		assert_eq!(second.version, "v2");
	}
}
