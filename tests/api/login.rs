//! tests/api/login.rs
use crate::helpers::assert_is_redirect_to;
use crate::helpers::spawn_app;
use redis::AsyncCommands;

#[tokio::test]
async fn an_error_flash_message_is_set_on_failure() {
    let app = spawn_app().await;

    let login_body = serde_json::json!({
        "username": "random-username",
        "password": "random-password"
    });
    let response = app.post_login(&login_body).await;

    assert_eq!(response.status().as_u16(), 303);
    assert_is_redirect_to(&response, "/login");

    let html_page = app.get_login_html().await;
    assert!(html_page.contains(r#"<p><i>Authentication failed</i></p>"#));

    let html_page = app.get_login_html().await;
    assert!(!html_page.contains(r#"<p><i>Authentication failed</i></p>"#));
}

#[tokio::test]
async fn redirect_to_admin_dashboard_after_login_success() {
    let app = spawn_app().await;

    let login_body = serde_json::json!({
        "username": &app.test_user.username,
        "password": &app.test_user.password,
    });
    let response = app.post_login(&login_body).await;
    assert_is_redirect_to(&response, "/admin/dashboard");

    let html_page = app.get_admin_dashboard_html().await;
    assert!(html_page.contains(&format!("Welcome {}", app.test_user.username)));
}

/// 连续尝试超过阈值后，登录接口返回 429，并带上 Retry-After。
///
/// 这里用的是**错误密码**：限流必须对"密码错误"也计数，
/// 否则攻击者靠不断试错密码就能绕过限流。这也是为什么限流必须放在密码校验【之前】。
#[tokio::test]
async fn too_many_failed_attempts_get_rate_limited() {
    let app = spawn_app().await;
    let username = uuid::Uuid::new_v4().to_string();

    let login_body = serde_json::json!({
        "username": username,
        "password": "definitely-not-the-password",
    });

    for attempt in 1..=5 {
        let response = app.post_login(&login_body).await;
        assert_eq!(
            303,
            response.status().as_u16(),
            "第 {attempt} 次尝试不应该被限流"
        );
    }

    let response = app.post_login(&login_body).await;
    assert_eq!(429, response.status().as_u16(), "超过阈值后应当返回 429");
    assert!(
        response.headers().get("Retry-After").is_some(),
        "429 应当带上 Retry-After 告诉客户端多久后再试"
    );
}

/// 窗口过后（计数归零）应当恢复放行。
#[tokio::test]
async fn the_rate_limiter_recovers_once_the_window_has_passed() {
    let app = spawn_app().await;
    let username = uuid::Uuid::new_v4().to_string();

    let login_body = serde_json::json!({
        "username": &username,
        "password": "wrong-password",
    });

    for _ in 0..5 {
        app.post_login(&login_body).await;
    }
    let limited = app.post_login(&login_body).await;
    assert_eq!(429, limited.status().as_u16());

    app.reset_login_rate_limit(&username)
        .await
        .expect("Failed to reset the rate limit key");

    let response = app.post_login(&login_body).await;
    assert_eq!(303, response.status().as_u16(), "计数归零后应当恢复放行");
}

/// 成功登录也要消耗配额 —— 防止"用正确密码无限打"。
#[tokio::test]
async fn successful_logins_also_consume_the_rate_limit_budget() {
    let app = spawn_app().await;

    let login_body = serde_json::json!({
        "username": &app.test_user.username,
        "password": &app.test_user.password,
    });

    for _ in 1..=5 {
        let response = app.post_login(&login_body).await;
        assert_is_redirect_to(&response, "/admin/dashboard");
    }

    let response = app.post_login(&login_body).await;
    assert_eq!(
        429,
        response.status().as_u16(),
        "限流在校验密码之前，所以成功登录也消耗配额"
    );
}

/// 【为什么必须用 Lua】—— 用实验证明"INCR 和 EXPIRE 分成两次往返"会出事。
///
/// 天真的写法是：
///     ① conn.incr(key, 1)            // 计数
///     ② if count == 1 { conn.expire(key, 60) }   // 设过期
///
/// 如果进程在 ① 和 ② 之间崩溃（或网络断开），这个 key **永远不会过期**，
/// 于是这个用户被【永久】限流 —— 一次崩溃 = 永久封禁一个账号。
///
/// 这个测试就是复现那个中间状态：只执行 INCR，不执行 EXPIRE，
/// 然后断言 TTL 是 -1（永不过期）。
/// 对比：走 Lua 脚本时，TTL 一定被设置（见下一个测试）。
#[tokio::test]
async fn without_lua_a_partial_execution_leaves_a_key_that_never_expires() {
    let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let mut conn = client.get_tokio_connection().await.unwrap();
    let key = format!("lua-demo:{}", uuid::Uuid::new_v4());

    // ① 只做"计数"，模拟在设置过期之前就崩了
    let _: i64 = conn.incr(&key, 1).await.unwrap();

    // TTL = -1 表示"存在但没有过期时间" → 永不过期
    let ttl: i64 = conn.ttl(&key).await.unwrap();
    assert_eq!(
        ttl, -1,
        "只执行 INCR 时，key 没有 TTL —— 这就是为什么两张往返的写法会永久限流一个用户"
    );

    let _: () = conn.del(&key).await.unwrap();
}

/// 对比上面：走 Lua 脚本时，"计数 + 设过期"是一个原子单元，
/// 所以 key 一定带上 TTL，不会出现"永不过期"。
#[tokio::test]
async fn the_lua_script_always_sets_a_ttl() {
    let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let mut conn = client.get_tokio_connection().await.unwrap();
    let key = format!("lua-demo:{}", uuid::Uuid::new_v4());

    let script = redis::Script::new(
        r#"
        local current = redis.call("INCR", KEYS[1])
        if current == 1 then
            redis.call("EXPIRE", KEYS[1], ARGV[2])
        end
        if current > tonumber(ARGV[1]) then
            return 0
        end
        return 1
        "#,
    );

    let allowed: i64 = script
        .key(&key)
        .arg(5)
        .arg(60)
        .invoke_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(allowed, 1, "第一次调用应当放行");

    let ttl: i64 = conn.ttl(&key).await.unwrap();
    assert!(
        ttl > 0 && ttl <= 60,
        "Lua 脚本必须同时把 TTL 设好，实际 TTL = {ttl}"
    );

    let _: () = conn.del(&key).await.unwrap();
}

/// 同一毫秒并发到达的请求，也只能有 MAX_ATTEMPTS 个通过。
///
/// 用一个多线程运行时 + 多个独立 HTTP client 来制造真正的并发
/// （每个 client 有自己的 cookie jar，互不干扰）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_respect_the_limit_atomically() {
    let app = spawn_app().await;
    let username = uuid::Uuid::new_v4().to_string();
    let body = serde_json::json!({
        "username": username,
        "password": "wrong-password",
    });

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let address = app.address.clone();
        let body = body.clone();
        tasks.push(tokio::spawn(async move {
            client
                .post(&format!("{address}/login"))
                .form(&body)
                .send()
                .await
                .expect("Failed to execute request.")
                .status()
                .as_u16()
        }));
    }

    let mut allowed = 0;
    let mut limited = 0;
    for task in tasks {
        match task.await.unwrap() {
            303 => allowed += 1,
            429 => limited += 1,
            other => panic!("unexpected status {other}"),
        }
    }

    assert_eq!(
        allowed, 5,
        "并发下也应当恰好放行 MAX_ATTEMPTS 次（Lua 保证判断+计数是原子的）"
    );
    assert_eq!(limited, 5, "其余的应当全部被限流");
}