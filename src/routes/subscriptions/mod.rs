//! src/routes/subscriptions/mod.rs
//!
//! 订阅生命周期的 HTTP 层。组织方式对齐 `routes/admin/`：
//! 一个动作一个文件夹，每个文件夹有自己的 mod.rs 做声明与 re-export。
//!
//! - `subscribe/`   GET|POST /subscriptions          订阅表单 + 提交处理
//! - `confirm/`     GET      /subscriptions/confirm  确认订阅
//! - `unsubscribe/` GET|POST /subscriptions/unsubscribe  退订（待实现）
mod confirm;
pub mod subscribe;
mod unsubscribe;

pub use confirm::confirm;
pub use subscribe::{error_chain_fmt, subscription_form, subscribe, StoreTokenError};