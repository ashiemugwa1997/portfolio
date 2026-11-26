use std::time::Duration;

use sqlx::{MySqlPool, Row};

use crate::backend::{Backend, BackendError, HitFuture};
use crate::Decision;

/// Sliding-window limits kept in MySQL: shared by every copy of a service, kept across
/// restarts, timed by the database's clock so servers with drifting clocks agree.
///
/// Each hit is a row; a hit over the limit is removed again, so refused requests never
/// extend the wait. Under a burst of simultaneous hits at the very edge of the limit, more
/// than one may be refused, never more than `limit` allowed.
///
/// ```no_run
/// # async fn run(pool: sqlx::MySqlPool) -> Result<(), Box<dyn std::error::Error>> {
/// use std::time::Duration;
/// use windowgate::{Backend, MySqlBackend};
///
/// let logins = MySqlBackend::new(pool, "login_hits", 5, Duration::from_secs(900))?;
/// logins.create_table().await?;
/// if logins.hit("user@example.com").await?.is_allowed() { /* ... */ }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct MySqlBackend {
    pool: MySqlPool,
    table: String,
    limit: i64,
    window_us: i64,
}

impl MySqlBackend {
    /// `limit` hits per `window` per key, counted in `table` (letters, digits and `_`).
    pub fn new(pool: MySqlPool, table: &str, limit: usize, window: Duration) -> Result<Self, BackendError> {
        let ok = !table.is_empty() && table.len() <= 64 && table.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !ok {
            return Err(BackendError::new(format!("table name {table:?}: use letters, digits and _ only")));
        }
        if limit == 0 || window.is_zero() {
            return Err(BackendError::new("limit and window must be above zero"));
        }
        let window_us = i64::try_from(window.as_micros()).map_err(BackendError::new)?;
        Ok(MySqlBackend { pool, table: table.to_owned(), limit: limit as i64, window_us })
    }

    /// Create the table if it is missing.
    pub async fn create_table(&self) -> Result<(), BackendError> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS `{}` (id BIGINT AUTO_INCREMENT PRIMARY KEY, k VARBINARY(255) NOT NULL, \
             at DATETIME(6) NOT NULL, KEY by_key (k, at), KEY by_time (at)) ENGINE=InnoDB",
            self.table
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(&self.pool).await.map_err(BackendError::new)?;
        Ok(())
    }

    /// Delete hits older than the window (also done now and then by `hit`).
    pub async fn prune(&self) -> Result<u64, BackendError> {
        let sql = format!("DELETE FROM `{}` WHERE at < UTC_TIMESTAMP(6) - INTERVAL ? MICROSECOND LIMIT 5000", self.table);
        let r = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(self.window_us).execute(&self.pool).await.map_err(BackendError::new)?;
        Ok(r.rows_affected())
    }

    async fn hit_inner(&self, key: &[u8]) -> Result<Decision, sqlx::Error> {
        let t = &self.table;
        let id = sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO `{t}` (k, at) VALUES (?, UTC_TIMESTAMP(6))")))
            .bind(key)
            .execute(&self.pool)
            .await?
            .last_insert_id();
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*), TIMESTAMPDIFF(MICROSECOND, UTC_TIMESTAMP(6), MIN(at) + INTERVAL ? MICROSECOND) \
             FROM `{t}` WHERE k = ? AND at > UTC_TIMESTAMP(6) - INTERVAL ? MICROSECOND AND id <= ?"
        )))
        .bind(self.window_us)
        .bind(key)
        .bind(self.window_us)
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let count: i64 = row.try_get(0)?;
        if count <= self.limit {
            if id % 1000 == 0 {
                let _ = self.prune().await;
            }
            return Ok(Decision::Allowed);
        }
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM `{t}` WHERE id = ?"))).bind(id).execute(&self.pool).await?;
        let wait_us: Option<i64> = row.try_get(1)?;
        let wait = Duration::from_micros(u64::try_from(wait_us.unwrap_or(0)).unwrap_or(0));
        Ok(Decision::Limited { retry_after: wait.max(Duration::from_millis(1)) })
    }
}

impl Backend for MySqlBackend {
    fn hit<'a>(&'a self, key: &'a str) -> HitFuture<'a> {
        Box::pin(async move {
            // long keys are hashed down to the column size
            let k: Vec<u8> = if key.len() <= 255 { key.as_bytes().to_vec() } else { fold_key(key) };
            self.hit_inner(&k).await.map_err(BackendError::new)
        })
    }
}

/// FNV-1a over the key, in 32 bytes: plenty to keep different long keys apart
fn fold_key(key: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    for seed in 0u64..4 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
        for b in key.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        out.extend_from_slice(&h.to_le_bytes());
    }
    out
}
