//! src/email_client.rs

use reqwest::{Client, Url};
use secrecy::{ExposeSecret, Secret};
use serde::Serialize;
use crate::domain::SubscriberEmail;

pub struct EmailClient {
    http_client: Client,
    base_url: String, // 用于存储发出api请求的URL
    sender: SubscriberEmail, // 发送方的邮件地址
    authorization_token: Secret<String>,
    
}

impl EmailClient {
    pub fn new(base_url: String, sender: SubscriberEmail, authorization_token: Secret<String>, timeout: std::time::Duration) -> EmailClient {
        let http_client = Client::builder()
            .timeout(timeout)
            .build()
            .unwrap();
        Self {
            http_client,
            base_url,
            sender,
            authorization_token,
        }
    }
    pub async fn send_email(
        &self,
        recipient: SubscriberEmail,
        subject: &str,
        html_content: &str,
        text_content: &str
    ) -> Result<(), reqwest::Error> {
        // reqwest_url是访问的默认base_url，也就是我们需要访问服务器资源的URL
        let url = Url::parse(&self.base_url).expect("Failed to parse base url")
            .join("/email").expect("Failed to parse endpoint");
        let request_body = SendEmailRequest {
            from: self.sender.as_ref(),
            to: recipient.as_ref(),
            subject,
            html_body: html_content,
            text_body: text_content
        };
         self.http_client
             .post(url)
             .header("X-Postmark-Server-Token", self.authorization_token.expose_secret())
             .json(&request_body)
             .send()
             .await?
             .error_for_status()?;// 调用Json方法要求请求体是可串行化的。 
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
}

#[cfg(test)]
mod tests {
    use claim::{assert_err, assert_ok};
    use fake::{Fake, Faker};
    use fake::faker::internet::en::SafeEmail;
    use fake::faker::lorem::en::{Paragraph, Sentence};
    use secrecy::Secret;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use crate::domain::SubscriberEmail;
    use crate::email_client::EmailClient;
    use wiremock::matchers::{any, header, header_exists, method, path};
    use wiremock::Request;

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
        EmailClient::new(base_url, email(), Secret::new(Faker.fake()), std::time::Duration::from_millis(200))
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
        let _ = email_client.send_email(email() ,&subject(), &content(), &content())
            .await;
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
        let outcome = email_client.send_email(email(), &subject(), &content(), &content())
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
        let outcome = email_client.send_email(email(), &subject(), &content(), &content())
            .await;
        assert_err!(outcome);
    }
    
    #[tokio::test]
    async fn send_email_times_out_if_the_server_takes_too_long() {
        // 测试超时时，send_email失败，即使服务器确实响应了
        let mock_server = MockServer::start().await; // 等待mock服务器启动
        let email_client = email_client(mock_server.uri());
        
        
        let response = ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_secs(180));
        Mock::given(any())
            .respond_with(response)
            .mount(&mock_server)
            .await;
        
        let outcome = email_client.send_email(email(), &subject(), &content(), &content())
            .await;
        assert_err!(outcome);
    }
}