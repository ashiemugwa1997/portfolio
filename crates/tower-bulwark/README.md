# tower-bulwark

A [tower](https://docs.rs/tower) layer that puts the usual defences in front of an axum (or any tower) service.

```rust,ignore
use std::time::Duration;
use axum::{routing::post, Router};
use tower_bulwark::{Bulwark, ClientIp};

let app = Router::new()
    .route("/api/contact", post(contact))
    .layer(
        Bulwark::builder()
            .hsts(Duration::from_secs(63_072_000))   // only when served over https
            .trust_proxy(true)                       // only behind your own proxy
            .rate_limit(120, Duration::from_secs(60))
            .build(),
    );

async fn contact(Extension(ClientIp(ip)): Extension<ClientIp>) { /* ... */ }
```

## What it does

| | |
|---|---|
| Security headers | Strict CSP, `nosniff`, `X-Frame-Options: DENY`, COOP/CORP, referrer and permissions policies, optional HSTS. Headers your handler already set are kept. |
| Same-origin writes | A POST/PUT/PATCH/DELETE whose `Origin` is another site gets `403`, which stops CSRF. |
| Client IP | Resolved once and stored as `ClientIp`. With `trust_proxy(true)` it takes the last `X-Forwarded-For` entry (the one your proxy appended), never the client's own claim. |
| Rate limiting | Per address, sliding window, bounded memory ([windowgate](../windowgate)). Answers `429` with `Retry-After`. Requests with no known address share one bucket. |
| Your own rules | Any `Fn(&RequestView) -> Result<(), Rejection>` or type implementing `Rule`. |

Refusals are JSON (`{"error": "..."}`) and carry the security headers too.

The peer address is read from a `std::net::SocketAddr` request extension by default; change it with `.peer(|ext| ...)`.

## Licence

MIT or Apache-2.0, at your option.
