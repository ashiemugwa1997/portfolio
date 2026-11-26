//! A [tower] layer that puts the usual defences in front of a web service.
//!
//! * Strict security headers on every response (CSP, nosniff, frame denial, COOP/CORP,
//!   referrer and permissions policies, optional HSTS). Headers a handler already set win.
//! * Same-origin check on writes: a browser POST from another site gets `403`.
//! * The visitor's address resolved once (peer address, or the entry your own proxy
//!   appended to `X-Forwarded-For`) and stored as [`ClientIp`] for handlers.
//! * Per-address rate limiting with `429` and `Retry-After` (feature `rate-limit`).
//! * Your own checks: implement [`Rule`] and add it with [`BulwarkBuilder::rule`].
//!
//! ```
//! use std::time::Duration;
//! use tower_bulwark::Bulwark;
//!
//! let layer = Bulwark::builder()
//!     .hsts(Duration::from_secs(63_072_000))
//!     .trust_proxy(true)
//!     .rate_limit(120, Duration::from_secs(60))
//!     .build();
//! // axum: Router::new().route(...).layer(layer)
//! # let _ = layer;
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod headers;
mod ip;
mod rules;
mod service;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use tower_layer::Layer;

pub use tower_cross_origin::CrossOriginProtection;

pub use headers::HeaderPolicy;
pub use ip::{ClientIp, ClientIpResolver};
pub use rules::{Rejection, RequestView, Rule, SameOriginWrites};
#[cfg(feature = "rate-limit")]
pub use rules::RateLimit;
pub use service::{BulwarkService, ResponseFuture};

/// Content-Security-Policy for a site that serves only its own scripts, styles and data.
pub const STRICT_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
font-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

// ── Layer ───────────────────────────────────────────────────────────────────

/// The layer. Cheap to clone; build it with [`Bulwark::builder`].
#[derive(Clone)]
pub struct Bulwark {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    pub(crate) headers: HeaderPolicy,
    pub(crate) resolver: ClientIpResolver,
    pub(crate) rules: Vec<Box<dyn Rule>>,
}

impl Bulwark {
    /// Start from safe defaults: strict CSP, same-origin writes, peer address as client IP.
    pub fn builder() -> BulwarkBuilder {
        BulwarkBuilder {
            headers: HeaderPolicy::strict(STRICT_CSP),
            resolver: ClientIpResolver::default(),
            same_origin: Some(CrossOriginProtection::new()),
            rules: Vec::new(),
        }
    }
}

impl Default for Bulwark {
    fn default() -> Self {
        Bulwark::builder().build()
    }
}

impl<S> Layer<S> for Bulwark {
    type Service = BulwarkService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BulwarkService::new(inner, self.shared.clone())
    }
}

// ── Builder ─────────────────────────────────────────────────────────────────

/// Configures a [`Bulwark`].
pub struct BulwarkBuilder {
    headers: HeaderPolicy,
    resolver: ClientIpResolver,
    same_origin: Option<CrossOriginProtection>,
    rules: Vec<Box<dyn Rule>>,
}

impl BulwarkBuilder {
    /// Replace the Content-Security-Policy.
    #[must_use]
    pub fn csp(mut self, policy: &str) -> Self {
        self.headers = self.headers.with("content-security-policy", policy);
        self
    }

    /// Send `Strict-Transport-Security` (only when the site is served over https).
    #[must_use]
    pub fn hsts(mut self, max_age: Duration) -> Self {
        let v = format!("max-age={}; includeSubDomains", max_age.as_secs());
        self.headers = self.headers.with("strict-transport-security", &v);
        self
    }

    /// Add or replace any response header.
    #[must_use]
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers = self.headers.with_value(name, value);
        self
    }

    /// Behind your own reverse proxy: take the client address from the last
    /// `X-Forwarded-For` entry. Leave off when clients connect directly, or anyone can
    /// claim any address.
    #[must_use]
    pub fn trust_proxy(mut self, on: bool) -> Self {
        self.resolver = self.resolver.trust_forwarded(on);
        self
    }

    /// How to find the connection's address in request extensions (default: a
    /// `std::net::SocketAddr` extension).
    #[must_use]
    pub fn peer(mut self, f: impl Fn(&http::Extensions) -> Option<IpAddr> + Send + Sync + 'static) -> Self {
        self.resolver = self.resolver.peer(f);
        self
    }

    /// Turn the same-origin check on writes on or off (on by default).
    #[must_use]
    pub fn same_origin_writes(mut self, on: bool) -> Self {
        self.same_origin = on.then(|| self.same_origin.take().unwrap_or_default());
        self
    }

    /// Also accept writes from `origin` (`scheme://host[:port]`).
    #[must_use]
    pub fn trust_origin(mut self, origin: &str) -> Self {
        self.same_origin = Some(self.same_origin.take().unwrap_or_default().trust_origin(origin));
        self
    }

    /// Replace the cross-origin policy (trusted origins, bypassed paths such as webhooks).
    #[must_use]
    pub fn cross_origin(mut self, policy: CrossOriginProtection) -> Self {
        self.same_origin = Some(policy);
        self
    }

    /// Limit each client address to `limit` requests per `window`.
    #[cfg(feature = "rate-limit")]
    #[must_use]
    pub fn rate_limit(self, limit: usize, window: Duration) -> Self {
        self.rule(RateLimit::new(windowgate::Limiter::new(limit, window)))
    }

    /// Add a check; rules run in the order added, after the built-in ones.
    #[must_use]
    pub fn rule(mut self, rule: impl Rule + 'static) -> Self {
        self.rules.push(Box::new(rule));
        self
    }

    /// Finish.
    pub fn build(self) -> Bulwark {
        let mut rules: Vec<Box<dyn Rule>> = Vec::with_capacity(self.rules.len() + 1);
        if let Some(policy) = self.same_origin {
            rules.push(Box::new(SameOriginWrites::new(policy)));
        }
        rules.extend(self.rules);
        Bulwark { shared: Arc::new(Shared { headers: self.headers, resolver: self.resolver, rules }) }
    }
}
