# Changelog

## 0.2.0

- CSRF check moved to the new `tower-cross-origin` crate and now follows Go's `CrossOriginProtection` rules: `Sec-Fetch-Site` is consulted first, and `same-site` requests (sibling subdomains) are refused unless trusted. **Behaviour change**: a request with `Sec-Fetch-Site: same-origin` but a mismatching `Origin` now passes (the browser's own report wins).
- `trust_origin` and `cross_origin(policy)` on the builder: trusted origins and bypassed paths.
- `SameOriginWrites` now holds a policy: `SameOriginWrites::default()` or `SameOriginWrites::new(policy)`.
- Depends on windowgate 0.2.
- Migration guide from tower-helmet.

## 0.1.0

- First release: security headers, same-origin writes, trusted-proxy client IP, rate limiting, custom rules.
