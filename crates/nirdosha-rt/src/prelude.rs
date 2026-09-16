//! The `.nir`-surface prelude (deferred-work entry #11, begun here).
//!
//! These are the Rust spellings of `.nir`'s unified-access constructs —
//! the vocabulary `nirdosha-v2` examples import so the *same source*
//! runs under plain cargo today and under the proprietary compiler
//! post-migration. The proprietary tier replaces the corpus-grade
//! bodies with the real runtime kernels behind these same names:
//! `sandbox` becomes a real OS process, `Chan`'s queue becomes the
//! runtime-kernels pool implementation, and so on. The *types* are the
//! contract.
//!
//! What rides here, in `.nir` vocabulary order:
//!
//! | `.nir` | here |
//! |---|---|
//! | `chan i64` + `send`/`recv` | [`Chan`] — unbounded MPMC, copyable handle, `send` never blocks |
//! | `froze i64` | [`Frozen`] — deeply-immutable shared value, copyable handle |
//! | `spawn f(args)` / `join h` | [`spawn`] / [`join`] over real OS threads |
//! | `sandbox` / `stop` | [`Sandbox`] — thread stand-in (process in the proprietary tier) |
//! | `file` + `open`/`send`/`recv`/`stop` | [`NirFile`] — the unified protocol over real files |
//! | `dec128` / `dec_from_str` / `dec_from_i64` | [`Dec128`] |
//! | `Money` / `USD()` | [`Money`] + [`Currency`] |
//! | `Measure` / `Kilogram()` | [`Measure`] + [`UnitCode`] |
//! | `Vector(f64, N)` / `dot` / `norm` | [`Vector`] |
//! | `Matrix(f64, R, C)` / `transpose` / `det` | [`Matrix`] |
//! | `txn_id` (saga idempotency key) | [`txn_id`] |

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

// ---------------------------------------------------------------------------
// chan — unbounded MPMC; handle is copyable; send never blocks.
// ---------------------------------------------------------------------------

/// An unbounded MPMC queue. Handles are freely copyable (each copy is a
/// new handle to the same queue); `send` never blocks; `recv` blocks
/// until a value is available.
#[derive(Clone)]
pub struct Chan<T> {
    shared: Arc<ChanShared<T>>,
}

struct ChanShared<T> {
    queue: Mutex<std::collections::VecDeque<T>>,
    ready: Condvar,
}

impl<T> Chan<T> {
    /// `let c: chan i64 = chan`
    pub fn new() -> Self {
        Self {
            shared: Arc::new(ChanShared {
                queue: Mutex::new(std::collections::VecDeque::new()),
                ready: Condvar::new(),
            }),
        }
    }

    /// Never blocks: an unbounded queue always accepts.
    pub fn send(&self, value: T) {
        self.shared.queue.lock().unwrap().push_back(value);
        self.shared.ready.notify_one();
    }

    /// Blocks until a value is available.
    pub fn recv(&self) -> T {
        let mut queue = self.shared.queue.lock().unwrap();
        loop {
            if let Some(value) = queue.pop_front() {
                return value;
            }
            queue = self.shared.ready.wait(queue).unwrap();
        }
    }
}

impl<T> Default for Chan<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// froze — deeply-immutable shared value, copyable handle.
// ---------------------------------------------------------------------------

/// `.nir`'s `froze`: heap-allocated like `box`, deliberately *not*
/// affine — safe to share with any number of readers, concurrently.
#[derive(Clone)]
pub struct Frozen<T> {
    inner: Arc<T>,
}

impl<T> Frozen<T> {
    /// `let f: froze i64 = froze 21`
    pub fn freeze(value: T) -> Self {
        Self { inner: Arc::new(value) }
    }

    /// Read through the handle (`*f` in `.nir`).
    pub fn get(&self) -> &T {
        &self.inner
    }
}

impl<T: fmt::Display> fmt::Display for Frozen<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// spawn / join / sandbox — concurrency surface.
// ---------------------------------------------------------------------------

/// `.nir`'s `spawn f(args)`: run on a real OS thread, return the
/// handle. The closure form is the Rust spelling of the argument list.
pub fn spawn<T, F>(f: F) -> std::thread::JoinHandle<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    std::thread::spawn(f)
}

/// `.nir`'s `join h`: block for the result, consuming the handle.
pub fn join<T>(handle: std::thread::JoinHandle<T>) -> T {
    handle.join().expect("spawned thread panicked")
}

