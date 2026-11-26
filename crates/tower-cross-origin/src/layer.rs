use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use http::{header, HeaderValue, Request, Response, StatusCode};
use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

use crate::CrossOriginProtection;

/// Tower layer that refuses cross-origin writes with `403` and a small JSON body.
#[derive(Debug, Clone, Default)]
pub struct CrossOriginLayer {
    policy: CrossOriginProtection,
}

impl CrossOriginLayer {
    /// Use `policy`.
    pub fn new(policy: CrossOriginProtection) -> Self {
        CrossOriginLayer { policy }
    }
}

impl<S> Layer<S> for CrossOriginLayer {
    type Service = CrossOriginService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CrossOriginService { inner, policy: self.policy.clone() }
    }
}

/// The service [`CrossOriginLayer`] wraps around yours.
#[derive(Debug, Clone)]
pub struct CrossOriginService<S> {
    inner: S,
    policy: CrossOriginProtection,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CrossOriginService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: From<String>,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        match self.policy.check(&req) {
            Ok(()) => ResponseFuture { kind: Kind::Inner { fut: self.inner.call(req) } },
            Err(why) => {
                let mut res = Response::new(ResBody::from(format!("{{\"error\":\"{why}\"}}")));
                *res.status_mut() = StatusCode::FORBIDDEN;
                res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                ResponseFuture { kind: Kind::Refused { res: Some(res) } }
            }
        }
    }
}

pin_project! {
    #[project = KindProj]
    enum Kind<F, B> {
        Inner { #[pin] fut: F },
        Refused { res: Option<Response<B>> },
    }
}

pin_project! {
    /// Future returned by [`CrossOriginService`].
    pub struct ResponseFuture<F, B> {
        #[pin]
        kind: Kind<F, B>,
    }
}

impl<F, B, E> Future for ResponseFuture<F, B>
where
    F: Future<Output = Result<Response<B>, E>>,
{
    type Output = Result<Response<B>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project().kind.project() {
            KindProj::Inner { fut } => Poll::Ready(ready!(fut.poll(cx))),
            KindProj::Refused { res } => Poll::Ready(Ok(res.take().expect("polled after completion"))),
        }
    }
}
