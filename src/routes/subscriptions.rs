//! src/routes/subscriptions.rs

use std::error::Error;
use std::fmt::Formatter;
use actix_web::{web, HttpResponse, ResponseError};
use actix_web::http::StatusCode;
use anyhow::Context;
use sqlx::{PgPool, Postgres, Transaction};
use chrono::Utc;
use uuid::Uuid;
use crate::domain::{SubscriberEmail, SubscriberName};
use crate::domain::NewSubscriber;
use crate::email_client::EmailClient;
use crate::startup::ApplicationBaseUrl;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
#[derive(serde::Deserialize)]
pub struct FormData {
    name: String,
    email: String,
}
// subscribe
#[tracing::instrument(
    name = "Adding a new subscriber",
    skip(form, pool, email_client, base_url),
    fields(
        subscriber_email = %form.email,
        subscriber_name = %form.name
    )
)]
pub async fn subscribe(
    form: web::Form<FormData>,
    pool: web::Data<PgPool>,
    email_client: web::Data<EmailClient>,
    base_url: web::Data<ApplicationBaseUrl>,
) -> Result<HttpResponse, SubscriberError> {
    // 显式指定目标类型
    let new_subscriber: NewSubscriber = form.0.try_into().map_err(SubscriberError::ValidationError)?;

    let mut transaction = pool.begin().await.context(
        "Failed to acquire a Postgres connection from the pool")?;
    // insert_subscriber 需要 &NewSubscriber
    let subscriber_id = insert_subscriber(&mut transaction, &new_subscriber).await.context("Failed to insert new subscriber in the database.")?;
    let subscriber_token = generate_subscription_token();
    store_token(&mut transaction, subscriber_id, &subscriber_token).await.context(
        "Failed to store the confirmation token for a new subscriber.")?;
    transaction.commit().await.context(
        "Failed to commit SQL transaction to store a new subscriber.")?;

    // send_confirmation_email 需要按值 NewSubscriber，所以 move 进去（或用 clone）
    send_confirmation_email(&email_client, new_subscriber, &base_url.0, &subscriber_token).await.context(
        "Failed to send a confirmation email.")?;
    Ok(HttpResponse::Ok().finish())
}

#[tracing::instrument(
    name = "Send a confirmation email to a new subscriber"
    skip(email_client, new_subscriber, base_url)
)]
pub async fn send_confirmation_email(email_client: &EmailClient, new_subscriber: NewSubscriber, base_url: &str, subscription_token: &str)
    -> Result<(), reqwest::Error>
{
    let confirmation_link =
        format!("{}/subscriptions/confirm?subscription_token={}", base_url, subscription_token);
    let plain_body = format!(
        "Welcome to our newsletter!\nVisit {} to confirm your subscription",
        confirmation_link
    );
    let html_body = format!("Welcome to our newsletter!<br />\
            Click <a href=\"{}\">here</a> to confirm your subscription.", confirmation_link);
    // 为新的订阅者发送一个邮件
   email_client.send_email(
        new_subscriber.email,
        "Welcome!",
        &html_body,
        &plain_body
    )
        .await
}
#[tracing::instrument(
    name = "Saving new subscriber details in the database",
    skip(new_subscriber, transaction)
)]
pub async fn insert_subscriber(transaction: &mut Transaction<'_, Postgres>, new_subscriber: &NewSubscriber) -> Result<Uuid, sqlx::Error> {
    let subscriber_id = Uuid::new_v4();
   sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at, status)
        VALUES ($1, $2, $3, $4, 'pending_confirmation')"#,
        subscriber_id,
        new_subscriber.email.as_ref(),
        new_subscriber.name.as_ref(),
        Utc::now(),
    )
        // 使用get_ref获得一个不可变引用
        // 引用到'web::Data'包装的’PgConnection'
        .execute(transaction)
        // 首先绑定这个插桩，然后等待这个future完成。
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute query: {:?}", e);
            e
        })?;
        Ok(subscriber_id)
}


/// 生成长度为25个字符且大小敏感订阅令牌
fn generate_subscription_token() -> String {
    let mut rng = thread_rng();
    std::iter::repeat_with(|| rng.sample(Alphanumeric))
        .map(char::from)
        .take(25)
        .collect()
}

#[tracing::instrument(
    name = "Store subscription token in the database",
    skip(subscriber_token, transaction)
)]
pub async fn store_token(
    transaction: &mut Transaction<'_, Postgres>,
    subscriber_id: Uuid,
    subscriber_token: &str
) -> Result<(), StoreTokenError> {
    sqlx::query!(r#"INSERT INTO subscription_tokens (subscription_token, subscriber_id)
            VALUES ($1, $2)"#,
            subscriber_token,
            subscriber_id
    )
        .execute(transaction)
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute query: {:?}", e);
            StoreTokenError(e)
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
    type Error = String;

    fn try_from(value: FormData) -> Result<NewSubscriber, Self::Error> {
        let email = SubscriberEmail::parse(value.email)?;
        let name = SubscriberName::parse(value.name)?;
        Ok(NewSubscriber {email, name})
    }
}

pub struct StoreTokenError(sqlx::Error); // 包装器

impl std::fmt::Debug for StoreTokenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
impl std::fmt::Display for StoreTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,
            "A database error was encountered while \
            trying to store a subscription token."
        )
    }
}

impl std::error::Error for StoreTokenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

fn error_chain_fmt(
    e: &impl std::error::Error,
    f: &mut std::fmt::Formatter<'_>
) -> std::fmt::Result {
    writeln!(f, "{}\n", e)?;
    let mut current = e.source();
    while let Some(cause) = current {
        writeln!(f, "Caused by:\n\t{}", cause)?;
        current = cause.source();
    }
    Ok(())
}

#[derive(thiserror::Error)]
pub enum SubscriberError {
    #[error("{0}")]
    ValidationError(String),
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}

impl std::fmt::Debug for SubscriberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}


impl ResponseError for SubscriberError {
    fn status_code(&self) -> StatusCode {
        match self {
            SubscriberError::ValidationError(_) => StatusCode::BAD_REQUEST,
            SubscriberError::UnexpectedError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
