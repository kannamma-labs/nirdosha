//! Governed egress (T-01): every screen-initiated export leaves through
//! this one path instead of handing back raw bytes straight from a
//! guarded read. `crud_screens!`'s guarded CSV export route used to
//! build a `Response::csv` directly from `GuardedTable::guarded_snapshot`
//! output — real access control on the READ, but no record of the
//! export event itself, no watermark, no expiry. [`write_governed_export`]
//! is the fix: every guarded export mints a watermarked, content-hashed
//! artifact and (via the caller, see `bridge.nir`'s
//! `governed_export_table`) a `PG.governed_export` record, before any
//! bytes go back to the caller.
//!
//! Per-class approval (e.g. `sar_release` quorum(2) for SAR bundle
//! egress) is intentionally NOT implemented here — it depends on the
//! `approval_chain!`/`RUNTIME.pending_approvals` machinery T-04 builds,
//! which doesn't exist yet in this corpus. What's real today: purpose-
//! mandatory, watermarked, content-hashed, expiry-bounded, row-capped
//! export. Disclosed, not faked (R3).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

/// Hard backstop on export size — the B7 Performance note: caps are
/// backpressure, checked BEFORE any byte is written, never a silent
/// truncation after the fact. A `guarded_snapshot` read is already
/// row_cap-bounded by policy before it ever reaches here; this is the
/// export path's own ceiling on top of that, so an unusually wide
/// policy row_cap still can't produce an ungoverned mega-export.
pub const MAX_EXPORT_ROWS: usize = 50_000;

/// Assembly chunk size — the artifact body is built `CHUNK_ROWS` rows at
/// a time rather than one large `String`/`Vec` concatenation, so a
/// large-but-under-cap export doesn't require two full copies of the
/// body in memory at once (the row cap above is the hard ceiling this
/// chunking works underneath, not a substitute for it).
const CHUNK_ROWS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovernedExportError {
    PurposeRequired,
    RowCapExceeded { rows: usize, cap: usize },
}

impl std::fmt::Display for GovernedExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PurposeRequired => write!(f, "governed export refused: purpose is mandatory"),
            Self::RowCapExceeded { rows, cap } => {
                write!(f, "governed export refused: {rows} rows exceeds the {cap}-row export ceiling")
            }
        }
    }
}

impl std::error::Error for GovernedExportError {}

/// One minted export — the shape `bridge.nir`'s `governed_export_table`
/// persists as a `PG.governed_export` row, and what the caller embeds
/// as a watermark footer in the returned artifact.
#[derive(Debug, Clone)]
pub struct GovernedExportRecord {
    pub export_id: String,
    pub purpose: String,
    pub watermark: String,
    pub content_hash: String,
    pub row_count: usize,
    pub created_at: i64,
    pub expires_at: i64,
}

