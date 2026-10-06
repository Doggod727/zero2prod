//! src/email_client.rs

use crate::domain::SubscriberEmail;
use reqwest::{Client, Url};
use secrecy::{ExposeSecret, Secret};
use serde::Serialize;

/// Postmark 的邮件头是 JSON body 里的一个数组，【不是】HTTP 请求头。
/// 名字必须和 Postmark API 一致（PascalCase 的 Name/Value）。
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmailHeader {
    pub name: String,
    pub value: String,
}

pub struct EmailClient {
    http_client: Client,
    base_url: String,        // 用于存储发出api请求的URL
    sender: SubscriberEmail, // 发送方的邮件地址
    authorization_token: Secret<String>,
}

impl EmailClient {
    pub fn new(
        base_url: String,
        sender: SubscriberEmail,
        authorization_token: Secret<String>,
        timeout: std::time::Duration,
    ) -> EmailClient {
        let http_client = Client::builder().timeout(timeout).build().unwrap();
        Self {
            http_client,
            base_url,
            sender,
            authorization_token,
        }
    }

    /// RFC 8058 一键退订需要的两个头。
    ///
    /// ⚠️ 它必须被塞进 JSON body 的 Headers 数组里。如果当成 HTTP 请求头
    ///    （.header("List-Unsubscribe", ...)）发出去，Postmark 会直接忽略它 ——
    ///    邮件发出去了、没有任何报错、但收件箱里不会出现原生"退订"按钮。
    ///    这是最典型的"合规静默失效"：本地 mock 测得过，真发出去不生效。
    ///
    /// 尖括号不能省：RFC 8058 规定 URI 要包在 <> 里。
    /// List-Unsubscribe-Post 是客户端发 POST 的授权声明，只写它等于告诉客户端
    /// "可以一键退订"，所以那个 POST 端点必须真的存在且不需要二次确认。
    ///
    /// 注意：RFC 8058 要求这个 URL 走 HTTPS，防止中间人篡改退订地址。
    /// 本地开发是 http，上生产必须换成 https。
    pub fn list_unsubscribe_headers(&self, unsubscribe_url: &str) -> Vec<EmailHeader> {
        vec![
            EmailHeader {
                name: "List-Unsubscribe".to_string(),
                value: format!("<{unsubscribe_url}>"),
            },
            EmailHeader {
                name: "List-Unsubscribe-Post".to_string(),
                value: "List-Unsubscribe=One-Click".to_string(),
            },
        ]
    }

    pub async fn send_email(
        &self,
        recipient: &SubscriberEmail,
        subject: &str,
        html_content: &str,
        text_content: &str,
        // 空 slice 表示这封信不带自定义头。调用方用 &[] 就行。
        headers: &[EmailHeader],
    ) -> Result<(), reqwest::Error> {
        // reqwest_url是访问的默认base_url，也就是我们需要访问服务器资源的URL
        let url = Url::parse(&self.base_url)
            .expect("Failed to parse base url")
            .join("/email")
            .expect("Failed to parse endpoint");
        let request_body = SendEmailRequest {
            from: self.sender.as_ref(),
            to: recipient.as_ref(),
            subject,
            html_body: html_content,
            text_body: text_content,
            headers,
        };
        self.http_client
            .post(url)
            .header(
                "X-Postmark-Server-Token",
                self.authorization_token.expose_secret(),
            )
            .json(&request_body)
            .send()
            .await?
            .error_for_status()?; // 调用Json方法要求请求体是可串行化的。
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")] // 对所有字段重新进行帕斯卡命名。
struct SendEmailRequest<'a> {
    from: &'a str,
    to: &'a str,
    subject: &'a str,
    html_body: &'a str,
    text_body: &'a str,
    /// 即使为空也要发出去（空数组），让请求形状保持稳定、便于 mock 断言。
    headers: &'a [EmailHeader],
}

#[cfg(test)]
mod tests {
    use crate::domain::SubscriberEmail;
    use crate::email_client::EmailClient;
    use claim::{assert_err, assert_ok};
    use fake::faker::internet::en::SafeEmail;
    use fake::faker::lorem::en::{Paragraph, Sentence};
    use fake::{Fake, Faker};
    use secrecy::Secret;
    use wiremock::matchers::{any, header, header_exists, method, path};
    use wiremock::Request;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct SendEmailBodyMatcher;

    impl wiremock::Match for SendEmailBodyMatcher {
        fn matches(&self, request: &Request) -> bool {
            // 尝试将body解析为json
            let result: Result<serde_json::Value, _> = serde_json::from_slice(&request.body);
            if let Ok(body) = result {
                body.get("From").is_some()
                    && body.get("To").is_some()
                    && body.get("Subject").is_some()
                    && body.get("HtmlBody").is_some()
                    && body.get("TextBody").is_some()
                    // ⚠️ Headers 必须是【数组】。如果哪天有人把它挪回 HTTP 请求头，
                    //    或者序列化成了对象，这个断言会失败 —— 那正是我们要的告警。
                    && body.get("Headers").map(|h| h.is_array()).unwrap_or(false)
            } else {
                false
            }
        }
    }

