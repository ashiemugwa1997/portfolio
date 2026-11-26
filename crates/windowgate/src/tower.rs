use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use http::{header, HeaderMap, HeaderValue, Request, Response, StatusCode};
use tower_layer::Layer;
use tower_service::Service;

use crate::backend::Backend;
use crate::Decision;

type KeyFn = Arc<dyn Fn(&http::Extensions, &HeaderMap) -> Option<String> + Send + Sync>;

/// What to do when the backend cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnError {
    /// Let the request through (default): a database hiccup should not take the site down.
    #[default]
    Allow,
    /// Refuse with 503.
    Deny,
}

/// Tower layer: limits requests per key with any [`Backend`], answering `429` with
/// `Retry-After`. The key defaults to the client's IP from a `std::net::SocketAddr`
/// request extension; requests with no key share one bucket.
///
/// ```
/// use std::time::Duration;
/// use windowgate::{Limiter, RateLimitLayer};
///
/// let layer = RateLimitLayer::new(Limiter::<String>::new(120, Duration::from_secs(60)))
///     .key(|_ext, headers| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_owned));
/// # let _ = layer;
/// ```
pub struct RateLimitLayer<B> {
    backend: Arc<B>,
    key: KeyFn,
    on_error: OnError,
}

impl<B> Clone for RateLimitLayer<B> {
    fn clone(&self) -> Self {
        RateLimitLayer { backend: self.backend.clone(), key: self.key.clone(), on_error: self.on_error }
    }
}

impl<B: Backend + 'static> RateLimitLayer<B> {
    /// Limit with `backend`, keyed by client IP.
    pub fn new(backend: B) -> Self {
        RateLimitLayer {
            backend: Arc::new(backend),
            key: Arc::new(|ext: &http::Extensions, _: &HeaderMap| ext.get::<SocketAddr>().map(|a| a.ip().to_string())),
            on_error: OnError::Allow,
        }
    }

    /// Choose the key from request extensions and headers.
    #[must_use]
    pub fn key(mut self, f: impl Fn(&http::Extensions, &HeaderMap) -> Option<String> + Send + Sync + 'static) -> Self {
        self.key = Arc::new(f);
        self
    }

    /// What to do when the backend fails (default: let the request through).
    #[must_use]
    pub fn on_error(mut self, policy: OnError) -> Self {
        self.on_error = policy;
        self
    }
}

impl<S, B> Layer<S> for RateLimitLayer<B> {
    type Service = RateLimit<S, B>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimit { inner, backend: self.backend.clone(), key: self.key.clone(), on_error: self.on_error }
    }
}

/// The service [`RateLimitLayer`] wraps around yours.
pub struct RateLimit<S, B> {
    inner: S,
    backend: Arc<B>,
    key: KeyFn,
    on_error: OnError,
}

impl<S: Clone, B> Clone for RateLimit<S, B> {
    fn clone(&self) -> Self {
        RateLimit { inner: self.inner.clone(), backend: self.backend.clone(), key: self.key.clone(), on_error: self.on_error }
    }
}

fn refusal<B: From<String>>(status: StatusCode, message: &str, retry_after: Option<std::time::Duration>) -> Response<B> {
    let mut res = Response::new(B::from(format!("{{\"error\":\"{message}\"}}")));
    *res.status_mut() = status;
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(after) = retry_after {
        let secs = after.as_secs() + u64::from(after.subsec_nanos() > 0);
        res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs.max(1)));
    }
    res
}

impl<S, B, ReqBody, ResBody> Service<Request<ReqBody>> for RateLimit<S, B>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    B: Backend + 'static,
    ReqBody: Send + 'static,
    ResBody: From<String>,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // the ready service handles this request; a fresh clone takes its place
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let key = (self.key)(req.extensions(), req.headers()).unwrap_or_else(|| "-".to_owned());
        let backend = self.backend.clone();
        let on_error = self.on_error;
        Box::pin(async move {
            match backend.hit(&key).await {
                Ok(Decision::Allowed) => inner.call(req).await,
                Ok(Decision::Limited { retry_after }) => {
                    Ok(refusal(StatusCode::TOO_MANY_REQUESTS, "Too many requests. Please slow down.", Some(retry_after)))
                }
                Err(_) if on_error == OnError::Allow => inner.call(req).await,
                Err(_) => Ok(refusal(StatusCode::SERVICE_UNAVAILABLE, "Please try again shortly.", None)),
            }
        })
    }
}
