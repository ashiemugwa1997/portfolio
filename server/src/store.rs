use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytes::Bytes;
use contact_guard::Message;
use sha2::{Digest, Sha256};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlSslMode};
use sqlx::{AssertSqlSafe, Executor, MySql, Row, Transaction};
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};
use tokio::sync::RwLock;

use crate::profile::{Case, Education, Group, Job, Link, Metric, Profile, Project, SkillArea, Stage};

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StoreError {
    NoProfile,
    Db(sqlx::Error),
    Encode(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NoProfile => f.write_str("no profile in the database: run `portfolio seed`"),
            StoreError::Db(e) => write!(f, "database: {e}"),
            StoreError::Encode(e) => write!(f, "encode: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        StoreError::Db(e)
    }
}

// ── Store ───────────────────────────────────────────────────────────────────

/// The profile as sent to browsers: JSON body and its ETag, built once per change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub body: Bytes,
    pub etag: String,
}

impl Rendered {
    pub fn of(p: &Profile) -> Result<Self, StoreError> {
        let body = serde_json::to_vec(p).map_err(StoreError::Encode)?;
        let digest = Sha256::digest(&body);
        let etag = format!("\"{}\"", hex(&digest[..8]));
        Ok(Rendered { body: Bytes::from(body), etag })
    }
}

pub(crate) fn hex(b: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(DIGITS[usize::from(x >> 4)] as char);
        s.push(DIGITS[usize::from(x & 15)] as char);
    }
    s
}

pub trait Store: Send + Sync + 'static {
    fn profile(&self) -> StoreFuture<'_, Arc<Rendered>>;
    fn save_message(&self, m: &Message, received: OffsetDateTime) -> StoreFuture<'_, ()>;
}

impl<S: Store + ?Sized> Store for Arc<S> {
    fn profile(&self) -> StoreFuture<'_, Arc<Rendered>> {
        (**self).profile()
    }
    fn save_message(&self, m: &Message, received: OffsetDateTime) -> StoreFuture<'_, ()> {
        (**self).save_message(m, received)
    }
}

// ── Cache ───────────────────────────────────────────────────────────────────

/// Keeps the rendered profile for `ttl`, so a burst of visitors costs one query.
/// Only one reload runs at a time; everyone else gets the fresh copy it produced.
pub struct Cached<S> {
    inner: S,
    ttl: Duration,
    slot: RwLock<Option<(Instant, Arc<Rendered>)>>,
}

impl<S: Store> Cached<S> {
    pub fn new(inner: S, ttl: Duration) -> Self {
        Cached { inner, ttl, slot: RwLock::new(None) }
    }

    fn fresh(&self, slot: Option<&(Instant, Arc<Rendered>)>) -> Option<Arc<Rendered>> {
        slot.filter(|(at, _)| at.elapsed() < self.ttl).map(|(_, r)| r.clone())
    }
}

impl<S: Store> Store for Cached<S> {
    fn profile(&self) -> StoreFuture<'_, Arc<Rendered>> {
        Box::pin(async move {
            if let Some(r) = self.fresh(self.slot.read().await.as_ref()) {
                return Ok(r);
            }
            let mut slot = self.slot.write().await;
            if let Some(r) = self.fresh(slot.as_ref()) {
                return Ok(r);
            }
            match self.inner.profile().await {
                Ok(r) => {
                    *slot = Some((Instant::now(), r.clone()));
                    Ok(r)
                }
                // database hiccup: keep serving the last good copy rather than an error page
                Err(e) => match slot.as_ref() {
                    Some((_, stale)) => {
                        tracing::warn!("profile reload failed, serving the cached copy: {e}");
                        Ok(stale.clone())
                    }
                    None => Err(e),
                },
            }
        })
    }

    fn save_message(&self, m: &Message, received: OffsetDateTime) -> StoreFuture<'_, ()> {
        self.inner.save_message(m, received)
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────

