use std::net::IpAddr;
use std::time::Duration;

use http::{HeaderMap, Method, StatusCode, Uri};

/// What a [`Rule`] sees of a request.
#[derive(Debug, Clone, Copy)]
pub struct RequestView<'a> {
    /// Request method.
    pub method: &'a Method,
    /// Request target.
    pub uri: &'a Uri,
    /// Request headers.
    pub headers: &'a HeaderMap,
    /// The resolved client address, when known.
    pub client_ip: Option<IpAddr>,
}

/// Why a request was turned away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// Response status.
    pub status: StatusCode,
    /// Short message for the JSON body (`{"error": "..."}`).
    pub message: &'static str,
    /// Sent as `Retry-After` when present.
    pub retry_after: Option<Duration>,
}

impl Rejection {
    /// A rejection with no retry hint.
    pub const fn new(status: StatusCode, message: &'static str) -> Self {
        Rejection { status, message, retry_after: None }
    }

    pub(crate) fn body(&self) -> String {
        format!("{{\"error\":\"{}\"}}", self.message.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A check run before the request reaches the service.
pub trait Rule: Send + Sync {
    /// `Err` stops the request with that response.
    fn check(&self, req: &RequestView<'_>) -> Result<(), Rejection>;
}

impl<F> Rule for F
where
    F: Fn(&RequestView<'_>) -> Result<(), Rejection> + Send + Sync,
{
    fn check(&self, req: &RequestView<'_>) -> Result<(), Rejection> {
        self(req)
    }
}

// ── Same origin ─────────────────────────────────────────────────────────────

/// Refuses cross-site writes. Browsers send `Origin` on every POST, PUT, PATCH and
/// DELETE; it must name this host. With no `Origin` the request is allowed only when
/// `Sec-Fetch-Site` is absent (a non-browser client, which cannot carry a victim's
/// cookies) or says same-origin.
#[derive(Debug, Clone, Copy, Default)]
pub struct SameOriginWrites;

impl Rule for SameOriginWrites {
    fn check(&self, req: &RequestView<'_>) -> Result<(), Rejection> {
        if matches!(*req.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return Ok(());
        }
        let host = req
            .headers
            .get(http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .or_else(|| req.uri.authority().map(http::uri::Authority::as_str));
        let ok = match req.headers.get(http::header::ORIGIN).map(|o| o.to_str()) {
            Some(Ok(origin)) => origin_host(origin).zip(host).is_some_and(|(o, h)| o.eq_ignore_ascii_case(h)),
            Some(Err(_)) => false,
            None => matches!(
                req.headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
                None | Some("same-origin" | "none")
            ),
        };
        if ok {
            Ok(())
        } else {
            Err(Rejection::new(StatusCode::FORBIDDEN, "Cross-site requests are not allowed."))
        }
    }
}

fn origin_host(origin: &str) -> Option<&str> {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"))?;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then_some(host)
}

// ── Rate limit ──────────────────────────────────────────────────────────────

/// Limits each client address with a [`windowgate::Limiter`]. Requests with no known
/// address share one bucket, so they cannot bypass the limit.
#[cfg(feature = "rate-limit")]
#[derive(Debug)]
pub struct RateLimit {
    limiter: windowgate::Limiter<Option<IpAddr>>,
}

#[cfg(feature = "rate-limit")]
impl RateLimit {
    /// Wrap a limiter you configured yourself.
    pub fn new(limiter: windowgate::Limiter<Option<IpAddr>>) -> Self {
        RateLimit { limiter }
    }
}

#[cfg(feature = "rate-limit")]
impl Rule for RateLimit {
    fn check(&self, req: &RequestView<'_>) -> Result<(), Rejection> {
        match self.limiter.check(&req.client_ip) {
            windowgate::Decision::Allowed => Ok(()),
            windowgate::Decision::Limited { retry_after } => Err(Rejection {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Too many requests. Please slow down.",
                retry_after: Some(retry_after.max(Duration::from_secs(1))),
            }),
        }
    }
}
