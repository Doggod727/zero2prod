use actix_web::{HttpResponse, Responder};

// TODO: 目前不看数据库/Redis，依赖挂了也返回 200。
// 生产需要拆成 readiness（带依赖检查）和 liveness（只看进程活着）。
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().finish()
}
