//! test/api/newsletter.rs
use crate::helpers::{assert_is_redirect_to, spawn_app, ConfirmationLinks, TestApp};
use fake::faker::internet::en::SafeEmail;
use fake::faker::name::en::Name;
use fake::Fake;
use std::time::Duration;
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockBuilder, ResponseTemplate};

fn newsletter_request_body() -> serde_json::Value {
    serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": uuid::Uuid::new_v4().to_string(),
    })
}

#[tokio::test]
async fn newsletters_are_not_delivered_to_unconfirmed_subscribers() {
    // 准备
    let app = spawn_app().await;
    create_unconfirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&app.email_server)
        .await;

    let response = app.post_newsletters(newsletter_request_body()).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    let html_page = app.get_newsletters_html().await;
    assert!(html_page.contains("<p><i>The newsletter issue has been published</i></p>"));
}
#[tokio::test]
async fn newsletters_are_delivered_to_confirmed_subscribers() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let response = app.post_newsletters(newsletter_request_body()).await;
    assert_is_redirect_to(&response, "/admin/newsletters");
}

#[tokio::test]
async fn newsletters_returns_400_for_invalid_data() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;
    let test_cases = vec![
        (
            serde_json::json!({
                "text_content": "Newsletter body as plain text",
                "html_content": "<p>Newsletter body as HTML</p>",
            }),
            "missing title",
        ),
        (
            serde_json::json!({"title": "Newsletter"}),
            "missing content",
        ),
    ];

    for (invalid_body, error_message) in test_cases {
        let response = app.post_newsletters(invalid_body).await;
        assert_eq!(
            400,
            response.status().as_u16(),
            "The API did not fail with 400 Bad Request when the payload was {}.",
            error_message
        );
    }
}
#[tokio::test]
async fn you_must_be_logged_in_to_publish_a_newsletter() {
    let app = spawn_app().await;

    let response = app.post_newsletters(newsletter_request_body()).await;
    assert_is_redirect_to(&response, "/login");
}

#[tokio::test]
async fn you_must_be_logged_in_to_see_the_newsletter_form() {
    let app = spawn_app().await;

    let response = app.get_newsletters().await;
    assert_is_redirect_to(&response, "/login");
}
#[tokio::test]
async fn newsletter_creation_is_idempotent() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let newsletter_request_body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key,
    });
    let response = app.post_newsletters(newsletter_request_body).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    let html_page = app.get_newsletters_html().await;
    assert!(html_page.contains("<p><i>The newsletter issue has been published</i></p>"));

    let newsletter_request_body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key,
    });
    let response = app.post_newsletters(newsletter_request_body).await;

    assert_is_redirect_to(&response, "/admin/newsletters");

    let html_page = app.get_newsletters_html().await;
    assert!(html_page.contains("<p><i>The newsletter issue has been published</i></p>"));
}

#[tokio::test]
async fn concurrent_form_submission_is_handled_gracefully() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let newsletter_request_body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key
    });

    let response1 = app.post_newsletters(newsletter_request_body);
    let newsletter_request_body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key
    });
    let response2 = app.post_newsletters(newsletter_request_body);
    let (response1, response2) = tokio::join!(response1, response2);

    assert_eq!(response1.status(), response2.status());
    assert_eq!(
        response1.text().await.unwrap(),
        response2.text().await.unwrap()
    );
}
fn when_sending_an_email() -> MockBuilder {
    Mock::given(path("/email")).and(method("POST"))
}
/// 发信失败不应该让用户看到 500 —— 任务已经被记录下来等待重试，
/// 所以请求正常返回 303；同时要证明"失败的任务留在队列里、带上了重试元数据"。
#[tokio::test]
async fn a_failed_delivery_leaves_a_retriable_task_in_the_queue() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    // 第一次发信失败
    when_sending_an_email()
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&app.email_server)
        .await;

    let response = app.post_newsletters(newsletter_request_body()).await;
    // 投递失败不再向外冒泡成 500：任务已入队并记下了退避时间，
    // 请求本身是成功的（303 跳回表单页）。
    assert_is_redirect_to(&response, "/admin/newsletters");

    // 任务没有被删掉，而且带上了"试过几次、下次什么时候再试、上次为什么失败"
    let task = sqlx::query!(
        r#"
        SELECT attempts, last_error, next_retry_at > now() AS in_future
        FROM email_delivery_queue
        WHERE task_type = 'newsletter'
        "#
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("失败的投递任务应当留在队列里");
    assert!(task.attempts >= 1, "attempts 应当已经被计数");
    assert!(
        task.last_error.is_some(),
        "应当记录下失败原因，否则事后没法排查"
    );
    assert_eq!(
        task.in_future,
        Some(true),
        "失败的投递必须把 next_retry_at 推到未来，否则会立刻重试（重试风暴）"
    );
}

