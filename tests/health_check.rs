// ! tests/health_check.rs
use sqlx::{Executor, PgConnection, PgPool, Connection};
use zero2prod::configurations::{get_configurations, DatabaseSettings};
use std::net::TcpListener;
use zero2prod::startup::run;
use zero2prod::telemetry::{get_subscriber, init_subscriber};
use once_cell::sync::Lazy;
// 使用once_cell确保tracing只能被初始化一次
static TRACING: Lazy<()> = Lazy::new(|| {
    let default_filter_level = "info".to_string();
    let subscriber_name ="test".to_string();
    // 由于'sink'是'get_subscriber'返回类型的一部分
    // 导致两个条件分支中'subscriber'的返回类型不一样
    // 因此没办法将其提取出来
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::sink);
        init_subscriber(subscriber);
    }
});
pub struct TestApp {
    pub address: String,
    pub db_pool: PgPool,
}
// 'tokio::test' 是 'tokio::main' 的测试等价物
// 它使得我们无需添加 '#[test]'
#[tokio::test]
async fn health_check_works() {
    // 准备
    let test_app = spawn_app().await;

    let client = reqwest::Client::new();

    // 执行
    let response = client
        .get(&format!("{}/health_check", test_app.address))
        .send()
        .await
        .expect("Failed to execute request.");

    // 断言
    assert!(response.status().is_success());
    assert_eq!(Some(0), response.content_length());
}

// 在后台某处启动应用程序
// spawn_app 是唯一合理依赖应用程序代码的部分。其他的一切测试都与底层实现细节无关。
async fn spawn_app() -> TestApp {
    Lazy::force(&TRACING);

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind random port");
    let port = listener.local_addr().unwrap().port(); // local_addr()获取端口
    let address = format!("http://127.0.0.1:{}", port);

    let mut configuration = get_configurations().expect("Failed to read configurations");
    configuration.database.database_name = uuid::Uuid::new_v4().to_string();
    let connection_pool = configure_database(&configuration.database).await;

    // 如果直接调用run，由于HttpServer::run()返回一个Server，其不会主动关闭，我们的测试就不会结束
    // tokio::spawn方法就十分方便，其接受一个future，并交给运行时轮询，而无需等待其完成。
    let server = run(listener, connection_pool.clone()).expect("Failed to bind address");
    // 启动服务器作为后台任务
    // tokio::spawn返回一个指向spawned future的handle
    let _ = tokio::spawn(server);
    TestApp { address, db_pool: connection_pool }
}

pub async fn configure_database(config: &DatabaseSettings) -> PgPool {
    // 创建数据库
    let mut connection = PgConnection::connect_with(&config.without_db())
        .await
        .expect("Failed to connect to Postgres");
    connection
        .execute(format!(r#"CREATE DATABASE "{}";"#, config.database_name).as_str())
        .await
        .expect("Failed to create database");
    // 迁移数据库
    let connection_pool = PgPool::connect_with(config.with_db())
        .await
        .expect("Failed to connect to Postgres");
    sqlx::migrate!("./migrations")
        .run(&connection_pool)
        .await
        .expect("Failed to migrate the database");
    connection_pool
}
#[tokio::test]
async fn subscribe_returns_a_200_for_valid_form_data() {
    // 准备
    let test_app = spawn_app().await;
    let client = reqwest::Client::new();
    // 执行
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";
    let response = client
        .post(&format!("{}/subscriptions", &test_app.address))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .expect("Failed to execute request.");

    // 断言
    assert_eq!(200, response.status().as_u16());
    // saved的类型
    // query!返回一个匿名的记录类型。
    // 每一个成员对应结果的一个列。
    let saved = sqlx::query!("SELECT email, name FROM subscriptions",)
        .fetch_one(&test_app.db_pool)
        .await
        .expect("Failed to fetch saved subscription.");
    assert_eq!(saved.email, "ursula_le_guin@gmail.com");
    assert_eq!(saved.name, "le guin");
}

#[tokio::test]
async fn subscribe_returns_a_400_when_data_is_missing() {
    // 准备
    let test_app = spawn_app().await;
    let client = reqwest::Client::new();
    let test_cases = vec![
        ("name=le%20guin", "missing the email"),
        ("email=ursula_le_guin%40gmail.com", "missing the name"),
        ("", "missing both name and email"),
    ];

    for (invalid_body, error_message) in test_cases {
        // 执行
        let response = client
            .post(&format!("{}/subscriptions", &test_app.address))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(invalid_body)
            .send()
            .await
            .expect("Failed to execute request");
        assert_eq!(400,
        response.status().as_u16(),
        "The API did not fail with 400 Bad Request when the payload was {}.", error_message);
    }
}