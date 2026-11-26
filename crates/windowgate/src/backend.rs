use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::{Decision, Limiter};

/// What [`Backend::hit`] returns.
pub type HitFuture<'a> = Pin<Box<dyn Future<Output = Result<Decision, BackendError>> + Send + 'a>>;

/// Where hits are counted. The in-memory [`Limiter`] is one; with the `mysql` feature,
/// [`MySqlBackend`](crate::MySqlBackend) shares counts between every copy of a service and
/// keeps them across restarts. Implement it for Redis, Postgres or anything else.
pub trait Backend: Send + Sync {
    /// Count a hit for `key` if it is under the limit.
    fn hit<'a>(&'a self, key: &'a str) -> HitFuture<'a>;
}

impl Backend for Limiter<String> {
    fn hit<'a>(&'a self, key: &'a str) -> HitFuture<'a> {
        let d = self.check(key);
        Box::pin(async move { Ok(d) })
    }
}

impl<B: Backend + ?Sized> Backend for std::sync::Arc<B> {
    fn hit<'a>(&'a self, key: &'a str) -> HitFuture<'a> {
        (**self).hit(key)
    }
}

/// A backend could not answer (database down, timeout...).
#[derive(Debug)]
pub struct BackendError(Box<dyn Error + Send + Sync>);

impl BackendError {
    /// Wrap any error.
    pub fn new(e: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        BackendError(e.into())
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rate limit backend: {}", self.0)
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.0)
    }
}
