//! src/startup.rs
use actix_web::{web, App, HttpServer};
use actix_web::dev::Server;
use std::net::TcpListener;
use sqlx::PgPool;
use tracing_actix_web::TracingLogger;
use crate::routes::health_check;
use crate::routes::subscribe;
pub fn run(listener: TcpListener, dp_pool: PgPool) -> Result<Server, std::io::Error>{
    let dp_pool = web::Data::new(dp_pool); // 创建一个链接的智能指针
    let server = HttpServer::new(move || {
        App::new()
            // 将中间件通过'wrap'方法加入到'App'中
            // 替代Logger::default()
            .wrap(TracingLogger::default())
            .route("/health_check", web::get().to(health_check)) // web::get().to(health_check) => Route::new().guard(guard::Get()).to(health_check)
            .route("/subscriptions", web::post().to(subscribe))
            // 将链接注册为应用程序状态的一部分
            .app_data(dp_pool.clone())
    })
        .listen(listener)?
        .run();
    Ok(server)
}