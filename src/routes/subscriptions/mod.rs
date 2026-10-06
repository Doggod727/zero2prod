//! src/routes/subscriptions/mod.rs
//!
//! 订阅生命周期的 HTTP 层。组织方式对齐 `routes/admin/`：
//! 一个动作一个文件夹，每个文件夹有自己的 mod.rs 做声明与 re-export。
//!
//! - `subscribe/`   GET|POST /subscriptions          订阅表单 + 提交处理
//! - `confirm/`     GET      /subscriptions/confirm  确认订阅
//! - `unsubscribe/` GET|POST /subscriptions/unsubscribe  退订（GET 只渲染，POST 才执行）
mod confirm;
pub mod subscribe;
// pub 是必须的：worker 在 crate 的另一处，要能取到拼退订链接的函数。
// 只 export 那一个函数，persistence 仍然关在里面。
pub mod unsubscribe;

pub use confirm::confirm;
pub use subscribe::{subscribe, subscription_form, StoreTokenError};
pub use unsubscribe::{unsubscribe, unsubscribe_form, unsubscribe_url, UnsubscribeError};
