//! Random requests against the real router: nothing a client sends may cause a 5xx or a panic.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use proptest::prelude::*;
use tower::ServiceExt;

use crate::app::{App, Options};
use crate::mail::Notifier;
use crate::profile::{Profile, DEFAULT_PROFILE};
use crate::store::MemoryStore;

fn body() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        2 => any::<Vec<u8>>(),
        2 => "\\{\"name\":\"[^\"]{0,30}\",\"email\":\"[^\"]{0,30}\",\"message\":\"[^\"]{0,60}\"(,\"website\":\"[a-z]{0,5}\")?\\}".prop_map(String::into_bytes),
        1 => "\\{(\"[a-z]{1,8}\":(null|[0-9]{1,4}|true|\\[\\]|\\{\\}|\"[a-z]{0,8}\"),?){0,6}\\}".prop_map(String::into_bytes),
    ]
}

fn path() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("/api/contact".to_string()),
        Just("/api/profile".to_string()),
        "/[a-zA-Z0-9._%~-]{0,20}(/[a-zA-Z0-9._%~-]{0,12}){0,3}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn no_request_breaks_the_server(
        method in prop_oneof![Just("GET"), Just("POST"), Just("HEAD"), Just("PUT"), Just("DELETE")],
        path in path(),
        body in body(),
        json in any::<bool>(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let status = rt.block_on(async {
            let store = Arc::new(MemoryStore::with(&Profile::from_json(DEFAULT_PROFILE).unwrap()));
            let opts = Options { static_dir: std::env::temp_dir().join("pf-fuzz-none"), trust_proxy: false, https: false, rate_limit: 1_000_000 };
            let app = App::new(store, Notifier::off(), b"fuzz-salt-fuzz-salt-fuzz-salt-32".to_vec()).router(&opts);
            let mut b = Request::builder().method(method).uri(&path).header("host", "site.test").header("origin", "http://site.test");
            if json {
                b = b.header("content-type", "application/json");
            }
            b.extensions_mut().unwrap().insert::<SocketAddr>("10.0.0.1:1".parse().unwrap());
            let Ok(req) = b.body(Body::from(body)) else { return StatusCode::OK };
            app.oneshot(req).await.unwrap().status()
        });
        prop_assert!(!status.is_server_error(), "{} {} -> {}", method, path, status);
    }
}
