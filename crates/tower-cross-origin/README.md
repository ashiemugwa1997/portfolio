# tower-cross-origin

[![crates.io](https://img.shields.io/crates/v/tower-cross-origin.svg)](https://crates.io/crates/tower-cross-origin)
[![docs.rs](https://docs.rs/tower-cross-origin/badge.svg)](https://docs.rs/tower-cross-origin)

CSRF protection without tokens, cookies or sessions, for axum and any tower service.

```rust,ignore
use tower_cross_origin::{CrossOriginLayer, CrossOriginProtection};

let app = Router::new()
    .route("/transfer", post(transfer))
    .layer(CrossOriginLayer::new(
        CrossOriginProtection::new()
            .trust_origin("https://admin.example.com")
            .bypass_prefix("/webhooks/"),        // signed by their sender
    ));
```

## How it works

Every current browser tells the server where a request comes from: `Sec-Fetch-Site`, and `Origin` on cross-origin writes. That is enough to stop CSRF without the token dance. The rules are the ones Go adopted for `net/http` (`CrossOriginProtection`):

1. GET, HEAD and OPTIONS, and bypassed paths, always pass. (Keep them free of side effects.)
2. `Sec-Fetch-Site: same-origin` or `none` passes; anything else (`cross-site`, `same-site`) passes only from a trusted origin.
3. Without `Sec-Fetch-Site`: no `Origin` means a non-browser client, which cannot carry a victim's cookies, and passes; otherwise `Origin` must name this host or a trusted origin.

`same-site` is refused on purpose: a takeover of `blog.example.com` should not be able to post to `app.example.com`.

Refusals are `403` with `{"error": "..."}`. For frameworks without `http::Request`, call `check_parts(method, path, headers, host)`. Property-tested against an independent statement of the rules.

Part of [tower-bulwark](../tower-bulwark), which adds security headers, client IP and rate limiting in the same layer.

## Licence

MIT or Apache-2.0, at your option.
