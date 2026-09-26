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

    // 断言
    assert_eq!(200, response.status().as_u16()); // 测试对应正确的数据，发送给POST /subscriptions端点，服务器能够正确解析，并且发送一个确认邮件，然后返回200
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
        assert_eq!(400,
                   response.status().as_u16(),
                   "The API did not fail with 400 Bad Request when the payload was {}.", error_message);
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
        assert_eq!(400, response.status().as_u16(), "The API did not return a 400 Bad Request when the payload was {}.", error_message);
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