# windowgate

Sliding-window rate limiting per key, with bounded memory. Thread-safe, no dependencies, no async runtime needed.

```rust
use std::time::Duration;
use windowgate::{Decision, Limiter, Overflow};

// 3 contact-form messages an hour per address
let per_ip: Limiter<String> = Limiter::new(3, Duration::from_secs(3600));

// 50 a day for the whole site, so nobody can flood the inbox by rotating addresses
let site: Limiter<String> = Limiter::new(50, Duration::from_secs(86_400));

let ip = "203.0.113.9";
match per_ip.check(ip) {
    Decision::Allowed if site.allow("all") => { /* accept */ }
    Decision::Limited { retry_after } => println!("try again in {retry_after:?}"),
    _ => println!("the site has had enough messages today"),
}
```

## Why another limiter

- **True sliding window**: no burst at the edge of a fixed window.
- **Bounded memory**: stale keys are swept, and at `max_keys` (default 50,000) new keys are refused (`Overflow::RefuseNew`) or let through untracked (`Overflow::AllowUntracked`). An attacker rotating source addresses cannot make the map grow without end.
- **Refused hits are not counted**: hammering while limited does not extend the wait.
- **Retry-after** for `Retry-After` headers.
- **Testable**: plug in your own clock with `.clock(...)`.

Used by [tower-bulwark](../tower-bulwark) and the server of [my portfolio](https://github.com/ashiemugwa1997/portfolio).

## Licence

MIT or Apache-2.0, at your option.
