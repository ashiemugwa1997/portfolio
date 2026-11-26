//! Sliding-window rate limiting per key (an IP address, a user id, "the whole site").
//!
//! * `limit` hits per `window` for each key, counted over a true sliding window.
//! * Tells you how long to wait ([`Decision::Limited`] carries a retry-after).
//! * Memory is bounded: stale keys are swept, and once `max_keys` are tracked new keys
//!   are refused until old ones expire, so a flood from rotating addresses cannot
//!   grow the map without end. Pick the policy with [`Overflow`].
//! * `Send + Sync`, no dependencies, no async runtime required.
//! * Optional: the [`Backend`] trait for shared limits (feature `mysql` brings a MySQL
//!   backend that every copy of a service shares) and a tower layer (feature `tower`).
//!
//! ```
//! use std::time::Duration;
//! use windowgate::{Decision, Limiter};
//!
//! let contact = Limiter::new(3, Duration::from_secs(3600));
//! for _ in 0..3 {
//!     assert!(contact.allow("203.0.113.9"));
//! }
//! match contact.check("203.0.113.9") {
//!     Decision::Limited { retry_after } => assert!(retry_after <= Duration::from_secs(3600)),
//!     Decision::Allowed => unreachable!(),
//! }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod backend;
#[cfg(feature = "mysql")]
mod mysql;
#[cfg(feature = "tower")]
mod tower;

pub use backend::{Backend, BackendError, HitFuture};
#[cfg(feature = "mysql")]
pub use mysql::MySqlBackend;
#[cfg(feature = "tower")]
pub use tower::{OnError, RateLimit, RateLimitLayer};

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Types ───────────────────────────────────────────────────────────────────

/// The answer for one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Go ahead; the hit was counted.
    Allowed,
    /// Over the limit; nothing was counted. Try again after `retry_after`.
    Limited {
        /// How long until a slot frees up.
        retry_after: Duration,
    },
}

impl Decision {
    /// True for [`Decision::Allowed`].
    pub fn is_allowed(self) -> bool {
        matches!(self, Decision::Allowed)
    }
}

/// What to do with a new key when `max_keys` are already tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// Refuse new keys until old ones expire (safe default: fails closed).
    #[default]
    RefuseNew,
    /// Let new keys through without tracking them (fails open: availability over strictness).
    AllowUntracked,
}

type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A sliding-window limiter. Cheap to share behind an `Arc`.
pub struct Limiter<K> {
    limit: usize,
    window: Duration,
    max_keys: usize,
    overflow: Overflow,
    sweep_every: u64,
    clock: Clock,
    state: Mutex<State<K>>,
}

struct State<K> {
    hits: HashMap<K, VecDeque<Instant>>,
    calls: u64,
}

impl<K> fmt::Debug for Limiter<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Limiter")
            .field("limit", &self.limit)
            .field("window", &self.window)
            .field("max_keys", &self.max_keys)
            .field("overflow", &self.overflow)
            .finish_non_exhaustive()
    }
}

// ── Limiter ─────────────────────────────────────────────────────────────────

impl<K: Hash + Eq + Clone> Limiter<K> {
    /// `limit` hits per `window` for each key. Tracks up to 50,000 keys.
    ///
    /// # Panics
    /// If `limit` is 0 or `window` is zero.
    pub fn new(limit: usize, window: Duration) -> Self {
        assert!(limit > 0, "limit must be at least 1");
        assert!(!window.is_zero(), "window must be longer than zero");
        Limiter {
            limit,
            window,
            max_keys: 50_000,
            overflow: Overflow::RefuseNew,
            sweep_every: 1_000,
            clock: Arc::new(Instant::now),
            state: Mutex::new(State { hits: HashMap::new(), calls: 0 }),
        }
    }

    /// Cap on tracked keys (default 50,000).
    #[must_use]
    pub fn max_keys(mut self, n: usize) -> Self {
        self.max_keys = n.max(1);
        self
    }

    /// What happens to new keys at the cap (default [`Overflow::RefuseNew`]).
    #[must_use]
    pub fn overflow(mut self, o: Overflow) -> Self {
        self.overflow = o;
        self
    }

