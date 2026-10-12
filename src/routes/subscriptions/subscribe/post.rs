//! src/routes/subscriptions/post.rs
use super::persistence::{insert_subscriber, store_token, StoreTokenError};
use crate::domain::{NewSubscriber, SubscriberEmail, SubscriberName, SubscriberStatus};
use crate::email_client::EmailClient;
use crate::issue_delivery_worker::{try_execute_task, ExecutionOutcome};
use crate::routes::subscriptions::unsubscribe::store_unsubscription_token;
use crate::startup::ApplicationBaseUrl;
use crate::utils::error_chain_fmt;
use crate::utils::generate_token;
use crate::utils::see_other;
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, ResponseError};
use actix_web_flash_messages::FlashMessage;
use anyhow::Context;
use sqlx::{PgPool, Postgres, Transaction};
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

    // insert_subscriber 只会返回两种状态：RETURNING 给的是这一行的【最终状态】，
    // 插入/更新后都是 PendingConfirmation；DO UPDATE 的 WHERE 不成立时返回 0 行，
    // 走它的 None 分支变成 Confirmed。所以这里不需要 match 第三个分支。
    // （退订的人能回来是在 SQL 那层解决的，不是在这里多写一个 arm。）
    if matches!(record.status, SubscriberStatus::Confirmed) {
        // 已经确认过的订阅者：不改状态、不重发、不降级。
        // 事务里什么都没写，直接结束即可（不是 commit，别把中间态落库）。
        transaction
            .commit()
            .await
            .context("Failed to commit SQL transaction for a confirmed subscriber")?;
        // 注意：这条消息会暴露「该邮箱已订阅」，是可用性与抗枚举之间的取舍（决策卡）。
        FlashMessage::info("This email is already subscribed.").send();
        Ok(see_other("/subscriptions"))
    } else {
        // 新订阅者 / 上次没确认现在重新提交 / 退订过又想回来 ——
        // 三者在走到这里时【已经没有区别】了：insert_subscriber 把它们的 status
        // 都统一成了 PendingConfirmation。
        let subscriber_token = generate_token();
        store_token(&mut transaction, record.id, &subscriber_token)
            .await
            .context("Failed to store the confirmation token for a new subscriber.")?;

        // 退订 token 也在订阅时建好：确认邮件从第一封起就带 List-Unsubscribe 头，
        // 等退订时才创建的话那个链接必然是死的。
        // 返回值不用留 —— 邮件由 worker 发，它会按收件人现查 token。
        let _unsubscription_token =
            store_unsubscription_token(&mut transaction, record.id, generate_token())
                .await
                .context("Failed to store the unsubscription token for a new subscriber.")?;

        // 部分唯一索引 UNIQUE (recipient) WHERE task_type='confirmation' 保证
        // "一个收件人最多一条待发确认任务"，否则重复提交会收到好几封确认邮件。
        // （UNIQUE(newsletter_issue_id, recipient) 在这里无效：确认任务的
        //   newsletter_issue_id 是 NULL，而 SQL 里 NULL 互不相等。）
        enqueue_confirmation_tasks(&mut transaction, &subscriber_token, new_subscriber.email)
            .await
            .context("Failed to insert a task")?;
        transaction
            .commit()
            .await
            .context("Failed to commit SQL transaction to store a new subscriber.")?;
        // 发信在事务之外（外部 I/O 不占数据库连接和行锁），并统一走
        // try_execute_task，不再写第二份"只发我这条"的逻辑 ——
        // 否则"发信 + 失败退避"会有两条代码路径，以后改退避策略必然漏掉一处。
        loop {
            let outcome = try_execute_task(&pool, &email_client, &base_url.0)
                .await
                .map_err(SubscriberError::UnexpectedError)?;
            if let ExecutionOutcome::EmptyQueue = outcome {
                break;
            }
        }

        FlashMessage::info("Check your inbox for the confirmation link. It is valid for 7 days.")
            .send();
        // PRG：303 跳回 GET，避免 F5 重复提交，也让 Flash 消息有地方显示。
        Ok(see_other("/subscriptions"))
    }
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

async fn enqueue_confirmation_tasks(
    transaction: &mut Transaction<'_, Postgres>,
    subscription_token: &str,
    recipient: SubscriberEmail,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO email_delivery_queue(task_type, recipient, subscription_token)
        VALUES ('confirmation', $1, $2)
        ON CONFLICT(recipient) WHERE task_type = 'confirmation'
        DO UPDATE SET subscription_token = EXCLUDED.subscription_token,
                      attempts = 0,
                      next_retry_at = now(),
                      last_error = NULL
        "#,
        recipient.as_ref(),
        subscription_token
    )
    .execute(transaction)
    .await?;
    Ok(())
}
