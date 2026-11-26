//! CSRF protection without tokens, cookies or sessions.
//!
//! Browsers say where a request comes from: `Sec-Fetch-Site` (all current browsers) and
//! `Origin` (every cross-origin POST). [`CrossOriginProtection`] refuses unsafe requests
//! (anything but GET, HEAD and OPTIONS) that another site made, following the same rules as
//! Go's `net/http` `CrossOriginProtection`:
//!
//! 1. Safe methods and bypassed paths always pass.
//! 2. `Sec-Fetch-Site: same-origin` or `none` (typed into the address bar) passes; any other
//!    value passes only from a trusted origin.
//! 3. Without `Sec-Fetch-Site`: no `Origin` means a non-browser client (it cannot carry a
//!    victim's cookies) and passes; otherwise `Origin` must name this host or a trusted one.
//!
//! Same-site requests from a sibling subdomain are refused unless trusted: a takeover of
//! `blog.example.com` should not be able to post to `app.example.com`.
//!
//! ```
//! use http::{Method, Request};
//! use tower_cross_origin::CrossOriginProtection;
//!
//! let guard = CrossOriginProtection::new().trust_origin("https://admin.example.com").bypass_prefix("/webhooks/");
//!
//! let evil = Request::post("/transfer").header("host", "bank.example").header("origin", "https://evil.example").body(()).unwrap();
//! assert!(guard.check(&evil).is_err());
//!
//! let own = Request::post("/transfer").header("host", "bank.example").header("sec-fetch-site", "same-origin").body(()).unwrap();
//! assert!(guard.check(&own).is_ok());
//! ```
//!
//! With the default `tower` feature, [`CrossOriginLayer`] puts it in front of any tower or
//! axum service and answers refusals with `403`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "tower")]
mod layer;

use std::fmt;
use std::sync::Arc;

use http::{header, HeaderMap, Method, Request};

#[cfg(feature = "tower")]
pub use layer::{CrossOriginLayer, CrossOriginService, ResponseFuture};

// ── Refusal ─────────────────────────────────────────────────────────────────

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `Sec-Fetch-Site` said another site sent it.
    CrossSite,
    /// `Origin` named a different, untrusted origin.
    OriginMismatch,
    /// `Origin` or `Sec-Fetch-Site` could not be read.
    Malformed,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::CrossSite => "cross-site request refused",
            Refusal::OriginMismatch => "request from another origin refused",
            Refusal::Malformed => "unreadable Origin or Sec-Fetch-Site header",
        })
    }
}

impl std::error::Error for Refusal {}

// ── Protection ──────────────────────────────────────────────────────────────

type Bypass = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The policy. Cheap to clone.
#[derive(Clone, Default)]
pub struct CrossOriginProtection {
    trusted: Vec<String>,
    bypass: Vec<Bypass>,
}

impl fmt::Debug for CrossOriginProtection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CrossOriginProtection").field("trusted", &self.trusted).field("bypass_rules", &self.bypass.len()).finish()
    }
}

impl CrossOriginProtection {
    /// Refuse every cross-origin write.
    pub fn new() -> Self {
        CrossOriginProtection::default()
    }

    /// Also accept writes from `origin` (`scheme://host[:port]`, exactly as browsers send it).
    ///
    /// # Panics
    /// If `origin` has a path, query or trailing slash: those never appear in an `Origin` header.
    #[must_use]
    pub fn trust_origin(mut self, origin: &str) -> Self {
        let o = origin.trim().to_ascii_lowercase();
        let host = o.strip_prefix("https://").or_else(|| o.strip_prefix("http://"));
        assert!(host.is_some_and(|h| !h.is_empty() && !h.contains('/')), "trusted origin must be scheme://host[:port], got {origin:?}");
        self.trusted.push(o);
        self
    }

    /// Never check paths starting with `prefix` (webhooks signed by their sender, for example).
    #[must_use]
    pub fn bypass_prefix(self, prefix: &str) -> Self {
        let p = prefix.to_owned();
        self.bypass(move |path| path.starts_with(&p))
    }

    /// Never check paths for which `rule` returns true.
    #[must_use]
    pub fn bypass(mut self, rule: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.bypass.push(Arc::new(rule));
        self
    }

