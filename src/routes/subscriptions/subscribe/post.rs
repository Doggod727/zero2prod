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
        // 新订阅者 / 之前发信失败现在重试 / 退订过又想回来：都走这条。
        //
        // 为什么 Unsubscribed 要并进这一支、而不是并进上面 Confirmed 那支：
        //   并进 Confirmed 的话，一个退订过的人【永远回不来】—— 他主动提交订阅表单，
        //   得到的是 "This email is already subscribed"（而且这句话是错的，他并没有订阅），
        //   同时因为 status 不是 confirmed，他也不会再收到任何邮件。
        //   退订是可逆的，重新订阅必须能把他激活回 pending_confirmation。
        SubscriberStatus::PendingConfirmation | SubscriberStatus::Unsubscribed => {
            let subscriber_token = generate_token();
            store_token(&mut transaction, record.id, &subscriber_token)
                .await
                .context("Failed to store the confirmation token for a new subscriber.")?;

            // 退订 token 在【订阅时】就建好，不是等到退订时才建。
            //
            // 原因：下面这封确认邮件从第一封起就带 List-Unsubscribe 头，
            // 而那个链接本身需要一个 token。等退订时才创建的话，
            // 第一封邮件里的退订链接必然是死的 —— 用户点了看到"链接无效"，
            // 而 Gmail 预取这个链接时会因为拿不到有效页而干脆不显示退订按钮。
            //
            // ⚠️ 必须用返回值。store_unsubscription_token 在冲突时【不轮换】，
            //    库里留的是更早那个 token；用自己生成的那个拼链接，
            //    用户点下去同样是"链接无效"。
            // 这里【不需要】把返回值留着：确认邮件由 worker 发，worker 会按收件人
            // 现查退订 token（见 issue_delivery_worker::get_unsubscription_token）。
            // 之所以仍然用返回值而不是忽略它，见 store_unsubscription_token 的注释。
            let _unsubscription_token =
                store_unsubscription_token(&mut transaction, record.id, generate_token())
                    .await
                    .context("Failed to store the unsubscription token for a new subscriber.")?;

            // 用户可能反复提交订阅表单，每次都会轮换 token。如果没有约束，
            // 同一个订阅者会被塞进多条确认任务，于是收到好几封确认邮件。
            //
            // 已有的 UNIQUE(newsletter_issue_id, recipient) 在这里【不起作用】：
            // 确认任务的 newsletter_issue_id 是 NULL，而 SQL 里 NULL 互不相等，
            // 所以 (NULL, 'a@b.com') 和 (NULL, 'a@b.com') 不算冲突（有实测证据）。
            //
            // 因此另建一条**部分唯一索引**，只约束确认任务：
            //   UNIQUE (recipient) WHERE task_type = 'confirmation'
            // 它保证"一个订阅者最多一条待发确认任务"，同时不影响
            // "同一个人既收确认邮件、又收 newsletter" 这种正常情况。
            //
            // 冲突时刷新 token 与重试状态：用户重新提交 = 想立刻再收到一封，
            // 所以把 attempts 归零、next_retry_at 拉回现在。
            enqueue_confirmation_tasks(&mut transaction, &subscriber_token, new_subscriber.email)
                .await
                .context("Failed to insert a task")?;
            transaction
                .commit()
                .await
                .context("Failed to commit SQL transaction to store a new subscriber.")?;
            // 发信放在事务之外：外部 I/O 不应该占着数据库连接和行锁。
            //
            // 这里复用 worker 的 try_execute_task，而不是另写一份"只发我刚刚入队那条"的
            // 逻辑。两个理由：
            //   1. 发信 + 成功删除 / 失败记退避 这一整套语义已经在 try_execute_task 里，
            //      再写一份就会有两条代码路径，以后改退避策略必然漏掉一处；
            //   2. drain 循环的语义是"顺手把队列里能发的都发掉"。我们刚入队的那条
            //      next_retry_at = now()，所以它就在可领取集合里；即使这一轮没轮到它，
            //      循环会继续直到队列空，它一定会被处理。
            //
            // 和 publish_newsletter 里的做法保持一致。
            // 注意：这里【不】直接调 send_confirmation_email ——
            // 发信统一走队列 + try_execute_task，否则"发信 + 失败退避"就有两条代码路径，
            // 以后改退避策略必然漏掉一处。
            loop {
                let outcome = try_execute_task(&pool, &email_client, &base_url.0)
                    .await
                    .map_err(SubscriberError::UnexpectedError)?;
                if let ExecutionOutcome::EmptyQueue = outcome {
                    break;
                }
            }

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
