use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use http::{header, HeaderValue, Request, Response};
use pin_project_lite::pin_project;
use tower_service::Service;

use crate::rules::{Rejection, RequestView};
use crate::{ClientIp, Shared};

/// The service the layer wraps around yours.
#[derive(Clone)]
pub struct BulwarkService<S> {
    inner: S,
    shared: Arc<Shared>,
}

impl<S> BulwarkService<S> {
    pub(crate) fn new(inner: S, shared: Arc<Shared>) -> Self {
        BulwarkService { inner, shared }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for BulwarkService<S>
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

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let ip = self.shared.resolver.resolve(req.headers(), req.extensions());
        if let Some(ip) = ip {
            req.extensions_mut().insert(ClientIp(ip));
        }
        let view = RequestView { method: req.method(), uri: req.uri(), headers: req.headers(), client_ip: ip };
        let refused = self.shared.rules.iter().find_map(|r| r.check(&view).err());

        let kind = match refused {
            Some(rejection) => Kind::Refused { res: Some(rejection_response(&rejection)) },
            None => Kind::Inner { fut: self.inner.call(req) },
        };
        ResponseFuture { kind, shared: self.shared.clone() }
    }
}

fn rejection_response<B: From<String>>(r: &Rejection) -> Response<B> {
    let mut res = Response::new(B::from(r.body()));
    *res.status_mut() = r.status;
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(after) = r.retry_after {
        res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(after.as_secs()));
    }
    res
}

pin_project! {
    #[project = KindProj]
    enum Kind<F, B> {
        Inner { #[pin] fut: F },
        Refused { res: Option<Response<B>> },
    }
}

pin_project! {
    /// Future returned by [`BulwarkService`].
    pub struct ResponseFuture<F, B> {
        #[pin]
        kind: Kind<F, B>,
        shared: Arc<Shared>,
    }
}

impl<F, B, E> Future for ResponseFuture<F, B>
where
    F: Future<Output = Result<Response<B>, E>>,
{
    type Output = Result<Response<B>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut res = match this.kind.project() {
            KindProj::Inner { fut } => ready!(fut.poll(cx))?,
            KindProj::Refused { res } => res.take().expect("polled after completion"),
        };
        this.shared.headers.apply(res.headers_mut());
        Poll::Ready(Ok(res))
    }
}