    /// 生成随机的邮件主题
    fn subject() -> String {
        Sentence(1..2).fake()
    }
    /// 生成随机的邮件内容
    fn content() -> String {
        Paragraph(1..10).fake()
    }
    /// 生成随机的订阅者电子邮件
    fn email() -> SubscriberEmail {
        SubscriberEmail::parse(SafeEmail().fake()).unwrap()
    }
    /// 获取'EmailClient'的实例
    fn email_client(base_url: String) -> EmailClient {
        EmailClient::new(
            base_url,
            email(),
            Secret::new(Faker.fake()),
            std::time::Duration::from_millis(200),
        )
    }
    #[tokio::test]
    async fn send_email_sends_the_expected_request() {
        // 期望一个请求发送到EmailClient::base_url的服务器。
        let mock_server = MockServer::start().await; // 等待mock服务器启动
        let email_client = email_client(mock_server.uri());

        // 该mock接受http请求，如果其请求头包含token字段
        Mock::given(header_exists("X-Postmark-Server-Token"))
            .and(header("Content-Type", "application/json"))
            .and(path("/email"))
            .and(method("POST"))
            .and(SendEmailBodyMatcher)
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        // 执行
        let _ = email_client
            .send_email(&email(), &subject(), &content(), &content(), &[])
            .await;
    }

    /// List-Unsubscribe 头必须【放在 body 的 Headers 数组里】，而不是 HTTP 请求头里。
    /// 这个测试用"反例断言"把这件事钉死：如果谁改回 HTTP 头，它会失败。
    #[tokio::test]
    async fn list_unsubscribe_headers_go_into_the_json_body() {
        let mock_server = MockServer::start().await;
        let email_client = email_client(mock_server.uri());

        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let headers = email_client.list_unsubscribe_headers(
            "https://example.com/subscriptions/unsubscribe?unsubscription_token=abc",
        );
        email_client
            .send_email(&email(), &subject(), &content(), &content(), &headers)
            .await
            .expect("send_email failed");

        let request = &mock_server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();

        let sent = body["Headers"].as_array().expect("Headers 不是数组");
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["Name"], "List-Unsubscribe");
        assert_eq!(
            sent[0]["Value"],
            "<https://example.com/subscriptions/unsubscribe?unsubscription_token=abc>"
        );
        assert_eq!(sent[1]["Name"], "List-Unsubscribe-Post");
        assert_eq!(sent[1]["Value"], "List-Unsubscribe=One-Click");

        // 反例：HTTP 请求头里【不该】出现它。
        //
        // 这里用遍历而不是 headers.get(...)：wiremock 用的 http crate 版本
        // 和 reqwest 不是同一个（两个不同的 HeaderName 类型），
        // 直接构造名字得跨 crate 转换，遍历最省事也最清楚。
        let sent_as_http_header = request
            .headers
            .iter()
            .any(|(name, _)| name.as_str().eq_ignore_ascii_case("list-unsubscribe"));
        assert!(
            !sent_as_http_header,
            "List-Unsubscribe 被当成 HTTP 头发了出去，Postmark 会忽略它"
        );
    }

    #[tokio::test]
    async fn send_email_succeeds_if_the_server_returns_200() {
        // 测试我们发送一个正确的HTTP请求后，服务器会返回200 Ok
        // 然后说明我们的邮件发送正确。
        let mock_server = MockServer::start().await; // 等待mock服务器启动
        let email_client = email_client(mock_server.uri());

        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        // 执行
        let outcome = email_client
            .send_email(&email(), &subject(), &content(), &content(), &[])
            .await;
        assert_ok!(outcome);
    }

    #[tokio::test]
    async fn send_email_fails_if_the_server_returns_500() {
        // 测试我们发送一个正确的HTTP请求后，服务器会返回200 Ok
        // 然后说明我们的邮件发送正确。
        let mock_server = MockServer::start().await; // 等待mock服务器启动
        let email_client = email_client(mock_server.uri());

        Mock::given(any())
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&mock_server)
            .await;

        // 执行
        let outcome = email_client
            .send_email(&email(), &subject(), &content(), &content(), &[])
            .await;
        assert_err!(outcome);
    }

    #[tokio::test]
    async fn send_email_times_out_if_the_server_takes_too_long() {
        // 测试超时时，send_email失败，即使服务器确实响应了
        let mock_server = MockServer::start().await; // 等待mock服务器启动
        let email_client = email_client(mock_server.uri());

        let response = ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(180));
        Mock::given(any())
            .respond_with(response)
            .mount(&mock_server)
            .await;

        let outcome = email_client
            .send_email(&email(), &subject(), &content(), &content(), &[])
            .await;
        assert_err!(outcome);
    }
}
