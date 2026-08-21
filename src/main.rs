//! src/main.rs
use std::net::TcpListener;
use zero2prod::startup::run;
use zero2prod::configurations::get_configurations;
use zero2prod::telemetry::{get_subscriber, init_subscriber};
use sqlx::postgres::PgPoolOptions;
use zero2prod::email_client::EmailClient;

#[tokio::main]
async fn main() -> std::io::Result<()>{
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    // 如果不能读取配置的话，发生panic
    let configuration = get_configurations().expect("Failed to read configurations.");
    let connection_pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_lazy_with(configuration.database.with_db());
    
    // 使用configuration构建一个EmailClient
    let sender_email = configuration.email_client.sender()
        .expect("Invalid sender email address.");
    let timeout = configuration.email_client.timeout();
    let email_client = EmailClient::new(
        configuration.email_client.base_url,
        sender_email,
        configuration.email_client.authorization_token,
        timeout
    );
    // 我们已经移除硬编码值'8000'，现在将会从配置中读取他
    let address = format!("{}:{}", configuration.application.host, configuration.application.port);
    let listener = TcpListener::bind(address)?;
    run(listener, connection_pool, email_client)?.await
}