    /// Check a request.
    pub fn check<B>(&self, req: &Request<B>) -> Result<(), Refusal> {
        let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).or_else(|| req.uri().authority().map(http::uri::Authority::as_str));
        self.check_parts(req.method(), req.uri().path(), req.headers(), host)
    }

    /// Check from the pieces, for frameworks that do not hand over an `http::Request`.
    pub fn check_parts(&self, method: &Method, path: &str, headers: &HeaderMap, host: Option<&str>) -> Result<(), Refusal> {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) || self.bypass.iter().any(|b| b(path)) {
            return Ok(());
        }
        let origin = match headers.get(header::ORIGIN).map(|v| v.to_str()) {
            Some(Ok(o)) => Some(o),
            Some(Err(_)) => return Err(Refusal::Malformed),
            None => None,
        };
        match headers.get("sec-fetch-site").map(|v| v.to_str()) {
            Some(Ok("same-origin" | "none")) => Ok(()),
            Some(Ok(_)) => {
                if origin.is_some_and(|o| self.is_trusted(o)) {
                    Ok(())
                } else {
                    Err(Refusal::CrossSite)
                }
            }
            Some(Err(_)) => Err(Refusal::Malformed),
            None => match origin {
                None => Ok(()),
                Some(o) if self.is_trusted(o) => Ok(()),
                Some(o) if origin_host(o).zip(host).is_some_and(|(oh, h)| oh.eq_ignore_ascii_case(h)) => Ok(()),
                Some(_) => Err(Refusal::OriginMismatch),
            },
        }
    }

    fn is_trusted(&self, origin: &str) -> bool {
        self.trusted.iter().any(|t| t.eq_ignore_ascii_case(origin.trim()))
    }
}

/// `host[:port]` of an `Origin` value, or None for `null` and anything unreadable
fn origin_host(origin: &str) -> Option<&str> {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"))?;
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: Method, path: &str, h: &[(&str, &str)]) -> Request<()> {
        let mut b = Request::builder().method(method).uri(path).header("host", "app.example");
        for (k, v) in h {
            b = b.header(*k, *v);
        }
        b.body(()).unwrap()
    }

    #[test]
    fn the_rules() {
        let g = CrossOriginProtection::new();
        let post = |h: &[(&str, &str)]| g.check(&req(Method::POST, "/x", h));
        assert_eq!(post(&[]), Ok(()), "non-browser client");
        assert_eq!(post(&[("sec-fetch-site", "same-origin")]), Ok(()));
        assert_eq!(post(&[("sec-fetch-site", "none")]), Ok(()));
        assert_eq!(post(&[("sec-fetch-site", "cross-site")]), Err(Refusal::CrossSite));
        assert_eq!(post(&[("sec-fetch-site", "same-site")]), Err(Refusal::CrossSite), "sibling subdomains are not trusted");
        assert_eq!(post(&[("origin", "https://app.example")]), Ok(()));
        assert_eq!(post(&[("origin", "https://APP.example")]), Ok(()));
        assert_eq!(post(&[("origin", "https://evil.example")]), Err(Refusal::OriginMismatch));
        assert_eq!(post(&[("origin", "https://app.example.evil.example")]), Err(Refusal::OriginMismatch));
        assert_eq!(post(&[("origin", "null")]), Err(Refusal::OriginMismatch));
        assert_eq!(post(&[("origin", "https://evil.example"), ("sec-fetch-site", "same-origin")]), Ok(()), "Sec-Fetch-Site wins");
    }

    #[test]
    fn safe_methods_and_bypass() {
        let g = CrossOriginProtection::new().bypass_prefix("/webhooks/");
        let evil = [("sec-fetch-site", "cross-site")];
        for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(g.check(&req(m, "/x", &evil)).is_ok());
        }
        assert!(g.check(&req(Method::POST, "/webhooks/stripe", &evil)).is_ok());
        assert!(g.check(&req(Method::POST, "/webhook", &evil)).is_err());
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(g.check(&req(m, "/x", &evil)).is_err());
        }
    }

    #[test]
    fn trusted_origins() {
        let g = CrossOriginProtection::new().trust_origin("https://admin.example");
        assert!(g.check(&req(Method::POST, "/x", &[("origin", "https://admin.example"), ("sec-fetch-site", "same-site")])).is_ok());
        assert!(g.check(&req(Method::POST, "/x", &[("origin", "https://admin.example")])).is_ok());
        assert!(g.check(&req(Method::POST, "/x", &[("origin", "http://admin.example")])).is_err(), "scheme matters");
    }

    #[test]
    #[should_panic(expected = "trusted origin must be")]
    fn trusted_origin_with_a_path_is_a_mistake() {
        let _ = CrossOriginProtection::new().trust_origin("https://admin.example/");
    }
}
