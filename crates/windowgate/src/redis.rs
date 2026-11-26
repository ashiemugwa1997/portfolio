use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::aio::ConnectionManager;
use redis::Script;

use crate::backend::{Backend, BackendError, HitFuture};
use crate::Decision;

/// One atomic step per hit: drop entries older than the window, count, then add or refuse.
/// Uses the server's clock (TIME), so every client agrees.
const HIT: &str = r"
local key = KEYS[1]
local limit = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000000 + tonumber(t[2])
redis.call('ZREMRANGEBYSCORE', key, '-inf', now - window)
if redis.call('ZCARD', key) < limit then
  redis.call('ZADD', key, now, ARGV[3])
  redis.call('PEXPIRE', key, math.ceil(window / 1000))
  return {1, 0}
end
local oldest = redis.call('ZRANGE', key, 0, 0, 'WITHSCORES')
return {0, tonumber(oldest[2]) + window - now}
";

/// Sliding-window limits kept in Redis: shared by every copy of a service, one atomic
/// script per hit (no race between counting and adding), timed by the Redis server's clock.
/// Keys expire on their own once their window has passed.
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use std::time::Duration;
/// use windowgate::{Backend, RedisBackend};
///
/// let client = redis::Client::open("redis://127.0.0.1/")?;
/// let api = RedisBackend::connect(client, "api", 120, Duration::from_secs(60)).await?;
/// if api.hit("203.0.113.9").await?.is_allowed() { /* ... */ }
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct RedisBackend {
    conn: ConnectionManager,
    prefix: String,
    limit: u64,
    window_us: u64,
    script: Script,
}

impl std::fmt::Debug for RedisBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisBackend").field("prefix", &self.prefix).field("limit", &self.limit).field("window_us", &self.window_us).finish_non_exhaustive()
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

impl RedisBackend {
    /// `limit` hits per `window` per key; keys are stored as `windowgate:{name}:{key}`.
    /// The connection reconnects by itself after network errors.
    pub async fn connect(client: redis::Client, name: &str, limit: usize, window: Duration) -> Result<Self, BackendError> {
        if limit == 0 || window < Duration::from_millis(1) {
            return Err(BackendError::new("limit must be above zero and the window at least 1 ms"));
        }
        let conn = ConnectionManager::new(client).await.map_err(BackendError::new)?;
        Ok(RedisBackend {
            conn,
            prefix: format!("windowgate:{name}:"),
            limit: limit as u64,
            window_us: u64::try_from(window.as_micros()).map_err(BackendError::new)?,
            script: Script::new(HIT),
        })
    }
}

/// unique per hit, so two hits in the same microsecond are both counted
fn member() -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("{}-{nanos}-{n}", std::process::id())
}

impl Backend for RedisBackend {
    fn hit<'a>(&'a self, key: &'a str) -> HitFuture<'a> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let (allowed, wait_us): (i64, i64) = self
                .script
                .key(format!("{}{key}", self.prefix))
                .arg(self.limit)
                .arg(self.window_us)
                .arg(member())
                .invoke_async(&mut conn)
                .await
                .map_err(BackendError::new)?;
            Ok(if allowed == 1 {
                Decision::Allowed
            } else {
                let wait = Duration::from_micros(u64::try_from(wait_us).unwrap_or(0));
                Decision::Limited { retry_after: wait.max(Duration::from_millis(1)) }
            })
        })
    }
}
