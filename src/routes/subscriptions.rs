//! src/routes/subscriptions.rs
use actix_web::{web, HttpResponse, Responder};
use sqlx::PgPool;
use chrono::Utc;
use uuid::Uuid;
#[derive(serde::Deserialize)]
pub struct FormData {
    name: String,
    email: String,
}
// subscribe
pub async fn subscribe(form: web::Form<FormData>, dp_pool: web::Data<PgPool>) -> impl Responder {
    match sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at)
        VALUES ($1, $2, $3, $4)"#,
        Uuid::new_v4(),
        form.email,
        form.name,
        Utc::now()
    )
    // 使用get_ref获得一个不可变引用
    // 引用到'web::Data'包装的’PgConnection'
        .execute(dp_pool.get_ref())
        .await {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(e) => {
            println!("Failed to  execute query: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}