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
/// ⚠️ 这个 handler 只【渲染页面】，一个字节都不改。
///
/// 为什么这条规则不能破：Gmail / Outlook 在展示邮件时会【预先抓取】邮件里的链接
/// （防钓鱼扫描、生成预览、决定要不要显示原生"退订"按钮）。
/// 如果 GET 里直接执行退订，用户还没看到邮件就已经被退订了 ——
/// 而且他永远不会知道为什么收不到信。
///
/// 所以：GET 看状态 → 渲染；POST（RFC 8058 的一键退订就发 POST）→ 执行。
///
/// 三个分支【都返回 200】：
///   token 不存在  → "链接无效"
///   已退订        → "你已经退订过了"
///   活跃          → "确认退订" 按钮
///
/// 为什么不给 404/410：HTTP 状态码描述的是【传输/协议层】的结果，不是业务结果。
/// 给 4xx 会让邮件客户端在收件箱里挂一个错误图标，用户看到的也是浏览器默认错误页，
/// 完全没有上下文。链接失效是一种【页面内容】。
pub async fn unsubscribe_form(
    parameters: web::Query<Parameters>,
    pool: web::Data<PgPool>,
    base_url: web::Data<ApplicationBaseUrl>,
) -> Result<HttpResponse, actix_web::Error> {
    let identity = find_subscriber_by_unsubscription_token(&pool, &parameters.unsubscription_token)
        .await
        .map_err(e500)?;

    let body = match identity {
        // token 对不上任何订阅者。可能是被转发/被截断的链接，也可能是伪造的。
        None => notice_page(
            &base_url.0,
            "链接无效",
            "这个退订链接我们认不出来。可能是链接被截断了，或者这个地址已经不在了。",
        ),
        // 已经退订过：不用再给按钮，给按钮反而会让人以为上次没成功。
        Some(who) if !who.status.is_active() => notice_page(
            &base_url.0,
            "你已经退订了",
            "这个地址已经不在我们的发信名单里，不会再收到任何邮件。",
        ),
        // 仍然活跃：渲染确认页。
        //
        // ⚠️ token 放在 form 的 action 里（query string），不放 hidden input。两个原因：
        //   1. 于是"人点按钮"和"邮件客户端一键退订"发出来的是同一个请求形状：
        //        POST /subscriptions/unsubscribe?unsubscription_token=xxx
        //      区别只有 body 里多不多一个 List-Unsubscribe=One-Click。
        //      POST handler 因此【只读 query 就够】，不用解析 body。
        //   2. 不用赌客户端带什么 Content-Type、body 长什么样。
        //      （Gmail 发的 body 不是我们定义的表单结构，用 web::Form 解析会 400。）
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

/// 提示页。POST 分支也复用它 —— 那里同样三种情况都要一个页面。
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

/// 只显示首字母 + 域名：确认页上不该把完整邮箱印出来
/// （这个页面可能出现在公共电脑、也可能被截图），但用户得能认出是不是自己。
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
        // 理论上不会发生（email 过过校验），保底不做假设。
        None => "***".to_string(),
    }
}