/// In-process store for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    profile: Mutex<Option<Arc<Rendered>>>,
    pub messages: Mutex<Vec<(String, String, String, OffsetDateTime)>>,
    pub loads: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl MemoryStore {
    pub fn with(p: &Profile) -> Self {
        let s = MemoryStore::default();
        s.set(p);
        s
    }

    pub fn set(&self, p: &Profile) {
        *self.profile.lock().unwrap() = Some(Arc::new(Rendered::of(p).unwrap()));
    }

    pub fn clear(&self) {
        *self.profile.lock().unwrap() = None;
    }
}

#[cfg(test)]
impl Store for MemoryStore {
    fn profile(&self) -> StoreFuture<'_, Arc<Rendered>> {
        self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = self.profile.lock().unwrap().clone();
        Box::pin(async move { p.ok_or(StoreError::NoProfile) })
    }

    fn save_message(&self, m: &Message, received: OffsetDateTime) -> StoreFuture<'_, ()> {
        self.messages.lock().unwrap().push((m.name().into(), m.email().into(), m.body().into(), received));
        Box::pin(async { Ok(()) })
    }
}

// ── MySQL ───────────────────────────────────────────────────────────────────

const TABLE_OPTS: &str = " ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci";

const SCHEMA: [&str; 13] = [
    "CREATE TABLE IF NOT EXISTS profile (id TINYINT PRIMARY KEY, `name` VARCHAR(100) NOT NULL, `role` VARCHAR(100) NOT NULL, tagline VARCHAR(200) NOT NULL, location VARCHAR(100) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS summary (pos INT PRIMARY KEY, body TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS links (pos INT PRIMARY KEY, label VARCHAR(50) NOT NULL, href VARCHAR(300) NOT NULL, `text` VARCHAR(200) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS metrics (pos INT PRIMARY KEY, `value` VARCHAR(30) NOT NULL, label VARCHAR(200) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS pipeline (pos INT PRIMARY KEY, step VARCHAR(50) NOT NULL, detail VARCHAR(200) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS jobs (pos INT PRIMARY KEY, slug VARCHAR(50) NOT NULL UNIQUE, title VARCHAR(100) NOT NULL, org VARCHAR(100) NOT NULL, period VARCHAR(100) NOT NULL, intro TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS job_items (job_slug VARCHAR(50) NOT NULL, grp_pos INT NOT NULL, grp VARCHAR(100) NOT NULL, pos INT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (job_slug, grp_pos, pos), FOREIGN KEY (job_slug) REFERENCES jobs(slug) ON DELETE CASCADE)",
    "CREATE TABLE IF NOT EXISTS cases (pos INT PRIMARY KEY, slug VARCHAR(50) NOT NULL UNIQUE, title VARCHAR(150) NOT NULL, context TEXT NOT NULL, `action` TEXT NOT NULL, result TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS skills (area_pos INT NOT NULL, area VARCHAR(60) NOT NULL, pos INT NOT NULL, item VARCHAR(60) NOT NULL, PRIMARY KEY (area_pos, pos))",
    "CREATE TABLE IF NOT EXISTS projects (pos INT PRIMARY KEY, `name` VARCHAR(100) NOT NULL, detail VARCHAR(300) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS education (pos INT PRIMARY KEY, title VARCHAR(150) NOT NULL, org VARCHAR(150) NOT NULL, `year` VARCHAR(10) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS credentials (pos INT PRIMARY KEY, body VARCHAR(200) NOT NULL)",
    "CREATE TABLE IF NOT EXISTS messages (id BIGINT AUTO_INCREMENT PRIMARY KEY, `name` VARCHAR(100) NOT NULL, email VARCHAR(200) NOT NULL, body TEXT NOT NULL, received DATETIME NOT NULL, KEY (received))",
];

/// children first, so deletes respect the foreign key
const CONTENT_TABLES: [&str; 12] =
    ["job_items", "jobs", "summary", "links", "metrics", "pipeline", "cases", "skills", "projects", "education", "credentials", "profile"];

pub struct MySqlStore {
    pool: MySqlPool,
}

