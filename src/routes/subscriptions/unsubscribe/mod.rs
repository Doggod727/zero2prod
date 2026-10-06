//! src/routes/subscriptions/unsubscribe/mod.rs
//!
//! - `GET  /subscriptions/unsubscribe?unsubscription_token=...` 只渲染确认页，不改状态
//! - `POST /subscriptions/unsubscribe?unsubscription_token=...` 执行退订
//!   （RFC 8058 一键退订发的也是 POST，而且只带 query 参数）
//!
//! 各函数的设计取舍见 persistence.rs。
mod get;
mod persistence;
mod post;

pub use get::{unsubscribe_form, Parameters};
pub use post::{unsubscribe, unsubscribe_url, UnsubscribeError};

// 下面这个不是给 HTTP 层用的，是给 subscribe 调用的：
// 退订 token 必须在【订阅时】就建好，否则第一封确认邮件里的
// List-Unsubscribe 链接必然是死的。
pub use persistence::{find_subscriber_by_unsubscription_token, store_unsubscription_token};
