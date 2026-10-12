//! src/routes/subscriptions/unsubscribe/mod.rs
//!
//! GET 只渲染确认页（邮件客户端会预取链接，不能改状态），POST 才执行。
//! 设计取舍见 persistence.rs。
mod get;
mod persistence;
mod post;

// Parameters 给 post.rs 用（super::Parameters）。
pub use get::{unsubscribe_form, Parameters};
pub use post::{unsubscribe, unsubscribe_url, UnsubscribeError};

// 给 subscribe 用：退订 token 必须在订阅时就建好，否则第一封确认邮件里的
// List-Unsubscribe 链接是死的。
pub use persistence::{find_subscriber_by_unsubscription_token, store_unsubscription_token};
