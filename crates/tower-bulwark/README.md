# tower-bulwark

[![crates.io](https://img.shields.io/crates/v/tower-bulwark.svg)](https://crates.io/crates/tower-bulwark)
[![docs.rs](https://docs.rs/tower-bulwark/badge.svg)](https://docs.rs/tower-bulwark)

One tower layer for the usual defences in front of an axum (or any tower) service: security headers, CSRF protection, client IP and rate limiting.

```rust,ignore
use std::time::Duration;
use axum::{routing::post, Extension, Router};
use tower_bulwark::{Bulwark, ClientIp};

let app = Router::new()
    .route("/api/contact", post(contact))
    .layer(
        Bulwark::builder()
            .hsts(Duration::from_secs(63_072_000))   // only when served over https
            .trust_proxy(true)                       // only behind your own proxy
            .trust_origin("https://admin.example.com")
            .rate_limit(120, Duration::from_secs(60))
            .build(),
    );

async fn contact(Extension(ClientIp(ip)): Extension<ClientIp>) { /* ... */ }
```

## What it does

| | |
|---|---|
| Security headers | Strict CSP, `nosniff`, `X-Frame-Options: DENY`, COOP/CORP, referrer and permissions policies, optional HSTS. Headers your handler already set are kept. |
| CSRF | Cross-origin writes get `403`, by [`tower-cross-origin`](../tower-cross-origin) (the rules of Go's `CrossOriginProtection`: `Sec-Fetch-Site`, then `Origin` against `Host`). No tokens or sessions. Trusted origins and bypassed paths (webhooks) supported. |
| Client IP | Resolved once and stored as `ClientIp`. With `trust_proxy(true)` it takes the last `X-Forwarded-For` entry (the one your proxy appended), never the client's own claim. |
| Rate limiting | Per address, sliding window, bounded memory ([windowgate](../windowgate)). `429` with `Retry-After`. For limits shared across servers use windowgate's `RateLimitLayer` with its MySQL backend. |
| Your own rules | Any `Fn(&RequestView) -> Result<(), Rejection>` or type implementing `Rule`. |

Refusals are JSON (`{"error": "..."}`) and carry the security headers too. Property-tested against independent statements of the rules.

## Moving from tower-helmet

`tower-helmet` has not been updated since January 2024.

| tower-helmet | tower-bulwark |
|---|---|
| `HelmetLayer::with_defaults()` | `Bulwark::default()` (or `Bulwark::builder()...build()`) |
| `ContentSecurityPolicy` | `.csp("default-src 'self'; ...")` |
| `StrictTransportSecurity` | `.hsts(Duration::from_secs(...))` |
| `XFrameOptions`, `XContentTypeOptions`, `ReferrerPolicy`, `CrossOriginOpenerPolicy`, `CrossOriginResourcePolicy` | on by default (`HeaderPolicy::strict`) |
| any other header | `.header(name, value)` |
| (none) | CSRF protection, client IP, rate limiting, custom rules |

Only want the CSRF part? Use [`tower-cross-origin`](../tower-cross-origin) on its own.

## Licence

MIT or Apache-2.0, at your option.
