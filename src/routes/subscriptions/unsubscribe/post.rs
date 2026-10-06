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
/// 两种调用方，请求形状是一样的：
///   人点确认页上的按钮 → POST，空 body
///   邮件客户端一键退订 → POST，body 是 `List-Unsubscribe=One-Click`（RFC 8058）
///
/// 所以这里【不解析 body】：token 一律从 query 里读。
/// 这同时躲开了两个坑：客户端带不带 Content-Type、以及
/// `List-Unsubscribe=One-Click` 用 web::Form 反序列化会不会 400。
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
        // 认不出来的 token：不 4xx，也不假装成功。给一个提示页。
        //
        // 这里【故意】不 Flash + 303：带着提示跳到订阅页再渲染一次，
        // 页面上的上下文（"你点的那个链接"）就丢了。
        FlashMessage::info("We could not recognise that unsubscription link.").send();
        return Ok(HttpResponse::Ok()
            .content_type(ContentType::html())
            .body(notice_page(
                &base_url.0,
                "链接无效",
                "这个退订链接我们认不出来。可能是链接被截断了，或者这个地址已经不在了。",
            )));
    };

    // 已经是 unsubscribed：幂等，什么都不做，也【不重发】通知。
    // 用户可能反复点旧邮件里的链接，每点一次发一封信就成了骚扰。
    if !who.status.is_active() {
        return Ok(HttpResponse::Ok()
            .content_type(ContentType::html())
            .body(notice_page(
                &base_url.0,
                "你已经退订了",
                "这个地址已经不在我们的发信名单里，不会再收到任何邮件。",
            )));
    }

    // 三张表的写入放进同一个事务：
    //   subscriptions        改状态
    //   unsubscription_tokens 确保 token 存在
    //   email_delivery_queue  删掉待发 newsletter + 入队退订通知
    //
    // ⚠️ 顺序无所谓，但【必须在同一个事务里】：enqueue 里那步 DELETE
    //    和 INSERT 之间如果被别的会话插进来，会撞部分唯一索引；
    //    而且"改了状态但通知没入队"和"通知入了队但状态没改"都不该出现。
    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;

    let changed = mark_as_unsubscribed(&mut transaction, who.id)
        .await
        .context("Failed to mark the subscriber as unsubscribed")?;

    // changed == false 意味着在我们读到 who 和这句 UPDATE 之间，
    // 另一个请求已经把他退订了。不报错、不重发通知，当作幂等处理。
    if changed {
        let token_in_db = store_unsubscription_token(&mut transaction, who.id, generate_token())
            .await
            .context("Failed to store the unsubscription token")?;

        // ⚠️ 用 token_in_db，不要用 generate_token() 的返回值。
        //    store_unsubscription_token 在冲突时不轮换，库里可能是更早的那个值；
        //    用自己那个拼链接，用户点下去会得到"链接无效"。
        enqueue_unsubscription_notice(&mut transaction, &who.email, &token_in_db)
            .await
            .context("Failed to enqueue the unsubscription notice")?;
    }

    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to unsubscribe a subscriber")?;

    // 事务之外，尽力把队列里能发的发掉 —— 让用户立刻收到那封通知。
    // 发不出去就留在队列里由后台 worker 重试（和 subscribe / publish_newsletter 一致）。
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

/// 拼出退订端点 URL。worker 和 subscribe 都用它，保证两边生成的是同一个地址。
///
/// ⚠️ 指向的是 POST 端点，不是 GET 那个确认页：
///    RFC 8058 规定客户端会发 `POST` + body `List-Unsubscribe=One-Click`，
///    打到 GET 上会 405。
///
/// token 直接拼进 query string 而不做 percent-encode —— 这是【有前提的】：
/// generate_token() 只产出 alnum，不需要转义。用 debug_assert 把这个前提钉住：
/// 哪天有人把 token 换成别的字符集，debug 构建下会立刻炸，
/// 而不是等用户点出一条"链接无效"。
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
        // 复用 subscribe 那边的 error chain 格式化，保证日志结构一致。
        error_chain_fmt(self, f)
    }
}

impl actix_web::ResponseError for UnsubscribeError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
    }
}
