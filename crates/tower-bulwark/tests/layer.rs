use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use http::{Method, Request, Response, StatusCode};
use tower::{service_fn, Layer, ServiceExt};
use tower_bulwark::{Bulwark, ClientIp, Rejection, RequestView};

/// echoes the client ip the layer worked out
fn app() -> impl tower::Service<Request<()>, Response = Response<String>, Error = Infallible> + Clone {
    service_fn(|req: Request<()>| async move {
        let ip = req.extensions().get::<ClientIp>().map(|c| c.0.to_string()).unwrap_or_default();
        Ok::<_, Infallible>(Response::new(ip))
    })
}

fn req(method: Method, peer: &str) -> http::request::Builder {
    let addr: SocketAddr = peer.parse().unwrap();
    let mut b = Request::builder().method(method).uri("/x").header("host", "site.test");
    b.extensions_mut().unwrap().insert(addr);
    b
}

#[tokio::test]
async fn adds_security_headers_but_keeps_the_handlers_own() {
    let svc = Bulwark::default().layer(service_fn(|_req: Request<()>| async {
        let mut res = Response::new(String::new());
        res.headers_mut().insert("x-frame-options", "SAMEORIGIN".parse().unwrap());
        Ok::<_, Infallible>(res)
    }));
    let res = svc.oneshot(req(Method::GET, "10.0.0.1:1").body(()).unwrap()).await.unwrap();
    let h = res.headers();
    assert!(h["content-security-policy"].to_str().unwrap().contains("frame-ancestors 'none'"));
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["cross-origin-opener-policy"], "same-origin");
    assert_eq!(h["x-frame-options"], "SAMEORIGIN");
    assert!(h.get("strict-transport-security").is_none(), "no HSTS unless asked");
}

#[tokio::test]
async fn hsts_when_configured() {
    let svc = Bulwark::builder().hsts(Duration::from_secs(600)).build().layer(app());
    let res = svc.oneshot(req(Method::GET, "10.0.0.1:1").body(()).unwrap()).await.unwrap();
    assert_eq!(res.headers()["strict-transport-security"], "max-age=600; includeSubDomains");
}

#[tokio::test]
async fn cross_site_post_is_refused() {
    let layer = Bulwark::default();
    let cases = [
        (Some("https://site.test"), None, StatusCode::OK),
        (Some("https://evil.test"), Some("same-origin"), StatusCode::OK), // Sec-Fetch-Site wins
        (Some("https://site.test"), Some("same-site"), StatusCode::FORBIDDEN), // sibling subdomain
        (Some("https://evil.test"), None, StatusCode::FORBIDDEN),
        (Some("https://site.test.evil.test"), None, StatusCode::FORBIDDEN),
        (Some("null"), None, StatusCode::FORBIDDEN),
        (None, Some("cross-site"), StatusCode::FORBIDDEN),
        (None, Some("same-origin"), StatusCode::OK),
        (None, None, StatusCode::OK),
    ];
    for (origin, fetch_site, want) in cases {
        let mut b = req(Method::POST, "10.0.0.1:1");
        if let Some(o) = origin {
            b = b.header("origin", o);
        }
        if let Some(s) = fetch_site {
            b = b.header("sec-fetch-site", s);
        }
        let res = layer.layer(app()).oneshot(b.body(()).unwrap()).await.unwrap();
        assert_eq!(res.status(), want, "origin {origin:?} sec-fetch-site {fetch_site:?}");
        if want == StatusCode::FORBIDDEN {
            assert_eq!(res.headers()["content-type"], "application/json");
            assert!(res.headers().contains_key("content-security-policy"), "refusals carry headers too");
        }
    }
}

#[tokio::test]
async fn get_is_never_origin_checked() {
    let res = Bulwark::default()
        .layer(app())
        .oneshot(req(Method::GET, "10.0.0.1:1").header("origin", "https://evil.test").body(()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn forwarded_for_is_ignored_unless_trusted() {
    let spoof = || req(Method::GET, "10.0.0.1:1").header("x-forwarded-for", "1.2.3.4").body(()).unwrap();
    let body = |r: Response<String>| r.into_body();

    let plain = Bulwark::default().layer(app()).oneshot(spoof()).await.unwrap();
    assert_eq!(body(plain), "10.0.0.1");

    let behind_proxy = Bulwark::builder().trust_proxy(true).build();
    let r = behind_proxy
        .layer(app())
        .oneshot(req(Method::GET, "10.0.0.1:1").header("x-forwarded-for", "6.6.6.6, 203.0.113.7").body(()).unwrap())
        .await
        .unwrap();
    assert_eq!(body(r), "203.0.113.7", "the entry the proxy appended, not the client's claim");
}

#[tokio::test]
async fn rate_limit_answers_429_with_retry_after() {
    let layer = Bulwark::builder().rate_limit(2, Duration::from_secs(60)).build();
    for _ in 0..2 {
        let r = layer.layer(app()).oneshot(req(Method::GET, "10.0.0.9:1").body(()).unwrap()).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    let r = layer.layer(app()).oneshot(req(Method::GET, "10.0.0.9:1").body(()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() >= 1);

    let other = layer.layer(app()).oneshot(req(Method::GET, "10.0.0.10:1").body(()).unwrap()).await.unwrap();
    assert_eq!(other.status(), StatusCode::OK, "limits are per address");
}

#[tokio::test]
async fn custom_rules_plug_in() {
    let deny_dotfiles = |r: &RequestView<'_>| {
        if r.uri.path().split('/').any(|s| s.starts_with('.')) {
            Err(Rejection::new(StatusCode::NOT_FOUND, "Not found."))
        } else {
            Ok(())
        }
    };
    let layer = Bulwark::builder().rule(deny_dotfiles).build();
    let r = layer
        .layer(app())
        .oneshot(Request::builder().uri("/.env").header("host", "site.test").body(()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(r.into_body(), r#"{"error":"Not found."}"#);
}

#[tokio::test]
async fn custom_peer_lookup() {
    #[derive(Clone)]
    struct Peer(IpAddr);
    let layer = Bulwark::builder().peer(|ext| ext.get::<Peer>().map(|p| p.0)).build();
    let mut r = Request::builder().uri("/").header("host", "site.test").body(()).unwrap();
    r.extensions_mut().insert(Peer("192.0.2.5".parse().unwrap()));
    let res = layer.layer(app()).oneshot(r).await.unwrap();
    assert_eq!(res.into_body(), "192.0.2.5");
}