/// 退避期过去之后，同一个任务会被重新领取并成功投递。
#[tokio::test]
async fn a_failed_delivery_is_retried_once_the_backoff_has_elapsed() {
    let app = spawn_app().await;
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    // 只对第一次发信返回 500
    when_sending_an_email()
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&app.email_server)
        .await;

    let response = app.post_newsletters(body_with(&idempotency_key)).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    // 把 newsletter 任务的退避时间拨回到过去，模拟"退避期已过"
    // （只改数据、不改代码，测的是真的重试路径）
    //
    // 注意：这里只拨 newsletter 任务。订阅产生的确认邮件任务不在本次测试范围内，
    // 把它也一起拨回会让"多挂一个 200 mock"把两封信都发出去，从而干扰断言。
    expire_backoff_of_newsletter_tasks(&app).await;

    when_sending_an_email()
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .named("Delivery retry")
        .mount(&app.email_server)
        .await;

    // 同一个幂等键：不会再扇出新任务，只会在请求内 drain 掉那条等待重试的任务
    let response = app.post_newsletters(body_with(&idempotency_key)).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    let remaining = sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue WHERE task_type = 'newsletter'"
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to count queued tasks.");
    assert_eq!(
        remaining.count,
        Some(0),
        "退避期过后任务应当被重新领取并成功投递（成功后从队列删除）"
    );
}

/// 重试次数用尽的任务会被移进死信表，并且从队列里移除 —— 这样队列不会被毒任务堵住。
///
/// 这里不去真的失败 4 次（那要等 4 次退避），而是**直接把一条到期任务插进队列**，
/// 让它成为队列里唯一可领取的任务；然后触发一次投递，观察它被判死。
#[tokio::test]
async fn a_task_that_exhausted_its_retries_is_moved_to_the_dead_letter_queue() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;

    // 造一条"已经试了很多次、且已经到期"的 newsletter 任务。
    // 它需要一个真实的 issue 行来满足外键。
    let issue_id = uuid::Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO newsletter_issues (newsletter_issue_id, title, text_content, html_content, published_at)
         VALUES ($1, 't', 'a', 'b', now())",
        issue_id
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to insert a newsletter issue.");

    sqlx::query!(
        r#"
        INSERT INTO email_delivery_queue
            (task_type, recipient, newsletter_issue_id, attempts, next_retry_at, last_error)
        VALUES ('newsletter', 'exhausted@example.com', $1, 99, now() - interval '1 second', 'boom')
        "#,
        issue_id
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to insert an exhausted task.");

    // 任何一次真实发信都不应该发生 —— 这条任务应当直接被判死
    when_sending_an_email()
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&app.email_server)
        .await;

    // 用一个新幂等键触发请求内的 drain（这条 publish 本身不会扇出任何任务：
    // 此时没有任何"已确认"的订阅者）
    let response = app.post_newsletters(newsletter_request_body()).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    let remaining = sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue WHERE task_type = 'newsletter'"
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("Failed to count queued tasks.");
    assert_eq!(remaining.count, Some(0), "超限的任务不应当留在队列里");

    let dead = sqlx::query!(
        "SELECT attempts, last_error FROM dead_letter_queue ORDER BY died_at DESC LIMIT 1"
    )
    .fetch_optional(&app.db_pool)
    .await
    .expect("Failed to look up the dead letter queue.");
    let dead = dead.expect("超限的任务应当被移进死信表");
    assert!(dead.attempts >= 1);
    assert!(!dead.last_error.is_empty(), "死信必须带上失败原因");
}

fn body_with(idempotency_key: &str) -> serde_json::Value {
    serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key,
    })
}