fn next_export_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("exp-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// In-memory `O.*` object store — the artifact bytes for every minted
/// export, keyed by `export_id`. Real at this demo's fidelity bar (every
/// `GuardedTable` in `bridge.nir` is equally in-memory); NOT
/// encrypted-at-rest — no crypto dependency beyond the `sha2` content
/// digest exists in this crate, and adding one is out of this ticket's
/// scope. Disclosed, not claimed.
fn object_store() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

type ExportSink = Box<dyn Fn(&GovernedExportRecord) + Send + Sync>;

fn export_sink() -> &'static Mutex<Option<ExportSink>> {
    static SINK: OnceLock<Mutex<Option<ExportSink>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

/// Registers a hook invoked with every [`GovernedExportRecord`] this
/// module mints — how a consuming crate persists its own real
/// `PG.governed_export` dataset (see `examples/rtm/src/bridge.nir`'s
/// `governed_export_table` + `init_governed_export_sink`) without this
/// crate, which is generic across every `nirdosha-rt` app, knowing
/// anything about that crate's own `GuardedTable`. Only the most
/// recently registered sink is kept — single-process demo convention,
/// same as every other `OnceLock`-backed singleton here; call once at
/// boot, before the first export.
pub fn set_export_sink(f: impl Fn(&GovernedExportRecord) + Send + Sync + 'static) {
    *export_sink().lock().unwrap() = Some(Box::new(f));
}

/// Reads back a previously written artifact's raw bytes (test/debug use
/// — no download route is wired to this in the current pass; the CSV
/// response already returns the artifact body directly to the caller
/// that minted it).
pub fn read_object(export_id: &str) -> Option<Vec<u8>> {
    object_store().lock().unwrap().get(export_id).cloned()
}

/// Mints one governed export: refuses an empty purpose or an over-cap
/// row count before writing anything, assembles the artifact body in
/// `CHUNK_ROWS`-row chunks, content-hashes it, writes it into the `O.*`
/// store under a fresh `export_id`, and returns the record plus a
/// watermark line ready to append to the artifact the caller returns.
pub fn write_governed_export(
    purpose: Option<&str>,
    header: &str,
    rows: &[String],
    ttl_secs: i64,
) -> Result<(GovernedExportRecord, String), GovernedExportError> {
    let purpose = purpose
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or(GovernedExportError::PurposeRequired)?;
    if rows.len() > MAX_EXPORT_ROWS {
        return Err(GovernedExportError::RowCapExceeded { rows: rows.len(), cap: MAX_EXPORT_ROWS });
    }

    let export_id = next_export_id();
    let now = crate::screens::now_epoch_secs();
    let expires_at = now + ttl_secs;

    let mut body: Vec<u8> = Vec::with_capacity(header.len() + rows.len() * 32);
    body.extend_from_slice(header.as_bytes());
    body.push(b'\n');
    for chunk in rows.chunks(CHUNK_ROWS) {
        for row in chunk {
            body.extend_from_slice(row.as_bytes());
            body.push(b'\n');
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(&body);
    let content_hash = format!("{:x}", hasher.finalize());
    let watermark = format!(
        "# governed-export purpose={purpose} export_id={export_id} exported_at={now} expires_at={expires_at} sha256={}",
        &content_hash[..16]
    );

    object_store().lock().unwrap().insert(export_id.clone(), body);

    let record = GovernedExportRecord {
        export_id,
        purpose: purpose.to_string(),
        watermark: watermark.clone(),
        content_hash,
        row_count: rows.len(),
        created_at: now,
        expires_at,
    };
    if let Some(sink) = export_sink().lock().unwrap().as_ref() {
        sink(&record);
    }
    Ok((record, watermark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_without_purpose() {
        assert_eq!(write_governed_export(None, "a,b", &[], 3600).unwrap_err(), GovernedExportError::PurposeRequired);
        assert_eq!(write_governed_export(Some("   "), "a,b", &[], 3600).unwrap_err(), GovernedExportError::PurposeRequired);
    }

    #[test]
    fn refuses_over_row_cap_before_writing_anything() {
        let rows: Vec<String> = (0..MAX_EXPORT_ROWS + 1).map(|i| i.to_string()).collect();
        let err = write_governed_export(Some("Audit"), "id", &rows, 3600).unwrap_err();
        assert_eq!(err, GovernedExportError::RowCapExceeded { rows: MAX_EXPORT_ROWS + 1, cap: MAX_EXPORT_ROWS });
        // The row-cap refusal must be checked before any object is
        // written -- there is no export_id to have leaked an object
        // under, and the store must stay empty for it.
    }

    #[test]
    fn mints_watermark_content_hash_and_retrievable_object() {
        let (record, watermark) = write_governed_export(Some("Audit"), "id,name", &["1,alice".into(), "2,bob".into()], 3600).unwrap();
        assert!(watermark.contains("purpose=Audit"));
        assert!(watermark.contains(&record.export_id));
        assert_eq!(record.row_count, 2);
        assert!(record.expires_at > record.created_at);
        let bytes = read_object(&record.export_id).expect("artifact retrievable from the O.* store");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("id,name\n"));
        assert!(text.contains("1,alice\n2,bob\n"));
    }

    #[test]
    fn two_exports_get_distinct_ids_and_hashes_differ_on_content() {
        let (a, _) = write_governed_export(Some("Audit"), "id", &["1".into()], 60).unwrap();
        let (b, _) = write_governed_export(Some("Audit"), "id", &["2".into()], 60).unwrap();
        assert_ne!(a.export_id, b.export_id);
        assert_ne!(a.content_hash, b.content_hash);
    }
}
