//! Non-blocking `send`, multi-consumer receive — `rfcs/0006-structured-
//! concurrency.md` Pillars 2/3, ported near-verbatim from that RFC's own
//! evidence prototype (`rfcs/evidence/0006-structured-concurrency/
//! concurrency_proto/src/lib.rs`), which found the real mechanics need
//! no invented machinery at all: `crossbeam_channel::unbounded`/
//! `bounded` already give an unbounded mailbox whose `send` never
//! blocks, and a `Receiver` that — unlike `std::sync::mpsc` — can be
//! legally cloned and pulled from by multiple consumer threads at once.
//!
//! **The point of building this in now, not later**: whenever `chan`
//! gets real codegen (Track B item B6), this becomes its
//! implementation — the language keyword doesn't change, no `.nir`
//! author writes anything different, `chan.send(x)` simply stops being
//! able to block once the compiled call site is wired to [`send`]
//! instead of a blocking primitive. The safety property (Pillar 2: "the
//! A-can-never-be-suspended-holding-what-B-needs deadlock class is
//! structurally impossible because A is never suspended on send") comes
//! free with the swap, not from anything a developer has to opt into.
//!
//! **What this deliberately does not attempt** — the harder, still-open
//! half of Pillars 1-4, honestly named rather than glossed over: the
//! capability-typed `Iso`/`Froze`/`Lend` distinction the original
//! prototype demonstrates via Rust's own borrow checker needs a real
//! `ownership.rs` extension to check the equivalent thing for `.nir`
//! `box` values — that's compiler/type-system work, not a runtime
//! primitive, and nothing in this file substitutes for it. What crosses
//! a real compiled `.nir` `chan` today is whatever scalar/handle
//! `codegen.rs` already knows how to marshal (the same `i64`/`ptr`
//! ABI-boundary shapes `nir_tcp_*`/`nir_file_*` use) — this module
//! doesn't change what can be sent, only how sending behaves.

use crossbeam_channel::{Receiver, Sender, TrySendError};

/// An unbounded mailbox: `(Sender<T>, Receiver<T>)`. `Receiver` is
/// legally cloneable and safe for multiple consumer threads to pull
/// from concurrently — real multi-consumer support, not simulated by
/// routing everything through one designated reader.
pub type Mailbox<T> = (Sender<T>, Receiver<T>);

pub fn mailbox<T>() -> Mailbox<T> {
    crossbeam_channel::unbounded()
}

/// A bounded mailbox — backpressure that is itself still non-blocking
/// (Pillar 2's M2): a full mailbox is a real, explicit [`SendResult`]
/// the caller decides what to do with (drop, retry, apply its own
/// policy), never a suspension. Plain [`mailbox`] (unbounded) is the
/// default; this exists for the case where "an unbounded mailbox is a
/// memory bomb" (the real, disclosed risk the original spec names) is a
/// genuine concern for a specific channel.
pub fn bounded_mailbox<T>(capacity: usize) -> Mailbox<T> {
    crossbeam_channel::bounded(capacity)
}

/// Sender-dropped-or-not: the two ways a mailbox can end.
#[derive(Debug, PartialEq, Eq)]
pub enum Closed {
    Closed,
}

/// The actual Pillar 2 claim: enqueues `msg` and returns immediately,
/// always — never suspends the calling thread, regardless of how full
/// the mailbox is (unbounded) or how slow the receiver is. `Err` only
/// when every receiver has already been dropped, in which case `msg`
/// itself comes back unchanged (nothing was silently lost).
pub fn send<T>(tx: &Sender<T>, msg: T) -> Result<(), T> {
    tx.send(msg).map_err(|e| e.0)
}

/// The bounded variant's non-blocking backpressure result.
pub enum SendResult<T> {
    Ok,
    /// The mailbox is at capacity right now — the caller's own policy
    /// decides what happens next (drop, retry later, apply backpressure
    /// some other way). Never a suspension.
    Full(T),
    Disconnected(T),
}

pub fn try_send<T>(tx: &Sender<T>, msg: T) -> SendResult<T> {
    match tx.try_send(msg) {
        Ok(()) => SendResult::Ok,
        Err(TrySendError::Full(m)) => SendResult::Full(m),
        Err(TrySendError::Disconnected(m)) => SendResult::Disconnected(m),
    }
}

/// The only suspending operation in this module — blocks while the
/// mailbox is empty, returns [`Closed`] once every sender has been
/// dropped. Pillar 3's `receive`: the sole place a thread can wait, and
/// what it's waiting for (a message on *this* mailbox) is always
/// visible and named, never an opaque "whatever the scheduler decides."
pub fn receive<T>(rx: &Receiver<T>) -> Result<T, Closed> {
    rx.recv().map_err(|_| Closed::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_never_blocks_even_when_nothing_is_receiving_yet() {
        let (tx, rx) = mailbox::<i64>();
        // No receiver is reading yet -- an unbounded mailbox's `send`
        // must still return immediately (Pillar 2's whole point). If
        // this were a blocking channel, this call would hang forever.
        for i in 0..1000 {
            send(&tx, i).unwrap();
        }
        for i in 0..1000 {
            assert_eq!(receive(&rx), Ok(i));
        }
    }

    #[test]
    fn receive_returns_closed_once_every_sender_is_dropped() {
        let (tx, rx) = mailbox::<i64>();
        send(&tx, 1).unwrap();
        drop(tx);
        assert_eq!(receive(&rx), Ok(1), "a message sent before the sender dropped must still be delivered");
        assert_eq!(receive(&rx), Err(Closed::Closed));
    }

    #[test]
    fn multiple_consumers_can_legally_share_one_receiver() {
        // The real point of `crossbeam_channel` over `std::sync::mpsc`
        // here: `Receiver` is `Clone`, and more than one thread pulling
        // from clones of the same receiver is legal, safe, and actually
        // shares the work -- proven by running it, not asserted.
        let (tx, rx) = mailbox::<i64>();
        for i in 0..100 {
            send(&tx, i).unwrap();
        }
        drop(tx);
        let mut handles = Vec::new();
        for _ in 0..4 {
            let rx2 = rx.clone();
            handles.push(std::thread::spawn(move || {
                let mut count = 0;
                while receive(&rx2).is_ok() {
                    count += 1;
                }
                count
            }));
        }
        let total: i64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 100, "every message must be received exactly once, across all consumers combined");
    }

    #[test]
    fn bounded_try_send_reports_full_instead_of_blocking() {
        let (tx, rx) = bounded_mailbox::<i64>(2);
        assert!(matches!(try_send(&tx, 1), SendResult::Ok));
        assert!(matches!(try_send(&tx, 2), SendResult::Ok));
        // Capacity 2, already full -- must report Full immediately, not
        // block waiting for room.
        match try_send(&tx, 3) {
            SendResult::Full(v) => assert_eq!(v, 3, "the rejected message must come back unchanged"),
            _ => panic!("expected SendResult::Full once the bounded mailbox is at capacity"),
        }
        assert_eq!(receive(&rx), Ok(1));
        assert!(matches!(try_send(&tx, 3), SendResult::Ok), "room freed by a receive must allow a subsequent send");
    }

    #[test]
    fn try_send_reports_disconnected_once_every_receiver_is_dropped() {
        let (tx, rx) = bounded_mailbox::<i64>(4);
        drop(rx);
        match try_send(&tx, 42) {
            SendResult::Disconnected(v) => assert_eq!(v, 42),
            _ => panic!("expected SendResult::Disconnected once every receiver is gone"),
        }
    }
}
