use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::{App, Options};
use crate::mail::Notifier;
use crate::profile::{Profile, DEFAULT_PROFILE};
use crate::store::MemoryStore;

struct Site {
    router: Router,
    store: Arc<MemoryStore>,
    dir: TempDir,
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn site() -> Site {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("pf-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>home</title>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
    let store = Arc::new(MemoryStore::with(&Profile::from_json(DEFAULT_PROFILE).unwrap()));
    let opts = Options { static_dir: dir.clone(), trust_proxy: false, https: false, rate_limit: 120 };
    let router = App::new(store.clone(), Notifier::off(), b"test-salt".to_vec()).router(&opts);
    Site { router, store, dir: TempDir(dir) }
}

fn req(method: &str, uri: &str, peer: &str) -> axum::http::request::Builder {
    let mut b = Request::builder().method(method).uri(uri).header("host", "site.test");
    b.extensions_mut().unwrap().insert::<SocketAddr>(peer.parse().unwrap());
    b
}

fn get(uri: &str) -> Request<Body> {
    req("GET", uri, "10.0.0.1:1").body(Body::empty()).unwrap()
}

async fn send(s: &Site, r: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let res = s.router.clone().oneshot(r).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned())
}

fn contact(body: &str) -> Request<Body> {
    req("POST", "/api/contact", "10.0.0.1:5000")
        .header("content-type", "application/json")
        .header("origin", "http://site.test")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn profile_comes_from_the_store_with_etag() {
    let s = site();
    let (status, h, body) = send(&s, get("/api/profile")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"name\":\"Ashley Mugwambi\""));
    let etag = h["etag"].to_str().unwrap().to_owned();
    let again = req("GET", "/api/profile", "10.0.0.1:1").header("if-none-match", &etag).body(Body::empty()).unwrap();
    assert_eq!(send(&s, again).await.0, StatusCode::NOT_MODIFIED);

    let mut p = Profile::from_json(DEFAULT_PROFILE).unwrap();
    p.tagline = "Changed in the database".into();
    s.store.set(&p);
    let (_, _, body) = send(&s, get("/api/profile")).await;
    assert!(body.contains("Changed in the database"));
}

#[tokio::test]
async fn every_response_has_security_headers() {
    let s = site();
    for uri in ["/", "/api/profile", "/healthz", "/assets/app.js", "/api/nothing"] {
        let (_, h, _) = send(&s, get(uri)).await;
        assert!(h["content-security-policy"].to_str().unwrap().contains("'wasm-unsafe-eval'"), "{uri}");
        assert_eq!(h["x-content-type-options"], "nosniff", "{uri}");
        assert_eq!(h["x-frame-options"], "DENY", "{uri}");
    }
}

#[tokio::test]
async fn valid_message_is_saved() {
    let s = site();
    let (status, _, _) = send(&s, contact(r#"{"name":" Rudo ","email":"rudo@example.com","message":"Hello"}"#)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let saved = s.store.messages.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].0, "Rudo");
}

#[tokio::test]
async fn bad_messages_are_refused() {
    let s = site();
    let cases = [
        (r#"{"name":"","email":"a@b.co","message":"x"}"#, StatusCode::UNPROCESSABLE_ENTITY),
        (r#"{"name":"A","email":"A <a@b.co>","message":"x"}"#, StatusCode::UNPROCESSABLE_ENTITY),
        (r#"{"name":"A\r\nBcc: v@x.co","email":"a@b.co","message":"x"}"#, StatusCode::UNPROCESSABLE_ENTITY),
        (r#"{"name":"A","email":"a@b.co","message":"x","admin":true}"#, StatusCode::BAD_REQUEST),
        ("not json", StatusCode::BAD_REQUEST),
    ];
    for (body, want) in cases {
        assert_eq!(send(&s, contact(body)).await.0, want, "{body}");
    }
    let plain = req("POST", "/api/contact", "10.0.0.1:1").header("origin", "http://site.test").body(Body::from("{}")).unwrap();
    assert_eq!(send(&s, plain).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(s.store.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn honeypot_is_accepted_but_not_saved() {
    let s = site();
    let (status, _, _) = send(&s, contact(r#"{"name":"Bot","email":"b@b.co","message":"buy","website":"x"}"#)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(s.store.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cross_site_post_is_refused() {
    let s = site();
    let r = req("POST", "/api/contact", "10.0.0.1:1")
        .header("content-type", "application/json")
        .header("origin", "https://evil.test")
        .body(Body::from(r#"{"name":"A","email":"a@b.co","message":"x"}"#))
        .unwrap();
    assert_eq!(send(&s, r).await.0, StatusCode::FORBIDDEN);
    assert!(s.store.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn three_messages_an_hour_per_address() {
    let s = site();
    let ok = r#"{"name":"A","email":"a@b.co","message":"hi"}"#;
    for _ in 0..3 {
        assert_eq!(send(&s, contact(ok)).await.0, StatusCode::ACCEPTED);
    }
    assert_eq!(send(&s, contact(ok)).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn oversized_body_is_refused() {
    let s = site();
    let big = format!(r#"{{"name":"A","email":"a@b.co","message":"{}"}}"#, "x".repeat(20_000));
    assert_eq!(send(&s, contact(&big)).await.0, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn static_files_spa_fallback_and_no_dotfiles() {
    let s = site();
    let (st, h, _) = send(&s, get("/assets/app.js")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h["cache-control"].to_str().unwrap().contains("immutable"));
    for page in ["/", "/experience"] {
        let (st, _, body) = send(&s, get(page)).await;
        assert_eq!(st, StatusCode::OK, "{page}");
        assert!(body.contains("<title>home</title>"), "{page}");
    }
    for bad in ["/.env", "/%2eenv", "/assets/../.env"] {
        let (st, _, body) = send(&s, get(bad)).await;
        assert!(!body.contains("SECRET"), "{bad} leaked the file");
        assert_ne!(st, StatusCode::OK, "{bad}");
    }
}

#[tokio::test]
async fn store_down_gives_503_not_a_crash() {
    let s = site();
    s.store.clear();
    let (st, _, body) = send(&s, get("/api/profile")).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("unavailable"));
}

#[tokio::test]
async fn limits_survive_a_restart() {
    let s = site();
    let ok = r#"{"name":"A","email":"a@b.co","message":"hi"}"#;
    for _ in 0..3 {
        assert_eq!(send(&s, contact(ok)).await.0, StatusCode::ACCEPTED);
    }
    // a fresh server on the same database: in-memory limits are empty, the database still counts
    let opts = Options { static_dir: s.dir.0.clone(), trust_proxy: false, https: false, rate_limit: 120 };
    let restarted = Site { router: App::new(s.store.clone(), Notifier::off(), b"test-salt".to_vec()).router(&opts), store: s.store.clone(), dir: TempDir(PathBuf::new()) };
    assert_eq!(send(&restarted, contact(ok)).await.0, StatusCode::TOO_MANY_REQUESTS);
    let saved = s.store.messages.lock().unwrap();
    assert_eq!(saved.len(), 3);
    assert!(saved.iter().all(|m| m.4.is_some()), "every message carries a sender hash");
}
