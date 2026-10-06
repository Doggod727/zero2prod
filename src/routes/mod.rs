//! src/routes/mod.rs
mod health_check;
mod home;
mod login;
// pub 是必须的：issue_delivery_worker 要用 subscriptions::unsubscribe_url
// 拼退订链接。这个模块只 export 少量东西（见 subscriptions/mod.rs），
// 其余（各 handler 的实现细节）仍然关在里面。
mod admin;
pub mod subscriptions;

pub use admin::*;
pub use health_check::*;
pub use home::*;
pub use login::*;
pub use subscriptions::*;
