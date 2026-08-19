//! src/main.rs
use std::net::TcpListener;
use sqlx::PgPool;
use zero2prod::startup::run;
use zero2prod::configurations::get_configurations;
use zero2prod::telemetry::{get_subscriber, init_subscriber};
use secrecy::ExposeSecret;
#[tokio::main]
async fn main() -> std::io::Result<()>{
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    // 如果不能读取配置的话，发生panic
    let configuration = get_configurations().expect("Failed to read configurations.");
    let connection_pool = PgPool::connect(&configuration.database.connection_string().expose_secret()).await.expect("Failed to connect to Postgres.");
    // 我们已经移除硬编码值'8000'，现在将会从配置中读取他
    let address = format!("127.0.0.1:{}", configuration.application_port);
    let listener = TcpListener::bind(address)?;
    run(listener, connection_pool)?.await
}
