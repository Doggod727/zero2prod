//! src/routes/subscriptions/unsubscribe/post.rs
use super::get::notice_page;
use super::persistence::{
    enqueue_unsubscription_notice, find_subscriber_by_unsubscription_token, mark_as_unsubscribed,
    store_unsubscription_token,
};
use crate::email_client::EmailClient;
use crate::issue_delivery_worker::{try_execute_task, ExecutionOutcome};
use crate::startup::ApplicationBaseUrl;
use crate::utils::{error_chain_fmt, generate_token};
use actix_web::http::header::ContentType;
use actix_web::{web, HttpResponse};
use actix_web_flash_messages::FlashMessage;
use anyhow::Context;
use sqlx::PgPool;
use std::fmt::Formatter;

use super::Parameters;

/// POST /subscriptions/unsubscribe?unsubscription_token=...
///
/// 两种调用方请求形状相同：人点确认页按钮（空 body），
/// 邮件客户端一键退订（body = `List-Unsubscribe=One-Click`，RFC 8058）。
/// 所以这里【不解析 body】，token 一律从 query 读 —— 躲开"客户端带不带
/// Content-Type"和"它的 body 不是我们的表单结构"两个坑。
#[tracing::instrument(
    name = "Unsubscribe a subscriber",
    skip(parameters, pool, email_client, base_url),
    fields(unsubscription_token = %parameters.unsubscription_token)
)]
pub async fn unsubscribe(
    parameters: web::Query<Parameters>,
    pool: web::Data<PgPool>,
    email_client: web::Data<EmailClient>,
    base_url: web::Data<ApplicationBaseUrl>,
) -> Result<HttpResponse, UnsubscribeError> {
    let token = &parameters.unsubscription_token;

    let Some(who) = find_subscriber_by_unsubscription_token(&pool, token)
        .await
        .context("Failed to look up the subscriber behind an unsubscription token")?
    else {
        // 认不出来的 token：不 4xx，也不假装成功。
        FlashMessage::info("We could not recognise that unsubscription link.").send();
        return Ok(HttpResponse::Ok()
            .content_type(ContentType::html())
            .body(notice_page(
                &base_url.0,
                "链接无效",
                "这个退订链接我们认不出来。可能是链接被截断了，或者这个地址已经不在了。",
            )));
    };

    // 已退订：幂等，什么都不做，也不重发通知（用户可能反复点旧链接）。
    if !who.status.is_active() {
        return Ok(HttpResponse::Ok()
            .content_type(ContentType::html())
            .body(notice_page(
                &base_url.0,
                "你已经退订了",
                "这个地址已经不在我们的发信名单里，不会再收到任何邮件。",
            )));
    }

    // 三张表的写入放进同一个事务（subscriptions / unsubscription_tokens /
    // email_delivery_queue）：顺序无所谓，但不能拆 —— enqueue 里 DELETE 与 INSERT
    // 之间被别的会话插进来会撞部分唯一索引，而且"改了状态没入队"和
    // "入了队没改状态"都不该出现。
    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;

    let changed = mark_as_unsubscribed(&mut transaction, who.id)
        .await
        .context("Failed to mark the subscriber as unsubscribed")?;

    // changed == false：读到 who 之后、这句 UPDATE 之前，另一个请求已经退订了他。
    // 不报错、不重发通知。
    if changed {
        let token_in_db = store_unsubscription_token(&mut transaction, who.id, generate_token())
            .await
            .context("Failed to store the unsubscription token")?;

        // ⚠️ 用 token_in_db：store_unsubscription_token 冲突时不轮换，
        //    用自己生成的那个拼链接会得到"链接无效"。
        enqueue_unsubscription_notice(&mut transaction, &who.email, &token_in_db)
            .await
            .context("Failed to enqueue the unsubscription notice")?;
    }

    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to unsubscribe a subscriber")?;

    // 事务之外尽力发掉，让用户立刻收到通知；发不出去就留给后台 worker 重试。
    if changed {
        loop {
            let outcome = try_execute_task(&pool, &email_client, &base_url.0)
                .await
                .map_err(UnsubscribeError::UnexpectedError)?;
            if let ExecutionOutcome::EmptyQueue = outcome {
                break;
            }
        }
    }

    Ok(HttpResponse::Ok()
        .content_type(ContentType::html())
        .body(notice_page(
            &base_url.0,
            "已经退订",
            "你不会再收到我们的邮件了。如果这是误操作，重新在订阅页提交一次即可。",
        )))
}

/// 拼出退订端点 URL（worker 和 subscribe 共用，保证两边地址一致）。
///
/// ⚠️ 指向 POST 端点：RFC 8058 的客户端发的是 POST，打到 GET 上会 405。
/// token 不 percent-encode，前提是 generate_token() 只产出 alnum —— 用
/// debug_assert 钉住这个前提，字符集一变得立刻炸，而不是等用户点到"链接无效"。
pub fn unsubscribe_url(base_url: &str, unsubscription_token: &str) -> String {
    debug_assert!(
        unsubscription_token.chars().all(|c| c.is_ascii_alphanumeric()),
        "unsubscription token 含需要 URL 转义的字符，拼进 query string 会坏掉: {unsubscription_token}"
    );
    format!(
        "{}/subscriptions/unsubscribe?unsubscription_token={}",
        base_url, unsubscription_token
    )
}
#[derive(thiserror::Error)]
pub enum UnsubscribeError {
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}

impl std::fmt::Debug for UnsubscribeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // 复用 utils 的 error chain 格式化，保证日志结构一致。
        error_chain_fmt(self, f)
    }
}

impl actix_web::ResponseError for UnsubscribeError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
    }
}
