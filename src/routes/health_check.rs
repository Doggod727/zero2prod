use actix_web::{Responder, HttpResponse};
// 健康检查端点
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().finish()
    // HttpResponse::Ok()返回一个状态码为200的基础HttpResponseBuilder
    // 调用finish()获取一个带有空响应体的HttpResponse
    // HttpResponse::Ok().finish()
    // 由于HttpResponseBuilder也实现Responder，所以可以直接返回。
}