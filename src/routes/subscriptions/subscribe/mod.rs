//! src/routes/subscriptions/subscribe/mod.rs
mod get;
mod persistence;
mod post;

pub use get::subscription_form;
pub use persistence::{error_chain_fmt, StoreTokenError};
pub use post::{send_confirmation_email, subscribe, SubscriberError};