use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web::dev::Server;
use std::net::TcpListener;
async fn greet(req: HttpRequest) -> impl Responder {
    let name = req.match_info().get("name").unwrap_or("World");
    format!("Hello {}!", name)
}

// 健康检查端点
async fn health_check() -> impl Responder {
    HttpResponse::Ok().finish()
    // HttpResponse::Ok()返回一个状态码为200的基础HttpResponseBuilder
    // 调用finish()获取一个带有空响应体的HttpResponse
    // HttpResponse::Ok().finish()
    // 由于HttpResponseBuilder也实现Responder，所以可以直接返回。
}


pub fn run(listener: TcpListener) -> Result<Server, std::io::Error>{
    let server = HttpServer::new(|| {
        App::new()
            .route("/health_check", web::get().to(health_check)) // web::get().to(health_check) => Route::new().guard(guard::Get()).to(health_check)

    })
        .listen(listener)?
        .run();
    Ok(server)
}