/// 只把 newsletter 任务的退避时间拨回到过去，模拟"退避期已过"。
async fn expire_backoff_of_newsletter_tasks(app: &TestApp) {
    sqlx::query!(
        "UPDATE email_delivery_queue SET next_retry_at = now() - interval '1 second'
         WHERE task_type = 'newsletter'"
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to expire the backoff of queued tasks.");
}
async fn create_unconfirmed_subscriber(app: &TestApp) -> ConfirmationLinks {
    let name: String = Name().fake();
    let email: String = SafeEmail().fake();
    let body = serde_urlencoded::to_string(serde_json::json!({
        "name": name,
        "email": email
    }))
    .unwrap();

    let _mock_guard = Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .named("Create unconfirmed subscriber")
        .expect(1)
        .mount_as_scoped(&app.email_server)
        .await;
    app.post_subscriptions(body)
        .await
        .error_for_status()
        .unwrap();

    let email_request = &app
        .email_server
        .received_requests()
        .await
        .unwrap()
        .pop()
        .unwrap();
    app.get_confirmation_links(email_request)
}

async fn create_confirmed_subscriber(app: &TestApp) {
    let confirmation_link = create_unconfirmed_subscriber(app).await;
    reqwest::get(confirmation_link.html)
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}

// ============================================================
// 幂等键的过期语义
// ============================================================
//
// 这三条测试钉住的是同一件事的不同侧面：**处理中的行和已完成的行，清扫规则完全不同**。
//
//   · 已完成 → TTL 到点，删掉（它只是一份缓存）
//   · 处理中且够旧 → 不能删，要让下一个请求【接管】（请求崩了，键不该被永久锁死）
//   · 处理中且还新 → 谁也不能碰（真的有请求在跑，接管会重复发信）

/// 把一个幂等键的 create_at 推到过去，模拟"时间已经过去了"。
///
/// 为什么直接改 create_at 而不是真的等：TTL 是 24 小时、租约是 5 分钟，
/// 真等的话测试跑不完。改 create_at 等价于"这个键是 N 分钟前创建的"，
/// 而且被测代码的判断条件就是 create_at，所以这是对被测语义的精确操控，
/// 不是对实现的取巧。
async fn backdate_idempotency_key(app: &TestApp, key: &str, minutes_ago: i32) {
    let user_id = app.test_user.user_id;
    let affected = sqlx::query!(
        "UPDATE idempotency SET create_at = now() - make_interval(mins => $3)
         WHERE user_id = $1 AND idempotency_key = $2",
        user_id,
        key,
        minutes_ago
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to backdate the idempotency key")
    .rows_affected();
    assert_eq!(affected, 1, "没有找到要回拨的幂等键，种子数据不对");
}

/// 种一条"处理中"的幂等键：response_body 为 NULL，表示有个请求开了头却没写完响应。
async fn seed_processing_key(app: &TestApp, key: &str) {
    sqlx::query!(
        "INSERT INTO idempotency (user_id, idempotency_key, create_at) VALUES ($1, $2, now())",
        app.test_user.user_id,
        key
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to seed an in-flight idempotency key");
}

/// 种一条"已完成"的幂等键。
async fn seed_completed_key(app: &TestApp, key: &str) {
    sqlx::query!(
        r#"
        INSERT INTO idempotency
            (user_id, idempotency_key, response_status_code, response_headers, response_body, create_at)
        VALUES ($1, $2, 303, '{}'::header_pair[], '\x00'::bytea, now())
        "#,
        app.test_user.user_id,
        key
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to seed a completed idempotency key");
}

/// ⭐ 崩溃恢复：一个开了头就死掉的请求，不该把这个幂等键永久锁死。
///
/// 场景：客户端提交 → 请求开跑 → 客户端断连 / 进程被杀 / 部署中断。
/// 那一行的 response_body 永远是 NULL。没有接管机制的话，用户重试这个 key
/// 会永远拿到 500，而且他什么也做不了 —— 这个键被【永久锁死】了。
#[tokio::test]
async fn a_stale_in_flight_idempotency_key_is_taken_over_instead_of_deadlocking() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let idempotency_key = uuid::Uuid::new_v4().to_string();
    seed_processing_key(&app, &idempotency_key).await;
    // 上一轮请求已经死了（比 5 分钟的租约旧）
    backdate_idempotency_key(&app, &idempotency_key, 6).await;

    let response = app.post_newsletters(body_with(&idempotency_key)).await;

    assert_is_redirect_to(&response, "/admin/newsletters");
    let page = app.get_newsletters_html().await;
    assert!(
        page.contains("<p><i>The newsletter issue has been published</i></p>"),
        "接管之后没有正常发布"
    );

    // 接管 = 把 create_at 推到 now()，于是这一行重新变成"刚被领取"，
    // 后续 save_response 能正常写进去。
    let row = sqlx::query!(
        "SELECT create_at, response_body FROM idempotency
         WHERE user_id = $1 AND idempotency_key = $2",
        app.test_user.user_id,
        idempotency_key
    )
    .fetch_one(&app.db_pool)
    .await
    .unwrap();
    assert!(
        row.response_body.is_some(),
        "接管之后响应没被存下来 —— 下一次重试又会当成全新请求"
    );
}

/// 反面：状态还新鲜的"处理中"键【不能】被接管。
///
/// 这正是 concurrent_form_submission_is_handled_gracefully 依赖的行为：
/// 用户双击提交 / 两个标签页同时发，第二个请求必须老老实实失败，
/// 而不是也去发一遍信。
#[tokio::test]
async fn a_fresh_in_flight_idempotency_key_is_not_taken_over() {
    let app = spawn_app().await;
    create_confirmed_subscriber(&app).await;
    app.test_user.login(&app).await;

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0) // 关键：一封都不该发出去
        .mount(&app.email_server)
        .await;

    let idempotency_key = uuid::Uuid::new_v4().to_string();
    seed_processing_key(&app, &idempotency_key).await;
    // 才 1 分钟，远在 5 分钟租约之内 —— 当它是"真的有请求在跑"
    backdate_idempotency_key(&app, &idempotency_key, 1).await;

    let response = app.post_newsletters(body_with(&idempotency_key)).await;

    assert_eq!(
        response.status().as_u16(),
        500,
        "新鲜的 in-flight 键被接管了 —— 两个请求会同时发信"
    );
}

/// sweeper 只回收【已完成】的过期条目，绝不碰处理中的行。
///
/// 为什么"不碰处理中的"这么重要，见 README 的决策卡，这里给两个具体的坏结果：
///   ① 原请求回来 save_response 时 UPDATE 0 行，成果被静默丢弃；
///   ② 用户重试同一个 key 会撞上"INSERT 冲突但查不到已存响应" → 500。
#[tokio::test]
async fn the_sweeper_only_deletes_completed_and_expired_entries() {
    let app = spawn_app().await;

    let old_completed = uuid::Uuid::new_v4().to_string();
    let fresh_completed = uuid::Uuid::new_v4().to_string();
    let old_processing = uuid::Uuid::new_v4().to_string();

    seed_completed_key(&app, &old_completed).await;
    seed_completed_key(&app, &fresh_completed).await;
    seed_processing_key(&app, &old_processing).await;

    let expired_minutes = zero2prod::idempotency::COMPLETED_TTL_MINUTES + 60;
    backdate_idempotency_key(&app, &old_completed, expired_minutes).await;
    // 这一条虽然处理中、而且很旧 —— 但它必须活下来
    backdate_idempotency_key(
        &app,
        &old_processing,
        zero2prod::idempotency::STALE_PROCESSING_TTL_MINUTES + 60,
    )
    .await;

    let outcome = zero2prod::idempotency::sweep(&app.db_pool)
        .await
        .expect("sweep failed");

    assert_eq!(
        outcome.deleted_completed, 1,
        "应该只删掉那一条已完成的过期条目"
    );

    let survivors: Vec<String> = sqlx::query!(
        "SELECT idempotency_key FROM idempotency WHERE user_id = $1",
        app.test_user.user_id
    )
    .fetch_all(&app.db_pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.idempotency_key)
    .collect();

    assert!(
        !survivors.contains(&old_completed),
        "已完成的过期条目没有被回收"
    );
    assert!(
        survivors.contains(&fresh_completed),
        "还在 TTL 内的条目被误删了 —— 用户在 TTL 内重试会重复发信"
    );
    assert!(
        survivors.contains(&old_processing),
        "处理中的条目被 sweeper 删掉了 —— 那个还在跑的请求成果会静默丢失，\
         用户重试同一个 key 会拿到 500"
    );
}