/// `.nir`'s `sandbox` handle. Corpus-grade body: the closure runs on a
/// detached OS thread (the proprietary tier runs a real OS process and
/// `stop` really kills it — that tier is where process isolation lives).
pub struct Sandbox {
    _worker: std::thread::JoinHandle<()>,
}

/// `sandbox background_work()` → `sandbox(move || background_work())`
pub fn sandbox<F>(f: F) -> Sandbox
where
    F: FnOnce() + Send + 'static,
{
    Sandbox { _worker: std::thread::spawn(f) }
}

/// `stop s` — yields the exit code. The corpus-grade body reports
/// "still running" (matching the examples' expected `-1` for a
/// never-ending worker) without waiting: the detached thread cannot be
/// killed in Rust, and dies with the process.
impl Stoppable for Sandbox {
    fn stop(self) -> i64 {
        if self._worker.is_finished() {
            0
        } else {
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// chan — free-fn spellings matching `.nir`'s `send(c, v)` / `recv(c)`.
// ---------------------------------------------------------------------------

/// `send(c, 14)` — never blocks.
pub fn send<T>(c: &Chan<T>, value: T) {
    c.send(value);
}

/// `recv(c)` — blocks until a value is available.
pub fn recv<T>(c: &Chan<T>) -> T {
    c.recv()
}

// ---------------------------------------------------------------------------
// file — the unified send/recv/stop protocol over a real file.
// ---------------------------------------------------------------------------

/// `.nir`'s `file` handle: affine, opened, used, `stop`ped in one
/// function. `send` writes a line; `recv` reads the content back.
pub struct NirFile {
    path: String,
    inner: std::fs::File,
}

/// `open(path, mode)` — `"r"`, `"w"`, `"a"`, like the examples.
pub fn open<P: AsRef<Path>>(path: P, mode: &str) -> NirFile {
    let file = match mode {
        "r" => std::fs::File::options().read(true).open(&path),
        "a" => std::fs::OpenOptions::new().append(true).create(true).open(&path),
        _ => std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path),
    }
    .expect("nirdosha-rt prelude: file open failed");
    NirFile { path: path.as_ref().to_string_lossy().into_owned(), inner: file }
}

impl NirFile {
    /// The unified protocol's `send` over a file: write a line.
    pub fn send(&mut self, line: &str) {
        use std::io::Write;
        writeln!(self.inner, "{line}").expect("file write failed");
    }

    /// The unified protocol's `recv` over a file: read the content back.
    pub fn recv(&self) -> String {
        std::fs::read_to_string(&self.path)
            .expect("file read failed")
            .trim_end_matches('\n')
            .to_string()
    }
}

/// The unified protocol's `stop`: one name over every affine handle
/// (file, sandbox, db, mq) — dispatch by type via [`Stoppable`].
pub trait Stoppable {
    /// Consume the handle; yield its exit status (0 = clean).
    fn stop(self) -> i64;
}

/// `stop h` — the single spelling for every handle type.
pub fn stop<T: Stoppable>(handle: T) -> i64 {
    handle.stop()
}

impl Stoppable for NirFile {
    fn stop(self) -> i64 {
        drop(self);
        0
    }
}

// ---------------------------------------------------------------------------
// dec128 / Money / Measure — the decimal prelude.
// ---------------------------------------------------------------------------

/// `.nir`'s `dec128`: a scaled decimal (i64 mantissa + scale).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Dec128 {
    pub mantissa: i64,
    pub scale: u32,
}

/// `dec_from_str("19.99")` — fallible, as in `.nir`: malformed input
/// is a real `Result`, not a silent 0.
pub fn dec_from_str(s: &str) -> Result<Dec128, String> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (mantissa, scale) = match rest.split_once('.') {
        None => (digits(rest)?, 0),
        Some((whole, frac)) => {
            let frac_digits = digits(frac)?;
            (
                digits(whole)?.wrapping_mul(10i64.pow(frac.len() as u32)) + frac_digits,
                frac.len() as u32,
            )
        }
    };
    Ok(Dec128 { mantissa: if neg { -mantissa } else { mantissa }, scale })
}

fn digits(s: &str) -> Result<i64, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("not a decimal: {s:?}"));
    }
    s.parse::<i64>().map_err(|e| e.to_string())
}

