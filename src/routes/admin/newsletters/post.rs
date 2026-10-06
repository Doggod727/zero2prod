//! src/routes/admin/newsletters/post.rs
use crate::authentication::UserId;
use crate::email_client::EmailClient;
use crate::idempotency::{save_response, try_processing, IdempotencyKey, NextAction};
use crate::issue_delivery_worker::{try_execute_task, ExecutionOutcome};
use crate::startup::ApplicationBaseUrl;
use crate::utils::{e400, e500, see_other};
use actix_web::{web, HttpResponse};
use actix_web_flash_messages::FlashMessage;
use anyhow::Context;
use sqlx::{PgPool, Postgres, Transaction};

#[derive(serde::Deserialize)]
pub struct FormData {
    title: String,
    text_content: String,
    html_content: String,
    idempotency_key: String,
}

#[tracing::instrument(
    name = "Publish a newsletter issue",
    skip(form, pool, email_client, user_id, base_url),
    fields(user_id=%*user_id)
)]
pub async fn publish_newsletter(
    form: web::Form<FormData>,
    user_id: web::ReqData<UserId>,
    pool: web::Data<PgPool>,
    email_client: web::Data<EmailClient>,
    base_url: web::Data<ApplicationBaseUrl>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = user_id.into_inner();
    let FormData {
        title,
        text_content,
        html_content,
        idempotency_key,
    } = form.0;
    let idempotency_key: IdempotencyKey = idempotency_key.try_into().map_err(e400)?;
    let response = match try_processing(&pool, &idempotency_key, *user_id)
        .await
        .map_err(e500)?
    {
        NextAction::StartProcessing(mut transaction) => {
            let issue_id =
                insert_newsletter_issue(&mut transaction, &title, &text_content, &html_content)
                    .await
                    .context("Failed to store newsletter issue details")
                    .map_err(e500)?;
            enqueue_delivery_tasks(&mut transaction, issue_id)
                .await
                .context("Failed to enqueue delivery task")
                .map_err(e500)?;
            let response = see_other("/admin/newsletters");
            save_response(transaction, &idempotency_key, *user_id, response)
                .await
                .map_err(e500)?
        }
        NextAction::ReturnSavedResponse(saved_response) => saved_response,
    };
    // 尽力在本请求内把队列里的邮件发出去；
    // 发送失败的邮件会留在队列里，由后台 worker 重试。
    loop {
        let outcome = try_execute_task(&pool, &email_client, &base_url.0)
            .await
            .map_err(e500)?;
        if let ExecutionOutcome::EmptyQueue = outcome {
            break;
        }
    }
    success_message().send();
    Ok(response)
}

fn success_message() -> FlashMessage {
    FlashMessage::info("The newsletter issue has been published")
}

#[tracing::instrument(skip_all)]
async fn insert_newsletter_issue(
    transaction: &mut Transaction<'_, Postgres>,
    title: &str,
    text_content: &str,
    html_content: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let newsletter_issue_id = uuid::Uuid::new_v4();
    sqlx::query!(
        r#"
        INSERT INTO newsletter_issues(
            newsletter_issue_id, title, text_content, html_content, published_at
        )
        VALUES ($1, $2, $3, $4, now())
        "#,
        newsletter_issue_id,
        title,
        text_content,
        html_content,
    )
    .execute(transaction)
    .await?;
    Ok(newsletter_issue_id)
}

#[tracing::instrument(skip_all)]
async fn enqueue_delivery_tasks(
    transaction: &mut Transaction<'_, Postgres>,
    newsletter_issue_id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    // 顺便把每个收件人的退订 token 写进 subscription_token 这一列。
    //
    // 为什么复用这一列而不是新加一列：这一列的语义已经变成
    // "发这封信时需要的那个凭据"——确认任务放确认 token，
    // newsletter 任务放退订 token。新加一列会立刻带来
    // "谁填、谁不填、payload 约束怎么写"三个问题，而这一列本来就 NOT NULL。
    //
    // ⚠️ INNER JOIN 意味着"没有退订 token 的订阅者收不到这封 newsletter"。
    //    靠迁移 20261006120000 的 backfill + subscribe 时建 token 来保证不漏人。
    //    迁移末尾那个 DO 块会在有遗漏时把迁移打回 —— 就是为了不让它静默丢人。
    sqlx::query!(
        r#"
        INSERT INTO email_delivery_queue(task_type, recipient, newsletter_issue_id, subscription_token)
        SELECT 'newsletter', s.email, $1, t.unsubscription_token
        FROM subscriptions s
        JOIN unsubscription_tokens t ON t.subscriber_id = s.id
        WHERE s.status = 'confirmed'
        "#,
        newsletter_issue_id
    )
        .execute(transaction)
        .await?;
    Ok(())
}
