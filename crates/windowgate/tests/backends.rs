use std::time::Duration;

use windowgate::{Backend, Limiter};

#[tokio::test]
async fn memory_limiter_is_a_backend() {
    let b: Limiter<String> = Limiter::new(2, Duration::from_secs(60));
    assert!(b.hit("k").await.unwrap().is_allowed());
    assert!(b.hit("k").await.unwrap().is_allowed());
    assert!(!b.hit("k").await.unwrap().is_allowed());
}

#[cfg(feature = "tower")]
mod layer {
    use super::*;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use http::{Request, Response, StatusCode};
    use tower::{service_fn, Layer, ServiceExt};
    use windowgate::{BackendError, HitFuture, OnError, RateLimitLayer};

    fn app() -> tower::util::BoxCloneService<Request<()>, Response<String>, Infallible> {
        tower::util::BoxCloneService::new(service_fn(|_req: Request<()>| async { Ok::<_, Infallible>(Response::new("ok".to_owned())) }))
    }

    fn from(ip: &str) -> Request<()> {
        let mut r = Request::builder().uri("/").body(()).unwrap();
        r.extensions_mut().insert::<SocketAddr>(format!("{ip}:1").parse().unwrap());
        r
    }

    #[tokio::test]
    async fn answers_429_with_retry_after_per_ip() {
        let layer = RateLimitLayer::new(Limiter::<String>::new(2, Duration::from_secs(30)));
        for _ in 0..2 {
            assert_eq!(layer.layer(app()).oneshot(from("10.0.0.1")).await.unwrap().status(), StatusCode::OK);
        }
        let r = layer.layer(app()).oneshot(from("10.0.0.1")).await.unwrap();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        let secs: u64 = r.headers()["retry-after"].to_str().unwrap().parse().unwrap();
        assert!((1..=30).contains(&secs));
        assert_eq!(layer.layer(app()).oneshot(from("10.0.0.2")).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn custom_keys() {
        let layer = RateLimitLayer::new(Limiter::<String>::new(1, Duration::from_secs(30)))
            .key(|_, h| h.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_owned));
        let with = |k: &str| Request::builder().uri("/").header("x-api-key", k).body(()).unwrap();
        assert_eq!(layer.layer(app()).oneshot(with("a")).await.unwrap().status(), StatusCode::OK);
        assert_eq!(layer.layer(app()).oneshot(with("a")).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(layer.layer(app()).oneshot(with("b")).await.unwrap().status(), StatusCode::OK);
    }

    struct Broken;
    impl Backend for Broken {
        fn hit<'a>(&'a self, _key: &'a str) -> HitFuture<'a> {
            Box::pin(async { Err(BackendError::new("database down")) })
        }
    }

    #[tokio::test]
    async fn backend_errors_follow_the_policy() {
        let open = RateLimitLayer::new(Broken);
        assert_eq!(open.layer(app()).oneshot(from("10.0.0.1")).await.unwrap().status(), StatusCode::OK);
        let closed = RateLimitLayer::new(Broken).on_error(OnError::Deny);
        assert_eq!(closed.layer(app()).oneshot(from("10.0.0.1")).await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

/// against a real MySQL: TEST_MYSQL_URL=mysql://root@127.0.0.1:3306/portfolio (any database you may create tables in)
#[cfg(feature = "mysql")]
mod mysql {
    use super::*;
    use windowgate::MySqlBackend;

    async fn backend(table: &str, limit: usize, window: Duration) -> Option<MySqlBackend> {
        let url = std::env::var("TEST_MYSQL_URL").ok()?;
        let opts: sqlx::mysql::MySqlConnectOptions = url.parse().unwrap();
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(8)
            .connect_with(opts.ssl_mode(sqlx::mysql::MySqlSslMode::Required))
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE IF EXISTS `{table}`"))).execute(&pool).await.unwrap();
        let b = MySqlBackend::new(pool, table, limit, window).unwrap();
        b.create_table().await.unwrap();
        Some(b)
    }

    #[tokio::test]
    async fn sliding_window_in_mysql() {
        let Some(b) = backend("wg_test_window", 2, Duration::from_millis(1500)).await else {
            eprintln!("skipped: set TEST_MYSQL_URL");
            return;
        };
        assert!(b.hit("alice").await.unwrap().is_allowed());
        assert!(b.hit("alice").await.unwrap().is_allowed());
        let refused = b.hit("alice").await.unwrap();
        assert!(!refused.is_allowed());
        if let windowgate::Decision::Limited { retry_after } = refused {
            assert!(retry_after <= Duration::from_millis(1500));
        }
        assert!(b.hit("bob").await.unwrap().is_allowed(), "keys are independent");
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert!(b.hit("alice").await.unwrap().is_allowed(), "the window slid");
        let long = "x".repeat(1000);
        assert!(b.hit(&long).await.unwrap().is_allowed(), "long keys are folded to fit");
    }

    #[tokio::test]
    async fn concurrent_hits_never_exceed_the_limit() {
        let Some(b) = backend("wg_test_burst", 5, Duration::from_secs(60)).await else { return };
        let b = std::sync::Arc::new(b);
        let tasks: Vec<_> = (0..40)
            .map(|_| {
                let b = b.clone();
                tokio::spawn(async move { b.hit("shared").await.unwrap().is_allowed() })
            })
            .collect();
        let mut allowed = 0;
        for t in tasks {
            allowed += usize::from(t.await.unwrap());
        }
        assert!(allowed <= 5, "{allowed} allowed");
        assert!(allowed >= 1);
    }

    #[test]
    fn table_names_are_checked() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let Ok(url) = std::env::var("TEST_MYSQL_URL") else { return };
            let pool = sqlx::mysql::MySqlPoolOptions::new().connect_lazy(&url).unwrap();
            assert!(MySqlBackend::new(pool.clone(), "hits; DROP TABLE x", 1, Duration::from_secs(1)).is_err());
            assert!(MySqlBackend::new(pool, "ok_name_1", 1, Duration::from_secs(1)).is_ok());
        });
    }
}

/// against a real Redis: TEST_REDIS_URL=redis://127.0.0.1/
#[cfg(feature = "redis")]
mod redis_backend {
    use super::*;
    use windowgate::RedisBackend;

    async fn backend(name: &str, limit: usize, window: Duration) -> Option<RedisBackend> {
        let url = std::env::var("TEST_REDIS_URL").ok()?;
        let client = redis::Client::open(url).unwrap();
        let mut c = client.get_multiplexed_async_connection().await.unwrap();
        let keys: Vec<String> = redis::cmd("KEYS").arg(format!("windowgate:{name}:*")).query_async(&mut c).await.unwrap();
        if !keys.is_empty() {
            let _: () = redis::cmd("DEL").arg(keys).query_async(&mut c).await.unwrap();
        }
        Some(RedisBackend::connect(client, name, limit, window).await.unwrap())
    }

    #[tokio::test]
    async fn sliding_window_in_redis() {
        let Some(b) = backend("t-window", 2, Duration::from_millis(1500)).await else {
            eprintln!("skipped: set TEST_REDIS_URL");
            return;
        };
        assert!(b.hit("alice").await.unwrap().is_allowed());
        assert!(b.hit("alice").await.unwrap().is_allowed());
        match b.hit("alice").await.unwrap() {
            windowgate::Decision::Limited { retry_after } => assert!(retry_after <= Duration::from_millis(1500)),
            windowgate::Decision::Allowed => panic!("third hit allowed"),
        }
        assert!(b.hit("bob").await.unwrap().is_allowed(), "keys are independent");
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert!(b.hit("alice").await.unwrap().is_allowed(), "the window slid");
    }

    #[tokio::test]
    async fn concurrent_hits_never_exceed_the_limit() {
        let Some(b) = backend("t-burst", 5, Duration::from_secs(60)).await else { return };
        let tasks: Vec<_> = (0..60)
            .map(|_| {
                let b = b.clone();
                tokio::spawn(async move { b.hit("shared").await.unwrap().is_allowed() })
            })
            .collect();
        let mut allowed = 0;
        for t in tasks {
            allowed += usize::from(t.await.unwrap());
        }
        assert_eq!(allowed, 5, "an atomic script lets exactly the limit through");
    }
}
