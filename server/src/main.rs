//! portfolio: serves the site, the profile from MySQL and the contact form.
//!
//!   portfolio                 run the server (settings from .env)
//!   portfolio seed [FILE]     load content into MySQL (default: content/profile.json)
//!   portfolio migrate         add missing tables and columns, content untouched

mod app;
mod config;
mod mail;
mod profile;
mod static_site;
mod store;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod fuzz_tests;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::Request;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tower::ServiceExt;

use crate::app::{App, Options};
use crate::config::Config;
use crate::mail::{Mailer, Notifier, Smtp};
use crate::profile::{Profile, DEFAULT_PROFILE};
use crate::store::{Cached, MySqlStore, Store};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let cfg = Config::load(".env");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("seed") => seed(&cfg, args.get(1).map(String::as_str)).await,
        Some("migrate") => migrate(&cfg).await,
        None | Some("serve") => serve(cfg).await,
        Some(other) => Err(format!("unknown command {other:?}: use `portfolio`, `portfolio seed [FILE]` or `portfolio migrate`")),
    };
    if let Err(e) = result {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

async fn seed(cfg: &Config, file: Option<&str>) -> Result<(), String> {
    let raw = match file {
        None | Some("default") => DEFAULT_PROFILE.to_owned(),
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?,
    };
    let profile = Profile::from_json(&raw)?;
    let url = cfg.admin_dsn.as_deref().ok_or("set MYSQL_ADMIN_DSN in .env")?;
    let store = MySqlStore::connect(url, 2).await.map_err(|e| e.to_string())?;
    store.seed(&profile).await.map_err(|e| e.to_string())?;
    tracing::info!("seeded: {} jobs, {} case studies, {} projects", profile.experience.len(), profile.cases.len(), profile.projects.len());
    Ok(())
}

async fn migrate(cfg: &Config) -> Result<(), String> {
    let url = cfg.admin_dsn.as_deref().ok_or("set MYSQL_ADMIN_DSN in .env")?;
    let store = MySqlStore::connect(url, 2).await.map_err(|e| e.to_string())?;
    store.migrate().await.map_err(|e| e.to_string())?;
    tracing::info!("schema up to date");
    Ok(())
}

async fn serve(cfg: Config) -> Result<(), String> {
    let url = cfg.dsn.as_deref().ok_or("set MYSQL_DSN in .env")?;
    let store = Arc::new(Cached::new(MySqlStore::connect(url, 10).await.map_err(|e| e.to_string())?, Duration::from_secs(30)));
    store.profile().await.map_err(|e| e.to_string())?;
    config::import_old_messages(store.as_ref(), "data/messages.jsonl").await;

    let notifier = if let Some(s) = cfg.smtp() {
        let mailer: Arc<dyn Mailer> = Arc::new(Smtp::new(&s.host, s.port, &s.user, &s.pass, &s.from, &s.to)?);
        tracing::info!("email on: messages are also emailed");
        Notifier::new(Some(mailer))
    } else {
        tracing::info!("email off: set SMTP_USER and SMTP_PASS in .env to get messages by email");
        Notifier::off()
    };

    let opts = Options { static_dir: cfg.static_dir.clone(), trust_proxy: cfg.trust_proxy, https: cfg.https, rate_limit: cfg.rate_limit };
    let salt = match &cfg.ip_salt {
        Some(s) if s.len() >= 32 => s.clone().into_bytes(),
        Some(_) => return Err("IP_HASH_SALT must be at least 32 characters (openssl rand -hex 32)".into()),
        None => return Err("set IP_HASH_SALT in .env (openssl rand -hex 32): it keeps sender hashes unlinkable".into()),
    };
    let app = App::new(store, notifier, salt).router(&opts);
    let listener = TcpListener::bind(&cfg.addr).await.map_err(|e| format!("{}: {e}", cfg.addr))?;
    tracing::info!("portfolio on http://{} (site {})", cfg.addr, cfg.static_dir.display());

    // bounded connections + header deadline: slow or idle clients cannot pin the server
    let slots = Arc::new(Semaphore::new(cfg.max_connections));
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(e) => { tracing::warn!("accept: {e}"); continue; }
            },
            _ = tokio::signal::ctrl_c() => { tracing::info!("shutting down"); return Ok(()); }
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |mut req: Request<Incoming>| {
                req.extensions_mut().insert::<SocketAddr>(peer);
                app.clone().oneshot(req)
            });
            let conn = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_buf_size(64 * 1024)
                .serve_connection(TokioIo::new(stream), svc);
            let _ = tokio::time::timeout(Duration::from_secs(300), conn).await;
            drop(permit);
        });
    }
}
