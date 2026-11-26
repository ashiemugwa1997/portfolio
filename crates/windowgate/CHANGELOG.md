# Changelog

## 0.2.1

- `RedisBackend` (feature `redis`): sliding-window limits in Redis, one atomic Lua script per hit (prune, count, add) on a sorted set, timed by the Redis server's clock; keys expire on their own. A burst of 60 concurrent hits lets exactly the limit through (tested against Redis in CI). The `redis` feature needs Rust 1.88, like the redis crate.

## 0.2.0

- `Backend` trait: count hits anywhere. `Limiter<String>` implements it.
- `MySqlBackend` (feature `mysql`): sliding-window limits shared by every copy of a service and kept across restarts, timed by the database's clock. Refused hits are removed so they never extend the wait; under a burst it never allows more than the limit (tested with 40 concurrent hits).
- `RateLimitLayer` (feature `tower`): per-key limiting for any tower/axum service with `429` and `Retry-After`, custom keys, and a policy for backend errors (`OnError::Allow` by default).
- Property tests against a reference sliding window.
- Default build still has no dependencies.

## 0.1.0

- First release: sliding-window `Limiter` with bounded keys and `Overflow` policy.
