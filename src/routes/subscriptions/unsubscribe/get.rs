//! src/routes/subscriptions/unsubscribe/get.rs
use super::persistence::find_subscriber_by_unsubscription_token;
use crate::startup::ApplicationBaseUrl;
use crate::utils::e500;
use actix_web::http::header::ContentType;
use actix_web::{web, HttpResponse};
use sqlx::PgPool;

#[derive(serde::Deserialize)]
pub struct Parameters {
    pub unsubscription_token: String,
}

/// GET /subscriptions/unsubscribe?unsubscription_token=...
///
/// ⚠️ 只渲染页面，一个字节都不改：邮件客户端（Gmail / Outlook）在展示邮件时会
///    预先抓取邮件里的链接，GET 里执行退订就等于"用户还没看到邮件就被退订了"。
///
/// 三个分支都返回 200（含 token 无效）—— 链接失效是一种【页面内容】，
/// 不是协议错误；给 4xx 会让客户端在收件箱里挂错误图标，
/// 用户看到的也只是浏览器默认错误页。
pub async fn unsubscribe_form(
    parameters: web::Query<Parameters>,
    pool: web::Data<PgPool>,
    base_url: web::Data<ApplicationBaseUrl>,
) -> Result<HttpResponse, actix_web::Error> {
    let identity = find_subscriber_by_unsubscription_token(&pool, &parameters.unsubscription_token)
        .await
        .map_err(e500)?;

    let body = match identity {
        // token 对不上任何订阅者：可能是被截断/转发的链接，也可能是伪造的。
        None => notice_page(
            &base_url.0,
            "链接无效",
            "这个退订链接我们认不出来。可能是链接被截断了，或者这个地址已经不在了。",
        ),
        // 已退订：不再给按钮，否则会让人以为上次没成功。
        Some(who) if !who.status.is_active() => notice_page(
            &base_url.0,
            "你已经退订了",
            "这个地址已经不在我们的发信名单里，不会再收到任何邮件。",
        ),
        // ⚠️ token 放在 form 的 action（query string）里，不放 hidden input：
        //    这样"人点按钮"和"邮件客户端一键退订"是同一个请求形状，
        //    POST handler 只读 query 就够，不用赌客户端带什么 Content-Type、body 长什么样。
        Some(who) => {
            let token = &parameters.unsubscription_token;
            let masked = mask_email(&who.email);
            format!(
                r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta http-equiv="content-type" content="text/html; charset=utf-8">
    <title>Unsubscribe</title>
</head>
<body>
    <h1>Unsubscribe</h1>
    <p>确定要让 <strong>{masked}</strong> 不再收到我们的邮件吗？</p>
    <form action="/subscriptions/unsubscribe?unsubscription_token={token}" method="post">
        <button type="submit">确认退订</button>
    </form>
</body>
</html>"#
            )
        }
    };

    Ok(HttpResponse::Ok()
        .content_type(ContentType::html())
        .body(body))
}

/// 提示页。POST 分支也复用它。
pub(super) fn notice_page(base_url: &str, heading: &str, detail: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta http-equiv="content-type" content="text/html; charset=utf-8">
    <title>{heading}</title>
</head>
<body>
    <h1>{heading}</h1>
    <p>{detail}</p>
    <p><a href="{base_url}/subscriptions">回到订阅页</a></p>
</body>
</html>"#
    )
}

/// 首字母 + 域名：确认页可能出现在公共电脑或被截图，但用户得能认出是不是自己。
fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            format!("{first}***@{domain}")
        }
        // 理论上不会发生（email 过了校验）。
        None => "***".to_string(),
    }
}
