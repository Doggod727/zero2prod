//! src/rate_limiting.rs
use redis::aio::ConnectionManager;
use redis::Script;

/// 每个限流键再窗口内允许的次数
pub const MAX_ATTEMPTS: i64 = 5;

/// 窗口长度
pub const WINDOW_SECONDS: i64 = 60;
const LUA_SOURCE: &str = r#"
local current = redis.call("INCR", KEYS[1])
if current == 1 then
    redis.call("EXPIRE", KEYS[1], ARGV[2])
end
if current > tonumber(ARGV[1]) then
    return 0
end
return 1"#;
pub struct LoginRateLimiter {
    connection_manager: ConnectionManager,
}

impl LoginRateLimiter {
    pub fn new(connection_manager: ConnectionManager) -> Self {
        Self {connection_manager}
    }
    pub async fn try_acquire(&self, username: &str) -> Result<bool, redis::RedisError> {

        let mut conn = self.connection_manager.clone();
        let key = format!("login:rate_limit:{username}");
        let script = Script::new(LUA_SOURCE);
        script
            .key(&key)
            .arg(MAX_ATTEMPTS)
            .arg(WINDOW_SECONDS)
            .invoke_async::<_, i64>(&mut conn)
            .await
            .map(|result| result == 1)
    }
}