impl MySqlStore {
    pub async fn connect(url: &str, max_conns: u32) -> Result<Self, StoreError> {
        let mut opts: MySqlConnectOptions = url.parse()?;
        // encrypted unless the URL says otherwise (ssl-mode=...); also keeps the password exchange off RSA
        if !url.to_ascii_lowercase().contains("ssl-mode=") {
            opts = opts.ssl_mode(MySqlSslMode::Required);
        }
        let opts = opts.charset("utf8mb4");
        let pool = MySqlPoolOptions::new()
            .max_connections(max_conns)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
            .connect_with(opts)
            .await?;
        Ok(MySqlStore { pool })
    }

    pub async fn load(&self) -> Result<Profile, StoreError> {
        let db = &self.pool;
        let head = sqlx::query("SELECT `name`, `role`, tagline, location FROM profile WHERE id = 1").fetch_optional(db).await?;
        let Some(head) = head else { return Err(StoreError::NoProfile) };

        let mut p = Profile {
            name: head.try_get(0)?,
            role: head.try_get(1)?,
            tagline: head.try_get(2)?,
            location: head.try_get(3)?,
            ..Profile::default()
        };
        p.summary = sqlx::query_scalar("SELECT body FROM summary ORDER BY pos").fetch_all(db).await?;
        p.links = sqlx::query_as::<_, (String, String, String)>("SELECT label, href, `text` FROM links ORDER BY pos")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(label, href, text)| Link { label, href, text })
            .collect();
        p.metrics = sqlx::query_as::<_, (String, String)>("SELECT `value`, label FROM metrics ORDER BY pos")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(value, label)| Metric { value, label })
            .collect();
        p.pipeline = sqlx::query_as::<_, (String, String)>("SELECT step, detail FROM pipeline ORDER BY pos")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(step, detail)| Stage { step, detail })
            .collect();

        p.experience = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT slug, title, org, period, intro FROM jobs ORDER BY pos",
        )
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|(id, title, org, period, intro)| Job { id, title, org, period, intro, groups: Vec::new() })
        .collect();
        let by_slug: HashMap<String, usize> = p.experience.iter().enumerate().map(|(i, j)| (j.id.clone(), i)).collect();
        let items = sqlx::query_as::<_, (String, i32, String, String)>(
            "SELECT job_slug, grp_pos, grp, body FROM job_items ORDER BY job_slug, grp_pos, pos",
        )
        .fetch_all(db)
        .await?;
        let mut last_group: HashMap<usize, i32> = HashMap::new();
        for (slug, grp_pos, grp, body) in items {
            let Some(&ji) = by_slug.get(&slug) else { continue };
            let job = &mut p.experience[ji];
            if last_group.insert(ji, grp_pos) != Some(grp_pos) {
                job.groups.push(Group { name: grp, items: Vec::new() });
            }
            if let Some(g) = job.groups.last_mut() {
                g.items.push(body);
            }
        }

        p.cases = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT slug, title, context, `action`, result FROM cases ORDER BY pos",
        )
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|(id, title, context, action, result)| Case { id, title, context, action, result })
        .collect();

        let mut last_area = None;
        for (area_pos, area, item) in
            sqlx::query_as::<_, (i32, String, String)>("SELECT area_pos, area, item FROM skills ORDER BY area_pos, pos").fetch_all(db).await?
        {
            if last_area.replace(area_pos) != Some(area_pos) {
                p.skills.push(SkillArea { area, items: Vec::new() });
            }
            if let Some(s) = p.skills.last_mut() {
                s.items.push(item);
            }
        }
        p.projects = sqlx::query_as::<_, (String, String)>("SELECT `name`, detail FROM projects ORDER BY pos")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(name, detail)| Project { name, detail })
            .collect();
        p.education = sqlx::query_as::<_, (String, String, String)>("SELECT title, org, `year` FROM education ORDER BY pos")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(title, org, year)| Education { title, org, year })
            .collect();
        p.credentials = sqlx::query_scalar("SELECT body FROM credentials ORDER BY pos").fetch_all(db).await?;
        Ok(p.sanitised())
    }

    /// create the tables and replace all content with `p`, in one transaction
    pub async fn seed(&self, p: &Profile) -> Result<(), StoreError> {
        for q in SCHEMA {
            self.pool.execute(AssertSqlSafe(format!("{q}{TABLE_OPTS}"))).await?;
        }
        let mut tx = self.pool.begin().await?;
        for t in CONTENT_TABLES {
            tx.execute(AssertSqlSafe(format!("DELETE FROM {t}"))).await?; // names are constants
        }
        insert_all(&mut tx, p).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn insert_all(tx: &mut Transaction<'_, MySql>, p: &Profile) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO profile (id, `name`, `role`, tagline, location) VALUES (1, ?, ?, ?, ?)")
        .bind(&p.name)
        .bind(&p.role)
        .bind(&p.tagline)
        .bind(&p.location)
        .execute(&mut **tx)
        .await?;
    for (i, s) in p.summary.iter().enumerate() {
        sqlx::query("INSERT INTO summary (pos, body) VALUES (?, ?)").bind(i as i32).bind(s).execute(&mut **tx).await?;
    }
    for (i, l) in p.links.iter().enumerate() {
        sqlx::query("INSERT INTO links (pos, label, href, `text`) VALUES (?, ?, ?, ?)")
            .bind(i as i32)
            .bind(&l.label)
            .bind(&l.href)
            .bind(&l.text)
            .execute(&mut **tx)
            .await?;
    }
    for (i, m) in p.metrics.iter().enumerate() {
        sqlx::query("INSERT INTO metrics (pos, `value`, label) VALUES (?, ?, ?)").bind(i as i32).bind(&m.value).bind(&m.label).execute(&mut **tx).await?;
    }
    for (i, s) in p.pipeline.iter().enumerate() {
        sqlx::query("INSERT INTO pipeline (pos, step, detail) VALUES (?, ?, ?)").bind(i as i32).bind(&s.step).bind(&s.detail).execute(&mut **tx).await?;
    }
    for (i, j) in p.experience.iter().enumerate() {
        sqlx::query("INSERT INTO jobs (pos, slug, title, org, period, intro) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(i as i32)
            .bind(&j.id)
            .bind(&j.title)
            .bind(&j.org)
            .bind(&j.period)
            .bind(&j.intro)
            .execute(&mut **tx)
            .await?;
        for (gi, g) in j.groups.iter().enumerate() {
            for (ii, item) in g.items.iter().enumerate() {
                sqlx::query("INSERT INTO job_items (job_slug, grp_pos, grp, pos, body) VALUES (?, ?, ?, ?, ?)")
                    .bind(&j.id)
                    .bind(gi as i32)
                    .bind(&g.name)
                    .bind(ii as i32)
                    .bind(item)
                    .execute(&mut **tx)
                    .await?;
            }
        }
    }
    for (i, c) in p.cases.iter().enumerate() {
        sqlx::query("INSERT INTO cases (pos, slug, title, context, `action`, result) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(i as i32)
            .bind(&c.id)
            .bind(&c.title)
            .bind(&c.context)
            .bind(&c.action)
            .bind(&c.result)
            .execute(&mut **tx)
            .await?;
    }
    for (ai, s) in p.skills.iter().enumerate() {
        for (i, item) in s.items.iter().enumerate() {
            sqlx::query("INSERT INTO skills (area_pos, area, pos, item) VALUES (?, ?, ?, ?)")
                .bind(ai as i32)
                .bind(&s.area)
                .bind(i as i32)
                .bind(item)
                .execute(&mut **tx)
                .await?;
        }
    }
    for (i, pr) in p.projects.iter().enumerate() {
        sqlx::query("INSERT INTO projects (pos, `name`, detail) VALUES (?, ?, ?)").bind(i as i32).bind(&pr.name).bind(&pr.detail).execute(&mut **tx).await?;
    }
    for (i, e) in p.education.iter().enumerate() {
        sqlx::query("INSERT INTO education (pos, title, org, `year`) VALUES (?, ?, ?, ?)")
            .bind(i as i32)
            .bind(&e.title)
            .bind(&e.org)
            .bind(&e.year)
            .execute(&mut **tx)
            .await?;
    }
    for (i, c) in p.credentials.iter().enumerate() {
        sqlx::query("INSERT INTO credentials (pos, body) VALUES (?, ?)").bind(i as i32).bind(c).execute(&mut **tx).await?;
    }
    Ok(())
}

/// DATETIME has no zone: store UTC
fn utc_naive(t: OffsetDateTime) -> PrimitiveDateTime {
    let t = t.to_offset(UtcOffset::UTC);
    PrimitiveDateTime::new(t.date(), t.time())
}

impl Store for MySqlStore {
    fn profile(&self) -> StoreFuture<'_, Arc<Rendered>> {
        Box::pin(async move { Ok(Arc::new(Rendered::of(&self.load().await?)?)) })
    }

    fn save_message(&self, m: &Message, received: OffsetDateTime) -> StoreFuture<'_, ()> {
        let (name, email, body) = (m.name().to_owned(), m.email().to_owned(), m.body().to_owned());
        Box::pin(async move {
            sqlx::query("INSERT INTO messages (`name`, email, body, received) VALUES (?, ?, ?, ?)")
                .bind(name)
                .bind(email)
                .bind(body)
                .bind(utc_naive(received))
                .execute(&self.pool)
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::DEFAULT_PROFILE;

    fn profile() -> Profile {
        Profile::from_json(DEFAULT_PROFILE).unwrap()
    }

    #[tokio::test]
    async fn cache_serves_repeat_requests_from_memory() {
        let mem = Arc::new(MemoryStore::with(&profile()));
        let cached = Cached::new(mem.clone(), Duration::from_secs(60));
        for _ in 0..20 {
            cached.profile().await.unwrap();
        }
        assert_eq!(mem.loads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cache_keeps_the_last_good_copy_when_the_source_fails() {
        let mem = Arc::new(MemoryStore::with(&profile()));
        let cached = Cached::new(mem.clone(), Duration::from_millis(1));
        let first = cached.profile().await.unwrap();
        mem.clear();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(cached.profile().await.unwrap(), first);
    }

    /// against a real MySQL: TEST_MYSQL_URL=mysql://root@127.0.0.1:3306 (an account that may create databases)
    #[tokio::test]
    async fn mysql_round_trip() {
        let Ok(base) = std::env::var("TEST_MYSQL_URL") else {
            eprintln!("skipped: set TEST_MYSQL_URL to run against MySQL");
            return;
        };
        let db = format!("portfolio_test_{}", std::process::id());
        let admin = MySqlStore::connect(&base, 1).await.unwrap();
        admin.pool.execute(AssertSqlSafe(format!("CREATE DATABASE `{db}`"))).await.unwrap();

        let store = MySqlStore::connect(&format!("{}/{db}", base.trim_end_matches('/')), 2).await.unwrap();
        let p = profile();
        store.seed(&p).await.unwrap();
        assert_eq!(store.load().await.unwrap(), p, "what goes in comes out, in order");

        let mut changed = p.clone();
        changed.experience.reverse();
        changed.skills.truncate(2);
        store.seed(&changed).await.unwrap();
        assert_eq!(store.load().await.unwrap(), changed, "re-seeding replaces everything");

        let msg = match contact_guard::Form::contact().check(contact_guard::Submission::new("Zoë 😀", "z@example.com", "multi\nline")) {
            contact_guard::Verdict::Accept(m) => m,
            v => panic!("{v:?}"),
        };
        store.save_message(&msg, OffsetDateTime::now_utc()).await.unwrap();
        let (name, body): (String, String) = sqlx::query_as("SELECT `name`, body FROM messages").fetch_one(&store.pool).await.unwrap();
        assert_eq!((name.as_str(), body.as_str()), ("Zoë 😀", "multi\nline"), "utf8mb4 end to end");

        admin.pool.execute(AssertSqlSafe(format!("DROP DATABASE `{db}`"))).await.unwrap();
    }

    #[tokio::test]
    async fn etag_changes_with_content() {
        let mut p = profile();
        let a = Rendered::of(&p).unwrap();
        p.name.push('!');
        let b = Rendered::of(&p).unwrap();
        assert_ne!(a.etag, b.etag);
        assert_eq!(a, Rendered::of(&profile()).unwrap());
    }
}
