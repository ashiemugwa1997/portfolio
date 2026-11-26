//! Random hit sequences against a reference sliding window.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use windowgate::{Decision, Limiter};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// every decision equals the obvious model: allowed iff fewer than `limit` allowed hits
    /// for that key in the last `window`
    #[test]
    fn matches_the_reference_model(
        limit in 1usize..6,
        window_ms in 1u64..500,
        hits in prop::collection::vec((0u8..4, 0u64..200), 1..200),
    ) {
        let start = Instant::now() + Duration::from_secs(1_000);
        let now_ms = Arc::new(AtomicU64::new(0));
        let clock = now_ms.clone();
        let l: Limiter<u8> = Limiter::new(limit, Duration::from_millis(window_ms))
            .clock(move || start + Duration::from_millis(clock.load(Ordering::SeqCst)));

        let mut model: HashMap<u8, Vec<u64>> = HashMap::new();
        let mut t = 0u64;
        for (key, step) in hits {
            t += step;
            now_ms.store(t, Ordering::SeqCst);
            let past = model.entry(key).or_default();
            past.retain(|&h| h + window_ms > t);
            let want = past.len() < limit;
            let got = l.check(&key);
            prop_assert_eq!(got.is_allowed(), want, "key {} at {}ms", key, t);
            if want {
                past.push(t);
            } else if let Decision::Limited { retry_after } = got {
                prop_assert!(retry_after <= Duration::from_millis(window_ms));
            }
        }
    }

    /// memory never exceeds the cap, whatever the key stream
    #[test]
    fn key_cap_holds(cap in 1usize..50, keys in prop::collection::vec(any::<u32>(), 0..400)) {
        let l: Limiter<u32> = Limiter::new(3, Duration::from_secs(60)).max_keys(cap);
        for k in keys {
            l.check(&k);
            prop_assert!(l.len() <= cap);
        }
    }
}
