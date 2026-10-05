//! src/routes/subscriptions/post.rs
use super::persistence::{
    error_chain_fmt, generate_subscription_token, insert_subscriber, store_token, StoreTokenError,
};
use crate::domain::{NewSubscriber, SubscriberEmail, SubscriberName, SubscriberStatus};
use crate::email_client::EmailClient;
use crate::startup::ApplicationBaseUrl;
use crate::utils::see_other;
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, ResponseError};
use actix_web_flash_messages::FlashMessage;
use anyhow::Context;
use sqlx::PgPool;
use std::fmt::Formatter;

/// POST /subscriptions 的表单字段。
#[derive(serde::Deserialize)]
pub struct FormData {
    name: String,
    email: String,
}

impl TryFrom<FormData> for NewSubscriber {
    type Error = String;

    fn try_from(value: FormData) -> Result<NewSubscriber, Self::Error> {
        let email = SubscriberEmail::parse(value.email)?;
        let name = SubscriberName::parse(value.name)?;
        Ok(NewSubscriber { email, name })
    }
}

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
    let new_subscriber: NewSubscriber = form
        .0
        .try_into()
        .map_err(SubscriberError::ValidationError)?;

    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;

    // 自然键幂等：同一个邮箱重复提交只会命中这一行，不会撞唯一约束。
    let record = insert_subscriber(&mut transaction, &new_subscriber)
        .await
        .context("Failed to insert new subscriber in the database.")?;

    match record.status {
        // 已经确认过的订阅者：不改状态、不重发、不降级。
        // 事务里什么都没写，直接结束即可（不是 commit，别把中间态落库）。
        SubscriberStatus::Confirmed => {
            transaction
                .commit()
                .await
                .context("Failed to commit SQL transaction for a confirmed subscriber")?;
            // 注意：这条消息会暴露「该邮箱已订阅」，是可用性与抗枚举之间的取舍（决策卡）。
            FlashMessage::info("This email is already subscribed.").send();
            Ok(see_other("/subscriptions"))
        }
        // 新订阅者，或之前发信失败、现在重试：轮换 token 并重发确认邮件。
        SubscriberStatus::PendingConfirmation => {
            let subscriber_token = generate_subscription_token();
            store_token(&mut transaction, record.id, &subscriber_token)
                .await
                .context("Failed to store the confirmation token for a new subscriber.")?;
            transaction
                .commit()
                .await
                .context("Failed to commit SQL transaction to store a new subscriber.")?;

            // 发信放在事务之外：外部 I/O 不应该占着数据库连接和行锁。
            send_confirmation_email(
                &email_client,
                new_subscriber,
                &base_url.0,
                &subscriber_token,
            )
            .await
            .context("Failed to send a confirmation email.")?;

            FlashMessage::info(
                "Check your inbox for the confirmation link. It is valid for 7 days.",
            )
            .send();
            // PRG：POST 之后 303 跳到 GET /subscriptions，
            // 既避免用户按 F5 重复提交，也让上面的 Flash 消息有地方显示。
            Ok(see_other("/subscriptions"))
        }
    }
}

#[tracing::instrument(
    name = "Send a confirmation email to a new subscriber"
    skip(email_client, new_subscriber, base_url)
)]
pub async fn send_confirmation_email(
    email_client: &EmailClient,
    new_subscriber: NewSubscriber,
    base_url: &str,
    subscription_token: &str,
) -> Result<(), reqwest::Error> {
    let confirmation_link = format!(
        "{}/subscriptions/confirm?subscription_token={}",
        base_url, subscription_token
    );
    let plain_body = format!(
        "Welcome to our newsletter!\nVisit {} to confirm your subscription",
        confirmation_link
    );
    let html_body = format!(
        "Welcome to our newsletter!<br />\
            Click <a href=\"{}\">here</a> to confirm your subscription.",
        confirmation_link
    );
    email_client
        .send_email(&new_subscriber.email, "Welcome!", &html_body, &plain_body)
        .await
}

#[derive(thiserror::Error)]
pub enum SubscriberError {
    #[error("{0}")]
    ValidationError(String),
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}

impl std::fmt::Debug for SubscriberError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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

// 让 `?` 能把 token 落库失败也映射成 500。
impl From<StoreTokenError> for SubscriberError {
    fn from(e: StoreTokenError) -> Self {
        SubscriberError::UnexpectedError(anyhow::Error::new(e))
    }
}