/// `dec_from_i64(value, scale)` — infallible.
pub fn dec_from_i64(mantissa: i64, scale: u32) -> Dec128 {
    Dec128 { mantissa, scale }
}

impl Dec128 {
    pub fn as_f64(&self) -> f64 {
        self.mantissa as f64 / 10f64.powi(self.scale as i32)
    }
}

impl fmt::Display for Dec128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.mantissa.to_string();
        let scale = self.scale as usize;
        if scale == 0 {
            return write!(f, "{s}");
        }
        let (sign, digits) = match s.strip_prefix('-') {
            Some(d) => ("-", d.to_string()),
            None => ("", s),
        };
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let split = padded.len() - scale;
        write!(f, "{sign}{}.{}", &padded[..split], &padded[split..])
    }
}

/// `.nir`'s `CurrencyCode`: a closed, exhaustively-matchable enum.
/// Displays with the original `USD()` spelling (the examples' shape).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    USD,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Currency::USD => write!(f, "USD()"),
        }
    }
}

/// `.nir`'s `Money`: type-checked amount + closed currency.
#[derive(Clone, Copy)]
pub struct Money {
    pub amount: Dec128,
    pub currency: Currency,
}

/// `.nir`'s `UnitCode`. (Field is `unit_code`, never `unit` — `unit` is
/// the type keyword in `.nir`.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnitCode {
    Kilogram,
}

impl fmt::Display for UnitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitCode::Kilogram => write!(f, "Kilogram()"),
        }
    }
}

/// `.nir`'s `Measure`: physical quantity.
#[derive(Clone, Copy)]
pub struct Measure {
    pub value: Dec128,
    pub unit_code: UnitCode,
}

// ---------------------------------------------------------------------------
// Vector / Matrix — the linear-algebra surface.
// ---------------------------------------------------------------------------

/// `.nir`'s `Vector(f64, N)`.
#[derive(Clone, Copy)]
pub struct Vector<const N: usize>(pub [f64; N]);

impl<const N: usize> std::ops::Add for Vector<N> {
    type Output = Vector<N>;
    fn add(self, rhs: Self) -> Self {
        let mut out = self.0;
        for i in 0..N {
            out[i] += rhs.0[i];
        }
        Vector(out)
    }
}

impl<const N: usize> fmt::Display for Vector<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|x| fmt_num(*x)).collect();
        write!(f, "[{}]", parts.join(", "))
    }
}

/// `dot(v, w)`
pub fn dot<const N: usize>(a: &Vector<N>, b: &Vector<N>) -> f64 {
    a.0.iter().zip(b.0.iter()).map(|(x, y)| x * y).sum()
}

/// `norm(v)`
pub fn norm<const N: usize>(v: &Vector<N>) -> f64 {
    v.0.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// `.nir`'s `Matrix(f64, R, C)`.
#[derive(Clone, Copy)]
pub struct Matrix<const R: usize, const C: usize>(pub [[f64; C]; R]);

impl<const R: usize, const C: usize> fmt::Display for Matrix<R, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rows: Vec<String> = self
            .0
            .iter()
            .map(|row| {
                let cells: Vec<String> = row.iter().map(|x| fmt_num(*x)).collect();
                format!("[{}]", cells.join(", "))
            })
            .collect();
        write!(f, "[{}]", rows.join(", "))
    }
}

/// `transpose(m)` — square matrices (the examples' shape).
pub fn transpose<const N: usize>(m: &Matrix<N, N>) -> Matrix<N, N> {
    let mut out = m.0;
    for r in 0..N {
        for c in 0..N {
            out[c][r] = m.0[r][c];
        }
    }
    Matrix(out)
}

/// `det(m)` — 2×2 (the examples' shape).
pub fn det(m: &Matrix<2, 2>) -> f64 {
    m.0[0][0] * m.0[1][1] - m.0[0][1] * m.0[1][0]
}

fn fmt_num(x: f64) -> String {
    format!("{x}")
}

// ---------------------------------------------------------------------------
// txn_id — the saga idempotency key (transact's implicit binding).
// ---------------------------------------------------------------------------

static TXN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// `transact`'s implicit `txn_id`: an unpredictable, always-unique key
/// generated before `network` runs (a crash-replay-safe dedupe handle).
pub fn txn_id() -> String {
    let n = TXN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let junk = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("txn-{junk:x}-{n:x}")
}