    /// Use your own clock, e.g. a fake one in tests.
    #[must_use]
    pub fn clock(mut self, clock: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Count a hit for `key` if it is under the limit.
    pub fn check<Q>(&self, key: &Q) -> Decision
    where
        Q: ToOwned<Owned = K> + ?Sized,
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq,
    {
        let now = (self.clock)();
        let cutoff = now.checked_sub(self.window);
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        st.calls += 1;
        if st.calls % self.sweep_every == 0 || st.hits.len() >= self.max_keys {
            sweep(&mut st.hits, cutoff);
        }

        if !st.hits.contains_key(key) && st.hits.len() >= self.max_keys {
            return match self.overflow {
                Overflow::AllowUntracked => Decision::Allowed,
                Overflow::RefuseNew => Decision::Limited { retry_after: self.window },
            };
        }

        let q = st.hits.entry(key.to_owned()).or_default();
        if let Some(c) = cutoff {
            while q.front().is_some_and(|t| *t <= c) {
                q.pop_front();
            }
        }
        if q.len() >= self.limit {
            let oldest = *q.front().expect("limit is at least 1");
            let retry_after = (oldest + self.window).saturating_duration_since(now);
            return Decision::Limited { retry_after };
        }
        q.push_back(now);
        Decision::Allowed
    }

    /// Shorthand for `check(key).is_allowed()`.
    pub fn allow<Q>(&self, key: &Q) -> bool
    where
        Q: ToOwned<Owned = K> + ?Sized,
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq,
    {
        self.check(key).is_allowed()
    }

    /// Forget every key with no hit inside the window.
    pub fn sweep(&self) {
        let cutoff = (self.clock)().checked_sub(self.window);
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        sweep(&mut st.hits, cutoff);
    }

    /// Keys currently tracked.
    pub fn len(&self) -> usize {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).hits.len()
    }

    /// True when no key is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn sweep<K>(hits: &mut HashMap<K, VecDeque<Instant>>, cutoff: Option<Instant>) {
    let Some(c) = cutoff else { return };
    hits.retain(|_, q| q.back().is_some_and(|t| *t > c));
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// a clock tests can move forward by hand
    fn fake_clock() -> (Arc<AtomicU64>, impl Fn() -> Instant + Send + Sync) {
        let start = Instant::now() + Duration::from_secs(10_000);
        let ms = Arc::new(AtomicU64::new(0));
        let m = ms.clone();
        (ms, move || start + Duration::from_millis(m.load(Ordering::SeqCst)))
    }

    #[test]
    fn allows_up_to_the_limit_then_refuses() {
        let l: Limiter<String> = Limiter::new(3, Duration::from_secs(60));
        assert!(l.allow("a") && l.allow("a") && l.allow("a"));
        assert!(!l.allow("a"));
        assert!(l.allow("b"), "keys are independent");
    }

    #[test]
    fn window_slides() {
        let (ms, clock) = fake_clock();
        let l: Limiter<String> = Limiter::new(2, Duration::from_secs(10)).clock(clock);
        assert!(l.allow("k"));
        ms.store(5_000, Ordering::SeqCst);
        assert!(l.allow("k"));
        assert!(!l.allow("k"));
        ms.store(10_001, Ordering::SeqCst); // first hit has left the window
        assert!(l.allow("k"));
        assert!(!l.allow("k"));
    }

    #[test]
    fn retry_after_counts_down_to_the_oldest_hit() {
        let (ms, clock) = fake_clock();
        let l: Limiter<String> = Limiter::new(1, Duration::from_secs(10)).clock(clock);
        assert!(l.allow("k"));
        ms.store(4_000, Ordering::SeqCst);
        assert_eq!(l.check("k"), Decision::Limited { retry_after: Duration::from_secs(6) });
    }

    #[test]
    fn refused_hits_are_not_counted() {
        let (ms, clock) = fake_clock();
        let l: Limiter<String> = Limiter::new(1, Duration::from_secs(10)).clock(clock);
        assert!(l.allow("k"));
        for _ in 0..50 {
            assert!(!l.allow("k"));
        }
        ms.store(10_001, Ordering::SeqCst);
        assert!(l.allow("k"), "hammering while limited must not extend the wait");
    }

    #[test]
    fn key_cap_refuses_new_keys_by_default() {
        let l: Limiter<u32> = Limiter::new(5, Duration::from_secs(60)).max_keys(100);
        for ip in 0..100 {
            assert!(l.allow(&ip));
        }
        assert!(!l.allow(&100_000), "new key past the cap");
        assert!(l.allow(&7), "known keys still work");
        assert_eq!(l.len(), 100);
    }

    #[test]
    fn key_cap_can_fail_open() {
        let l: Limiter<u32> = Limiter::new(5, Duration::from_secs(60)).max_keys(1).overflow(Overflow::AllowUntracked);
        assert!(l.allow(&1));
        assert!(l.allow(&2));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn stale_keys_are_swept() {
        let (ms, clock) = fake_clock();
        let l: Limiter<u32> = Limiter::new(5, Duration::from_secs(1)).max_keys(10).clock(clock);
        for ip in 0..10 {
            l.allow(&ip);
        }
        ms.store(2_000, Ordering::SeqCst);
        assert!(l.allow(&99), "cap reached, but every old key is stale");
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn many_threads_never_exceed_the_limit() {
        let l: Arc<Limiter<String>> = Arc::new(Limiter::new(100, Duration::from_secs(60)));
        let allowed = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (l, allowed) = (l.clone(), allowed.clone());
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        if l.allow("shared") {
                            allowed.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(allowed.load(Ordering::SeqCst), 100);
    }

    #[test]
    #[should_panic(expected = "limit must be at least 1")]
    fn zero_limit_panics() {
        let _: Limiter<u8> = Limiter::new(0, Duration::from_secs(1));
    }
}
