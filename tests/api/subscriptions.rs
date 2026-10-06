//! tests/api/subscriptions.rs

use crate::helpers::spawn_app;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};
// 每一个客户端都需要发送subscriptions，总结为一个方法
#[tokio::test]
async fn subscribe_returns_a_200_for_valid_form_data() {
    // 准备
    let test_app = spawn_app().await;
    // 执行
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&test_app.email_server)
        .await;
    let response = test_app.post_subscriptions(body.into()).await;

    assert_eq!(303, response.status().as_u16());
    assert_eq!(
        response.headers().get("LOCATION").unwrap(),
        "/subscriptions"
    );
}
#[tokio::test]
pub async fn subscribe_persists_the_new_subscriber() {
    // 准备
    let test_app = spawn_app().await;
    // 执行
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&test_app.email_server)
        .await;
    test_app.post_subscriptions(body.into()).await;
    // saved的类型
    // query!返回一个匿名的记录类型。
    // 每一个成员对应结果的一个列。
    let saved = sqlx::query!("SELECT email, name, status FROM subscriptions",)
        .fetch_one(&test_app.db_pool)
        .await
        .expect("Failed to fetch saved subscription.");
    assert_eq!(saved.email, "ursula_le_guin@gmail.com");
    assert_eq!(saved.name, "le guin");
    assert_eq!(saved.status, "pending_confirmation")
}
#[tokio::test]
async fn subscribe_returns_a_400_when_data_is_missing() {
    // 准备
    let test_app = spawn_app().await;
    let test_cases = vec![
        ("name=le%20guin", "missing the email"),
        ("email=ursula_le_guin%40gmail.com", "missing the name"),
        ("", "missing both name and email"),
    ];

    for (invalid_body, error_message) in test_cases {
        // 执行
        let response = test_app.post_subscriptions(invalid_body.into()).await;
        assert_eq!(
            400,
            response.status().as_u16(),
            "The API did not fail with 400 Bad Request when the payload was {}.",
            error_message
        );
    }
}

#[tokio::test]
async fn subscribe_returns_a_400_when_fields_are_present_but_invalid() {
    // 准备
    let test_app = spawn_app().await;
    let test_cases = vec![
        ("name=&email=ursula_le_guin%40gmail.com", "empty name"),
        ("name=Ursula&email=", "empty name"),
        ("name=Ursula&email=definitely-not-an-email", "invalid email"),
    ];

    for (body, error_message) in test_cases {
        let response = test_app.post_subscriptions(body.into()).await;
        // 断言
        assert_eq!(
            400,
            response.status().as_u16(),
            "The API did not return a 400 Bad Request when the payload was {}.",
            error_message
        );
    }
}

#[tokio::test]
async fn subscribe_sends_a_confirmation_email_for_valid_date() {
    // 验证是否会真的发送邮件
    let app = spawn_app().await; // 获取后台运行服务器的address，数据库连接，以及其邮件客户端发送的目标服务器
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;
    // 执行
    app.post_subscriptions(body.into()).await;
    // 断言
    // Mock会在这里断言
}

#[tokio::test]
async fn subscribe_sends_a_confirmation_email_with_a_link() {
    let app = spawn_app().await;
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;
    // 不在设计预期值
    app.post_subscriptions(body.into()).await;
    let email_request = &app.email_server.received_requests().await.unwrap()[0];
    let confirmation_links = app.get_confirmation_links(email_request);
    assert_eq!(confirmation_links.html, confirmation_links.plain_text);
}

#[tokio::test]
async fn subscribe_fails_if_there_is_a_fatal_database_error() {
    // 准备
    let app = spawn_app().await;
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";
    sqlx::query!("AlTER TABLE subscription_tokens DROP COLUMN subscription_token;",)
        .execute(&app.db_pool)
        .await
        .unwrap();

    let response = app.post_subscriptions(body.into()).await;

    assert_eq!(response.status().as_u16(), 500);
}

// ============================================================================
// P0: 订阅接口的自然键幂等（重复提交 / 已确认 / 发信失败后重试）
// ============================================================================

