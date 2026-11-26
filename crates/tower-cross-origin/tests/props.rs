//! Random headers against an independent statement of the rules.
use std::convert::Infallible;

use http::{HeaderValue, Method, Request, Response, StatusCode};
use proptest::prelude::*;
use tower::{service_fn, Layer, ServiceExt};
use tower_cross_origin::{CrossOriginLayer, CrossOriginProtection};

fn value() -> impl Strategy<Value = String> {
    prop_oneof!["[ -~]{0,30}", "https?://[a-z.]{1,12}(:[0-9]{1,5})?", Just("null".into())]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn writes_follow_the_rules(
        method in prop_oneof![Just(Method::GET), Just(Method::POST), Just(Method::PUT), Just(Method::DELETE), Just(Method::HEAD)],
        host in "[a-z.]{1,12}(:[0-9]{1,5})?",
        origin in prop::option::of(value()),
        site in prop::option::of("same-origin|same-site|cross-site|none|[a-z-]{0,10}"),
        trusted in prop::option::of("https?://[a-z.]{1,12}"),
    ) {
        let mut g = CrossOriginProtection::new();
        if let Some(t) = &trusted { g = g.trust_origin(t); }
        let mut b = Request::builder().method(method.clone()).uri("/x").header("host", &host);
        if let Some(o) = origin.as_deref().and_then(|o| HeaderValue::from_str(o).ok()) { b = b.header("origin", o); }
        if let Some(s) = &site { b = b.header("sec-fetch-site", s.as_str()); }
        let req = b.body(()).unwrap();

        let sent = req.headers().get("origin").and_then(|v| v.to_str().ok());
        let is_trusted = sent.zip(trusted.as_deref()).is_some_and(|(o, t)| o.eq_ignore_ascii_case(t));
        let same_host = sent
            .and_then(|o| o.strip_prefix("https://").or_else(|| o.strip_prefix("http://")))
            .is_some_and(|h| !h.is_empty() && !h.contains('/') && h.eq_ignore_ascii_case(&host));
        let safe = matches!(method, Method::GET | Method::HEAD);
        let want = safe || match site.as_deref() {
            Some("same-origin" | "none") => true,
            Some(_) => is_trusted,
            None => sent.is_none() || is_trusted || same_host,
        };
        prop_assert_eq!(g.check(&req).is_ok(), want);
    }
}

#[tokio::test]
async fn layer_answers_403_as_json() {
    let svc = CrossOriginLayer::new(CrossOriginProtection::new())
        .layer(service_fn(|_r: Request<()>| async { Ok::<_, Infallible>(Response::new(String::from("ok"))) }));
    let evil = Request::post("/x").header("host", "a.example").header("sec-fetch-site", "cross-site").body(()).unwrap();
    let res = svc.clone().oneshot(evil).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(res.headers()["content-type"], "application/json");
    assert_eq!(res.into_body(), r#"{"error":"cross-site request refused"}"#);
    let own = Request::post("/x").header("host", "a.example").header("sec-fetch-site", "same-origin").body(()).unwrap();
    assert_eq!(svc.oneshot(own).await.unwrap().into_body(), "ok");
}
