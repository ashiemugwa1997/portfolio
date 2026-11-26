//! Random headers against the same-origin rule and the client address resolver.
use http::{HeaderMap, HeaderValue, Method, Uri};
use proptest::prelude::*;
use tower_bulwark::{ClientIpResolver, RequestView, Rule, SameOriginWrites};

fn header_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ -~]{0,40}",
        "https?://[a-z.]{1,15}(:[0-9]{1,5})?(/[a-z]{0,5})?",
        Just("null".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// a write is let through only when Origin names exactly this host (or there is no
    /// Origin and the browser did not say cross-site)
    #[test]
    fn writes_pass_only_from_this_host(origin in prop::option::of(header_text()), host in "[a-z.]{1,15}(:[0-9]{1,5})?", fetch_site in prop::option::of("cross-site|same-site|same-origin|none|[a-z]{0,8}")) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_str(&host).unwrap());
        if let Some(o) = &origin {
            if let Ok(v) = HeaderValue::from_str(o) { headers.insert("origin", v); }
        }
        if let Some(s) = &fetch_site {
            headers.insert("sec-fetch-site", HeaderValue::from_str(s).unwrap());
        }
        let uri: Uri = "/x".parse().unwrap();
        let view = RequestView { method: &Method::POST, uri: &uri, headers: &headers, client_ip: None };
        let allowed = SameOriginWrites::default().check(&view).is_ok();

        let sent_origin = headers.get("origin").and_then(|v| v.to_str().ok());
        let same_host = sent_origin
            .and_then(|o| o.strip_prefix("https://").or_else(|| o.strip_prefix("http://")))
            .is_some_and(|h| !h.is_empty() && !h.contains('/') && h.eq_ignore_ascii_case(&host));
        let expected = match fetch_site.as_deref() {
            Some("same-origin" | "none") => true,
            Some(_) => false,
            None => sent_origin.is_none() || same_host,
        };
        prop_assert_eq!(allowed, expected);
    }

    /// whatever a client puts in X-Forwarded-For, an untrusting resolver ignores it,
    /// and a trusting one takes at most the last entry
    #[test]
    fn forwarded_for_never_overrides_untrusted(xff in "[0-9a-f.:, ]{0,60}") {
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&xff) { headers.insert("x-forwarded-for", v); }
        let mut ext = http::Extensions::new();
        let peer: std::net::SocketAddr = "10.9.8.7:1".parse().unwrap();
        ext.insert(peer);
        prop_assert_eq!(ClientIpResolver::default().resolve(&headers, &ext), Some(peer.ip()));

        let trusted = ClientIpResolver::default().trust_forwarded(true).resolve(&headers, &ext);
        let last = xff.split(',').next_back().and_then(|s| s.trim().parse::<std::net::IpAddr>().ok());
        prop_assert_eq!(trusted, last.or(Some(peer.ip())));
    }
}
