//! src/rate_limiting.rs
use redis::aio::ConnectionManager;
use redis::Script;

/// 每个限流键再窗口内允许的次数
pub const MAX_ATTEMPTS: i64 = 5;

/// 窗口长度
pub const WINDOW_SECONDS: i64 = 60;
const LUA_SOURCE: &str = r#"
redis.call("ZREMRANGEBYSCORE", KEYS[1], 0, ARGV[1])
local count = redis.call("ZCARD", KEYS[1])
if count >= tonumber(ARGV[3]) then
    return 0
end
redis.call("ZADD", KEYS[1], ARGV[2], ARGV[2] .. ":" .. ARGV[4])
redis.call("PEXPIRE", KEYS[1], tonumber(ARGV[2]) - tonumber(ARGV[1]))
return 1"#;
pub struct LoginRateLimiter {
    connection_manager: ConnectionManager,
}

impl LoginRateLimiter {
    pub fn new(connection_manager: ConnectionManager) -> Self {
        Self { connection_manager }
    }
    pub async fn try_acquire(&self, username: &str) -> Result<bool, redis::RedisError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_millis() as i64;
        let window_start = now_ms - WINDOW_SECONDS * 1000;
        let mut conn = self.connection_manager.clone();
        let key = format!("login:rate_limit:{username}");
        let script = Script::new(LUA_SOURCE);
        script
            .key(&key)
            .arg(window_start)
            .arg(now_ms)
            .arg(MAX_ATTEMPTS)
            .arg(uuid::Uuid::new_v4().to_string())
            .invoke_async::<_, i64>(&mut conn)
            .await
            .map(|result| result == 1)
    }
}
