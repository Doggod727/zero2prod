//! src/startup.rs
use actix_web::{web, App, HttpServer};
use actix_web::dev::Server;
use std::net::TcpListener;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing_actix_web::TracingLogger;
use crate::configurations::{DatabaseSettings, Settings};
use crate::email_client::EmailClient;
use crate::routes::{confirm, health_check};
use crate::routes::subscribe;



// 一个新的类型，用来保存新构建的服务器及其端口
pub struct Application {
    port: u16,
    server: Server,
}

impl Application {
    // 我们将build函数转换为Application的构造函数
    pub async fn build(configuration: Settings) -> Result<Application, std::io::Error> {

        let connection_pool = get_connection_pool(&configuration.database);

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
        let port = listener.local_addr()?.port();
        let server = run(listener, connection_pool, email_client, configuration.application.base_url)?;

        // 将绑定值保存在Application结构体中
        Ok(Self {
            port,
            server,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    // 表明此函数只在程序停止后停止
    pub async fn run_until_stopped(self) -> Result<(), std::io::Error> {
        self.server.await
    }
}
// 使用包装类型，便于'subscribe'中获取URL
pub struct ApplicationBaseUrl(pub String);
pub fn run(listener: TcpListener, dp_pool: PgPool, email_client: EmailClient, base_url: String) -> Result<Server, std::io::Error>{
    let dp_pool = web::Data::new(dp_pool); // 创建一个链接的智能指针
    let email_client = web::Data::new(email_client);
    let base_url = web::Data::new(ApplicationBaseUrl(base_url));
    let server = HttpServer::new(move || {
        App::new()
            // 将中间件通过'wrap'方法加入到'App'中
            // 替代Logger::default()
            .wrap(TracingLogger::default())
            .route("/health_check", web::get().to(health_check)) // web::get().to(health_check) => Route::new().guard(guard::Get()).to(health_check)
            .route("/subscriptions", web::post().to(subscribe))
            .route("/subscriptions/confirm", web::get().to(confirm))
            // 将链接注册为应用程序状态的一部分
            .app_data(dp_pool.clone())
            .app_data(email_client.clone())
            .app_data(base_url.clone())
    })
        .listen(listener)?
        .run();
    Ok(server)
}

pub fn get_connection_pool(
    configuration: &DatabaseSettings
) -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_lazy_with(configuration.with_db())
}