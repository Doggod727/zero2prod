//! src/main.rs
use zero2prod::configurations::get_configurations;
use zero2prod::startup::Application;
use zero2prod::telemetry::{get_subscriber, init_subscriber};

#[tokio::main]
async fn main() -> std::io::Result<()>{
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    // 如果不能读取配置的话，发生panic
    let configuration = get_configurations().expect("Failed to read configurations.");
    let server = Application::build(configuration).await?;
    server.run_until_stopped().await.expect("Failed to run");
    Ok(())
}
