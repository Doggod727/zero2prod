//! src/routes/subscriptions/mod.rs
//!
//! - `subscribe/`   GET|POST /subscriptions
//! - `confirm/`     GET      /subscriptions/confirm
//! - `unsubscribe/` GET|POST /subscriptions/unsubscribe
mod confirm;
pub mod subscribe;
// pub：worker 需要 unsubscribe_url，其余仍关在模块内。
pub mod unsubscribe;

pub use confirm::confirm;
pub use subscribe::{subscribe, subscription_form, StoreTokenError};
pub use unsubscribe::{unsubscribe, unsubscribe_form, unsubscribe_url, UnsubscribeError};