/// 同一个邮箱提交两次：只落一行，且两次都返回 303（跳回表单页）。
/// 改造前第二次会撞 email 唯一约束 → 500（这就是 P0 要修的 bug）。
#[tokio::test]
async fn subscribe_is_idempotent_for_the_same_email() {
    let app = spawn_app().await;
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;

    let first = app.post_subscriptions(body.into()).await;
    assert_eq!(303, first.status().as_u16(), "第一次订阅应当成功");

    let second = app.post_subscriptions(body.into()).await;
    assert_eq!(
        303,
        second.status().as_u16(),
        "重复订阅不应该失败（改造前这里是 500）"
    );

    let saved = sqlx::query!(
        "SELECT count(*) AS count FROM subscriptions WHERE email = $1",
        "ursula_le_guin@gmail.com"
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to count subscriptions.");
    assert_eq!(saved.count, Some(1), "重复订阅不应该产生第二行");

    let status = sqlx::query!(
        "SELECT status FROM subscriptions WHERE email = $1",
        "ursula_le_guin@gmail.com"
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to fetch status.");
    assert_eq!(status.status, "pending_confirmation");
}

/// 已确认的订阅者再次提交：返回 303 跳转，并且【不重发】邮件、【不改动】记录。
/// 用 expect(0) 的 mock 断言"一封都没发"。
#[tokio::test]
async fn subscribe_does_not_resend_or_modify_when_already_confirmed() {
    let app = spawn_app().await;
    let email = "ursula_le_guin@gmail.com";
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    // 预先造一个已确认的订阅者
    sqlx::query!(
        "INSERT INTO subscriptions (id, email, name, subscribed_at, status)
         VALUES ($1, $2, $3, now(), 'confirmed')",
        uuid::Uuid::new_v4(),
        email,
        "le guin"
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to insert a confirmed subscriber.");

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&app.email_server)
        .await;

    let response = app.post_subscriptions(body.into()).await;
    let status = response.status().as_u16();
    assert_eq!(
        303,
        status,
        "已确认的订阅者再次提交应当返回 303（实际 {}），响应体: {:?}",
        status,
        response.text().await
    );

    let saved = sqlx::query!(
        "SELECT count(*) AS count, min(status) AS status FROM subscriptions WHERE email = $1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to fetch subscription.");
    assert_eq!(saved.count, Some(1));
    assert_eq!(saved.status.as_deref(), Some("confirmed"));

    let tokens = sqlx::query!("SELECT count(*) AS count FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("Failed to count tokens.");
    assert_eq!(tokens.count, Some(0), "已确认的订阅者不该再生成 token");
}

/// 第一次发信失败（邮件服务返回 500）后，用户重试能够成功收到新的确认邮件，
/// 而且旧 token 会被轮换掉（同一个订阅者只留一行 token）。
#[tokio::test]
async fn subscribe_can_recover_from_a_failed_confirmation_email() {
    let app = spawn_app().await;
    let email = "ursula_le_guin@gmail.com";
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    // 只在第一次请求时返回 500；之后交给下面挂的 200 mock
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&app.email_server)
        .await;

    let first = app.post_subscriptions(body.into()).await;
    assert_ne!(
        200,
        first.status().as_u16(),
        "发信失败时不应该返回 200（尽力投递失败）"
    );

    // 订阅者已经落库，并且拿到了一个 token
    let after_first = sqlx::query!("SELECT status FROM subscriptions WHERE email = $1", email)
        .fetch_one(&app.db_pool)
        .await
        .expect("订阅者应当已经落库，否则用户重试还是会撞唯一约束");
    assert_eq!(after_first.status, "pending_confirmation");

    let first_token = sqlx::query!("SELECT subscription_token FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("第一次就应该已经写入 token")
        .subscription_token;

    // 邮件服务恢复正常
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;

    let second = app.post_subscriptions(body.into()).await;
    let status = second.status().as_u16();
    assert_eq!(
        303,
        status,
        "邮件服务恢复后，重试应当成功并 303 跳回表单页（实际 {}），响应体: {:?}",
        status,
        second.text().await
    );

    let count = sqlx::query!(
        "SELECT count(*) AS count FROM subscriptions WHERE email = $1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to count subscriptions.");
    assert_eq!(count.count, Some(1), "重试不应该产生第二行订阅者");

    // token 被轮换：仍然只有一行，而且值变了
    let second_token = sqlx::query!("SELECT subscription_token FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("重试后应当有且只有一个 token")
        .subscription_token;
    assert_ne!(first_token, second_token, "重试应当轮换 token，旧链接失效");

    let token_count = sqlx::query!("SELECT count(*) AS count FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("Failed to count tokens.");
    assert_eq!(
        token_count.count,
        Some(1),
        "唯一索引应当保证一个订阅者只有一行 token"
    );
}

/// 完整闭环：POST 订阅 → 303 → GET /subscriptions 看到 Flash 提示。
///
/// 这条测试是「Flash 消息真的有地方显示」的证明：
/// 303 的目标页面必须存在，而且要把 {msg_html} 渲染出来，
/// 否则消息发出去就丢了（改造前 303 指向一个不存在的端点，用户拿到 405）。
#[tokio::test]
async fn subscription_form_shows_a_flash_message_after_subscribing() {
    let app = spawn_app().await;
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;

    // 1) 提交表单
    let response = app.post_subscriptions(body.into()).await;
    assert_eq!(303, response.status().as_u16());
    assert_eq!(
        response.headers().get("LOCATION").unwrap(),
        "/subscriptions"
    );

    // 2) 顺着 303 落到表单页：Flash 消息存在 cookie 里，由下一个请求读出来渲染
    let html = app.get_subscriptions_html().await;
    assert!(
        html.contains("Check your inbox"),
        "表单页应当渲染出 Flash 提示，实际内容: {}",
        html
    );
    // 表单本身也得在，否则用户没法再次提交
    assert!(
        html.contains(r#"action="/subscriptions""#),
        "表单页应当包含提交表单，实际内容: {}",
        html
    );

    // 3) 再提交一次（此时状态是 pending，会重发确认邮件）仍然 303 + 有提示
    let response = app.post_subscriptions(body.into()).await;
    assert_eq!(303, response.status().as_u16());
    let html = app.get_subscriptions_html().await;
    assert!(
        html.contains("Check your inbox"),
        "重复提交后也应当渲染提示，实际内容: {}",
        html
    );
}

/// 已确认的订阅者再次提交：同样 303 回到表单页，并看到「已订阅」提示。
#[tokio::test]
async fn subscription_form_shows_an_already_subscribed_message() {
    let app = spawn_app().await;
    let email = "ursula_le_guin@gmail.com";
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    sqlx::query!(
        "INSERT INTO subscriptions (id, email, name, subscribed_at, status)
         VALUES ($1, $2, $3, now(), 'confirmed')",
        uuid::Uuid::new_v4(),
        email,
        "le guin"
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to insert a confirmed subscriber.");

    let response = app.post_subscriptions(body.into()).await;
    assert_eq!(303, response.status().as_u16());

    let html = app.get_subscriptions_html().await;
    assert!(
        html.contains("already subscribed"),
        "已确认用户再次提交后应当看到提示，实际内容: {}",
        html
    );
}

/// 反复提交订阅表单不会堆积多条确认任务。
///
/// 这条测试针对的是一个很容易忽略的坑：**已有的
/// `UNIQUE (newsletter_issue_id, recipient)` 对确认任务完全不起作用** ——
/// 确认任务的 newsletter_issue_id 是 NULL，而 SQL 里 NULL 互不相等，
/// 所以 (NULL, 'a@b.com') 和 (NULL, 'a@b.com') 不算冲突。
///
/// 真正的保证来自那条**部分唯一索引**：
///     UNIQUE (recipient) WHERE task_type = 'confirmation'
///
/// 这里让第一次发信失败，好让任务【留在队列里】，否则它会被成功发出去然后删掉，
/// 就观察不到"重复入队"这件事了。
#[tokio::test]
async fn repeated_submissions_do_not_accumulate_confirmation_tasks() {
    let app = spawn_app().await;
    let email = "ursula_le_guin@gmail.com";
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";

    // 发信一律失败 → 确认任务会带着退避留在队列里
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&app.email_server)
        .await;

    // 提交三次
    for _ in 0..3 {
        let response = app.post_subscriptions(body.into()).await;
        assert_eq!(303, response.status().as_u16(), "重复订阅仍然应当 303");
    }

    // 队列里只能有一条确认任务 —— 这就是部分唯一索引在起作用
    let queued = sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue
         WHERE task_type = 'confirmation' AND recipient = $1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to count queued confirmation tasks.");
    assert_eq!(
        queued.count,
        Some(1),
        "同一个订阅者不应当堆积多条确认任务（部分唯一索引没生效？）"
    );

    // 确认 token 也必须只有一行 —— 每次提交都会轮换，但只有一个订阅者
    let tokens = sqlx::query!("SELECT count(*) AS count FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("Failed to count tokens.");
    assert_eq!(tokens.count, Some(1), "轮换 token 不应该留下多行");

    // 而且队列里那条任务带的是【最新】的 token（否则用户点开会是失效链接）
    let queued_token = sqlx::query!(
        "SELECT subscription_token FROM email_delivery_queue
         WHERE task_type = 'confirmation' AND recipient = $1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to fetch the queued token.")
    .subscription_token;
    let stored_token = sqlx::query!("SELECT subscription_token FROM subscription_tokens")
        .fetch_one(&app.db_pool)
        .await
        .expect("Failed to fetch the stored token.")
        .subscription_token;
    assert_eq!(
        queued_token.as_deref(),
        Some(stored_token.as_str()),
        "队列里的任务必须带最新的 token，否则用户点开的是失效链接"
    );
}
