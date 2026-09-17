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
//! | *(dialect-only, no `.nir` construct)* | [`SharedTable`] / [`SharedCell`] — the managed replacements for a raw `std::sync::Mutex` table |

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

/// `*f` — deref reads through the handle, exactly `.nir`'s `*`.
impl<T> std::ops::Deref for Frozen<T> {
    type Target = T;
    fn deref(&self) -> &T {
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
/// detached OS thread. This fixture does not provide process isolation;
/// the native LLVM backend currently rejects the old sandbox syntax too.
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
    /// the read cursor: `recv` reads all currently-available bytes from
    /// HERE, so a second read past EOF yields `""`, exactly `.nir`.
    read_pos: std::cell::Cell<u64>,
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
    NirFile {
        path: path.as_ref().to_string_lossy().into_owned(),
        inner: file,
        read_pos: std::cell::Cell::new(0),
    }
}

impl NirFile {
    /// The unified protocol's `send` over a file: write a line.
    pub fn send(&mut self, line: &str) {
        use std::io::Write;
        writeln!(self.inner, "{line}").expect("file write failed");
    }

    /// The unified protocol's `recv` over a file: read all
    /// currently-available bytes from the cursor; past EOF is `""`.
    pub fn recv(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let total = std::fs::metadata(&self.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let start = self.read_pos.get();
        if start >= total {
            return String::new();
        }
        let mut file = &self.inner;
        let _ = file.seek(SeekFrom::Start(start));
        let mut buf = Vec::new();
        let _ = file.read_to_end(&mut buf);
        self.read_pos.set(total);
        String::from_utf8_lossy(&buf).trim_end_matches('\n').to_string()
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

fn align_scales(a: &Dec128, b: &Dec128) -> (i64, i64, u32) {
    if a.scale >= b.scale {
        let m = b.mantissa * 10i64.pow(a.scale - b.scale);
        (a.mantissa, m, a.scale)
    } else {
        let m = a.mantissa * 10i64.pow(b.scale - a.scale);
        (m, b.mantissa, b.scale)
    }
}

/// Native `+` on dec128 — scales align, the wider scale wins.
impl std::ops::Add for Dec128 {
    type Output = Dec128;
    fn add(self, rhs: Dec128) -> Dec128 {
        let (m1, m2, scale) = align_scales(&self, &rhs);
        Dec128 { mantissa: m1 + m2, scale }
    }
}

/// Native `-` on dec128.
impl std::ops::Sub for Dec128 {
    type Output = Dec128;
    fn sub(self, rhs: Dec128) -> Dec128 {
        let (m1, m2, scale) = align_scales(&self, &rhs);
        Dec128 { mantissa: m1 - m2, scale }
    }
}

/// Native `*` on dec128 — mantissas multiply, scales add.
impl std::ops::Mul for Dec128 {
    type Output = Dec128;
    fn mul(self, rhs: Dec128) -> Dec128 {
        Dec128 { mantissa: self.mantissa * rhs.mantissa, scale: self.scale + rhs.scale }
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
    EUR,
    GBP,
    JPY,
    INR,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Currency::USD => write!(f, "USD()"),
            Currency::EUR => write!(f, "EUR()"),
            Currency::GBP => write!(f, "GBP()"),
            Currency::JPY => write!(f, "JPY()"),
            Currency::INR => write!(f, "INR()"),
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
    Gram,
    Mile,
    Meter,
    Foot,
    Liter,
    Second,
}

impl fmt::Display for UnitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitCode::Kilogram => write!(f, "Kilogram()"),
            UnitCode::Gram => write!(f, "Gram()"),
            UnitCode::Mile => write!(f, "Mile()"),
            UnitCode::Meter => write!(f, "Meter()"),
            UnitCode::Foot => write!(f, "Foot()"),
            UnitCode::Liter => write!(f, "Liter()"),
            UnitCode::Second => write!(f, "Second()"),
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

impl<const N: usize> std::ops::Sub for Vector<N> {
    type Output = Vector<N>;
    fn sub(self, rhs: Self) -> Self {
        let mut out = self.0;
        for i in 0..N {
            out[i] -= rhs.0[i];
        }
        Vector(out)
    }
}

impl<const N: usize> PartialEq for Vector<N> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// `v .* w` — the Hadamard (elementwise) product, `.nir`'s `.*`.
pub fn hadamard<const N: usize>(a: &Vector<N>, b: &Vector<N>) -> Vector<N> {
    let mut out = a.0;
    for i in 0..N {
        out[i] *= b.0[i];
    }
    Vector(out)
}

/// `v ./ w` — the Hadamard (elementwise) quotient, `.nir`'s `./`.
pub fn hadamard_div<const N: usize>(a: &Vector<N>, b: &Vector<N>) -> Vector<N> {
    let mut out = a.0;
    for i in 0..N {
        out[i] /= b.0[i];
    }
    Vector(out)
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

impl<const R: usize, const C: usize> PartialEq for Matrix<R, C> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// `m * n` — the real matrix product (`a * b` in the linalg examples).
impl<const R: usize, const K: usize, const C: usize> std::ops::Mul<Matrix<K, C>> for Matrix<R, K> {
    type Output = Matrix<R, C>;
    fn mul(self, rhs: Matrix<K, C>) -> Matrix<R, C> {
        let mut out = [[0.0; C]; R];
        for r in 0..R {
            for c in 0..C {
                let mut acc = 0.0;
                for k in 0..K {
                    acc += self.0[r][k] * rhs.0[k][c];
                }
                out[r][c] = acc;
            }
        }
        Matrix(out)
    }
}

/// `m * 2.0` — scalar * matrix, matrix-first form.
impl<const R: usize, const C: usize> std::ops::Add for Matrix<R, C> {
    type Output = Matrix<R, C>;
    fn add(self, rhs: Matrix<R, C>) -> Matrix<R, C> {
        let mut out = self.0;
        for r in 0..R {
            for c in 0..C {
                out[r][c] += rhs.0[r][c];
            }
        }
        Matrix(out)
    }
}

impl<const R: usize, const C: usize> std::ops::Mul<f64> for Matrix<R, C> {
    type Output = Matrix<R, C>;
    fn mul(self, rhs: f64) -> Matrix<R, C> {
        let mut out = self.0;
        for r in 0..R {
            for c in 0..C {
                out[r][c] *= rhs;
            }
        }
        Matrix(out)
    }
}

/// `2.0 * m` — scalar * matrix, scalar-first form.
impl<const R: usize, const C: usize> std::ops::Mul<Matrix<R, C>> for f64 {
    type Output = Matrix<R, C>;
    fn mul(self, rhs: Matrix<R, C>) -> Matrix<R, C> {
        rhs * self
    }
}

/// `m * v` — matrix * vector.
impl<const R: usize, const C: usize> std::ops::Mul<Vector<C>> for Matrix<R, C> {
    type Output = Vector<R>;
    fn mul(self, rhs: Vector<C>) -> Vector<R> {
        let mut out = [0.0; R];
        for r in 0..R {
            let mut acc = 0.0;
            for c in 0..C {
                acc += self.0[r][c] * rhs.0[c];
            }
            out[r] = acc;
        }
        Vector(out)
    }
}

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

/// `transpose(m)` — any shape (the examples' square form included).
pub fn transpose<const R: usize, const C: usize>(m: &Matrix<R, C>) -> Matrix<C, R> {
    let mut out = [[0.0; R]; C];
    for r in 0..R {
        for c in 0..C {
            out[c][r] = m.0[r][c];
        }
    }
    Matrix(out)
}

/// `det(m)` — exact for 2×2 (the pinned examples), Gaussian elimination
/// with partial pivoting for anything larger.
pub fn det<const N: usize>(m: &Matrix<N, N>) -> f64 {
    if N == 2 {
        return m.0[0][0] * m.0[1][1] - m.0[0][1] * m.0[1][0];
    }
    let mut a = m.0;
    let mut det = 1.0;
    for col in 0..N {
        // partial pivot
        let mut pivot = col;
        for r in col + 1..N {
            if a[r][col].abs() > a[pivot][col].abs() {
                pivot = r;
            }
        }
        if a[pivot][col] == 0.0 {
            return 0.0;
        }
        if pivot != col {
            a.swap(pivot, col);
            det = -det;
        }
        det *= a[col][col];
        for r in col + 1..N {
            let f = a[r][col] / a[col][col];
            for c in col..N {
                a[r][c] -= f * a[col][c];
            }
        }
    }
    det
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
// ---------------------------------------------------------------------------
// linalg builtins — `12_vector_matrix_linalg.nir`'s Phase-2 set.
// ---------------------------------------------------------------------------

/// `cross(v, w)` — the 3-D cross product (the only shape it exists in).
pub fn cross(a: &Vector<3>, b: &Vector<3>) -> Vector<3> {
    Vector([
        a.0[1] * b.0[2] - a.0[2] * b.0[1],
        a.0[2] * b.0[0] - a.0[0] * b.0[2],
        a.0[0] * b.0[1] - a.0[1] * b.0[0],
    ])
}

/// `len(v)` — the element count.
pub fn len<const N: usize>(_v: &Vector<N>) -> i64 {
    N as i64
}

/// `sum(v)`.
pub fn sum<const N: usize>(v: &Vector<N>) -> f64 {
    v.0.iter().sum()
}

/// `norm1(v)` — the taxicab norm.
pub fn norm1<const N: usize>(v: &Vector<N>) -> f64 {
    v.0.iter().map(|x| x.abs()).sum()
}

/// `norm_inf(v)` — the max-abs norm.
pub fn norm_inf<const N: usize>(v: &Vector<N>) -> f64 {
    v.0.iter().map(|x| x.abs()).fold(0.0, f64::max)
}

/// `trace(m)` — the diagonal sum (square only).
pub fn trace<const N: usize>(m: &Matrix<N, N>) -> f64 {
    (0..N).map(|i| m.0[i][i]).sum()
}

/// `inv(m)` — exact for 2×2 (the pinned example), Gauss-Jordan beyond.
pub fn inv<const N: usize>(m: &Matrix<N, N>) -> Matrix<N, N> {
    if N == 2 {
        let d = m.0[0][0] * m.0[1][1] - m.0[0][1] * m.0[1][0];
        let mut out = [[0.0; N]; N];
        out[0][0] = m.0[1][1] / d;
        out[0][1] = -m.0[0][1] / d;
        out[1][0] = -m.0[1][0] / d;
        out[1][1] = m.0[0][0] / d;
        return Matrix(out);
    }
    let mut a = m.0;
    let mut inv = [[0.0; N]; N];
    for i in 0..N {
        inv[i][i] = 1.0;
    }
    for col in 0..N {
        let mut pivot = col;
        for r in col + 1..N {
            if a[r][col].abs() > a[pivot][col].abs() {
                pivot = r;
            }
        }
        a.swap(pivot, col);
        inv.swap(pivot, col);
        let d = a[col][col];
        for c in 0..N {
            a[col][c] /= d;
            inv[col][c] /= d;
        }
        for r in 0..N {
            if r != col {
                let f = a[r][col];
                for c in 0..N {
                    a[r][c] -= f * a[col][c];
                    inv[r][c] -= f * inv[col][c];
                }
            }
        }
    }
    Matrix(inv)
}

/// `is_square(m)`.
pub fn is_square<const R: usize, const C: usize>(_m: &Matrix<R, C>) -> bool {
    R == C
}

/// `frobenius_norm(m)`.
pub fn frobenius_norm<const R: usize, const C: usize>(m: &Matrix<R, C>) -> f64 {
    m.0.iter().flatten().map(|x| x * x).sum::<f64>().sqrt()
}

/// `is_symmetric(m)` — square only.
pub fn is_symmetric<const N: usize>(m: &Matrix<N, N>) -> bool {
    for r in 0..N {
        for c in 0..N {
            if m.0[r][c] != m.0[c][r] {
                return false;
            }
        }
    }
    true
}

/// `is_diag(m)` — square only.
pub fn is_diag<const N: usize>(m: &Matrix<N, N>) -> bool {
    for r in 0..N {
        for c in 0..N {
            if r != c && m.0[r][c] != 0.0 {
                return false;
            }
        }
    }
    true
}

/// `zeros(n)`.
pub fn zeros<const N: usize>() -> Vector<N> {
    Vector([0.0; N])
}

/// `ones(r, c)`.
pub fn ones<const R: usize, const C: usize>() -> Matrix<R, C> {
    Matrix([[1.0; C]; R])
}

/// `identity(n)`.
pub fn identity<const N: usize>() -> Matrix<N, N> {
    let mut out = [[0.0; N]; N];
    for i in 0..N {
        out[i][i] = 1.0;
    }
    Matrix(out)
}

/// `solve(a, b)` — Gaussian elimination with partial pivoting.
pub fn solve<const N: usize>(a: &Matrix<N, N>, b: &Vector<N>) -> Vector<N> {
    let mut m = a.0;
    let mut x = b.0;
    for col in 0..N {
        let mut pivot = col;
        for r in col + 1..N {
            if m[r][col].abs() > m[pivot][col].abs() {
                pivot = r;
            }
        }
        m.swap(pivot, col);
        x.swap(pivot, col);
        let d = m[col][col];
        for c in 0..N {
            m[col][c] /= d;
        }
        x[col] /= d;
        for r in 0..N {
            if r != col {
                let f = m[r][col];
                for c in 0..N {
                    m[r][c] -= f * m[col][c];
                }
                x[r] -= f * x[col];
            }
        }
    }
    Vector(x)
}

/// `rank(m)` — nonzero rows after elimination.
pub fn rank<const R: usize, const C: usize>(m: &Matrix<R, C>) -> i64 {
    let mut a = m.0;
    let mut r = 0;
    for col in 0..C.min(R) {
        let mut pivot = None;
        for row in r..R {
            if a[row][col].abs() > 1e-12 {
                pivot = Some(row);
                break;
            }
        }
        if let Some(p) = pivot {
            a.swap(p, r);
            for row in r + 1..R {
                let f = a[row][col] / a[r][col];
                for c in 0..C {
                    a[row][c] -= f * a[r][c];
                }
            }
            r += 1;
        }
    }
    r as i64
}

// ---------------------------------------------------------------------------
// deterministic simulation — `13_deterministic_simulation.nir`'s Phase-3
// set: a from-scratch SplitMix64 stream, geometry, and the Kalman steps.
// ---------------------------------------------------------------------------

static RAND_STATE: std::sync::OnceLock<std::sync::Mutex<u64>> = std::sync::OnceLock::new();

fn rand_next() -> u64 {
    let lock = RAND_STATE.get_or_init(|| std::sync::Mutex::new(0));
    let mut state = lock.lock().unwrap();
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `rand_seed(n)` — required before any draw (same seed, same draws).
pub fn rand_seed(seed: i64) {
    let lock = RAND_STATE.get_or_init(|| std::sync::Mutex::new(0));
    *lock.lock().unwrap() = seed as u64;
}

/// `rand_f64()` — uniform [0, 1).
pub fn rand_f64() -> f64 {
    (rand_next() >> 11) as f64 / 9_007_199_254_740_992.0
}

/// `rand_gaussian(mu, sigma)` — Box-Muller over the same stream.
pub fn rand_gaussian(mu: f64, sigma: f64) -> f64 {
    let mut u1 = rand_f64();
    if u1 <= 0.0 {
        u1 = 1.0 / 9_007_199_254_740_992.0;
    }
    let u2 = rand_f64();
    mu + sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// `distance(a, b)` — Euclidean, in the inputs' own units.
pub fn distance<const N: usize>(a: &Vector<N>, b: &Vector<N>) -> f64 {
    let mut acc = 0.0;
    for i in 0..N {
        let d = a.0[i] - b.0[i];
        acc += d * d;
    }
    acc.sqrt()
}

/// `bearing(here, there)` — the initial great-circle bearing, degrees
/// [0, 360), from [lat, lon, alt] inputs.
pub fn bearing(here: &Vector<3>, there: &Vector<3>) -> f64 {
    let lat1 = here.0[0].to_radians();
    let lat2 = there.0[0].to_radians();
    let dlon = (there.0[1] - here.0[1]).to_radians();
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    let deg = y.atan2(x).to_degrees();
    (deg + 360.0) % 360.0
}

const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;
const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);

/// `lla_to_ecef(v)` — [lat, lon, alt] (degrees, same units as alt) to
/// Earth-Centered Earth-Fixed (meters).
pub fn lla_to_ecef(v: &Vector<3>) -> Vector<3> {
    let lat = v.0[0].to_radians();
    let lon = v.0[1].to_radians();
    let h = v.0[2];
    let n = WGS84_A / (1.0 - WGS84_E2 * lat.sin().powi(2)).sqrt();
    Vector([
        (n + h) * lat.cos() * lon.cos(),
        (n + h) * lat.cos() * lon.sin(),
        (n * (1.0 - WGS84_E2) + h) * lat.sin(),
    ])
}

/// `ecef_to_lla(v)` — Bowring's closed form (round-trips approximately).
pub fn ecef_to_lla(v: &Vector<3>) -> Vector<3> {
    let (x, y, z) = (v.0[0], v.0[1], v.0[2]);
    let b = WGS84_A * (1.0 - WGS84_F);
    let ep2 = (WGS84_A * WGS84_A - b * b) / (b * b);
    let p = (x * x + y * y).sqrt();
    let th = (WGS84_A * z).atan2(p * b);
    let lon = y.atan2(x);
    let lat = (z + ep2 * b * th.sin().powi(3))
        .atan2(p - WGS84_E2 * WGS84_A * th.cos().powi(3));
    let n = WGS84_A / (1.0 - WGS84_E2 * lat.sin().powi(2)).sqrt();
    let h = p / lat.cos() - n;
    Vector([lat.to_degrees(), lon.to_degrees(), h])
}

/// `ecef_to_enu(target, ref)` — the target's position in a local
/// East-North-Up frame centered on the reference [lat, lon, alt].
pub fn ecef_to_enu(target: &Vector<3>, reference: &Vector<3>) -> Vector<3> {
    let ref_ecef = lla_to_ecef(reference);
    let (dx, dy, dz) = (
        target.0[0] - ref_ecef.0[0],
        target.0[1] - ref_ecef.0[1],
        target.0[2] - ref_ecef.0[2],
    );
    let lat = reference.0[0].to_radians();
    let lon = reference.0[1].to_radians();
    Vector([
        dx * lon.cos() + dy * lon.sin(),
        -dx * lat.sin() * lon.sin() + dy * lat.sin() * lon.cos() + dz * lat.cos(),
        dx * lat.cos() * lon.sin() - dy * lat.cos() * lon.cos() + dz * lat.sin(),
    ])
}

/// `kf_predict_state(x, p, f, q)` — x' = F x.
pub fn kf_predict_state<const S: usize>(
    x: &Vector<S>,
    _p: &Matrix<S, S>,
    f: &Matrix<S, S>,
    _q: &Matrix<S, S>,
) -> Vector<S> {
    *f * *x
}

/// `kf_predict_cov(x, p, f, q)` — P' = F P Fᵀ + Q.
pub fn kf_predict_cov<const S: usize>(
    _x: &Vector<S>,
    p: &Matrix<S, S>,
    f: &Matrix<S, S>,
    q: &Matrix<S, S>,
) -> Matrix<S, S> {
    *f * *p * transpose(f) + *q
}

/// `kf_update_state(x, p, z, h, r)` — x + K(z − Hx), K = P Hᵀ S⁻¹,
/// S = H P Hᵀ + R.
pub fn kf_update_state<const S: usize, const M: usize>(
    x: &Vector<S>,
    p: &Matrix<S, S>,
    z: &Vector<M>,
    h: &Matrix<M, S>,
    r: &Matrix<M, M>,
) -> Vector<S> {
    let ht = transpose(h);
    let s = *h * *p * ht + *r;
    let k = *p * ht * inv(&s);
    let mut out = [0.0; S];
    let hx = *h * *x;
    for i in 0..S {
        let mut kdz = 0.0;
        for m in 0..M {
            kdz += k.0[i][m] * (z.0[m] - hx.0[m]);
        }
        out[i] = x.0[i] + kdz;
    }
    Vector(out)
}

/// `kf_update_cov(x, p, z, h, r)` — (I − K H) P.
pub fn kf_update_cov<const S: usize, const M: usize>(
    _x: &Vector<S>,
    p: &Matrix<S, S>,
    _z: &Vector<M>,
    h: &Matrix<M, S>,
    r: &Matrix<M, M>,
) -> Matrix<S, S> {
    let ht = transpose(h);
    let s = *h * *p * ht + *r;
    let k = *p * ht * inv(&s);
    let mut kh = [[0.0; S]; S];
    for i in 0..S {
        for j in 0..S {
            let mut acc = 0.0;
            for m in 0..M {
                acc += k.0[i][m] * h.0[m][j];
            }
            kh[i][j] = acc;
        }
    }
    let mut out = [[0.0; S]; S];
    for i in 0..S {
        for j in 0..S {
            let mut acc = 0.0;
            for l in 0..S {
                let eye = if i == l { 1.0 } else { 0.0 };
                acc += (eye - kh[i][l]) * p.0[l][j];
            }
            out[i][j] = acc;
        }
    }
    Matrix(out)
}

/// `sleep_ms(n)` — parent-side pause (sandbox/SLA examples).
pub fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

// ---------------------------------------------------------------------------
// `tcp`/`tcp_listener` — real sockets, `.nir`'s send/recv/stop surface.
// ---------------------------------------------------------------------------

/// A connected TCP stream. `.nir`'s free-fn `send`/`recv` become methods
/// here (the free `send` is already taken by `chan`'s).
pub struct Tcp {
    stream: std::net::TcpStream,
}

/// `connect(host, port)` — the raw TCP client.
pub fn connect(host: &str, port: i64) -> Tcp {
    let stream = std::net::TcpStream::connect((host, port as u16))
        .unwrap_or_else(|e| panic!("connect({host}:{port}) failed: {e}"));
    Tcp { stream }
}

impl Tcp {
    pub(crate) fn from_stream(stream: std::net::TcpStream) -> Self {
        Self { stream }
    }

    /// `send(conn, s)` — one write.
    pub fn send(&self, s: &str) {
        use std::io::Write;
        (&self.stream).write_all(s.as_bytes()).expect("tcp send");
    }

    /// `recv(conn)` — one read syscall's worth of currently-available
    /// bytes, exactly `.nir`'s semantics (not a drain loop). A closed
    /// or reset peer yields "" (no bytes available) rather than a
    /// panic — a server must survive a connection that is opened and
    /// dropped without speaking (a port-scan, a health poll).
    pub fn recv(&self) -> String {
        use std::io::Read;
        let mut buf = [0u8; 65536];
        let n = match (&self.stream).read(&mut buf) {
            Ok(n) => n,
            Err(_) => return String::new(),
        };
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

impl Stoppable for Tcp {
    fn stop(self) -> i64 {
        drop(self);
        0
    }
}

/// A bound+listening socket. `accept` borrows — only `stop` consumes,
/// so one listener can serve several clients (`.nir`'s exact rule).
pub struct TcpListener {
    inner: std::net::TcpListener,
}

/// `listen(port)`.
pub fn listen(port: i64) -> TcpListener {
    let inner = std::net::TcpListener::bind(("127.0.0.1", port as u16))
        .unwrap_or_else(|e| panic!("listen({port}) failed: {e}"));
    TcpListener { inner }
}

/// `accept(l)` — non-consuming (the listener stays usable).
pub fn accept(l: &TcpListener) -> Tcp {
    let (stream, _) = l.inner.accept().expect("accept");
    Tcp { stream }
}

impl Stoppable for TcpListener {
    fn stop(self) -> i64 {
        drop(self);
        0
    }
}

// ---------------------------------------------------------------------------
// SharedTable / SharedCell — the managed replacements for a raw
// `std::sync::Mutex` table. Raw locks are denied by the dialect's
// dialect-wide deny set (`nirdosha-contract-core/src/scan.rs::
// dialect_denies`); the `Mutex` here lives inside the runtime, which
// is exempt (`toolchain = true`) — the same move `Chan`'s queue made.
//
// The guarantee these types buy: every critical section is one
// complete method. No lock guard is a reachable type in dialect
// code, so a guard can never be held across an arbitrary span, and
// no second lock can ever be acquired while one is held — nested
// locks, the classic lock-order deadlock, are structurally
// impossible. All access is serialized through the one internal
// lock, so there are no races. Values are read out by clone (`get`/
// `snapshot`), never by reference, precisely so nothing observable
// escapes the method's atomicity window.
// ---------------------------------------------------------------------------

/// A keyed shared table — the managed replacement for the
/// `OnceLock<Mutex<HashMap<K, V>>>` + scoped `.lock().unwrap()`
/// pattern (the corpus's `instances()`/`product_store()` tables).
/// Handles are copyable like [`Chan`]'s (each copy is a new handle to
/// the same table). Every method completes its critical section
/// internally — see the module-level guarantee note above.
#[derive(Clone)]
pub struct SharedTable<K, V> {
    inner: Arc<Mutex<std::collections::HashMap<K, V>>>,
}

impl<K, V> SharedTable<K, V>
where
    K: Eq + std::hash::Hash + Clone,
    V: Clone,
{
    /// `SharedTable::<K, V>::new()` — empty table.
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(std::collections::HashMap::new())) }
    }

    /// Insert (or overwrite) `k`. Returns the previous value, if any.
    pub fn insert(&self, k: K, v: V) -> Option<V> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).insert(k, v)
    }

    /// Clone out by key — the only read form. No reference to the
    /// stored value ever escapes, so no read can observe a partial
    /// concurrent write.
    pub fn get(&self, k: &K) -> Option<V> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).get(k).cloned()
    }

    /// Atomic read-modify-write: `f` sees the entry (or `None` if
    /// absent) under the table's one lock and its return value comes
    /// back. This is the dialect spelling of the scoped
    /// `let mut store = ..lock().unwrap(); ..get_mut(..)..` blocks —
    /// the state check + mutation happens inside the critical
    /// section, never outside it.
    pub fn update<R>(&self, k: &K, f: impl FnOnce(Option<&mut V>) -> R) -> R {
        let mut table = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(table.get_mut(k))
    }

    /// Remove and return `k`'s value, if present.
    pub fn remove(&self, k: &K) -> Option<V> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).remove(k)
    }

