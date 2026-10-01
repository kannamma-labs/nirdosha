//! `SharedTable`/`SharedCell` — the managed replacements for a raw
//! `std::sync::Mutex` table (`docs/nirdosha-rt-dialect.md`, dialect-wide
//! lock deny; issue #77). What these tests must prove:
//!
//! 1. the API covers every operation the corpus's `Mutex<HashMap>`
//!    tables performed (insert / get-clone / atomic update / remove /
//!    len / snapshot),
//! 2. `snapshot` is deterministic (sorted, whatever the insertion or
//!    hashing order),
//! 3. concurrent writers through copyable handles never lose an
//!    update (the race-freedom property a hand-written lock was
//!    supposed to buy, now without the hand-written lock),
//! 4. a panicking writer cannot wedge the table (poison recovery is
//!    `into_inner`, not a panic on the next reader).

use nirdosha_rt::prelude::{SharedCell, SharedTable};

#[test]
fn table_covers_the_corpus_operations() {
    let t: SharedTable<i64, String> = SharedTable::new();
    assert!(t.is_empty());
    assert_eq!(t.insert(1, "a".into()), None);
    assert_eq!(t.insert(1, "b".into()), Some("a".into()));
    assert_eq!(t.get(&1), Some("b".into()));
    assert_eq!(t.get(&2), None);
    // the `get_mut` state-check-and-mutate pattern, atomically
    let flipped = t.update(&1, |entry| {
        entry.map(|s| {
            let upper = s.to_uppercase();
            *s = upper.clone();
            upper
        })
    });
    assert_eq!(flipped, Some("B".into()));
    assert_eq!(t.get(&1), Some("B".into()));
    assert_eq!(t.len(), 1);
    assert_eq!(t.remove(&1), Some("B".into()));
    assert_eq!(t.remove(&1), None);
    assert!(t.is_empty());
    // the `entry(k).or_default()` accumulate pattern, atomically
    t.upsert_with(7, |v: &mut String| v.push_str("a"));
    t.upsert_with(7, |v: &mut String| v.push_str("b"));
    assert_eq!(t.get(&7), Some("ab".into()));
    assert_eq!(t.get(&8), None, "upsert_with only creates the key it was given");
    let n: SharedTable<i64, i64> = SharedTable::new();
    n.upsert_with(1, |v| *v += 3);
    n.upsert_with(1, |v| *v += 4);
    assert_eq!(n.get(&1), Some(7));
}

#[test]
fn snapshot_is_sorted_whatever_the_insertion_order() {
    let t: SharedTable<i64, i64> = SharedTable::new();
    for (k, v) in [(3, 30), (1, 10), (2, 20)] {
        t.insert(k, v);
    }
    let keys: Vec<i64> = t.snapshot().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, [1, 2, 3]);
}

#[test]
fn concurrent_writers_never_lose_an_update() {
    // 8 threads × 200 increments through copyable handles — the
    // load a hand-written `Mutex<HashMap>` would carry, with no
    // hand-written lock anywhere in dialect code. Handles are joined,
    // so when the loop ends every write has landed.
    let t: SharedTable<i64, i64> = SharedTable::new();
    let mut handles = Vec::new();
    for i in 0..8 {
        t.insert(i, 0);
        let h = t.clone();
        handles.push(nirdosha_rt::prelude::spawn(move || {
            for _ in 0..200 {
                h.update(&i, |entry| {
                    if let Some(v) = entry {
                        *v += 1;
                    }
                });
            }
        }));
    }
    for handle in handles {
        nirdosha_rt::prelude::join(handle);
    }
    for k in 0..8 {
        assert_eq!(t.get(&k), Some(200), "key {k} lost updates");
    }
}

#[test]
fn cell_is_the_with_store_pattern() {
    struct Store {
        rows: Vec<String>,
    }
    let cell = SharedCell::new(Store { rows: vec![] });
    let n = cell.with(|store| {
        store.rows.push("r1".into());
        store.rows.len()
    });
    assert_eq!(n, 1);
    assert_eq!(cell.with(|store| store.rows.len()), 1);
    let old = cell.replace(Store { rows: vec![] });
    assert_eq!(old.rows.len(), 1);
    assert_eq!(cell.with(|store| store.rows.len()), 0);
}

#[test]
fn a_panicking_writer_does_not_wedge_the_table() {
    let t: SharedTable<i64, i64> = SharedTable::new();
    t.insert(1, 10);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        t.update(&1, |mut entry| {
            let _ = entry.take();
            panic!("writer panicked mid-update");
        })
    }));
    // The lock is poisoned (the update panicked while holding it) —
    // but the managed primitive recovers the data and keeps serving,
    // which a raw `.lock().unwrap()` would not.
    assert_eq!(t.get(&1), Some(10));
    assert_eq!(t.insert(2, 20), None);
    assert_eq!(t.len(), 2);
}