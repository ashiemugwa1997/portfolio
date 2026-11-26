# windowgate

[![crates.io](https://img.shields.io/crates/v/windowgate.svg)](https://crates.io/crates/windowgate)
[![docs.rs](https://docs.rs/windowgate/badge.svg)](https://docs.rs/windowgate)

Sliding-window rate limiting with bounded memory, and limits that can be shared by every copy of your service. No dependencies by default.

```rust
use std::time::Duration;
use windowgate::{Decision, Limiter};

let per_ip: Limiter<String> = Limiter::new(3, Duration::from_secs(3600));
match per_ip.check("203.0.113.9") {
    Decision::Allowed => { /* accept */ }
    Decision::Limited { retry_after } => println!("try again in {retry_after:?}"),
}
```

## Shared limits (features `redis` and `mysql`)

In-memory limiters (this one, `governor`) count per process: run three copies and a client gets three times the limit, and a restart forgets everything. `MySqlBackend` keeps the window in a table every copy shares, timed by the database's clock:

```rust,ignore
use windowgate::{Backend, MySqlBackend};

let logins = MySqlBackend::new(pool, "login_hits", 5, Duration::from_secs(900))?;
logins.create_table().await?;
if !logins.hit(&email).await?.is_allowed() { /* 429 */ }
```

Or with Redis, where each hit is one atomic Lua script on a sorted set:

```rust,ignore
use windowgate::RedisBackend;

let api = RedisBackend::connect(redis::Client::open("redis://127.0.0.1/")?, "api", 120, Duration::from_secs(60)).await?;
```

Both use the server's clock, so instances with drifting clocks agree. Refused hits never extend the wait, and a burst never gets more than the limit through (tested with 40 concurrent hits against MySQL 8.4 and 60 against Redis). Anything else (Postgres, DynamoDB) is one trait method: `Backend::hit`.

## Tower / axum (feature `tower`)

```rust,ignore
use windowgate::{Limiter, MySqlBackend, OnError, RateLimitLayer};

let app = Router::new()
    .route("/api/login", post(login))
    .layer(RateLimitLayer::new(MySqlBackend::new(pool, "api_hits", 120, Duration::from_secs(60))?)
        .key(|ext, headers| /* default: client IP from a SocketAddr extension */ None)
        .on_error(OnError::Allow)); // a database hiccup should not take the site down
```

Answers `429 Too Many Requests` with `Retry-After`. A maintained alternative to `tower_governor` that also works across servers.

## Why another limiter

- **True sliding window**: no burst at the edge of a fixed window.
- **Bounded memory**: stale keys are swept; at `max_keys` new keys are refused (`Overflow::RefuseNew`) or let through untracked (`Overflow::AllowUntracked`), so rotating addresses cannot exhaust memory.
- **Refused hits are not counted** and the wait is reported for `Retry-After`.
- **Testable**: plug in your own clock with `.clock(...)`; property-tested against a reference model.

## Licence

MIT or Apache-2.0, at your option.