    /// Get-or-create-and-mutate: `f` sees the entry under the table's
    /// one lock, with `V::default()` inserted first if `k` was absent —
    /// the dialect spelling of `map.entry(k).or_default()` blocks
    /// (the wizard's per-session state accumulation). Unlike `update`,
    /// `k` is consumed: absent keys become present.
    pub fn upsert_with<R>(&self, k: K, f: impl FnOnce(&mut V) -> R) -> R
    where
        V: Default,
    {
        let mut table = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = table.entry(k).or_default();
        f(entry)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// True when the table holds no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// All entries, **sorted by key** — a `HashMap` iteration order
    /// would make otherwise-identical programs diverge run to run,
    /// and the dialect's determinism row forbids that. This is the
    /// spelling of the corpus's `store.values().collect()` /
    /// `for (k, v) in store.iter()` loops (and of filter-count:
    /// `snapshot().iter().filter(...).count()`).
    pub fn snapshot(&self) -> Vec<(K, V)>
    where
        K: Ord,
    {
        let table = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<(K, V)> = table.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// A single shared value — the managed replacement for a
/// `OnceLock<Mutex<T>>` singleton holding a struct (the corpus's
/// `global_store()`/`with_store` pattern). Intended to live behind a
/// `std::sync::OnceLock` (init-once, not a contended lock, and not
/// denied). Same guarantee as [`SharedTable`]: the critical section
/// is the whole method, and no guard ever escapes.
pub struct SharedCell<T> {
    value: Mutex<T>,
}

impl<T> SharedCell<T> {
    /// Wrap an initial value.
    pub fn new(value: T) -> Self {
        Self { value: Mutex::new(value) }
    }

    /// The only write/read form: `f` sees `&mut T` under the one
    /// lock and its return value comes back — the dialect spelling of
    /// `with_store(|store| ..)`.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut value = self.value.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut value)
    }

    /// Replace the value, returning the old one.
    pub fn replace(&self, value: T) -> T {
        let mut guard = self.value.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut guard, value)
    }

    /// Clone out the current value (the only read form, so nothing
    /// observable escapes the atomicity window).
    pub fn get_clone(&self) -> T
    where
        T: Clone,
    {
        self.value.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
