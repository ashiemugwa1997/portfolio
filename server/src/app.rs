use std::any::Any;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use contact_guard::{Form, Submission, Verdict};
use serde_json::json;
use tower_bulwark::{Bulwark, ClientIp, Rejection, RequestView};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use windowgate::Limiter;

use crate::mail::Notifier;
use crate::static_site::StaticSite;
use crate::store::Store;

/// Wasm needs this on top of the strict default.
const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; \
font-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub struct Options {
    pub static_dir: PathBuf,
    pub trust_proxy: bool,
    pub https: bool,
    /// requests per minute per visitor address
    pub rate_limit: usize,
}

pub struct App<S> {
    store: S,
    form: Form,
    notifier: Notifier,
    per_ip: Limiter<IpAddr>,
    site_daily: Limiter<()>,
}

impl<S: Store> App<S> {
    pub fn new(store: S, notifier: Notifier) -> Self {
        App {
            store,
            form: Form::contact(),
            notifier,
            per_ip: Limiter::new(3, Duration::from_secs(3600)),
            site_daily: Limiter::new(50, Duration::from_secs(86_400)),
        }
    }

    pub fn router(self, opts: &Options) -> Router {
        let mut guard = Bulwark::builder()
            .csp(CSP)
            .trust_proxy(opts.trust_proxy)
            .rate_limit(opts.rate_limit.max(1), Duration::from_secs(60))
            .rule(deny_dotfiles);
        if opts.https {
            guard = guard.hsts(Duration::from_secs(63_072_000));
        }

        let api = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/api/profile", get(profile::<S>))
            .route("/api/contact", post(contact::<S>))
            .route("/api/*rest", get(not_found).post(not_found))
            .with_state(Arc::new(self));

        // each Router::layer wraps the ones before it: the last added runs first
        api.merge(static_site(&opts.static_dir))
            .layer(CompressionLayer::new().gzip(true).br(true))
            .layer(guard.build())
            .layer(RequestBodyLimitLayer::new(16 * 1024))
            .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(15)))
            .layer(CatchPanicLayer::custom(panic_response))
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Not found.")
}

async fn profile<S: Store>(State(app): State<Arc<App<S>>>, headers: HeaderMap) -> Response {
    let rendered = match app.store.profile().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("profile: {e}");
            return error(StatusCode::SERVICE_UNAVAILABLE, "The profile is unavailable right now.");
        }
    };
    let etag = HeaderValue::from_str(&rendered.etag).expect("hex etag");
    if headers.get(header::IF_NONE_MATCH) == Some(&etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            (header::ETAG, etag),
        ],
        Body::from(rendered.body.clone()),
    )
        .into_response()
}

async fn contact<S: Store>(
    State(app): State<Arc<App<S>>>,
    ip: Option<Extension<ClientIp>>,
    body: Result<Json<Submission>, JsonRejection>,
) -> Response {
    let sub = match body {
        Ok(Json(s)) => s,
        Err(JsonRejection::MissingJsonContentType(_)) => return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON."),
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => return error(StatusCode::PAYLOAD_TOO_LARGE, "That message is too long."),
        Err(_) => return error(StatusCode::BAD_REQUEST, "That message could not be read."),
    };
    let msg = match app.form.check(sub) {
        Verdict::Accept(m) => m,
        Verdict::Bot => return (StatusCode::ACCEPTED, Json(json!({ "status": "received" }))).into_response(),
        Verdict::Reject(p) => {
            let (field, text) = p.first().expect("rejected with problems");
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": text, "field": field.as_str() }))).into_response();
        }
    };
    let ip = ip.map_or(IpAddr::from([0, 0, 0, 0]), |Extension(ClientIp(ip))| ip);
    if !app.per_ip.allow(&ip) || !app.site_daily.allow(&()) {
        return error(StatusCode::TOO_MANY_REQUESTS, "Thanks, I have your messages. Please try again later.");
    }
    if let Err(e) = app.store.save_message(&msg, time::OffsetDateTime::now_utc()).await {
        tracing::error!("contact store: {e}");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Your message could not be saved. Please email me instead.");
    }
    app.notifier.notify(msg);
    (StatusCode::ACCEPTED, Json(json!({ "status": "received" }))).into_response()
}

// ── Static site ─────────────────────────────────────────────────────────────

fn static_site(dir: &Path) -> Router {
    let site = StaticSite::load(dir);
    tracing::info!("site: {} files held in memory", site.len());
    Router::new().fallback(crate::static_site::serve).with_state(Arc::new(site))
}

fn deny_dotfiles(r: &RequestView<'_>) -> Result<(), Rejection> {
    if r.uri.path().split('/').any(|seg| seg.starts_with('.') || seg.starts_with("%2e") || seg.starts_with("%2E")) {
        Err(Rejection::new(StatusCode::NOT_FOUND, "Not found."))
    } else {
        Ok(())
    }
}

#[allow(clippy::needless_pass_by_value)] // the signature CatchPanicLayer asks for
fn panic_response(err: Box<dyn Any + Send + 'static>) -> Response<String> {
    let detail = err.downcast_ref::<&str>().map(|s| (*s).to_string()).or_else(|| err.downcast_ref::<String>().cloned());
    tracing::error!("handler panicked: {}", detail.unwrap_or_default());
    let mut res = Response::new(r#"{"error":"Something went wrong."}"#.to_string());
    *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    res
}
