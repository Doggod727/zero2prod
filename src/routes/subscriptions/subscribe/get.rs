//! src/routes/subscriptions/form.rs
use actix_web::http::header::ContentType;
use actix_web::HttpResponse;
use actix_web_flash_messages::IncomingFlashMessages;
use std::fmt::Write;

/// GET /subscriptions —— 渲染订阅表单。
///
/// Flash 消息存在 cookie 里、由「下一个请求」读出来渲染，
/// 所以 subscribe 结束（成功或已订阅）后都 303 跳到这里，用户才有机会看到提示。
pub async fn subscription_form(
    flash_messages: IncomingFlashMessages,
) -> Result<HttpResponse, actix_web::Error> {
    let mut msg_html = String::new();
    for m in flash_messages.iter() {
        writeln!(msg_html, "<p><i>{}</i></p>", m.content()).unwrap();
    }

    Ok(HttpResponse::Ok()
        .content_type(ContentType::html())
        .body(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta http-equiv="content-type" content="text/html; charset=utf-8">
    <title>Subscribe to our newsletter</title>
</head>
<body>
    {msg_html}
    <form action="/subscriptions" method="post">
        <label>Name
            <input
                type="text"
                placeholder="Enter your name"
                name="name"
            >
        </label>
        <br>
        <label>Email
            <input
                type="email"
                placeholder="Enter your email"
                name="email"
            >
        </label>
        <br>
        <button type="submit">Subscribe</button>
    </form>
</body>
</html>"#)))
}