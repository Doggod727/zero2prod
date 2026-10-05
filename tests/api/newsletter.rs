//! test/api/newsletter.rs
use crate::helpers::{assert_is_redirect_to, spawn_app, ConfirmationLinks, TestApp};
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockBuilder, ResponseTemplate};
use std::time::Duration;
use fake::Fake;
use fake::faker::internet::en::SafeEmail;
use fake::faker::name::en::Name;

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
    assert!(html_page.contains(
        "<p><i>The newsletter issue has been published</i></p>"
    ));
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
    assert!(
        html_page.contains(
            "<p><i>The newsletter issue has been published</i></p>"
        )
    );

    let newsletter_request_body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Newsletter body as plain text",
        "html_content": "<p>Newsletter body as HTML</p>",
        "idempotency_key": idempotency_key,
    });
    let response = app.post_newsletters(newsletter_request_body).await;

    assert_is_redirect_to(&response, "/admin/newsletters");

    let html_page = app.get_newsletters_html().await;
    assert!(
        html_page.contains(
            "<p><i>The newsletter issue has been published</i></p>"
        )
    );
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
    assert_eq!(response1.text().await.unwrap(), response2.text().await.unwrap());
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

    let response = app
        .post_newsletters(body_with(&idempotency_key))
        .await;
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
    let response = app
        .post_newsletters(body_with(&idempotency_key))
        .await;
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
    let body = serde_urlencoded::to_string(&serde_json::json!({
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
    app.post_subscriptions(body.into())
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
    app.get_confirmation_links(&email_request)
}

async fn create_confirmed_subscriber(app: &TestApp) {
    let confirmation_link = create_unconfirmed_subscriber(app).await;
    reqwest::get(confirmation_link.html)
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}
