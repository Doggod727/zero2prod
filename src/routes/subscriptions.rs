//! src/routes/subscriptions.rs
use actix_web::{web, HttpResponse};
use sqlx::PgPool;
use chrono::Utc;
use uuid::Uuid;
use crate::domain::{SubscriberEmail, SubscriberName};
use crate::domain::NewSubscriber;

#[derive(serde::Deserialize)]
pub struct FormData {
    name: String,
    email: String,
}
// subscribe
#[tracing::instrument(
    name = "Adding a new subscriber",
    skip(form, dp_pool),
    fields(
        subscriber_email = %form.email,
        subscriber_name = %form.name
    )
)]
pub async fn subscribe(form: web::Form<FormData>, dp_pool: web::Data<PgPool>) -> HttpResponse {
    let new_subscriber = match form.0.try_into() {
        Ok(new_subscriber) => new_subscriber,
        Err(_) => return HttpResponse::BadRequest().finish(),
    };
    match insert_subscriber(&dp_pool, &new_subscriber).await {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(_) => HttpResponse::InternalServerError().finish()
    }
}

#[tracing::instrument(
    name = "Saving new subscriber details in the database",
    skip(new_subscriber, pool)
)]
pub async fn insert_subscriber(pool: &PgPool, new_subscriber: &NewSubscriber) -> Result<(), sqlx::Error> {
   sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at)
        VALUES ($1, $2, $3, $4)"#,
        Uuid::new_v4(),
        new_subscriber.email.as_ref(),
        new_subscriber.name.as_ref(),
        Utc::now()
    )
        // 使用get_ref获得一个不可变引用
        // 引用到'web::Data'包装的’PgConnection'
        .execute(pool)
        // 首先绑定这个插桩，然后等待这个future完成。
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute query: {:?}", e);
            e
        })?;
        Ok(())
}

// parse_subscriber -> domain模型，用来验证handler解析的数据的有效性。
// pub fn parse_subscriber(form: FormData) -> Result<NewSubscriber, String> {
//     let email = SubscriberEmail::parse(form.email)?;
//     let name = SubscriberName::parse(form.name)?;
//     Ok(NewSubscriber {email, name})
// }

impl TryFrom<FormData> for NewSubscriber {
    type Error =  String;

    fn try_from(value: FormData) -> Result<NewSubscriber, Self::Error> {
        let email = SubscriberEmail::parse(value.email)?;
        let name = SubscriberName::parse(value.name)?;
        Ok(NewSubscriber {email, name})
    }
}